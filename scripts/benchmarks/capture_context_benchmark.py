#!/usr/bin/env python3
"""Capture unmodified MCP responses for local context-token comparisons.

Usage: python3 scripts/benchmarks/capture_context_benchmark.py [--output PATH]
Requires an already-built archguard binary and the three checked-out examples.
No tokenizer, external API, or browser is used. Normal MCP caches may be updated.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import select
import subprocess
import tempfile
import time


CASES = [(case['id'], case['root'], case['target']) for case in
         json.loads(Path(__file__).with_name('context_benchmark_cases.json').read_text(encoding='utf-8'))]


def compact(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=False)


def collect_pages(call, target, max_bytes, revision=None):
    result = {'complete': False, 'error': None, 'revision': revision,
              'items': [], 'calls': []}
    offset, page_budget, total = 0, max_bytes, None
    while True:
        arguments = {'targets': [target], 'depth': 1,
                     'max_bytes': page_budget, 'offset': offset}
        if result['revision']:
            arguments['expected_revision'] = result['revision']
        record = {'arguments': arguments, 'arguments_text': compact(arguments)}
        result['calls'].append(record)
        started = time.monotonic()
        try:
            response_text = call(arguments)
            record['response_text'] = response_text
            record['response_bytes'] = len(response_text.encode('utf-8'))
            page = json.loads(response_text)
        except Exception as error:
            record['error'] = str(error)
            result['error'] = str(error)
            return result
        finally:
            record['seconds'] = time.monotonic() - started
        if result['revision'] and page['revision'] != result['revision']:
            result['error'] = 'revision changed during capture'
            return result
        result['revision'] = page['revision']
        if total is not None and page['total'] != total:
            result['error'] = 'total changed during capture'
            return result
        total = page['total']
        result['total'] = total
        if page['offset'] != offset:
            result['error'] = 'response offset does not match request cursor'
            return result
        if page.get('omitted_oversized') or page.get('omitted_metadata'):
            if page_budget < 16384:
                # Retry the same page. Count both responses, but accept only
                # the complete retry into the resulting logical item list.
                page_budget = 16384
                continue
            result['error'] = 'unrecoverable omission at maximum byte budget'
            return result
        result['items'].extend(page['items'])
        next_offset = page['next_offset']
        if next_offset is None:
            if len(result['items']) != total:
                result['error'] = 'pagination completed with missing or duplicate items'
            else:
                result['complete'] = True
            return result
        if not isinstance(next_offset, int) or next_offset <= offset:
            result['error'] = 'pagination cursor did not advance'
            return result
        offset, page_budget = next_offset, max_bytes


@contextmanager
def mcp(binary, root, timeout):
    with tempfile.TemporaryFile(mode='w+t') as stderr:
        process = subprocess.Popen(
            [str(binary), 'mcp', '--root', str(root)], cwd=root,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr,
            text=True, encoding='utf-8', bufsize=1,
        )
        request_id = 0

        def send(value):
            process.stdin.write(compact({'jsonrpc': '2.0', **value}) + '\n')
            process.stdin.flush()

        def rpc(method, params):
            nonlocal request_id
            request_id += 1
            send({'id': request_id, 'method': method, 'params': params})
            if not select.select([process.stdout], [], [], timeout)[0]:
                raise TimeoutError(f'MCP {method} response exceeded {timeout}s')
            line = process.stdout.readline()
            if not line:
                stderr.seek(0)
                raise RuntimeError(f'MCP exited: {stderr.read()[-4000:]}')
            response = json.loads(line)
            if response.get('id') != request_id:
                raise RuntimeError(f'Unexpected MCP response id: {response}')
            if 'error' in response:
                raise RuntimeError(compact(response['error']))
            return response['result']

        try:
            rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                              'clientInfo': {'name': 'archguard-context-benchmark', 'version': '1'}})
            send({'method': 'notifications/initialized'})
            yield rpc
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)


def source_manifest(root, items):
    """Resolve only explicitly reported file paths; do not guess node mappings."""
    paths = {}

    def visit(value, kind):
        if isinstance(value, dict):
            for key, child in value.items():
                # reference_file is a logical graph alias; file is the explicit
                # physical mapping supplied alongside it by change_context.
                if key == 'file' and isinstance(child, str):
                    paths.setdefault(child, set()).add(kind)
                visit(child, kind)
        elif isinstance(value, list):
            for child in value:
                visit(child, kind)

    for item in items:
        visit(item, item['kind'])
    manifest = []
    for name, kinds in sorted(paths.items()):
        path = (root / name).resolve()
        entry = {'file': name, 'kinds': sorted(kinds), 'exists': path.is_file()}
        if path.is_relative_to(root) and path.is_file():
            content = path.read_bytes()
            entry.update({'path': str(path), 'bytes': len(content),
                          'sha256': hashlib.sha256(content).hexdigest()})
        else:
            entry['unresolved'] = True
        manifest.append(entry)
    return manifest


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/archguard'))
    parser.add_argument('--output', type=Path,
                        default=Path('.archguard/reports/context-token-benchmark/mcp'))
    parser.add_argument('--case', choices=[case[0] for case in CASES], action='append')
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[2]
    binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    summaries = []
    for name, relative_root, target in CASES:
        if args.case and name not in args.case:
            continue
        root = (repository / relative_root).resolve()
        if not (root / target).is_file():
            raise FileNotFoundError(root / target)
        print(f'Capturing {name}: {target}', flush=True)
        with mcp(binary, root, args.timeout) as rpc:
            tools = rpc('tools/list', {})['tools']
            if not (args.output / 'tools.json').exists():
                write_json(args.output / 'tools.json', tools)
                write_json(args.output / 'change_context_tool.json',
                           next(tool for tool in tools if tool['name'] == 'change_context'))

            def call(arguments):
                result = rpc('tools/call', {'name': 'change_context', 'arguments': arguments})
                if result.get('isError'):
                    raise RuntimeError(compact(result))
                return result['content'][0]['text']

            default = collect_pages(call, target, 4096)
            record = {'case': name, 'root': str(root), 'target': target,
                      'default': default}
            write_json(args.output / f'{name}.json', record)
            large = collect_pages(call, target, 16384, revision=default['revision'])
            record['large'] = large
            record['same_items'] = default['items'] == large['items']
            record['sources'] = source_manifest(root, default['items'])
            record['config_files'] = [str(path) for path in
                                      (root / '.archguard/archguard.toml', root / 'archguard.toml')
                                      if path.is_file()]
            write_json(args.output / f'{name}.json', record)
        summary = {'case': name, 'same_items': record['same_items'],
                   'source_files': len(record['sources'])}
        for variant in ('default', 'large'):
            value = record[variant]
            summary[variant] = {
                'complete': value['complete'], 'error': value['error'],
                'calls': len(value['calls']), 'items': len(value['items']),
                'response_bytes': sum(call.get('response_bytes', 0) for call in value['calls']),
                'seconds': sum(call['seconds'] for call in value['calls']),
                'omissions': sum(json.loads(call['response_text']).get('omitted_oversized', 0)
                                 for call in value['calls'] if 'response_text' in call),
            }
        summaries.append(summary)
        print(compact(summary), flush=True)
    write_json(args.output / 'summary.json', summaries)
    if any(not row[variant]['complete'] for row in summaries for variant in ('default', 'large')):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
