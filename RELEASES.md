# Native releases

The **Release builds** workflow builds and smoke-tests five default-feature CLI
variants on native GitHub-hosted machines:

| Platform | Rust target | Runner | Archive |
| --- | --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` | Windows 2025 | ZIP |
| macOS Apple Silicon | `aarch64-apple-darwin` | macOS 15 ARM64 | tar.gz |
| macOS Intel | `x86_64-apple-darwin` | macOS 15 Intel | tar.gz |
| Linux x64 | `x86_64-unknown-linux-musl` | Ubuntu 24.04 x64 | tar.gz |
| Linux ARM64 | `aarch64-unknown-linux-musl` | Ubuntu 24.04 ARM64 | tar.gz |

Linux uses musl to avoid a dependency on the build runner's glibc version.
The optional `db` feature is not included. Windows ARM64 is not built. Binaries
are not Authenticode-signed or Apple Developer ID-signed/notarized; operating
systems can therefore show an unrecognized-publisher warning. The unsigned
desktop app packages are described in [Desktop app (unsigned)](#desktop-app-unsigned).

## Build and security flow

Only `v*` tags and manual runs build all variants; pushes to `main` and pull
requests run CI and Security only, because the macOS and Windows runners are
slow and expensive.
Every native binary must pass its version/help, real dependency-analysis and
HTML-export smoke test. Packaging includes the binary, README, this guide,
project licenses, Cargo.lock and `release-info.json` (version, target, commit,
features and binary SHA-256). Each archive has its own SHA-256 file.

An independent Linux job downloads **all five** archives, rejects missing or
unexpected files, verifies checksums, commit/version/target metadata and exact
archive contents, and safely extracts only regular expected files. ClamAV with
fresh signatures scans both the final archives and their extracted contents.
Detections, scanner errors and scan limits fail the job. Only after success is
`release-scanned` uploaded with the archives and `SHA256SUMS` as an internal
verified handoff. For downloads, use **release-bundle**: it contains
`archguard-<version>-release-bundle.zip` and its SHA-256 checksum. The short-lived `release-candidate-*` artifacts are internal,
not yet malware-cleared build outputs.

Tag runs also call the existing complete CI and Security workflows for the same
revision (Rust tests/clippy, dependency advisories, source malware and Git-history
secret scanning). The release job requires **all** of these gates. It downloads
the scanned bundle and re-verifies its contents and checksums before uploading;
it never rebuilds binaries after the scan.

For merge protection, require the CI and Security checks in the repository
ruleset. **Verify and scan all release artifacts** does not run on pull requests,
so it cannot be a required merge check. The workflow does not change GitHub
branch-protection settings.

## Create a version

1. Set the workspace version in `Cargo.toml`, update `Cargo.lock` if required,
   and merge the reviewed change through the normal CI process.
2. Create and push the matching tag, for example `v0.1.0`. A mismatched tag fails.
3. Inspect the successful **Release builds** run. It creates a **draft** GitHub
   release with the five scanned archives, `SHA256SUMS`, the report ZIP, and the
   scanned desktop packages with `SHA256SUMS-desktop`.
4. Review the draft's notes and assets, then publish it in GitHub.

Manual builds only produce workflow artifacts; they do not create a
GitHub release. Only the final tag-release job has `contents: write`; scanners
and build jobs have read-only permissions. Re-running a tag whose draft/release
already exists fails instead of silently replacing its assets. Deliberately
remove the draft first if you need to regenerate it.

## Use an archive

Choose your OS and CPU, verify the archive against `SHA256SUMS`, and extract it.
Run `archguard --version` (`archguard.exe --version` on Windows), then
`archguard serve /path/to/project --port 8787`. The viewer is embedded. External
compiler and scanner requirements still depend on the analyzed language and KPI;
see the viewer's help and `archguard doctor`.

SHA-256 checksums verify consistency with the downloaded manifest; they are not
a publisher signature. A successful malware scan is not a guarantee of safety.

## Desktop app (unsigned)

The same workflow packages the native desktop app (`crates/archguard-desktop`):

| Platform | Rust target | Runner | Package |
| --- | --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | macOS 15 ARM64 | `ArchGuard-<version>-aarch64-apple-darwin.dmg` |
| macOS Intel | `x86_64-apple-darwin` | macOS 15 Intel | `ArchGuard-<version>-x86_64-apple-darwin.dmg` |
| Windows x64 | `x86_64-pc-windows-msvc` | Windows 2025 | `.msi` installer and a plain `.zip` |
| Linux x64 | `x86_64-unknown-linux-gnu` | Ubuntu 24.04 x64 | `.AppImage` |

`scripts/release/package_desktop.py` builds the packages; its unit tests cover
the Info.plist, `.desktop` and WiX source, the file names and the complete-set
check. The `.dmg` holds `ArchGuard.app` and an `/Applications` link. The `.msi`
(WiX 5.0.2) installs to `Program Files\ArchGuard` with a Start menu shortcut. The
AppImage is built with appimagetool 1.9.1 and type2 runtime 20251108, both
pinned and checked by SHA-256. Linux uses glibc (Slint needs fontconfig), not
musl: the AppImage needs glibc 2.39 or newer and the system's fontconfig.

Each desktop job checks its package without starting the app: the `.dmg` is
mounted and its app checked with `plutil -lint` and `codesign --verify`, the
`.msi` is unpacked by an administrative install, the AppImage is extracted and
its `.desktop` file validated. The scan job then rejects missing or extra
packages, verifies every `.sha256`, writes `SHA256SUMS-desktop` and runs ClamAV
over all packages and the extracted Windows ZIP. ClamAV unpacks the `.dmg` and
`.msi` as far as it supports these formats; the AppImage's SquashFS is scanned
as one file. `archguard update` updates the CLI only, never the desktop app.

The desktop packages are **not signed**: there is no Apple Developer ID, no
notarization and no Windows code-signing certificate. The macOS app has only an
ad-hoc signature, which Apple Silicon needs to start it at all. On first start:

- **macOS:** drag `ArchGuard.app` to Applications, then right-click it and choose
  **Open**, and confirm. Or remove the quarantine flag:
  `xattr -dr com.apple.quarantine /Applications/ArchGuard.app`.
- **Windows:** when SmartScreen shows "Windows protected your PC", click
  **More info**, then **Run anyway**.
- **Linux:** `chmod +x ArchGuard-<version>-x86_64-unknown-linux-gnu.AppImage` and
  run it.

The app links Slint under the Slint Royalty-free License 2.0; the attribution it
requires is shown in the app's About dialog.

## Update an installed binary

`archguard update --check` reports whether a newer release exists;
`archguard update` installs it in place. It reads the latest **published**
release from the GitHub API (drafts and pre-releases are invisible to it), picks
the archive for the target the binary was built for, verifies the archive
against `SHA256SUMS` and the binary against `release-info.json`, and only then
replaces the running executable. Any mismatch leaves the installed binary
untouched and exits with code 3. A binary built for a target without a release
archive (for example `*-linux-gnu` from `cargo build`) cannot update itself.
`ARCHGUARD_UPDATE_API` replaces `https://api.github.com`, e.g. for a mirror.

## Report ZIP

The `release-bundle` artifact and the tag release draft include one ZIP containing:

- `report.html`: readable offline report, also printable from the browser.
- `report.json` and `report.txt`: structured and plain-text outcomes.
- `logs/release-malware.log` and `logs/release-signatures.log`, when produced:
  scanner version, signature update and the full scan summary.
- `packages/`: the five unchanged, verified platform archives, their SHA-256
  files and `SHA256SUMS`, only when the applicable gates permit inclusion.
- `CONTENTS.sha256`: checksums for the ZIP contents. The ZIP itself has a separate
  `.zip.sha256` file alongside it.

The report records commit, UTC generation time, platform targets, job outcomes,
artifact hashes and a link to the full pipeline/build logs. Individual platform
build/smoke success is confirmed only when the entire native build matrix passed;
failed matrix runs require inspecting their linked job logs for individual results.
ClamAV scans the platform archives and extracted binaries before the report ZIP
is assembled; the report states this scope explicitly.

On manual runs, the tag-only CI/security jobs are explicitly marked
**not run**, not passed. If any gate failed/cancelled, the ZIP contains only a
diagnostic report and available logs, without binaries. A failed scan before log
creation is recorded as failed without inventing a scan result. Release creation
still requires every original gate **and** successful report packaging.
