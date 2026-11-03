# Dependency review

Run `cargo audit --file Cargo.lock` (CI installs cargo-audit 0.22.2 with
`--locked`). CI also runs weekly, even without source changes. Dependabot
checks Cargo and action pins weekly. No advisory IDs are ignored.

Review on 2026-10-27: 169 locked packages, RustSec database commit
`3279c99911354c400e1d41e47c7d60e39258b147`, zero vulnerability-class findings,
one informational **unsound** advisory:

- [RUSTSEC-2026-0202](https://rustsec.org/advisories/RUSTSEC-2026-0202.html)
  affects `cxx::let_cxx_string!` before 1.0.195: a panic during initializer
  evaluation can drop uninitialized storage.
- The optional `kuzu =0.11.3` backend pins `cxx =1.0.138`; its matching
  `cxx-build` pin is needed for compatible generated bridge symbols.
- Decision: retain these pins for this release. Neither workspace code nor
  the locked kuzu Rust sources/build script calls `let_cxx_string` or
  `StackString` (checked with `rg`); this advisory's affected API has no call
  site in our DB integration. The slim build does not enable kuzu/cxx.
  This is a scoped reachability assessment, not a claim that cxx is fixed.
- Keep the warning visible in audit output. Reassess when upgrading kuzu/cxx
  or adding bridge code; prefer a backend release using cxx >=1.0.195 with
  matching generated bindings. Real Kuzu integration tests pass, but tests
  alone do not prove absence of memory-safety defects.

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
