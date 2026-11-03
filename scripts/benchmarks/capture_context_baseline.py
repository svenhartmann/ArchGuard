#!/usr/bin/env python3
"""Replay fixed, read-only source inspection plans for three benchmark samples.

These are captured rg/file-read plans, not a general-purpose search agent.
Requires rg and the checked-out samples. No graph, MCP, or API is consulted.
"""
import argparse
import json
from pathlib import Path
import shlex
import subprocess


def capture_case(repository, case):
    root = (repository / case['root']).resolve()
    records = []
    for argv in case['commands']:
        result = subprocess.run(
            argv, cwd=root, capture_output=True, text=True, encoding='utf-8',
        )
        records.append({
            'index': len(records) + 1, 'cwd': str(root),
            'argv': argv, 'command': shlex.join(argv),
            'stdout': result.stdout, 'stderr': result.stderr,
            'exit_code': result.returncode,
        })
    return records


def main():
    repository = Path(__file__).resolve().parents[2]
    cases = json.loads(Path(__file__).with_name('context_benchmark_cases.json').read_text(encoding='utf-8'))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path,
                        default=repository / '.archguard/reports/context-token-benchmark/baseline')
    parser.add_argument('--case', choices=[case['id'] for case in cases], action='append')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for case in cases:
        if args.case and case['id'] not in args.case:
            continue
        records = capture_case(repository, case)
        path = args.output / (case['id'] + '.json')
        path.write_text(json.dumps(records, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
        print(f"Captured {case['id']}: {len(records)} commands -> {path}")


if __name__ == '__main__':
    main()
