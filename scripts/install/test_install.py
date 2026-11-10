"""Runs install.sh against a fake local release served over file://."""
import hashlib
import io
import os
import platform
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / 'install.sh'
VERSION = '9.9.9'


def host_target():
    system, machine = platform.system(), platform.machine().lower()
    arch = 'aarch64' if machine in ('arm64', 'aarch64') else 'x86_64'
    if system == 'Darwin':
        return f'{arch}-apple-darwin'
    if system == 'Linux':
        return f'{arch}-unknown-linux-musl'
    raise unittest.SkipTest('install.sh test runs on macOS and Linux')


def fake_desktop(directory, corrupt=False):
    """Adds the host's desktop package and SHA256SUMS-desktop; returns the installed path."""
    target = host_target()
    if target.endswith('apple-darwin'):
        package = directory / f'ArchGuard-{VERSION}-{target}.dmg'
        stage = directory / 'stage'
        (stage / 'ArchGuard.app/Contents').mkdir(parents=True)
        (stage / 'ArchGuard.app/Contents/marker').write_text(VERSION)
        subprocess.run(['hdiutil', 'create', '-quiet', '-volname', 'ArchGuard', '-srcfolder', str(stage),
                        '-fs', 'HFS+', '-format', 'UDZO', '-ov', str(package)], check=True)
        installed = 'apps/ArchGuard.app/Contents/marker'
    elif target.startswith('x86_64'):
        package = directory / f'ArchGuard-{VERSION}-x86_64-unknown-linux-gnu.AppImage'
        package.write_text(VERSION)
        installed = 'bin/archguard-desktop'
    else:
        raise unittest.SkipTest('no desktop package for ' + target)
    digest = '0' * 64 if corrupt else hashlib.sha256(package.read_bytes()).hexdigest()
    (directory / 'SHA256SUMS-desktop').write_text(f'{digest}  {package.name}\n')
    return installed


def fake_release(directory, corrupt=False):
    target = host_target()
    stem = f'archguard-{VERSION}-{target}'
    binary = b'#!/bin/sh\necho "archguard ' + VERSION.encode() + b'"\n'
    archive = directory / f'{stem}.tar.gz'
    with tarfile.open(archive, 'w:gz') as bundle:
        info = tarfile.TarInfo(f'{stem}/archguard')
        info.size, info.mode = len(binary), 0o755
        bundle.addfile(info, io.BytesIO(binary))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if corrupt:
        digest = '0' * 64
    (directory / 'SHA256SUMS').write_text(f'{digest}  {archive.name}\n')


class InstallScriptTest(unittest.TestCase):
    def run_install(self, corrupt=False, desktop=None, **extra):
        root = Path(tempfile.mkdtemp())
        release, bindir = root / 'release', root / 'bin'
        release.mkdir()
        fake_release(release, corrupt)
        self.desktop = root / fake_desktop(release, desktop == 'corrupt') if desktop else None
        env = dict(os.environ, ARCHGUARD_VERSION=VERSION,
                   ARCHGUARD_DOWNLOAD_URL=release.as_uri(),
                   ARCHGUARD_INSTALL_DIR=str(bindir),
                   ARCHGUARD_APP_DIR=str(root / 'apps'), **extra)
        result = subprocess.run(['sh', str(SCRIPT)], env=env, capture_output=True, text=True)
        return result, bindir / 'archguard'

    def test_installs_verified_binary(self):
        result, binary = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        out = subprocess.run([str(binary)], capture_output=True, text=True).stdout
        self.assertIn(VERSION, out)

    def test_rejects_checksum_mismatch(self):
        result, binary = self.run_install(corrupt=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('checksum', result.stderr.lower())
        self.assertFalse(binary.exists())

    def test_release_without_desktop_still_installs_cli(self):
        result, binary = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('No desktop app', result.stdout)

    def test_installs_verified_desktop_app(self):
        result, _ = self.run_install(desktop='ok')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.desktop.read_text(), VERSION)

    def test_rejects_desktop_checksum_mismatch(self):
        result, _ = self.run_install(desktop='corrupt')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('checksum mismatch', result.stderr)
        self.assertFalse(self.desktop.exists())

    def test_desktop_opt_out(self):
        result, binary = self.run_install(desktop='ok', ARCHGUARD_DESKTOP='0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(binary.exists())
        self.assertFalse(self.desktop.exists())


if __name__ == '__main__':
    unittest.main()
