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
   - **Linux:** Landlock (the Landlock maintainers' `landlock` crate),
     applied by the child between fork and exec. It needs ABI 4 (kernel
     6.7+, file and TCP rules); an older kernel counts as unavailable.
     `/proc` and `/sys` are readable (rustup's proxies find themselves
     through `/proc/self/exe`); Landlock forbids ptrace on processes
     outside the domain, so other processes' `/proc/<pid>/environ` stays
     closed. Rules that cannot be built make the start fail, never run
     the tool unconfined.
   - **Windows:** an AppContainer (`ArchGuard.Confined`, no network
     capability) through the starter `archguard-confine.exe`, because
     Rust's `Command` cannot start AppContainer processes yet
     (`raw_attribute` is unstable). The starter grants the container
     read/execute on the toolchain, read on the project and write on the
     scratch directory (an inheritable ACE, added once and kept), starts
     the tool in a kill-on-close job object and passes its exit code on.
     The engine finds the starter next to the running program (or one
     directory up); without it Windows counts as unconfined.
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
- Linux kernels before 6.7, and Windows installations without
  `archguard-confine.exe` next to the program, get no runtime scenarios
  for Rust and TypeScript unless the variable is set. The release
  packages do not ship the starter yet.
- On Windows the AppContainer's rights stay on the toolchain, project and
  temporary directories after the analysis.
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
- Linux (Docker Desktop, kernel 7.0, Debian bookworm image): the same
  tests plus "another process' /proc/<pid>/environ is closed, its own
  /proc readable"; Node confined; rust-analyzer confined resolves the 7
  calls of the `rs-clean` copy with the `build.rustc` and toolchain traps
  in place, the trap script never ran. Clippy clean on Linux.
- Programs are started under the path they were found at (not the
  symlink target): rustup's proxies choose the tool by that name.
- Windows: compiles and is clippy-clean for `x86_64-pc-windows-gnu`
  (checked in the Linux container); the Windows tests in
  `compiler_sandbox.rs` and the `sandbox.yml` workflow (Windows, Linux,
  macOS) have **not run yet**: the branch was merged into main at the
  user's request (2026-11-08) without a push. Run on Windows:
  `cargo build -p archguard-engine --bin archguard-confine` then
  `cargo test -p archguard-engine --lib compiler_sandbox`.

## History

- **2026-11-09:** Holes from a security audit closed (branch
  `fix/sandbox-holes`):
  - **Home directory.** A tool in `~/bin` made the home directory "the
    installation it comes from", so `~/.ssh` and the rest of HOME were
    readable and executable. When that directory is HOME or above it
    (compared as real paths), only the program's own directory is
    granted.
  - **Namespaces on Linux.** Landlock's network rules (ABI 4) cover TCP
    only. The child now also moves into a new user and network
    namespace (`unshare(CLONE_NEWUSER | CLONE_NEWNET)`, before Landlock),
    so UDP and the host's abstract Unix sockets are closed as well. Where
    unprivileged user namespaces are off (probed once at start; e.g.
    Docker's default seccomp profile), Landlock alone confines the tooling
    as before and a warning says once on stderr that UDP and abstract
    sockets stay open. Linux still counts as confined then.
  - **Environment.** Confined tools no longer inherit the environment
    (tokens, agent sockets such as `SSH_AUTH_SOCK`). They get only `PATH`,
    `HOME`, `TMPDIR`/`TMP`/`TEMP`, `RUSTUP_HOME`, `CARGO_HOME`,
    `RUSTUP_TOOLCHAIN`, `LANG`, `LC_ALL` and, on Windows, `SystemRoot`,
    `windir`, `USERPROFILE`, `LOCALAPPDATA`, `PATHEXT`, `ComSpec`; the
    pinning and the callers set their own variables on top.
  - **Version probe.** `rust-analyzer --version` ran unconfined, on the
    repository's toolchain, in the current directory and without a time
    limit. It now runs like the index: confined, pinned, in the scratch
    directory, under the same time and output budget.
  - **Verified:** the tests in `compiler_sandbox.rs` and `rust_scip.rs` on
    macOS, and on Linux in Docker (kernel 7.0, Debian bookworm) both with
    namespaces (`seccomp=unconfined`) and without them (default seccomp,
    Landlock-only fallback); rust-analyzer confined resolves the calls of
    `rust_calls`' test on macOS and in both Linux modes. Clippy clean.
  - **Remaining gap:** on Linux, pathname Unix sockets (for example
    `/var/run/docker.sock` or an ssh-agent socket) stay reachable: neither
    Landlock ABI 4 nor a network namespace restricts connecting to them;
    clearing the environment only hides their names. Windows (allowlist,
    starter) is still untested.
