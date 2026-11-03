"""Release archives must be complete, verifiable and safe to extract for scanning."""
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

import release_artifacts as release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / 'binary'
        self.binary.write_bytes(b'native binary fixture')
        self.out = self.root / 'dist'
        self.revision = 'a' * 40

    def build_all(self):
        for target in release.TARGETS:
            release.pack(self.binary, target, self.out, self.revision)

    def test_all_platforms_roundtrip_and_permissions(self):
        self.build_all()
        extracted = self.root / 'scanned'
        archives = release.verify(self.out, extracted, self.revision)
        self.assertEqual(len(archives), 5)
        for target in release.TARGETS:
            name = 'archguard.exe' if 'windows' in target else 'archguard'
            binary = extracted / release.stem(target) / name
            self.assertEqual(binary.read_bytes(), self.binary.read_bytes())
            archive = self.out / release.filename(target)
            if archive.suffix == '.zip':
                with zipfile.ZipFile(archive) as bundle:
                    self.assertEqual(bundle.getinfo(release.stem(target) + '/' + name).external_attr >> 16 & 0o777, 0o755)
            else:
                with tarfile.open(archive) as bundle:
                    self.assertEqual(bundle.getmember(release.stem(target) + '/' + name).mode, 0o755)
        self.assertEqual(len((self.out / 'SHA256SUMS').read_text().splitlines()), 5)

    def test_missing_platform_blocks_release(self):
        release.pack(self.binary, next(iter(release.TARGETS)), self.out, self.revision)
        with self.assertRaisesRegex(ValueError, 'complete platform set'):
            release.verify(self.out, self.root / 'scan', self.revision)

    def test_corruption_blocks_release(self):
        self.build_all()
        archive = next(self.out.glob('*.zip'))
        archive.write_bytes(archive.read_bytes() + b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            release.verify(self.out, self.root / 'scan', self.revision)

    def test_wrong_revision_blocks_release(self):
        self.build_all()
        with self.assertRaisesRegex(ValueError, 'revision'):
            release.verify(self.out, self.root / 'scan', 'b' * 40)

    def test_path_traversal_blocks_extraction_even_with_valid_checksum(self):
        self.build_all()
        archive = next(self.out.glob('*.tar.gz'))
        with tarfile.open(archive, 'w:gz') as bundle:
            info = tarfile.TarInfo('../escaped');info.size = 4
            bundle.addfile(info, io.BytesIO(b'evil'))
        archive.with_name(archive.name + '.sha256').write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\n')
        with self.assertRaises(ValueError):
            release.verify(self.out, self.root / 'scan', self.revision)
        self.assertFalse((self.root / 'escaped').exists())

    def test_binary_digest_checked_inside_archive(self):
        self.build_all()
        archive = next(self.out.glob('*.zip'))
        with zipfile.ZipFile(archive) as bundle:
            entries = [(info, bundle.read(info)) for info in bundle.infolist()]
        with zipfile.ZipFile(archive, 'w') as bundle:
            for info, data in entries:
                if info.filename.endswith('/release-info.json'):
                    metadata = json.loads(data)
                    metadata['binary_sha256'] = '0' * 64
                    data = json.dumps(metadata).encode()
                bundle.writestr(info, data)
        archive.with_name(archive.name + '.sha256').write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\n')
        with self.assertRaisesRegex(ValueError, 'Binary checksum'):
            release.verify(self.out, self.root / 'scan', self.revision)

    def test_links_are_never_extracted(self):
        archive = self.root / 'link.tar.gz'
        with tarfile.open(archive, 'w:gz') as bundle:
            info = tarfile.TarInfo('root/link')
            info.type = tarfile.SYMTYPE
            info.linkname = '/outside'
            bundle.addfile(info)
        with self.assertRaises(ValueError):
            release.read_bundle(archive, {'root/link'})
        archive = self.root / 'link.zip'
        with zipfile.ZipFile(archive, 'w') as bundle:
            info = zipfile.ZipInfo('root/link')
            info.create_system = 3
            info.external_attr = 0o120777 << 16
            bundle.writestr(info, '/outside')
        with self.assertRaises(ValueError):
            release.read_bundle(archive, {'root/link'})

    def test_empty_binary_is_not_a_release(self):
        self.binary.write_bytes(b'')
        with self.assertRaises(ValueError):
            release.pack(self.binary, release.TARGETS[0], self.out, self.revision)

    def test_tag_must_match_workspace_version(self):
        self.assertEqual(release.validate_tag('v' + release.version()), release.version())
        with self.assertRaises(ValueError):
            release.validate_tag('v99.0.0')
        with self.assertRaises(ValueError):
            release.validate_tag('v0.1.0/../../oops')


if __name__ == '__main__':
    unittest.main()
