#!/usr/bin/env python3
"""Portable smoke test of the actual native release binary (no external compiler)."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from release_artifacts import version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='archguard release ') as folder:
        root = Path(folder)
        (root / 'lib').mkdir()
        (root / 'app.py').write_text('from lib import library\nprint(library.value)\n')
        (root / 'lib' / 'library.py').write_text('value = 42\n')
        (root / 'archguard.toml').write_text('derive_layers = false\n[project]\nlanguages = ["py"]\n')

        def run(*arguments):
            result = subprocess.run([str(binary), *map(str, arguments)], cwd=root,
                                    capture_output=True, text=True, encoding='utf-8', timeout=60)
            if result.returncode:
                raise RuntimeError(f'{arguments}: exit {result.returncode}\n{result.stderr}')
            return result.stdout

        assert run('--version').strip() == 'archguard ' + version()
        assert 'serve' in run('--help')
        checked = json.loads(run('check', '.', '--format', 'json'))
        assert checked['summary']['nodes'] >= 2, checked
        assert checked['summary']['edges'] >= 1, checked
        assert checked['findings'] == [], checked
        html = run('graph', '.', '--format', 'html')
        assert '<html' in html and 'app.py' in html and 'library.py' in html
        print('Native release smoke: version, help, Python dependency extraction, clean check and embedded HTML passed')


if __name__ == '__main__':
    main()
