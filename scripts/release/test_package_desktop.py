"""Unsigned desktop packages must have valid metadata, fixed names and a complete set."""
import configparser
import os
from pathlib import Path
import plistlib
import tempfile
import unittest
import xml.etree.ElementTree as ElementTree
import zipfile

import package_desktop as desktop
import release_artifacts as release

WIX = '{http://wixtoolset.org/schemas/v4/wxs}'


class MetadataTests(unittest.TestCase):
    def test_info_plist_carries_workspace_version_and_bundle_id(self):
        info = plistlib.loads(desktop.info_plist(release.version()))
        self.assertEqual(info['CFBundleIdentifier'], 'eu.svenhartmann.archguard')
        self.assertEqual(info['CFBundleExecutable'], 'archguard-desktop')
        self.assertEqual(info['CFBundleIconFile'], 'ArchGuard')
        self.assertEqual(info['CFBundleShortVersionString'], release.version())
        self.assertEqual(info['CFBundleVersion'], release.version())
        self.assertEqual(info['CFBundlePackageType'], 'APPL')
        self.assertTrue(info['NSHighResolutionCapable'])

    def test_desktop_entry_launches_the_binary_with_its_icon(self):
        entry = configparser.ConfigParser(interpolation=None)
        entry.optionxform = str
        entry.read_string(desktop.desktop_entry())
        section = entry['Desktop Entry']
        self.assertEqual(section['Type'], 'Application')
        self.assertEqual(section['Exec'], 'archguard-desktop')
        self.assertEqual(section['Icon'], 'archguard')
        self.assertEqual(section['Terminal'], 'false')
        self.assertTrue(section['Categories'].endswith(';'))

    def test_wxs_installs_the_exe_with_a_start_menu_shortcut(self):
        root = ElementTree.fromstring(desktop.wxs('1.2.3', 'archguard-desktop.exe'))
        package = root.find(WIX + 'Package')
        self.assertEqual(package.get('Version'), '1.2.3')
        self.assertEqual(package.get('UpgradeCode'), desktop.UPGRADE_CODE)
        files = root.findall('.//' + WIX + 'File')
        self.assertEqual([f.get('Source') for f in files], ['archguard-desktop.exe'])
        shortcut = root.find('.//' + WIX + 'Shortcut')
        self.assertEqual(shortcut.get('Target'), '[INSTALLFOLDER]archguard-desktop.exe')
        menus = [d for d in root.iter(WIX + 'StandardDirectory') if d.get('Id') == 'ProgramMenuFolder']
        self.assertEqual(len(menus), 1)
        refs = {ref.get('Id') for ref in root.iter(WIX + 'ComponentRef')}
        components = {c.get('Id') for c in root.iter(WIX + 'Component')}
        self.assertEqual(refs, components)

    def test_wxs_escapes_the_source_path(self):
        root = ElementTree.fromstring(desktop.wxs('1.2.3', 'dir&<x>\\archguard-desktop.exe'))
        self.assertEqual(root.find('.//' + WIX + 'File').get('Source'), 'dir&<x>\\archguard-desktop.exe')

    def test_msi_version_drops_prerelease_and_checks_ranges(self):
        self.assertEqual(desktop.msi_version('0.2.1'), '0.2.1')
        self.assertEqual(desktop.msi_version('1.4.0-rc.1'), '1.4.0')
        for bad in ('256.0.0', '1.256.0', '1.0.65536', '1.0'):
            with self.assertRaises(ValueError):
                desktop.msi_version(bad)


class NamingTests(unittest.TestCase):
    def test_package_names_per_target(self):
        v = release.version()
        self.assertEqual(desktop.packages('aarch64-apple-darwin'), (f'ArchGuard-{v}-aarch64-apple-darwin.dmg',))
        self.assertEqual(desktop.packages('x86_64-apple-darwin'), (f'ArchGuard-{v}-x86_64-apple-darwin.dmg',))
        self.assertEqual(desktop.packages('x86_64-pc-windows-msvc'),
                         (f'ArchGuard-{v}-x86_64-pc-windows-msvc.msi', f'ArchGuard-{v}-x86_64-pc-windows-msvc-portable.zip'))
        self.assertEqual(desktop.packages('x86_64-unknown-linux-gnu'), (f'ArchGuard-{v}-x86_64-unknown-linux-gnu.AppImage',))

    def test_unknown_or_musl_target_is_rejected(self):
        with self.assertRaises(ValueError):
            desktop.packages('x86_64-unknown-linux-musl')

    def test_names_never_collide_with_cli_assets(self):
        # GitHub release asset names are case-insensitive.
        cli = {release.filename(t).lower() for t in release.TARGETS} | {'sha256sums'}
        names = {n.lower() for t in desktop.TARGETS for n in desktop.packages(t)} | {desktop.SUMS.lower()}
        self.assertFalse(cli & names)
        self.assertFalse(cli & {n + '.sha256' for n in names})


class LayoutTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.binary = self.root / 'archguard-desktop'
        self.binary.write_bytes(b'desktop binary fixture')

    def test_app_bundle_layout(self):
        app = desktop.app_layout(self.binary, self.root / 'stage')
        self.assertEqual(app.name, 'ArchGuard.app')
        executable = app / 'Contents/MacOS/archguard-desktop'
        self.assertEqual(executable.read_bytes(), self.binary.read_bytes())
        self.assertTrue(os.access(executable, os.X_OK))
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleExecutable'], executable.name)
        self.assertTrue((app / 'Contents/Resources').is_dir())

    def test_iconset_covers_every_icns_size(self):
        sizes = dict(desktop.iconset_sizes())
        self.assertEqual(sizes['icon_16x16.png'], 16)
        self.assertEqual(sizes['icon_16x16@2x.png'], 32)
        self.assertEqual(sizes['icon_512x512@2x.png'], 1024)
        self.assertEqual(len(sizes), 10)

    def test_appdir_layout(self):
        appdir = desktop.appdir_layout(self.binary, self.root / 'ArchGuard.AppDir')
        self.assertEqual((appdir / 'usr/bin/archguard-desktop').read_bytes(), self.binary.read_bytes())
        self.assertTrue(os.access(appdir / 'usr/bin/archguard-desktop', os.X_OK))
        self.assertEqual(os.readlink(appdir / 'AppRun'), 'usr/bin/archguard-desktop')
        self.assertEqual((appdir / 'archguard.desktop').read_text(), desktop.desktop_entry())
        self.assertEqual((appdir / 'archguard.png').read_bytes(), desktop.ICON_PNG.read_bytes())

    def test_windows_zip_contains_exe_and_licenses(self):
        archive = desktop.windows_zip(self.binary, self.root / 'dist')
        prefix = archive.name.removesuffix('.zip') + '/'
        with zipfile.ZipFile(archive) as bundle:
            self.assertEqual(set(bundle.namelist()),
                             {prefix + n for n in ('archguard-desktop.exe', 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE')})
            self.assertEqual(bundle.read(prefix + 'archguard-desktop.exe'), self.binary.read_bytes())

    def test_windows_keeps_wix_side_files_out_of_the_package_directory(self):
        def fake_wix(*command, env=None):
            msi = Path(command[command.index('-o') + 1])
            msi.write_bytes(b'msi fixture')
            msi.with_suffix('.wixpdb').write_bytes(b'debug symbols')  # real WiX writes this next to -o
        original, desktop.run = desktop.run, fake_wix
        self.addCleanup(setattr, desktop, 'run', original)
        work = self.root / 'work'
        work.mkdir()
        out = self.root / 'dist'
        desktop.windows(self.binary, out, work)
        self.assertEqual({p.name for p in out.iterdir()}, set(desktop.packages('x86_64-pc-windows-msvc')))

    def test_empty_binary_is_rejected(self):
        self.binary.write_bytes(b'')
        with self.assertRaises(ValueError):
            desktop.app_layout(self.binary, self.root / 'stage')


class VerifyTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.dist = self.root / 'dist'
        self.dist.mkdir()
        binary = self.root / 'archguard-desktop.exe'
        binary.write_bytes(b'exe fixture')
        for target in desktop.TARGETS:
            for name in desktop.packages(target):
                path = self.dist / name
                if name.endswith('.zip'):
                    desktop.windows_zip(binary, self.dist)
                else:
                    path.write_bytes(b'package fixture ' + name.encode())
                desktop.write_checksum(path)

    def test_complete_set_verifies_and_writes_sums(self):
        packages = desktop.verify(self.dist, self.root / 'scan')
        self.assertEqual(len(packages), 5)
        self.assertEqual(len((self.dist / desktop.SUMS).read_text().splitlines()), 5)
        self.assertEqual(len(list((self.root / 'scan').rglob('archguard-desktop.exe'))), 1)

    def test_missing_package_blocks_release(self):
        next(self.dist.glob('*.AppImage')).unlink()
        with self.assertRaisesRegex(ValueError, 'complete desktop set'):
            desktop.verify(self.dist, self.root / 'scan')

    def test_extra_file_blocks_release(self):
        (self.dist / 'unexpected.exe').write_bytes(b'x')
        with self.assertRaisesRegex(ValueError, 'complete desktop set'):
            desktop.verify(self.dist, self.root / 'scan')

    def test_corruption_blocks_release(self):
        dmg = next(self.dist.glob('*.dmg'))
        dmg.write_bytes(dmg.read_bytes() + b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            desktop.verify(self.dist, self.root / 'scan')


if __name__ == '__main__':
    unittest.main()
