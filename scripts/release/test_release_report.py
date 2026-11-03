import json
from pathlib import Path
import tempfile
import unittest
import zipfile

import release_artifacts as release
import release_report as report


class ReleaseReportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name);self.dist = self.root / 'dist';self.dist.mkdir()
        self.logs = self.root / 'logs';self.logs.mkdir()
        self.revision = 'a' * 40
        self.states = {name: {'result': 'success'} for name in ('prepare', 'build', 'scan', 'quality', 'security')}
        binary = self.root / 'binary';binary.write_bytes(b'report test binary')
        for target in release.TARGETS:
            release.pack(binary, target, self.dist, self.revision)
        (self.logs / 'release-malware.log').write_text('Engine version: 1.5.4\nScanned files: 40\nInfected files: 0\n')
        (self.logs / 'release-signatures.log').write_text('Signature database updated\n')

    def bundle(self):
        return report.bundle(self.dist, self.logs, self.states, self.revision,
                             'https://github.com/example/repo/actions/runs/123', self.root / 'out')

    def test_success_includes_reports_exact_packages_logs_and_checksums(self):
        archive = self.bundle()
        with zipfile.ZipFile(archive) as package:
            metadata = json.loads(package.read('report.json'))
            self.assertTrue(metadata['release_eligible'])
            self.assertEqual(len(metadata['platforms']), 5)
            for target in release.TARGETS:
                self.assertEqual(package.read('packages/' + release.filename(target)), (self.dist / release.filename(target)).read_bytes())
            self.assertIn('packages/SHA256SUMS', package.namelist())
            self.assertIn('logs/release-malware.log', package.namelist())
            self.assertIn('logs/release-signatures.log', package.namelist())
            for line in package.read('CONTENTS.sha256').decode().splitlines():
                digest, name = line.split('  ', 1)
                self.assertEqual(release.digest(package.read(name)), digest)
            self.assertIn('Prüfbericht', package.read('report.html').decode())
            self.assertIn(self.revision, package.read('report.txt').decode())
        self.assertEqual(archive.with_name(archive.name + '.sha256').read_text(), release.digest(archive.read_bytes()) + '  ' + archive.name + '\n')

    def test_failed_gate_produces_diagnostics_without_packages(self):
        self.states['security']['result'] = 'failure'
        with zipfile.ZipFile(self.bundle()) as package:
            self.assertFalse(any(name.startswith('packages/') for name in package.namelist()))
            self.assertFalse(json.loads(package.read('report.json'))['release_eligible'])

    def test_skipped_security_is_not_reported_as_passed(self):
        self.states['quality']['result'] = self.states['security']['result'] = 'skipped'
        with zipfile.ZipFile(self.bundle()) as package:
            metadata = json.loads(package.read('report.json'))
            self.assertFalse(metadata['release_eligible'])
            self.assertTrue(metadata['packages_included'])
            self.assertIn('Nicht ausgeführt', package.read('report.html').decode())

    def test_failed_scan_with_missing_artifacts_still_has_report(self):
        self.states['scan']['result'] = 'failure';self.dist = self.root / 'missing'
        (self.logs / 'release-malware.log').unlink()
        with zipfile.ZipFile(self.bundle()) as package:
            self.assertFalse(json.loads(package.read('report.json'))['packages_included'])

    def test_clean_status_requires_evidence_and_intact_packages(self):
        (self.logs / 'release-malware.log').write_text('Infected files: 1\n')
        with self.assertRaises(ValueError):
            self.bundle()
        (self.logs / 'release-malware.log').write_text('Infected files: 0\n')
        next(self.dist.glob('*.zip')).write_bytes(b'corrupt')
        with self.assertRaises(ValueError):
            self.bundle()

    def test_html_escapes_untrusted_report_text(self):
        html = report.render_html({'version':'<script>alert(1)</script>', 'revision':self.revision,
                                  'run_url':'javascript:alert(1)', 'generated_at':'now', 'packages_included':False,
                                  'release_eligible':False, 'checks':{}, 'platforms':[]})
        self.assertNotIn('<script>', html)
        self.assertNotIn('href="javascript:', html)


if __name__ == '__main__':
    unittest.main()
