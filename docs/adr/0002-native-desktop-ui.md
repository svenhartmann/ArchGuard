# ADR 0002: Native desktop UI with Slint

- **Status:** accepted
- **Date:** 2026-11-05

## Context

ArchGuard has one interactive UI: the HTML viewer in
`crates/archguard-report/src/viewer*.js` (about 11,000 lines, 10 tabs,
116 tests in `scripts/viewer/`). It runs in the browser, through
`archguard serve`, and as a self-contained HTML report.

We want a native desktop app for macOS, Windows and Linux. It must look
polished and offer the same layout and functions as the web viewer. The
web viewer stays: it remains the format for shared reports, CI artifacts
and PR comments.

A prototype on branch `feat/slint-desktop` (`crates/archguard-desktop`)
shows the module graph with layered layout, selection, detail panel,
search and zoom. It runs the same cached pipeline as the CLI
(`analyze_run_quiet`).

## Decision

1. **Toolkit: Slint** (1.18), used under the Slint Royalty-free License
   2.0. The app shows the `AboutSlint` widget in its About dialog, as that
   license requires. ArchGuard's own code stays `MIT OR Apache-2.0`; only
   `archguard-desktop` depends on Slint.
2. **Same UI as the web viewer.** The desktop app uses the web viewer's
   tab structure (Start, Architecture, Dashboard | Graph, Matrix, Calls,
   Services | Quality, Security, Review), its explorer (Modules, Cycles,
   Review), its breadcrumbs, and its light and dark color tokens
   (`viewer.html`, `:root`). Both languages (de, en) are supported.
3. **One logic, two renderers.** Both UIs show data computed in Rust.
   Logic that today lives in the viewer's JS (for example
   `viewer_call_cycles_model.js`, `viewer_tree_layout.js`) moves into a
   shared Rust crate when the desktop app needs it, and the web viewer
   reads the result from the report payload. No metric is computed twice.
4. **Missing views link to the web viewer.** Until a tab exists natively,
   it shows a placeholder with "Open in web viewer", which starts
   `archguard serve` for the same project. Every function stays reachable
   from the first release.

## Plan

Effort figures are estimates for one developer, not measurements.

| Phase | Content | Estimate |
|---|---|---|
| 1 | Shell: tabs, explorer, breadcrumbs, web color tokens (light/dark), de/en, web viewer fallback. Graph tab at web parity: drill-down, incoming/outgoing colors, cycles explorer, edges routed around cards. | 1–2 weeks |
| 2 | Matrix (DSM) | 1 week |
| 3 | Dashboard and Quality | 2–3 weeks |
| 4 | Architecture and arc42 | 2–3 weeks |
| 5 | Calls and Services | 2–3 weeks |
| 6 | Security and Review | 2 weeks |
| 7 | World view | 3–4 weeks |
| 8 | Release: signed builds for macOS, Windows, Linux in CI | 1 week |

Each phase ends with unit tests for the moved logic and a screenshot
comparison of the native view against the web view on the same project
(the app's `snapshot` feature renders the window to PNG).

## Alternatives

- **Tauri around the existing viewer.** Cheapest (2–3 weeks) and reuses
  all views, but the UI stays HTML in a system web view. Rejected: the
  goal is a native renderer.
- **egui.** Pure Rust and fast, but its look is that of a tool UI.
  Rejected for the visual goal.
- **GPUI with gpui-component.** Best look and speed (Zed), but no stable
  standalone release; apps depend on weekly snapshots. Rejected for now.
- **Qt.** Closest to native widgets, but C++ with awkward Rust bindings
  and LGPL or commercial licensing. Rejected.
- **One native UI per platform.** About three times the effort. Rejected.

## Consequences

- Two UIs need maintenance. Every new view or change in the web viewer
  needs a matching native change, or a documented gap with a fallback
  link.
- Logic moves from JS into Rust over time; this also makes it testable
  without a browser.
- Distributing the app requires the Slint attribution and code-signing
  certificates (Apple Developer Program, Windows code signing).
- A fork that ships the desktop app must keep the Slint attribution.
- Windows and Linux have not been tested yet; the prototype was checked
  on macOS only.
