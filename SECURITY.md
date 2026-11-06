# Dependency review

Run `cargo audit --file Cargo.lock` (CI installs cargo-audit 0.22.2 with
`--locked`). CI also runs weekly, even without source changes. Dependabot
checks Cargo and action pins weekly. No advisory IDs are ignored.

Review on 2026-11-06 (branch `chore/ladybugdb`): 659 locked packages,
RustSec database commit `26fd2d43f7280a1fb925a3743ec463017afe7119`,
cargo-audit 0.22.2, zero vulnerability-class findings, seven informational
**unmaintained** warnings: RUSTSEC-2025-0141 (`bincode`), RUSTSEC-2026-0441
(`pin-utils`), RUSTSEC-2026-0192 (`ttf-parser`), RUSTSEC-2024-0449
(`unsafe-libyaml`), RUSTSEC-2026-0370/-0371/-0372 (`zvariant`,
`zbus_names`, `zvariant_derive`). All seven are also in the lock of the
previous commit; none is a dependency of `archguard-store` (checked with
`cargo tree -p archguard-store --features db --target all`). They are not
assessed here.

Optional `db` backend (`lbug =0.22.1`, the official Rust crate of
[LadybugDB](https://github.com/LadybugDB/ladybug), MIT):

- [RUSTSEC-2026-0202](https://rustsec.org/advisories/RUSTSEC-2026-0202.html)
  (`cxx::let_cxx_string!` before 1.0.195 is unsound) no longer applies. The
  former `kuzu =0.11.3` backend pinned `cxx =1.0.138`; lbug requests `cxx` and
  `cxx-build` as `^1.0.138`, and the lock now holds both at 1.0.205.
- lbug's build script, unless told otherwise, downloads the newest
  prebuilt `liblbug` release from GitHub, without a checksum and independent
  of the pinned crate version. `.cargo/config.toml` sets
  `LBUG_BUILD_FROM_SOURCE=1`, so the native library is built from the C++
  sources inside the pinned, checksummed crate, as with kuzu before.
- The source build needs CMake and OpenSSL 3.x headers and libraries
  (`find_package(OpenSSL 3)`; OpenSSL 4 is rejected). The macOS DB build of
  the CLI ends up linking neither `libssl` nor `libcrypto` (checked with
  `otool -L` and `nm`); Linux and Windows are not checked. The slim build
  does not enable lbug, cxx or OpenSSL.
- Real LadybugDB integration tests pass, but tests alone do not prove the
  absence of memory-safety defects in the C++ engine.

Action SHAs were resolved from their upstream Git repositories on 2026-10-27.
See [checkout releases](https://github.com/actions/checkout/releases) and
[cargo-audit releases](https://github.com/rustsec/rustsec/releases).

## Pipeline security checks

The `Security` workflow runs on pull requests, pushes to `main`, every Monday,
and manually via **Run workflow**. Its independent jobs complement the existing
RustSec `cargo audit` check in `CI`:

| Check | Scope | Failure condition |
| --- | --- | --- |
| Malware / ClamAV | Exact committed tree (including fixtures and bundled files), then the newly built default Linux release CLI | Detection, scanner error, missing scan target, failed signature update, or exceeded size/recursion limits |
| Secrets / Gitleaks | All history reachable from fetched Git refs, including the PR checkout | Detected secret or scanner/install error |
| Advisory check / cargo audit | Rust dependencies in `Cargo.lock` | Vulnerability finding or audit error; informational advisories remain visible as described above |

ClamAV comes from Ubuntu's signed packages; `freshclam` updates its signatures
on each run. Archives are inspected. The per-file limit is 200 MiB and the
expanded per-container limit is 500 MiB; exceeding a limit is an alert rather
than a silent pass. Symlinks are not followed. The source scan uses `git archive`
so Git internals, untracked files and local caches are outside its scope. The
regular Security workflow scans the default Linux CLI. The separate
[Release builds workflow](RELEASES.md) builds Windows, macOS (Intel/Apple Silicon)
and Linux (x64/ARM64), verifies the complete artifact set, and scans every final
archive and extracted binary before creating a release draft. Optional DB builds,
external downloads and container images are not included.

Gitleaks 8.30.1 is downloaded with a pinned SHA-256 checksum. Output is fully
redacted; no raw secret report is uploaded. Rotate/revoke a real leaked credential
even if it has since been deleted from the current tree. Do not add a blanket
history baseline or disable a gate to hide findings; investigate false positives
and document narrowly scoped exceptions in review.

No job uses `continue-on-error`, write permissions or repository secrets.
To make merge blocking mandatory, require **Malware / ClamAV**,
**Secrets / Gitleaks**, and the existing **fmt / clippy / test** check in the
repository's branch protection/ruleset. Workflow files alone do not configure
that GitHub setting.

Local equivalents (install the tools and update ClamAV signatures first):

```sh
bash scripts/release/scan_malware.sh path/to/source-copy path/to/archguard
# Run from the repository root; findings are redacted.
gitleaks git . --log-opts="--all" --redact=100 --no-banner --exit-code=1
cargo audit --file Cargo.lock
```

A clean scan is not proof that software is harmless. Malware signatures, secret
patterns, dependency advisories and the existing tests address different risks.
See the [ClamAV scanning documentation](https://docs.clamav.net/manual/Usage/Scanning.html)
and [Gitleaks CLI documentation](https://github.com/gitleaks/gitleaks#usage).
