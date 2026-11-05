"""Runs pr_comment.py against a fake `gh` on PATH; no network."""
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / 'pr_comment.py'
MARKER = '<!-- archguard-pr-comment -->'
BOT = {'login': 'github-actions[bot]'}
LIMIT = 65536

# A stand-in for `gh api` that keeps the PR's comments in a JSON file.
FAKE_GH = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, os, sys
    from urllib.parse import urlsplit, parse_qs
    BOT = 'github-actions[bot]'
    state_file = os.environ['FAKE_GH_STATE']
    state = json.load(open(state_file))
    args = sys.argv[1:]
    with open(state_file + '.log', 'a') as log:
        log.write(json.dumps(args) + '\\n')
    if os.environ.get('FAKE_GH_FAIL'):
        sys.stderr.write('HTTP 403: Resource not accessible by integration\\n')
        sys.exit(1)
    assert args[0] == 'api', args
    method = args[args.index('-X') + 1] if '-X' in args else 'GET'
    path = [a for a in args[1:] if a.startswith('repos/')][0]
    if method == 'GET':
        query = parse_qs(urlsplit(path).query)
        per_page, page = int(query['per_page'][0]), int(query['page'][0])
        start = (page - 1) * per_page
        print(json.dumps(state['comments'][start:start + per_page]))
        sys.exit(0)
    body = json.load(sys.stdin)['body']
    if method == 'POST':
        comment = {'id': state['next_id'], 'body': body, 'user': {'login': BOT}}
        state['next_id'] += 1
        state['comments'].append(comment)
    elif method == 'PATCH':
        comment_id = int(path.rsplit('/', 1)[1])
        comment = [c for c in state['comments'] if c['id'] == comment_id][0]
        comment['body'] = body
    json.dump(state, open(state_file, 'w'))
    print(json.dumps(comment))
''')


class PrCommentTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        bindir = self.root / 'bin'
        bindir.mkdir()
        gh = bindir / 'gh'
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.state = self.root / 'state.json'
        self.env = dict(os.environ, FAKE_GH_STATE=str(self.state),
                        PATH=f'{bindir}{os.pathsep}{os.environ["PATH"]}')
        self.body = self.root / 'comment.md'
        self.body.write_text('## ArchGuard review\n\nNo new findings.\n')

    def seed(self, comments):
        self.state.write_text(json.dumps({'comments': comments, 'next_id': 9000}))

    def comments(self):
        return json.loads(self.state.read_text())['comments']

    def calls(self):
        log = Path(str(self.state) + '.log')
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

    def run_script(self, *extra, env=None):
        args = [sys.executable, str(SCRIPT), '--repo', 'owner/name', '--pr', '7',
                '--body', str(self.body), *extra]
        return subprocess.run(args, env=env or self.env, capture_output=True, text=True)

    def test_creates_comment_with_marker_when_none_exists(self):
        self.seed([{'id': 1, 'body': 'LGTM'}])
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        created = self.comments()[-1]
        self.assertEqual(created['id'], 9000)
        self.assertTrue(created['body'].startswith(MARKER), created['body'])
        self.assertIn('No new findings.', created['body'])
        self.assertEqual(self.comments()[0], {'id': 1, 'body': 'LGTM'})
        self.assertIn('created', result.stdout)

    def test_updates_marked_comment_on_a_later_page(self):
        # 150 comments: the marker sits on page 2 of 100.
        comments = [{'id': n, 'body': f'comment {n}'} for n in range(1, 151)]
        comments[120].update(body=f'{MARKER}\nold review', user=BOT)
        self.seed(comments)
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        after = self.comments()
        self.assertEqual(len(after), 150, 'must not create a second comment')
        self.assertIn('No new findings.', after[120]['body'])
        self.assertTrue(after[120]['body'].startswith(MARKER))
        patches = [c for c in self.calls() if 'PATCH' in c]
        self.assertEqual(len(patches), 1)
        self.assertIn('repos/owner/name/issues/comments/121', patches[0])
        self.assertIn('updated', result.stdout)

    def test_running_twice_keeps_one_sticky_comment(self):
        self.seed([])
        self.assertEqual(self.run_script().returncode, 0)
        self.body.write_text('## ArchGuard review\n\nOne new cycle.\n')
        self.assertEqual(self.run_script().returncode, 0)
        marked = [c for c in self.comments() if MARKER in c['body']]
        self.assertEqual(len(marked), 1)
        self.assertIn('One new cycle.', marked[0]['body'])

    def test_body_with_marker_is_not_duplicated(self):
        self.seed([])
        self.body.write_text(f'{MARKER}\nalready marked\n')
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.comments()[0]['body'].count(MARKER), 1)

    def test_marked_comment_by_someone_else_is_left_alone(self):
        foreign = {'id': 5, 'body': f'{MARKER}\nquoted by a human', 'user': {'login': 'mallory'}}
        self.seed([foreign])
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.comments()[0], foreign)
        self.assertEqual(len(self.comments()), 2)
        self.assertIn('No new findings.', self.comments()[1]['body'])
        self.assertFalse([c for c in self.calls() if 'PATCH' in c])

    def test_oversized_body_is_truncated_with_a_note(self):
        self.seed([])
        self.body.write_text('## ArchGuard review\n' + 'x' * 80000 + '\nEND\n')
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        posted = self.comments()[0]['body']
        self.assertLess(len(posted), LIMIT)
        self.assertTrue(posted.startswith(MARKER))
        self.assertNotIn('END', posted)
        self.assertIn('truncated', posted.splitlines()[-1].lower())

    def test_gh_failure_propagates(self):
        self.seed([])
        result = self.run_script(env=dict(self.env, FAKE_GH_FAIL='1'))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('403', result.stderr)

    def test_empty_body_is_an_error(self):
        self.seed([])
        self.body.write_text('  \n')
        result = self.run_script()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.comments(), [])


if __name__ == '__main__':
    unittest.main()
