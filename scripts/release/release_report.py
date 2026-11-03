#!/usr/bin/env python3
"""Bundle verified releases with an offline report; failures produce diagnostics only."""
import argparse
from datetime import datetime, timezone
import html
import json
import os
from pathlib import Path
import re
import tempfile
import zipfile

import release_artifacts as release

CHECKS = {
    'prepare': 'Version und Paketprüfungen',
    'build': 'Alle fünf Builds und native Funktionstests',
    'scan': 'Archivintegrität und ClamAV-Scan',
    'quality': 'Vollständige CI inkl. RustSec-Abhängigkeitsprüfung',
    'security': 'Quellcode-Malware- und Secret-Prüfung',
}
STATUS = {'success':'Bestanden', 'failure':'Fehlgeschlagen', 'cancelled':'Abgebrochen',
          'skipped':'Nicht ausgeführt', 'unknown':'Nicht bestätigt'}
LOGS = ('release-malware.log', 'release-signatures.log')


def render_html(report):
    escape = lambda value: html.escape(str(value), quote=True)
    checks = ''.join(f'<tr><th>{escape(CHECKS[key])}</th><td>{escape(STATUS.get(value, STATUS["unknown"]))}</td></tr>' for key, value in report['checks'].items())
    platforms = ''.join(f'<tr><th>{escape(row["target"])}</th><td>{escape(STATUS.get(row["native_checks"], STATUS["unknown"]))}</td></tr>' for row in report['platforms'])
    label = 'Release-Prüfungen bestanden' if report['release_eligible'] else 'Geprüfte Build-Pakete · Release-Prüfungen nicht vollständig ausgeführt' if report['packages_included'] else 'Diagnosebericht · keine Release-Pakete enthalten'
    run = report['run_url']
    link = f'<a href="{escape(run)}">Pipeline-Lauf und vollständige Build-Protokolle</a>' if run.startswith('https://') else 'Pipeline-Link nicht verfügbar'
    logs = ''.join(f'<li><a href="logs/{escape(name)}">{escape(name)}</a></li>' for name in report.get('logs', []))
    return f'''<!doctype html><html lang="de"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ArchGuard · Release-Prüfbericht</title><style>
body{{font:15px/1.6 system-ui,sans-serif;color:#172033;background:#f4f6fa;margin:0;padding:24px}}main{{max-width:980px;margin:auto;background:white;border:1px solid #dce3ed;border-radius:18px;padding:32px}}h1{{margin:4px 0 12px}}h2{{margin-top:30px;font-size:19px}}small,.muted{{color:#586579}}.status{{padding:12px 16px;background:#edf2ff;border-radius:10px}}table{{border-collapse:collapse;width:100%}}th,td{{padding:10px;text-align:left;border-bottom:1px solid #e5eaf2;overflow-wrap:anywhere}}th{{font-weight:500}}code{{overflow-wrap:anywhere}}a{{color:#2459c4}}@media(max-width:600px){{body{{padding:10px}}main{{padding:18px}}th,td{{padding:8px;font-size:13px}}}}@media print{{body{{padding:0;background:white}}main{{border:0}}}}
</style><main><small>ArchGuard · RELEASE-NACHWEIS</small><h1>Prüfbericht {escape(report['version'])}</h1>
<p class="status">{escape(label)}</p><p>Commit: <code>{escape(report['revision'])}</code><br>Erstellt (UTC): {escape(report['generated_at'])}<br>{link}</p>
<h2>Prüfungen</h2><table><thead><tr><th>Prüfung</th><th>Ergebnis dieses Laufs</th></tr></thead><tbody>{checks}</tbody></table>
<h2>Plattformen</h2><p class="muted">Funktionstest: Version, CLI-Hilfe, echte Importanalyse, Architekturprüfung und HTML-Export. Einzelne Matrix-Ergebnisse werden nur bei erfolgreichem Gesamt-Build bestätigt; bei Fehlern siehe Pipeline-Protokolle.</p><table><thead><tr><th>Rust-Zielplattform</th><th>Build und Funktionstest</th></tr></thead><tbody>{platforms}</tbody></table>
<h2>Scan-Nachweise und Download</h2><ul>{logs}</ul><p>ClamAV prüft die Plattformarchive und die entpackten Programme. Der Sammelbericht wird anschließend erstellt. Die Protokolle enthalten Scanner-Version, Signaturaktualisierung und Scan-Zusammenfassung, soweit der jeweilige Schritt ausgeführt wurde.</p>
<p>Bei enthaltenen Paketen: <code>packages/SHA256SUMS</code> prüft die Plattformarchive. <code>CONTENTS.sha256</code> prüft alle Inhalte dieses ZIP. <code>report.json</code> enthält die maschinenlesbaren Ergebnisse.</p>
<p class="muted">Nicht ausgeführte Prüfungen gelten nicht als bestanden. Binaries sind nicht signiert oder notarisiert; die optionale DB-Funktion ist nicht enthalten. Ein unauffälliger Scan garantiert keine Schadsoftwarefreiheit.</p></main></html>'''


def bundle(directory, logs, needs, revision, run_url, out):
    if not re.fullmatch(r'[a-f0-9]{40}', revision):
        raise ValueError('Expected full Git revision')
    checks = {key: needs.get(key, {}).get('result', 'unknown') for key in CHECKS}
    include = all(checks[key] == 'success' for key in ('prepare', 'build', 'scan')) and all(checks[key] in ('success', 'skipped') for key in ('quality', 'security'))
    entries = {}
    if include:
        evidence = logs / 'release-malware.log'
        if not evidence.is_file() or not re.search(r'^Infected files:\s+0\s*$', evidence.read_text(encoding='utf-8'), re.MULTILINE):
            raise ValueError('Successful scan requires its clean ClamAV log')
        with tempfile.TemporaryDirectory(prefix='archguard-bundle-verify-') as temporary:
            release.verify(directory, Path(temporary), revision)
        for path in sorted(directory.iterdir()):
            entries['packages/' + path.name] = path.read_bytes()
    log_names = []
    for name in LOGS:
        path = logs / name
        if path.is_file():
            entries['logs/' + name] = path.read_bytes();log_names.append(name)
    report = {
        'schema_version': 1, 'version': release.version(), 'revision': revision,
        'generated_at': datetime.now(timezone.utc).isoformat(), 'run_url': run_url,
        'checks': checks, 'release_eligible': all(value == 'success' for value in checks.values()),
        'packages_included': include, 'logs': log_names,
        'platforms': [{'target': target, 'native_checks': 'success' if checks['build'] == 'success' else 'unknown'} for target in release.TARGETS],
        'scan_scope': 'Platform archives and extracted binaries; the report bundle is assembled afterwards.',
        'smoke_checks': ['version', 'help', 'dependency extraction', 'architecture check', 'HTML export'],
        'archives': [{'name': release.filename(target), 'sha256': release.digest(entries['packages/' + release.filename(target)])} for target in release.TARGETS] if include else [],
    }
    entries['report.json'] = (json.dumps(report, indent=2, ensure_ascii=False) + '\n').encode()
    entries['report.html'] = render_html(report).encode()
    entries['report.txt'] = ('ArchGuard Release-Prüfbericht ' + report['version'] + '\nCommit: ' + revision + '\nPipeline: ' + run_url + '\n\n' + '\n'.join(CHECKS[key] + ': ' + STATUS.get(value, STATUS['unknown']) for key, value in checks.items()) + '\n\nRelease-Pakete enthalten: ' + ('Ja' if include else 'Nein') + '\nDetails: report.html / report.json\n').encode()
    entries['CONTENTS.sha256'] = ''.join(release.digest(data) + '  ' + name + '\n' for name, data in sorted(entries.items())).encode()
    out.mkdir(parents=True, exist_ok=True)
    archive = out / f'archguard-{release.version()}-release-bundle.zip'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as package:
        for name, data in entries.items():
            package.writestr(name, data, compress_type=zipfile.ZIP_STORED if name.endswith(('.zip', '.tar.gz')) else zipfile.ZIP_DEFLATED)
    archive.with_name(archive.name + '.sha256').write_text(release.digest(archive.read_bytes()) + '  ' + archive.name + '\n')
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--logs', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--run-url', required=True)
    args = parser.parse_args()
    print(bundle(args.directory, args.logs, json.loads(os.environ['RELEASE_JOB_RESULTS']), args.revision, args.run_url, args.out))


if __name__ == '__main__':
    main()
