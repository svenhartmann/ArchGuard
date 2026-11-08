# ADR 0007: Confined project compiler tooling

- **Status:** accepted
- **Date:** 2026-11-08

## Context

Call resolution for Rust and TypeScript runs tooling that reads the
repository's own configuration: rust-analyzer (through cargo and rustup)
and the project's installed TypeScript compiler, which is JavaScript from
the repository. A repository can therefore start programs of its own:
`rust-toolchain.toml` (a path toolchain), `.cargo/config.toml`
(`build.rustc`), or any code in `node_modules/typescript`. Until now this
ran only when the operator set `ARCHGUARD_TRUST_PROJECT_COMPILER=1`, and
then unconfined.

Already in place before this ADR: build scripts and proc macros are off
for rust-analyzer, cargo is offline, the rustc wrapper variables are
cleared, and `NODE_OPTIONS` / `NODE_PATH` are removed.

The desktop app was changed to trust project compilers by default
(ADR 0002, 2026-11-08) so that arc42 chapter 6 shows runtime scenarios;
the user then asked whether the compiler could be restricted to the
project like in a sandbox.

## Decision

1. **Confinement** (`archguard-engine/src/compiler_sandbox.rs`). The
   tooling runs under the platform sandbox where one exists. The profile
   denies everything and allows: reading the system locations, the
   toolchain (`~/.rustup`, `~/.cargo`, `RUSTUP_HOME`, `CARGO_HOME`, the
   installation the program comes from) and the project; writing only the
   analysis' scratch directory (and `/dev/null`); starting programs only
   from system and toolchain locations. There is no network rule, so no
   network. File metadata stays readable.
   - **macOS:** `sandbox-exec` with a generated profile (Apple lists the
     tool as deprecated; it is what Bazel uses on macOS).
   - **Linux, Windows:** not confined yet (Landlock or bubblewrap on
     Linux could follow; nothing comparable without AppContainer on
     Windows).
2. **Pinned Rust toolchain.** rust-analyzer runs with
   `RUSTUP_TOOLCHAIN` set to the user's default toolchain (asked from the
   home directory, not the repository) and `RUSTC` /
   `CARGO_BUILD_RUSTC` set to its rustc, so `rust-toolchain.toml` and
   `build.rustc` from the repository do not apply. Without rustup the
   environment stays as it is.
3. **Trust.** The environment variable keeps working everywhere (the
   operator decides; confinement applies where available). The embedding
   application's switch (`trust_project_compiler`, used by the desktop by
   default) counts only where the tooling runs confined; elsewhere the
   tooling stays off unless the variable is set.

## Alternatives

- **Trust without confinement** (the desktop default before this ADR):
  an untrusted repository opened in the app could run commands with the
  user's rights.
- **A container or VM:** strongest isolation, but a heavy dependency for
  a desktop tool and slow to start per analysis.
- **Only pinning the toolchain:** closes the two Rust configuration
  paths, not the TypeScript compiler, which is repository code.

## Consequences

- On macOS, opening a repository in the app runs its tooling confined:
  it cannot read other directories (SSH keys, tokens, other projects),
  write outside its scratch directory, reach the network or start
  programs from the repository. It still runs repository code
  (TypeScript compiler) inside these limits.
- Linux and Windows users of the app get no runtime scenarios for Rust
  and TypeScript unless they set the variable.
- The profile is maintained by hand; a toolchain that needs another
  read location fails to resolve calls instead of reading more.

## Verification (2026-11-08)

- Tests in `compiler_sandbox.rs` (macOS): the project is readable, another
  directory is not; writing works only in the scratch directory; a local
  TCP listener is unreachable; a program copied into the project does not
  start; Node runs from its own installation and reads the project but
  not another directory.
- rust-analyzer confined: 7 calls in a copy of `fixtures/rs-clean`
  (with a Cargo manifest), 47,800 on this repository, in the same 31 s as
  unconfined.
- A copy of `rs-clean` with `.cargo/config.toml` `build.rustc =
  "./evil.sh"` (which would create a marker file) and a
  `rust-toolchain.toml` naming a missing toolchain: the script never ran,
  calls still resolved.
- Not checked: the project's TypeScript compiler confined (none
  installed locally; only Node itself was tested).
