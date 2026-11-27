#!/usr/bin/env python3
"""Package the native desktop app as installers and verify the complete set.

macOS: ArchGuard.app in a .dmg, ad-hoc signed, or with --sign-identity signed
with a Developer ID (hardened runtime) and with --notary-profile notarized and
stapled. Windows: WiX .msi and a plain .zip, with --sign-thumbprint the exe and
the .msi Authenticode-signed. Linux (glibc): AppImage for x86_64 and aarch64.
Credentials never reach this script: the release workflow imports them into a
temporary keychain or certificate store and passes only names.
"""
import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile
import time
from xml.sax.saxutils import quoteattr
import zipfile

import release_artifacts as release

ROOT = release.ROOT
ICON_PNG = ROOT / 'crates/archguard-desktop/assets/icon.png'
BINARY = 'archguard-desktop'
BUNDLE_ID = 'eu.svenhartmann.archguard'
# Fixed forever: Windows Installer matches upgrades of the same product by this code.
UPGRADE_CODE = '{159AF971-673D-48A0-9DB5-D3CE17470023}'
SUMS = 'SHA256SUMS-desktop'
KINDS = {
    'aarch64-apple-darwin': ('.dmg',),
    'x86_64-apple-darwin': ('.dmg',),
    # GitHub asset names ignore case, so the zip must not match the CLI archive's name.
    'x86_64-pc-windows-msvc': ('.msi', '-portable.zip'),
    'x86_64-unknown-linux-gnu': ('.AppImage',),
    'aarch64-unknown-linux-gnu': ('.AppImage',),
}
TARGETS = tuple(KINDS)
ZIP_DOCUMENTS = ('README.md', 'LICENSE-MIT', 'LICENSE-APACHE')
# RFC 3161 timestamp server for Authenticode, so signatures outlive the certificate.
TIMESTAMP_URL = 'http://timestamp.digicert.com'


def packages(target):
    if target not in KINDS:
        raise ValueError('Unsupported desktop target: ' + target)
    return tuple(f'ArchGuard-{release.version()}-{target}{suffix}' for suffix in KINDS[target])


def msi_version(version):
    # Windows Installer only compares major.minor.build (255.255.65535).
    match = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?', version)
    if not match:
        raise ValueError('Unsupported MSI version: ' + version)
    major, minor, build = (int(part) for part in match.groups())
    if major > 255 or minor > 255 or build > 65535:
        raise ValueError('Version exceeds the MSI range: ' + version)
    return f'{major}.{minor}.{build}'


def info_plist(version):
    return plistlib.dumps({
        'CFBundleName': 'ArchGuard', 'CFBundleDisplayName': 'ArchGuard',
        'CFBundleIdentifier': BUNDLE_ID, 'CFBundleExecutable': BINARY,
        'CFBundleIconFile': 'ArchGuard', 'CFBundlePackageType': 'APPL',
        'CFBundleShortVersionString': version, 'CFBundleVersion': version,
        'CFBundleInfoDictionaryVersion': '6.0', 'LSMinimumSystemVersion': '11.0',
        'LSApplicationCategoryType': 'public.app-category.developer-tools',
        'NSHighResolutionCapable': True,
    })


def desktop_entry():
    return ('[Desktop Entry]\nType=Application\nName=ArchGuard\n'
            'GenericName=Architecture viewer\n'
            'Comment=Explore and check the module architecture of a code base\n'
            f'Exec={BINARY}\nIcon=archguard\nTerminal=false\nCategories=Development;\n')


def wxs(version, source):
    return f'''<?xml version="1.0" encoding="utf-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="ArchGuard" Manufacturer="Sven Hartmann" Version="{version}" UpgradeCode="{UPGRADE_CODE}">
    <MajorUpgrade DowngradeErrorMessage="A newer version of ArchGuard is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <StandardDirectory Id="ProgramFiles64Folder">
      <Directory Id="INSTALLFOLDER" Name="ArchGuard">
        <Component Id="DesktopExecutable">
          <File Id="ArchGuardDesktopExe" Source={quoteattr(source)} Name="{BINARY}.exe" KeyPath="yes" />
        </Component>
      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder">
      <Component Id="StartMenuShortcut">
        <Shortcut Id="ArchGuardShortcut" Name="ArchGuard" Description="ArchGuard architecture viewer"
                  Target="[INSTALLFOLDER]{BINARY}.exe" WorkingDirectory="INSTALLFOLDER" />
        <RegistryValue Root="HKCU" Key="Software\\ArchGuard" Name="StartMenuShortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </StandardDirectory>
    <Feature Id="Main" Title="ArchGuard">
      <ComponentRef Id="DesktopExecutable" />
      <ComponentRef Id="StartMenuShortcut" />
    </Feature>
  </Package>
</Wix>
'''


def iconset_sizes():
    for size in (16, 32, 128, 256, 512):
        yield f'icon_{size}x{size}.png', size
        yield f'icon_{size}x{size}@2x.png', size * 2


def checked_binary(binary):
    if not binary.is_file() or binary.stat().st_size == 0 or binary.stat().st_size > release.MAX_FILE:
        raise ValueError('Missing, empty or oversized binary: ' + str(binary))
    return binary


def install(binary, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(checked_binary(binary), destination)
    destination.chmod(0o755)


def app_layout(binary, stage):
    app = stage / 'ArchGuard.app'
    install(binary, app / 'Contents/MacOS' / BINARY)
    (app / 'Contents/Resources').mkdir(parents=True, exist_ok=True)
    (app / 'Contents/Info.plist').write_bytes(info_plist(release.version()))
    return app


def appdir_layout(binary, appdir):
    install(binary, appdir / 'usr/bin' / BINARY)
    (appdir / 'AppRun').symlink_to('usr/bin/' + BINARY)
    (appdir / 'archguard.desktop').write_text(desktop_entry())
    shutil.copyfile(ICON_PNG, appdir / 'archguard.png')
    return appdir


def windows_zip(binary, out):
    out.mkdir(parents=True, exist_ok=True)
    archive = out / packages('x86_64-pc-windows-msvc')[1]
    prefix = archive.name.removesuffix('.zip') + '/'
    payload = {name: (ROOT / name).read_bytes() for name in ZIP_DOCUMENTS}
    payload[BINARY + '.exe'] = checked_binary(binary).read_bytes()
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as bundle:
        for name, data in payload.items():
            info = zipfile.ZipInfo(prefix + name)
            info.create_system = 3
            info.external_attr = (0o100755 if name.endswith('.exe') else 0o100644) << 16
            bundle.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED)
    return archive


def write_checksum(path):
    line = release.digest(path.read_bytes()) + '  ' + path.name + '\n'
    path.with_name(path.name + '.sha256').write_text(line)
    return line


def run(*command, env=None):
    print('+ ' + ' '.join(str(part) for part in command), flush=True)
    subprocess.run([str(part) for part in command], check=True, env=env)


def macos(binary, target, out, work, identity=None, keychain=None, notary_profile=None):
    if notary_profile and not identity:
        raise ValueError('Notarization needs a Developer ID signature (--sign-identity)')
    if identity and not keychain:
        raise ValueError('Signing with an identity needs the --keychain that holds it')
    app = app_layout(binary, work / 'stage')
    iconset = work / 'ArchGuard.iconset'
    iconset.mkdir()
    for name, size in iconset_sizes():
        run('sips', '-z', size, size, ICON_PNG, '--out', iconset / name)
    run('iconutil', '-c', 'icns', iconset, '-o', app / 'Contents/Resources/ArchGuard.icns')
    if identity:
        # Notarization requires the hardened runtime and a secure timestamp.
        run('codesign', '--force', '--options', 'runtime', '--timestamp',
            '--keychain', keychain, '--sign', identity, app)
    else:
        run('codesign', '--force', '--deep', '--sign', '-', app)
    run('codesign', '--verify', '--deep', '--strict', '--verbose=2', app)
    (work / 'stage/Applications').symlink_to('/Applications')
    out.mkdir(parents=True, exist_ok=True)
    dmg = out / packages(target)[0]
    # GitHub's macOS runners fail hdiutil create now and then with "Resource busy" (v0.3.5); retry a few times.
    for attempt in range(5):
        try:
            run('hdiutil', 'create', '-volname', 'ArchGuard ' + release.version(), '-srcfolder', work / 'stage',
                '-fs', 'HFS+', '-format', 'UDZO', '-ov', dmg)
            break
        except subprocess.CalledProcessError:
            if attempt == 4:
                raise
            time.sleep(10 * (attempt + 1))
    if notary_profile:
        run('xcrun', 'notarytool', 'submit', dmg, '--keychain-profile', notary_profile,
            '--keychain', keychain, '--wait')
        # Fails without an accepted ticket, so a rejected submission stops the release.
        run('xcrun', 'stapler', 'staple', dmg)
        run('xcrun', 'stapler', 'validate', dmg)
    return [dmg]


def linux(binary, target, out, work, tool, runtime):
    image = out / packages(target)[0]
    appdir = appdir_layout(binary, work / 'ArchGuard.AppDir')
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, ARCH=target.split('-')[0], APPIMAGE_EXTRACT_AND_RUN='1')
    run(tool, '--no-appstream', '--runtime-file', runtime, appdir, image, env=env)
    return [image]


def signtool(path, thumbprint):
    run('signtool', 'sign', '/sha1', thumbprint, '/fd', 'SHA256',
        '/tr', TIMESTAMP_URL, '/td', 'SHA256', path)
    run('signtool', 'verify', '/pa', '/v', path)


def windows(binary, out, work, thumbprint=None):
    if thumbprint:
        signtool(checked_binary(binary), thumbprint)  # before the .msi and .zip take copies
    source = work / 'ArchGuard.wxs'
    source.write_text(wxs(msi_version(release.version()), str(checked_binary(binary).resolve())), encoding='utf-8')
    out.mkdir(parents=True, exist_ok=True)
    msi = out / packages('x86_64-pc-windows-msvc')[0]
    # Built in work/ so WiX's .wixpdb next to it never reaches the release set.
    run('wix', 'build', '-arch', 'x64', '-o', work / msi.name, source)
    shutil.move(work / msi.name, msi)
    if thumbprint:
        signtool(msi, thumbprint)
    return [msi, windows_zip(binary, out)]


def verify(directory, extract):
    names = [name for target in TARGETS for name in packages(target)]
    actual = {p.name for p in directory.iterdir()} - {SUMS}
    if actual != set(names) | {name + '.sha256' for name in names}:
        raise ValueError('Expected complete desktop set with checksums and no extra files')
    if extract.exists() and any(extract.iterdir()):
        raise ValueError('Extraction directory must be empty')
    sums = []
    for name in names:
        checksum = release.digest((directory / name).read_bytes()) + '  ' + name + '\n'
        if (directory / (name + '.sha256')).read_text() != checksum:
            raise ValueError('Package checksum mismatch: ' + name)
        sums.append(checksum)
    extract_zip(directory / packages('x86_64-pc-windows-msvc')[1], extract)
    (directory / SUMS).write_text(''.join(sums))
    return [directory / name for name in names]


def extract_zip(archive, extract):
    prefix = archive.name.removesuffix('.zip') + '/'
    expected = {prefix + name for name in (*ZIP_DOCUMENTS, BINARY + '.exe')}
    for name, data in release.read_bundle(archive, expected).items():
        destination = extract / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('macos', 'linux', 'windows', 'verify'))
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--target', choices=TARGETS)
    parser.add_argument('--out', type=Path, required=True, help='output directory (verify: package directory)')
    parser.add_argument('--extract', type=Path, help='verify: empty directory for scannable contents')
    parser.add_argument('--appimagetool', type=Path)
    parser.add_argument('--runtime', type=Path)
    parser.add_argument('--sign-identity', help='macos: Developer ID identity; default ad-hoc signature')
    parser.add_argument('--keychain', help='macos: keychain with the identity and the notary profile')
    parser.add_argument('--notary-profile', help='macos: notarytool keychain profile; notarize and staple')
    parser.add_argument('--sign-thumbprint', help='windows: certificate in CurrentUser\\My for signtool')
    args = parser.parse_args()
    if args.action == 'verify':
        for path in verify(args.out, args.extract):
            print('Verified ' + path.name)
        return
    with tempfile.TemporaryDirectory(prefix='archguard-desktop-') as temporary:
        work = Path(temporary)
        if args.action == 'macos':
            built = macos(args.binary, args.target, args.out, work,
                          args.sign_identity, args.keychain, args.notary_profile)
        elif args.action == 'linux':
            built = linux(args.binary, args.target, args.out, work, args.appimagetool, args.runtime)
        else:
            built = windows(args.binary, args.out, work, args.sign_thumbprint)
    for path in built:
        print(write_checksum(path), end='')


if __name__ == '__main__':
    main()
