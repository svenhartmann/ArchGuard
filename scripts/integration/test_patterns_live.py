#!/usr/bin/env python3
"""Check pattern detection with synthetic sources and the real Java compiler."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from viewer_payload import payload_from_html


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path, nargs='?', default=Path('target/debug/archguard'))
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='archguard-patterns-') as temporary:
        root = Path(temporary)
        inputs = {}

        def write(name, content):
            inputs[name] = content
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(content)

        def run(directory, fmt='json'):
            result = subprocess.run([str(binary), 'patterns', str(directory), '--format', fmt], capture_output=True, text=True, timeout=90, check=True)
            return json.loads(result.stdout) if fmt == 'json' else result.stdout

        def row(report, name):
            return next(p for p in report['assessments'] if p['pattern'] == name)

        write('ports/archguard.toml', "derive_layers=false\n[project]\nlanguages=['java']\nsource_roots=['src']\nexclude=[]\n")
        write('ports/src/core/Store.java', 'package core; public interface Store { void save(); }\n')
        write('ports/src/core/Use.java', 'package core; public class Use { public void run(Store store) { store.save(); } }\n')
        write('ports/src/infra/Db.java', 'package infra; public class Db implements core.Store { public void save() {} }\n')
        write('ports/compose.yaml', 'services:\n  api:\n    build: ./api\n    environment:\n      WORKER_URL: http://worker:8080\n  worker:\n    build: ./worker\n')
        write('ports/api/Dockerfile', 'FROM scratch\n')
        write('ports/worker/Dockerfile', 'FROM scratch\n')
        write('ports/asyncapi.yaml', "asyncapi: 3.0.0\ninfo:\n  title: Synthetic messaging\n  version: '1'\nchannels:\n  orders:\n    address: orders.created\noperations:\n  emit:\n    action: send\n    channel:\n      $ref: '#/channels/orders'\n")
        ports = run(root / 'ports')
        assert ports['coverage']['semantic_status'] == 'ready', ports['coverage']
        assert any(a['backend'] == 'javac' for a in ports['coverage']['semantic_analyzers'])
        port = row(ports, 'ports_and_adapters')
        assert port['status'] == 'candidate', port
        assert {e['kind'] for e in port['evidence']} >= {'port', 'port_call', 'implementation'}
        assert all((root / 'ports' / e['file']).is_file() for e in port['evidence']), port['evidence']
        assert row(ports, 'microservices')['status'] == 'candidate'
        assert row(ports, 'event_driven')['status'] == 'candidate'
        html = run(root / 'ports', 'html')
        assert payload_from_html(html)['insights']['patterns'] == ports
        (output / 'example.html').write_text(html)
        (output / 'ports.json').write_text(json.dumps(ports, indent=2) + '\n')

        write('layers/archguard.toml', "derive_layers=false\n[project]\nlanguages=['ts']\nsource_roots=['src']\nexclude=[]\n")
        write('layers/src/web/page.ts', "import '../application/use';\n")
        write('layers/src/application/use.ts', "import '../data/store';\n")
        write('layers/src/data/store.ts', 'export const value=1;\n')
        layers = run(root / 'layers')
        assert row(layers, 'layered')['status'] == 'candidate'
        assert row(layers, 'pipeline')['status'] == 'candidate'
        (output / 'layers.json').write_text(json.dumps(layers, indent=2) + '\n')

        write('modules/archguard.toml', "derive_layers=false\n[project]\nlanguages=['ts']\nsource_roots=['src']\nexclude=[]\n")
        for package in ('sales', 'stock'):
            write(f'modules/src/{package}/use.ts', "import './model';\n")
            write(f'modules/src/{package}/model.ts', 'export const value=1;\n')
        modules = run(root / 'modules')
        assert row(modules, 'modular_monolith')['status'] == 'candidate'
        (output / 'modules.json').write_text(json.dumps(modules, indent=2) + '\n')
        assert not list(root.rglob('*.class')), 'Analysis must not compile project bytecode'
        for name, content in inputs.items():
            assert (root / name).read_text() == content, name
        (output / 'synthetic-inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
        result = {'syntheticSources': True, 'realCompiler': 'javac', 'checks': ['six pattern candidates across three fixtures', 'actual interface implementation and call', 'original source locations', 'standalone HTML parity', 'no source or config edits', 'no project bytecode']}
        (output / 'verification.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result))


if __name__ == '__main__':
    main()
