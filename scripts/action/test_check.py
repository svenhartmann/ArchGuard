"""Runs check.sh and review.sh with a fake archguard binary; no network."""
import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE / 'check.sh'
REVIEW = HERE / 'review.sh'

# Prints its arguments as the "report" and exits with $FAKE_EXIT.
FAKE_ARCHGUARD = textwrap.dedent('''\
    #!/bin/sh
    echo "$*" >> "$FAKE_LOG"
    case "$*" in
        *pr-comment*)
            while [ $# -gt 0 ]; do
                if [ "$1" = --out ]; then printf '## review of %s\\n' "$FAKE_LABEL" > "$2"; fi
                shift
            done ;;
        *) echo '{"version":"2.1.0","runs":[]}' ;;
    esac
    exit "${FAKE_EXIT:-0}"
''')


def git(cwd, *args):
    subprocess.run(['git', '-C', str(cwd), *args], check=True, capture_output=True,
                   env=dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull,
                            GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='t',
                            GIT_AUTHOR_EMAIL='t@example.invalid', GIT_COMMITTER_NAME='t',
                            GIT_COMMITTER_EMAIL='t@example.invalid'))


class FakeBinary(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.binary = self.root / 'archguard'
        self.binary.write_text(FAKE_ARCHGUARD)
        self.binary.chmod(0o755)
        self.output = self.root / 'github_output'
        self.output.write_text('')
        self.log = self.root / 'calls.log'
        self.env = dict(os.environ, ARCHGUARD_BIN=str(self.binary), FAKE_LOG=str(self.log),
                        GITHUB_OUTPUT=str(self.output), FAKE_LABEL='pr')
        self.env.pop('GITHUB_STEP_SUMMARY', None)

    def outputs(self):
        return dict(line.split('=', 1) for line in self.output.read_text().splitlines())


class CheckScriptTest(FakeBinary):
    def run_check(self, exit_code, fail_on_violation='true'):
        self.sarif = self.root / 'out' / 'archguard.sarif'
        env = dict(self.env, FAKE_EXIT=str(exit_code), ARCHGUARD_PATH='.',
                   ARCHGUARD_SARIF=str(self.sarif),
                   ARCHGUARD_FAIL_ON_VIOLATION=fail_on_violation)
        run = subprocess.run(['sh', str(CHECK), 'run'], env=env, capture_output=True, text=True)
        return run, env

    def gate(self, env):
        return subprocess.run(['sh', str(CHECK), 'gate'], env=env, capture_output=True,
                              text=True)

    def test_clean_run_writes_sarif_and_passes_gate(self):
        run, env = self.run_check(0)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertIn('"runs"', self.sarif.read_text())
        self.assertEqual(self.outputs()['exit-code'], '0')
        self.assertEqual(self.outputs()['sarif-file'], str(self.sarif))
        self.assertIn('check . --format sarif', self.log.read_text())
        self.assertEqual(self.gate(dict(env, ARCHGUARD_EXIT_CODE='0')).returncode, 0)

    def test_violation_is_recorded_then_fails_the_gate(self):
        run, env = self.run_check(1)
        self.assertEqual(run.returncode, 0, 'run defers exit 1 so the comment can post')
        self.assertEqual(self.outputs()['exit-code'], '1')
        self.assertTrue(self.sarif.exists())
        gate = self.gate(dict(env, ARCHGUARD_EXIT_CODE='1'))
        self.assertEqual(gate.returncode, 1)
        self.assertIn('::error', gate.stdout)

    def test_violation_only_warns_when_fail_on_violation_is_false(self):
        run, env = self.run_check(1, 'false')
        self.assertEqual(run.returncode, 0)
        gate = self.gate(dict(env, ARCHGUARD_EXIT_CODE='1'))
        self.assertEqual(gate.returncode, 0)
        self.assertIn('::warning', gate.stdout)

    def test_usage_and_analysis_errors_fail_immediately(self):
        for code in (2, 3, 101):
            for fail in ('true', 'false'):
                with self.subTest(code=code, fail=fail):
                    run, _ = self.run_check(code, fail)
                    self.assertEqual(run.returncode, code)
                    self.assertIn('::error', run.stdout)
                    self.assertEqual(self.outputs()['exit-code'], str(code))

    def test_gate_never_passes_errors_even_if_run_was_bypassed(self):
        _, env = self.run_check(0, 'false')
        self.assertEqual(self.gate(dict(env, ARCHGUARD_EXIT_CODE='3')).returncode, 3)

    def test_invalid_fail_on_violation_is_a_usage_error(self):
        run, _ = self.run_check(0, 'yes')
        self.assertEqual(run.returncode, 2)
        self.assertIn('fail-on-violation', run.stdout)
        self.assertFalse(self.log.exists(), 'must not start the analysis')


class ReviewScriptTest(FakeBinary):
    def setUp(self):
        super().setUp()
        origin = self.root / 'origin'
        origin.mkdir()
        git(origin, 'init', '--quiet')
        (origin / 'a.txt').write_text('base\n')
        git(origin, 'add', 'a.txt')
        git(origin, 'commit', '--quiet', '-m', 'base')
        self.base = subprocess.run(['git', '-C', str(origin), 'rev-parse', 'HEAD'],
                                   capture_output=True, text=True, check=True).stdout.strip()
        (origin / 'a.txt').write_text('head\n')
        git(origin, 'commit', '--quiet', '-am', 'head')
        git(origin, 'config', 'uploadpack.allowAnySHA1InWant', 'true')
        # Like actions/checkout: one commit deep, base commit not present.
        self.clone = self.root / 'clone'
        git(self.root, 'clone', '--quiet', '--depth=1', origin.as_uri(), str(self.clone))
        self.comment = self.root / 'comment.md'

    def run_review(self, exit_code=0, base=None):
        env = dict(self.env, FAKE_EXIT=str(exit_code), ARCHGUARD_PATH=str(self.clone),
                   ARCHGUARD_BASE=base or self.base, ARCHGUARD_COMMENT_FILE=str(self.comment))
        return subprocess.run(['sh', str(REVIEW)], env=env, capture_output=True, text=True)

    def test_fetches_missing_base_and_renders_comment(self):
        shallow = subprocess.run(['git', '-C', str(self.clone), 'cat-file', '-e',
                                  f'{self.base}^{{commit}}'], capture_output=True)
        self.assertNotEqual(shallow.returncode, 0, 'fixture: base must be missing')
        run = self.run_review()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn(f'--base {self.base} --format pr-comment', self.log.read_text())
        self.assertIn('## review of pr', self.comment.read_text())
        self.assertEqual(self.outputs()['comment-file'], str(self.comment))

    def test_review_errors_propagate(self):
        for code in (2, 3):
            with self.subTest(code=code):
                run = self.run_review(code)
                self.assertEqual(run.returncode, code)
                self.assertIn('::error', run.stdout)

    def test_unknown_base_fails_with_hint(self):
        run = self.run_review(base='0' * 40)
        self.assertEqual(run.returncode, 3)
        self.assertIn('fetch-depth', run.stdout)
        self.assertFalse(self.log.exists(), 'must not start the analysis')


if __name__ == '__main__':
    unittest.main()
