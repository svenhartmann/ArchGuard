#!/usr/bin/env python3
"""Compare two committed branches over MCP without using the current checkout."""
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/archguard').resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull)
with tempfile.TemporaryDirectory(prefix='archguard-branches-mcp-') as directory:
    root = Path(directory)
    def git(*args):
        return subprocess.check_output(['git', '-C', str(root), *args], env=env, stderr=subprocess.DEVNULL)
    git('init', '-q', '--template=', '-b', 'main')
    git('config', 'user.name', 'Fixture')
    git('config', 'user.email', 'fixture@example.invalid')
    git('config', 'commit.gpgsign', 'false')
    (root / 'archguard.toml').write_text("derive_layers=false\n[project]\nlanguages=['ts']\n[modules]\ndepth=0\n")
    (root / 'a.ts').write_text("import './b';\n")
    (root / 'b.ts').write_text('export const b=1;\n')
    git('add', '.'); git('commit', '-qm', 'base')
    git('switch', '-qc', 'feature/architecture')
    (root / 'b.ts').write_text("import './a';\n")
    git('add', '.'); git('commit', '-qm', 'cycle')
    git('switch', '-q', 'main')
    before = (root / '.git/index').read_bytes()
    head = git('rev-parse', 'HEAD')
    process = subprocess.Popen([str(binary), 'mcp', '--root', str(root)], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    identifier = 0
    def rpc(method, params):
        global identifier
        identifier += 1
        process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': identifier, 'method': method, 'params': params}) + '\n')
        process.stdin.flush()
        assert select.select([process.stdout], [], [], 30)[0], 'MCP timeout'
        reply = json.loads(process.stdout.readline())
        assert reply['id'] == identifier
        return reply
    def compare(**args):
        return rpc('tools/call', {'name': 'compare', 'arguments': args})
    try:
        rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {}, 'clientInfo': {'name': 'branch-test', 'version': '1'}})
        process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); process.stdin.flush()
        for dirty in [False, True]:
            if dirty:
                (root / 'archguard.toml').write_text('[invalid')
                (root / 'b.ts').write_text('export const ignored=1;\n')
            reply = compare(base='main', target='feature/architecture')
            assert 'error' not in reply and not reply['result'].get('isError'), reply
            report = json.loads(reply['result']['content'][0]['text'])
            assert report.get('target_ref') == 'feature/architecture', 'compare must use the requested target branch, not the checkout'
            assert report['target_commit'] == git('rev-parse', 'feature/architecture').decode().strip()
            assert report['added_edges_total'] == 1
            assert any(f['kind'] == 'cycle' for f in report['new_findings']), report
        outside = compare(path='../', base='main', target='feature/architecture')
        assert 'error' in outside or outside['result'].get('isError'), outside
        missing = compare(base='main', target='missing')
        assert 'error' in missing or missing['result'].get('isError'), missing
        assert (root / '.git/index').read_bytes() == before
        assert git('rev-parse', 'HEAD') == head
        print('MCP branch comparison: exact target commit, new cycle, ignored worktree policy, root scope and unchanged Git state passed')
    finally:
        process.stdin.close()
        process.wait(timeout=10)
