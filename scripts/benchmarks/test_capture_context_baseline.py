"""Replay records retain the exact source-inspection arguments and outputs."""
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest


SOURCE = Path(__file__).resolve().with_name('capture_context_baseline.py')


class BaselineCaptureTests(unittest.TestCase):
    def test_replay_preserves_literal_arguments_errors_and_continues(self):
        spec = importlib.util.spec_from_file_location('capture_context_baseline', SOURCE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            name = 'source $(touch unexpected).ts'
            (root / name).write_text('alpha beta\n', encoding='utf-8')
            commands = [
                ['rg', '-n', r'\balpha\b', name],
                ['rg', '-n', 'absent', name],
                ['cat', 'missing.ts'],
                ['cat', name],
            ]
            records = module.capture_case(root, {'root': '.', 'commands': commands})
            self.assertEqual([row['argv'] for row in records], commands)
            self.assertEqual([row['exit_code'] for row in records], [0, 1, 1, 0])
            self.assertEqual(records[0]['stdout'], '1:alpha beta\n')
            self.assertEqual(records[1]['stdout'], '')
            self.assertIn('missing.ts', records[2]['stderr'])
            self.assertEqual(records[3]['stdout'], 'alpha beta\n')
            self.assertEqual(records[0]['command'], shlex.join(commands[0]))
            self.assertEqual(records[0]['cwd'], str(root.resolve()))
            self.assertFalse((root / 'unexpected').exists())

    def test_case_flag_writes_only_selected_transcript(self):
        with tempfile.TemporaryDirectory() as folder:
            result = subprocess.run(
                [sys.executable, str(SOURCE), '--case', 'ts-cyclic', '--output', folder],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            paths = sorted(path.name for path in Path(folder).iterdir())
            self.assertEqual(paths, ['ts-cyclic.json'])
            records = json.loads((Path(folder) / paths[0]).read_text())
            self.assertEqual(records[1]['argv'], ['nl', '-ba', 'src/a/alpha.ts'])
            self.assertIn('import { beta }', records[1]['stdout'])
            self.assertEqual([row['index'] for row in records], list(range(1, len(records) + 1)))


if __name__ == '__main__':
    unittest.main()
