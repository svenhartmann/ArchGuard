# ADR 0009: Generated Slint code in its own crate; engine stays one crate

- **Status:** accepted
- **Date:** 2026-11-10

## Context

Two things looked like structural problems on 2026-11-09:
`archguard-engine` (about 52,800 lines in 73 modules) and desktop files
that change with almost every feature. A core with plugins and hooks was
considered first and rejected. Language scanners already have a registry
(`archguard-extract` `extract::registry`), external scanners and probes
already run as separate processes, and no consumer for hooks exists.

The first draft of this ADR proposed splitting the engine into `scan`,
`calls`, `services` and `quality` crates. Step 0 of that draft (measure
first) refuted its premise.

### Measurements (2026-11-10, commit 03ca09c, 16 cores, dev profile)

`cargo build --workspace` after one real code change, median of three
runs:

| Change in | Today | Engine alone | With UI crate (prototype) |
| --- | --- | --- | --- |
| `desktop/src/state.rs` | 16.0 s | – | **1.9 s** |
| `engine/src/services.rs` | 20.1 s | 1.4 s | **3.5 s** |
| `desktop/ui/theme.slint` | 16.7 s | – | 17.6 s |

Further findings:

- **No-op builds took 18–22 s.** The desktop `build.rs` rewrote the
  merged `.po` file on every run, and Slint makes cargo watch that file,
  so every build recompiled the desktop crate. Fixed by writing the file
  only when its content changes: no-op 0.2 s, and `.po` edits still
  rebuild. This change is independent of the decision below.
- **The engine is cheap.** It recompiles in 1.4 s after a change. The
  rest of a rebuild is the desktop crate.
- **The desktop crate carries 34 MB of generated Rust** (`out/app.rs`
  from `ui/app.slint`). Any Rust change in the desktop, and any change in
  a crate it depends on, recompiles that code too.

The engine split would not reduce this. A change in any of the proposed
engine crates still rebuilds the desktop with its generated code.

## Decision

1. **Keep the `build.rs` fix** (write the merged `.po` only when it
   changed).
2. **Move the Slint UI into a crate `archguard-desktop-ui`.** It holds
   `ui/`, `lang/`, `build.rs` and a `lib.rs` with
   `slint::include_modules!()`. The desktop's `src/ui.rs` becomes
   `pub use archguard_desktop_ui::*;`, and the tab modules keep importing
   from `crate::ui`. The prototype compiled with no other source change.
3. **Do not split the engine for build time.** A split for
   compiler-enforced layers remains possible later. The layer analysis of
   the first draft (a DAG: `scan` ← `calls`, `services` ← `quality` ←
   `engine`; `go_calls` as the only misplaced hub with
   `execute`/`wait`/`Stop`) is in this file's git history.

### What the UI crate move must cover

Found in the prototype or by search:

- **Translation domain.** Slint looks for
  `<out>/lang/<lang>/LC_MESSAGES/<crate name>.po`, so `build.rs` writes
  `archguard-desktop-ui.po`.
- **Icon.** `app.slint` uses `../assets/icon.png`, and
  `src/main.rs`, `scripts/desktop/bundle-macos.sh` and
  `scripts/release/package_desktop.py` use
  `crates/archguard-desktop/assets/icon.png`. The icon stays where it
  is, and `app.slint` points to `../../archguard-desktop/assets/icon.png`.
- **Test-only `slint!` blocks** with `ui/...` imports in
  `security_check.rs`, `fields_check.rs`, `tree_width_check.rs`,
  `architecture_ui/check.rs`, `services_ui/tree_check.rs` and
  `start_ui/narrow_check.rs`, plus `texts_check.rs`, which reads
  `ui/texts.slint` and `lang/de`. They point to the new crate.
- **ArchGuard self-check.** It reports the same 20 unresolved
  `crate::ui::*` imports before and after the move: the scanner never
  sees generated Slint items. The move adds one edge
  (desktop → desktop-ui) and no finding.
- **Font-token test** in `platform.rs` reads `ui/` through
  `CARGO_MANIFEST_DIR` and points to the new crate.
- **Workspace.** Add the crate to `members` and commit `Cargo.lock`
  (the release builds with `--locked`). `slint-build` moves to the new
  crate. The `.archguard` layers do not list the desktop, so they stay
  as they are.

Done: `cargo test --workspace`, `cargo clippy --workspace
--all-targets` and `archguard check` green. The real app starts with
German texts and the icon, as does `--features snapshot`, and the
measurements above are repeated.

### Preconditions

`ui/` and `lang/` move as a whole. Uncommitted edits by other sessions
in `ui/*.slint` (on 2026-11-10: `architecture_visual.slint` from the
sequence work) and the desktop `Cargo.toml` (the UI-tests session) must
land first.

## Alternatives

- **Split the engine into layered crates** (first draft). Rejected for
  build time by the measurement; see Decision 3.
- **Core with plugins and hooks.** Rejected, see Context.
- **Smaller `app.slint`** (moving root properties into `GraphData`,
  `MatrixData` and `ShellData` globals). This reduces merge conflicts in
  `app.slint`, not build time. It is worth doing separately if those
  conflicts recur.

## Consequences

- Rust changes in the desktop rebuild in about 2 s instead of 16 s.
  Engine changes rebuild in 3.5 s instead of 20 s.
- `.slint` changes cost as much as today.
- One more crate. UI files move from `crates/archguard-desktop/ui` to
  `crates/archguard-desktop-ui/ui`, which open branches that edit them
  must rebase over.

## History

- **2026-11-10:** Implemented on branch `refactor/desktop-ui-crate`.
  `cargo test` for both desktop crates (533 tests), `cargo clippy
  --workspace --all-targets` and `archguard check` pass. A German
  snapshot of the real app (`--features snapshot`,
  `ARCHGUARD_DESKTOP_LANG=de`) shows translated texts. Measured again:
  a change in `state.rs` rebuilds in 1.9 s, and a change in `services.rs`
  in 3.7–5.1 s.
