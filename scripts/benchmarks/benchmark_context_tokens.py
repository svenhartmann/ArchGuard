#!/usr/bin/env python3
"""Count captured retrieval text, not billable API usage. Requires tiktoken.

Run capture_context_baseline.py and capture_context_benchmark.py first.
Inputs and results default to .archguard/reports/context-token-benchmark/.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
from pathlib import Path


def count_commands(commands, count):
    return sum(count(row[key]) for row in commands
               for key in ('command', 'stdout', 'stderr'))


def count_variant(run, target_read, count):
    if not run['complete'] or run.get('error'):
        raise ValueError('incomplete MCP capture cannot be compared')
    arguments = sum(count(row['arguments_text']) for row in run['calls'])
    responses = sum(count(row['response_text']) for row in run['calls'])
    return {'calls': len(run['calls']), 'arguments': arguments,
            'responses': responses, 'target_read': target_read,
            'total': arguments + responses + target_read}


def savings(baseline, candidate):
    return round(100 * (baseline - candidate) / baseline, 2) if baseline else None


def totals(rows, single_schema, catalog_schema):
    baseline = sum(row['baseline'] for row in rows)
    result = {'baseline': baseline}
    for name in ('default', 'large'):
        total = sum(row[name]['total'] for row in rows)
        result[name] = {'total': total, 'saving_percent': savings(baseline, total),
                        'with_single_schema': total + single_schema,
                        'with_catalog': total + catalog_schema}
    return result


def main():
    import tiktoken

    base = Path('.archguard/reports/context-token-benchmark')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, default=base / 'baseline')
    parser.add_argument('--mcp', type=Path, default=base / 'mcp')
    parser.add_argument('--output', type=Path, default=base / 'tokens.json')
    args = parser.parse_args()
    cases = json.loads(Path(__file__).with_name('context_benchmark_cases.json').read_text())
    artifacts = {}

    def read(path):
        data = path.read_bytes()
        artifacts[str(path)] = hashlib.sha256(data).hexdigest()
        return json.loads(data)

    schemas = {name: json.dumps(read(args.mcp / (name + '.json')),
                               separators=(',', ':'), ensure_ascii=False)
               for name in ('tools', 'change_context_tool')}
    result = {'measured_at': datetime.now(timezone.utc).isoformat(),
              'metric': 'retrieval text tokens; not API usage',
              'tiktoken_version': importlib.metadata.version('tiktoken'),
              'encodings': {}, 'cases': []}
    captures = []
    for case in cases:
        name = case['id']
        baseline = read(args.baseline / (name + '.json'))
        capture = read(args.mcp / (name + '.json'))
        if not capture['same_items']:
            raise ValueError(f'{name}: variants do not contain the same items')
        if capture['target'] != case['target']:
            raise ValueError(f'{name}: different target in capture')
        followup_path = args.baseline / (name + '-mcp-followup.json')
        followup = read(followup_path) if followup_path.is_file() else []
        captures.append((case, baseline, capture, followup))
        result['cases'].append({'case': name, 'target': case['target'],
                                'revision': capture['default']['revision'],
                                'sources': capture['sources']})
    for encoding_name in ('o200k_base', 'cl100k_base'):
        encoding = tiktoken.get_encoding(encoding_name)

        def count(text):
            return len(encoding.encode(text, disallowed_special=()))

        rows = []
        for case, baseline, capture, followup in captures:
            source = count_commands([baseline[case['target_read_index']]], count)
            row = {'case': case['id'], 'baseline': count_commands(baseline, count),
                   'target_read': source, 'mcp_followup': count_commands(followup, count)}
            for variant in ('default', 'large'):
                row[variant] = count_variant(capture[variant], source, count)
                row[variant]['saving_percent'] = savings(row['baseline'], row[variant]['total'])
                row[variant]['with_followup'] = row[variant]['total'] + row['mcp_followup']
            rows.append(row)
        single, catalog = count(schemas['change_context_tool']), count(schemas['tools'])
        result['encodings'][encoding_name] = {
            'cases': rows, 'single_tool_schema': single, 'catalog_schema': catalog,
            'totals': totals(rows, single, catalog),
        }
    result['artifact_sha256'] = artifacts
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print(json.dumps({key: value['totals'] for key, value in result['encodings'].items()}, indent=2))


if __name__ == '__main__':
    main()
