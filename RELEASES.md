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
systems can therefore show an unrecognized-publisher warning.

## Build and security flow

Pushes to `main`, pull requests, manual runs and `v*` tags build all variants.
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

For merge protection, require **Verify and scan all release artifacts** alongside
the existing CI/security checks in the repository ruleset. The workflow does not
change GitHub branch-protection settings.

## Create a version

1. Set the workspace version in `Cargo.toml`, update `Cargo.lock` if required,
   and merge the reviewed change through the normal CI process.
2. Create and push the matching tag, for example `v0.1.0`. A mismatched tag fails.
3. Inspect the successful **Release builds** run. It creates a **draft** GitHub
   release with the five scanned archives, `SHA256SUMS`, and the report ZIP.
4. Review the draft's notes and assets, then publish it in GitHub.

Manual/branch/PR builds only produce workflow artifacts; they do not create a
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

On branch/PR/manual runs, the tag-only CI/security jobs are explicitly marked
**not run**, not passed. If any gate failed/cancelled, the ZIP contains only a
diagnostic report and available logs, without binaries. A failed scan before log
creation is recorded as failed without inventing a scan result. Release creation
still requires every original gate **and** successful report packaging.
