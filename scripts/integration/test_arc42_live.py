#!/usr/bin/env python3
"""Test actual CLI/HTML/MCP arc42 parity with source, config and imported evidence."""
import argparse
import json
from pathlib import Path
import select
import subprocess
import tempfile
from viewer_payload import payload_from_html


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='archguard-arc42-') as directory:
        root = Path(directory)
        (root / 'src/app').mkdir(parents=True)
        (root / 'src/domain').mkdir(parents=True)
        (root / 'archguard.toml').write_text("derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['java']\n[modules]\ndepth=0\n[[architecture.goals]]\nname='Modifiability'\npriority='high'\nrationale='Independent changes'\n[[architecture.decisions]]\nid='ADR-1'\ntitle='Use ports'\nstatus='proposed'\nchosen_option='Interface boundary'\n")
        (root / 'src/app/Use.java').write_text('package app; public class Use { public int run(int n){if(n>0)return new domain.Model().value(); return 0;} }')
        (root / 'src/domain/Model.java').write_text('package domain; public class Model { public int value(){return 3;} }')
        (root / 'compose.yaml').write_text('services:\n  api:\n    image: api\n    environment:\n      WORKER_URL: http://worker:8080\n  worker:\n    image: worker\n')

        def run(command, *arguments):
            result = subprocess.run([str(binary), command, str(root), *arguments], capture_output=True, text=True, timeout=90)
            assert result.returncode == 0, result.stderr
            return result.stdout

        inputs = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
        full = run('doc', '--arc42', '--stdout')
        (out / 'example.arc42.md').write_text(full)
        assert full.count('\n## ') == 12
        assert 'src/app/Use.java:1' in full and 'statische' in full
        assert 'Modifiability' in full and 'proposed' in full
        assert 'Gemessene Klassenwerte' in full and 'Mermaid' not in full  # diagram uses a language fence
        assert '```mermaid' in full and 'compose' in full
        page = run('graph', '--format', 'html')
        arc42 = payload_from_html(page)['insights']['arc42']
        assert arc42['markdown'].rstrip('\n') == full.rstrip('\n'), 'CLI and UI must use the same original evidence and renderer'
        for chapter in arc42['chapters']:
            expected = run('doc', '--arc42', '--chapter', str(chapter['number']), '--stdout')
            assert chapter['markdown'].rstrip('\n') == expected.rstrip('\n')
        proc = subprocess.Popen([str(binary), 'mcp', '--root', str(root)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        sequence = 0

        def rpc(method, params):
            nonlocal sequence
            sequence += 1
            proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}) + '\n')
            proc.stdin.flush()
            assert select.select([proc.stdout], [], [], 90)[0], 'MCP timeout'
            response = json.loads(proc.stdout.readline())
            assert response['id'] == sequence and 'error' not in response, response
            return response['result']

        try:
            rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {}, 'clientInfo': {'name': 'arc42-test', 'version': '1'}})
            proc.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            proc.stdin.flush()
            text = ''
            offset = 0
            pages = 0
            while True:
                response = rpc('tools/call', {'name': 'architecture_document', 'arguments': {'template': 'arc42', 'offset': offset, 'limit': 2000}})
                assert not response.get('isError'), response
                content = response['content'][0]['text']
                assert len(content.encode()) <= 4096
                chunk = json.loads(content)
                text += chunk['markdown']
                pages += 1
                if chunk['next_offset'] is None:
                    break
                assert chunk['next_offset'] > offset
                offset = chunk['next_offset']
            assert text == arc42['markdown'], 'MCP must reconstruct exactly the UI document'
        finally:
            proc.stdin.close()
            proc.wait(timeout=10)
        assert not (root / 'arc42.md').exists(), 'stdout and MCP must not write documentation'
        for path, before in inputs.items():
            assert (root / path).read_bytes() == before, path
        (out / 'example.html').write_text(page)
        (out / 'example.arc42.md').write_text(arc42['markdown'])
        result = {'chapters': 12, 'cli_ui_mcp_parity': True, 'mcp_pages': pages, 'mcp_bounded': True, 'original_sources_unchanged': True}
        (out / 'live.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result))


if __name__ == '__main__':
    main()
