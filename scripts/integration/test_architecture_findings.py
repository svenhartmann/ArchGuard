#!/usr/bin/env python3
"""Live MCP finding discovery, repair context and re-analysis over real stdio."""
import json
from pathlib import Path
import select
import subprocess
import sys
import tempfile


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/archguard').resolve()
    with tempfile.TemporaryDirectory(prefix='archguard-findings-') as directory:
        base = Path(directory)
        root = base / 'project'
        root.mkdir()
        (base / 'outside').mkdir()
        sources = {
            'src/app/A.ts': 'import {b} from "../core/B";\nexport const a=b;\n',
            'src/core/B.ts': 'import {a} from "../app/A";\nexport const b=a;\n',
        }
        for name, content in sources.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        (root / 'archguard.toml').write_text('''derive_layers=false
[project]
source_roots=['src']
languages=['ts']
[modules]
depth=1
[rules]
deny_cycles='module'
[[dependency_rules]]
name='Core must not use app'
from=['core/**']
to=['app/**']
severity='error'
[architecture.model]
name='Boundaries'
severity='error'
require_assignment=true
[[architecture.model.components]]
name='Application'
include=['app/**']
[[architecture.model.components]]
name='Core'
include=['core/**']
[[architecture.model.allowed_dependencies]]
from='Application'
to='Core'
''')
        process = subprocess.Popen([str(binary), 'mcp', '--root', str(root)], cwd=base,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, bufsize=1)
        request_id = 0

        def rpc(method, params):
            nonlocal request_id
            request_id += 1
            process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': request_id, 'method': method, 'params': params})+'\n')
            process.stdin.flush()
            assert select.select([process.stdout], [], [], 60)[0], 'MCP timeout'
            response = json.loads(process.stdout.readline())
            assert response.get('id') == request_id, response
            return response

        def call(name, **arguments):
            return rpc('tools/call', {'name': name, 'arguments': arguments})

        def value(name, **arguments):
            response = call(name, **arguments)
            assert 'error' not in response and not response['result'].get('isError'), response
            text = response['result']['content'][0]['text']
            assert len(text.encode()) <= 4096, len(text.encode())
            return json.loads(text)

        def all_pages(name, **arguments):
            items, offset, revision = [], 0, None
            while True:
                page = value(name, **arguments, offset=offset, limit=1,
                             **({'expected_revision': revision} if revision else {}))
                revision = page['metadata']['revision']
                assert all(not row.get('omitted') for row in page['items']), page
                items.extend(page['items'])
                if page['next_offset'] is None:
                    assert len(items) == page['total'], page
                    return page, items
                assert page['next_offset'] > offset
                offset = page['next_offset']

        try:
            rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                               'clientInfo': {'name': 'finding-test', 'version': '1'}})
            process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            process.stdin.flush()
            names = {t['name'] for t in rpc('tools/list', {})['result']['tools']}
            assert {'architecture_findings', 'architecture_finding'} <= names, 'Live finding list and repair detail must be discoverable'
            first, findings = all_pages('architecture_findings')
            kinds = {f['finding_kind'] for f in findings}
            assert {'cycle', 'dependency_rule', 'target_architecture'} <= kinds, kinds
            assert first['metadata']['evidence_status']['semantic'] != 'ready'
            assert first['metadata']['evidence_gaps_total'] > 0
            assert len({f['id'] for f in findings}) == len(findings)
            cycle = next(f for f in findings if f['finding_kind'] == 'cycle')
            detail, rows = all_pages('architecture_finding', finding_id=cycle['id'])
            assert detail['metadata']['finding_id'] == cycle['id']
            assert any(row['kind'] == 'remediation' for row in rows)
            refs = [r for r in rows if r['kind'] == 'source_reference']
            assert refs and all((root / r['physical_path']).is_file() for r in refs), refs
            assert any('import' in r['reference']['raw'] and r['reference']['line'] == 1 for r in refs)
            assert any(r['kind'] == 'source_dependency' for r in rows)
            assert any(r['kind'] == 'cycle_scope' and r['file_cycle_groups'] == 1 for r in rows)
            context_call = next(r for r in rows if r['kind'] == 'next_tool' and r['name'] == 'change_context')
            context = value(context_call['name'], **context_call['arguments'])
            assert context['items'], context
            filtered, hits = all_pages('architecture_findings', kind='dependency_rule', severity='error')
            assert hits and all(f['finding_kind'] == 'dependency_rule' for f in hits)
            assert filtered['metadata']['findings_total'] == len(findings)
            for arguments in [{'limit': 0}, {'path': '../outside'}, {'kind': 'invented'}, {'severity': 'urgent'}]:
                assert call('architecture_findings', **arguments).get('error', {}).get('code') == -32602
            assert call('architecture_finding', finding_id=cycle['id'], path='../outside').get('error', {}).get('code') == -32602
            assert all((root / name).read_text() == content for name, content in sources.items())
            assert not (root / '.archguard/review.json').exists()
            assert not (root / '.archguard/reviews.json').exists()
            # A real source repair invalidates paging and removes the old observation.
            (root / 'src/core/B.ts').write_text('export const b=1;\n')
            assert call('architecture_findings', expected_revision=first['metadata']['revision'], offset=1).get('error', {}).get('code') == -32602
            after, remaining = all_pages('architecture_findings')
            assert not any(f['finding_kind'] in {'cycle', 'dependency_rule', 'target_architecture'} for f in remaining), remaining
            missing = value('architecture_finding', finding_id=cycle['id'])
            assert missing['status'] == 'not_found'
            assert missing['metadata']['evidence_gaps_total'] > 0
            assert 'not proof' in missing['metadata']['interpretation'].lower()
            assert after['metadata']['revision'] != first['metadata']['revision']
            gate = value('check')
            assert gate['status'] == 'pass', gate
            # Imported findings remain queryable, and source edits do not imply
            # the external producer was rerun or its result is now repaired.
            sarif = root / 'quality.sarif'
            sarif.write_text(json.dumps({'version': '2.1.0', 'runs': [{
                'tool': {'driver': {'name': 'fixture-linter'}},
                'results': [{'ruleId': 'maintainability', 'level': 'warning',
                             'message': {'text': 'Inspect this responsibility'},
                             'locations': [{'physicalLocation': {
                                 'artifactLocation': {'uri': 'src/core/B.ts'},
                                 'region': {'startLine': 1}}}]}]}]}))
            imported = subprocess.run([str(binary), 'quality', str(root), '--report', str(sarif),
                                       '--source-root', str(root), '--format', 'json'], capture_output=True, text=True)
            assert imported.returncode == 0, imported.stderr
            imported_page, imported_findings = all_pages('architecture_findings', category='quality')
            assert len(imported_findings) == 1, imported_findings
            assert imported_findings[0]['source_status'] == 'unchanged_since_import'
            (root / 'src/core/B.ts').write_text('export const b=2;\n')
            stale_page, stale_findings = all_pages('architecture_findings', category='quality')
            assert stale_findings[0]['id'] == imported_findings[0]['id']
            assert stale_findings[0]['source_status'] == 'changed_since_import'
            assert stale_page['metadata']['evidence_status']['quality'] == 'stale'
            assert stale_page['metadata']['revision'] != imported_page['metadata']['revision']
            assert not (root / '.archguard/review.json').exists()
            print('Architecture findings MCP: discovery, structural kinds, bounded pages, source evidence, usable follow-up calls, filters, path guards, read-only behavior, re-analysis and stale imported findings passed')
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)


if __name__ == '__main__':
    main()
