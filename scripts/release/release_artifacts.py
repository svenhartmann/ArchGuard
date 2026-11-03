#!/usr/bin/env python3
"""Package native CLIs and verify the complete release set before malware scanning."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[2]
TARGETS = (
    'x86_64-pc-windows-msvc',
    'aarch64-apple-darwin',
    'x86_64-apple-darwin',
    'x86_64-unknown-linux-musl',
    'aarch64-unknown-linux-musl',
)
DOCUMENTS = ('README.md', 'RELEASES.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'Cargo.lock')
MAX_FILE = 200 * 1024 * 1024


def version():
    value = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', value):
        raise ValueError('Unsupported release version')
    return value


def validate_tag(tag):
    if tag and tag != 'v' + version():
        raise ValueError('Release tag must match v' + version() + ' from Cargo.toml')
    return version()


def stem(target):
    if target not in TARGETS:
        raise ValueError('Unsupported release target')
    return f'archguard-{version()}-{target}'


def filename(target):
    return stem(target) + ('.zip' if 'windows' in target else '.tar.gz')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pack(binary, target, out, revision):
    prefix = stem(target)
    if not re.fullmatch(r'[a-f0-9]{40}', revision):
        raise ValueError('Expected full Git revision')
    if not binary.is_file() or binary.stat().st_size == 0 or binary.stat().st_size > MAX_FILE:
        raise ValueError('Missing, empty or oversized binary')
    binary_name = 'archguard.exe' if 'windows' in target else 'archguard'
    payload = {name: (ROOT / name).read_bytes() for name in DOCUMENTS}
    payload[binary_name] = binary.read_bytes()
    payload['release-info.json'] = (json.dumps({
        'version': version(), 'revision': revision, 'target': target,
        'features': [], 'binary_sha256': digest(payload[binary_name]),
    }, indent=2) + '\n').encode()
    out.mkdir(parents=True, exist_ok=True)
    archive = out / filename(target)
    if archive.suffix == '.zip':
        with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as bundle:
            for name, data in payload.items():
                info = zipfile.ZipInfo(prefix + '/' + name)
                info.create_system = 3
                info.external_attr = (0o100755 if name == binary_name else 0o100644) << 16
                bundle.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED)
    else:
        with tarfile.open(archive, 'w:gz') as bundle:
            for name, data in payload.items():
                info = tarfile.TarInfo(prefix + '/' + name)
                info.size = len(data)
                info.mode = 0o755 if name == binary_name else 0o644
                bundle.addfile(info, io.BytesIO(data))
    archive.with_name(archive.name + '.sha256').write_text(digest(archive.read_bytes()) + '  ' + archive.name + '\n')
    return archive


def read_bundle(archive, expected):
    # Never extract archive paths or links directly onto the filesystem.
    payload = {}
    if archive.suffix == '.zip':
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                if member.filename not in expected or member.filename in payload or member.file_size > MAX_FILE:
                    raise ValueError('Unexpected archive member')
                if (member.external_attr >> 16) & 0o170000 != 0o100000:
                    raise ValueError('Non-regular archive member')
                payload[member.filename] = bundle.read(member)
    else:
        with tarfile.open(archive, 'r:gz') as bundle:
            for member in bundle:
                if member.name not in expected or member.name in payload or not member.isfile() or member.size > MAX_FILE:
                    raise ValueError('Unexpected archive member')
                payload[member.name] = bundle.extractfile(member).read()
    if payload.keys() != expected:
        raise ValueError('Incomplete archive contents')
    return payload


def verify(directory, extract, revision):
    archives = [directory / filename(target) for target in TARGETS]
    expected_files = {p.name for p in archives} | {p.name + '.sha256' for p in archives}
    actual = {p.name for p in directory.iterdir()} - {'SHA256SUMS'}
    if actual != expected_files:
        raise ValueError('Expected complete platform set with checksums and no extra files')
    if extract.exists() and any(extract.iterdir()):
        raise ValueError('Extraction directory must be empty')
    sums = []
    for target, archive in zip(TARGETS, archives):
        checksum = digest(archive.read_bytes()) + '  ' + archive.name + '\n'
        if archive.with_name(archive.name + '.sha256').read_text() != checksum:
            raise ValueError('Archive checksum mismatch: ' + archive.name)
        binary_name = 'archguard.exe' if 'windows' in target else 'archguard'
        prefix = stem(target)
        names = {prefix + '/' + name for name in (*DOCUMENTS, binary_name, 'release-info.json')}
        payload = read_bundle(archive, names)
        info = json.loads(payload[prefix + '/release-info.json'])
        if info.get('revision') != revision or info.get('target') != target or info.get('version') != version() or info.get('features') != []:
            raise ValueError('Release revision, target, version or feature mismatch')
        if digest(payload[prefix + '/' + binary_name]) != info.get('binary_sha256'):
            raise ValueError('Binary checksum mismatch')
        for name, data in payload.items():
            destination = extract / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            destination.chmod(0o755 if destination.name == binary_name else 0o644)
        sums.append(checksum)
    (directory / 'SHA256SUMS').write_text(''.join(sums))
    return archives


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest='action', required=True)
    v = actions.add_parser('version');v.add_argument('--tag', default='')
    p = actions.add_parser('pack');p.add_argument('--binary', required=True, type=Path);p.add_argument('--target', required=True, choices=TARGETS);p.add_argument('--out', required=True, type=Path);p.add_argument('--revision', required=True)
    v = actions.add_parser('verify');v.add_argument('--directory', required=True, type=Path);v.add_argument('--extract', required=True, type=Path);v.add_argument('--revision', required=True)
    args = parser.parse_args()
    if args.action == 'version':
        print(validate_tag(args.tag))
    elif args.action == 'pack':
        print(pack(args.binary, args.target, args.out, args.revision))
    else:
        for archive in verify(args.directory, args.extract, args.revision):
            print('Verified ' + archive.name)


if __name__ == '__main__':
    main()
