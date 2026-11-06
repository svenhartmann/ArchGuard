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

## History

- **2026-11-05:** Phase 1 implemented on `feat/slint-desktop`. The Graph
  tab offers the web viewer's Tree, Dependencies and Network displays;
  the layered prototype view was dropped in favour of Network. Opening a
  package expands its path instead of re-rooting the scope, as the web
  viewer's `navigateExplorerNode` does. The tree order ports the web's
  construction levels and crossing search, and open single-child chains
  are compressed into one row; on this repository scope counts, ACD,
  filter counts, priorities and row order match the web viewer.
  Known deviation: the Explorer shows compressed chains as one row too.
- **2026-11-05:** Phase 2 (Matrix) implemented. `buildTreeMatrix` and
  `treeMatrixRows` are ported to Rust (`matrix.rs`); on this repository
  file, evidence, row, column and cell counts match the web viewer.
  Column headers add the parent path under the short name. Not yet
  native: the "Jump to target" list and opening a row in the inspector.
  The app has an icon (`assets/icon.svg`, PNG for the macOS Dock);
  signed bundles with `.icns` remain part of phase 8.
- **2026-11-05:** Phase 4 (Architecture) implemented: arc42 chapters from
  `arc42_document`, the web's arc42 Markdown and package maps ported to
  Rust; Markdown of all 12 chapters is byte-identical to the web.
  Open: chapter 7 deployment map, chapter 4 component model, the target
  architecture section; the diagram uses the web's fallback grid layout,
  not Graphviz.
- **2026-11-05:** Scope extended by the user. Additional decisions:
  5. **Documents.** Every open project is a document tab with its own
     analysis and view state. For a monorepo the app offers its projects
     (`analyze_services`) to open as tabs.
  6. **Panes.** The content area is a split tree of panes; each pane has
     view tabs ("Graph · shop-api"). Views take their data as properties
     of one model struct per view, so the same view kind can show two
     projects side by side. Tab state lives in one Rust struct per view
     and document.
  7. **Settings.** Global settings in the user config directory
     (`desktop.toml`); per-project personal settings there keyed by
     project path; team-shared settings (dashboards) in `archguard.toml`
     under `[desktop]`. Project overrides global.
  8. **Configuration editor.** A view with a schema-driven form
     (`schemars` from the config structs) and the TOML text side by side,
     provenance per value (declared/derived/default), graphical editors
     for layers, dependency rules, thresholds and globs, a preview of the
     changed findings before saving, and comment-preserving writes
     (`toml_edit`). The same editor edits desktop settings and the
     dashboard designer.
  Revised order: documents, panes, settings, configuration editor,
  dashboard with designer, then Calls, Services, Security, World.
- **2026-11-05:** Renderer measured (release build, this repository,
  `SLINT_DEBUG_PERFORMANCE=refresh_full_speed`, 120 Hz display): FemtoVG
  over OpenGL (Slint's default, GPU) 120 fps on the Start world and on
  the Graph tree with `archguard-engine/src` open; Skia over Metal
  112–120 fps. Decision: keep FemtoVG. Skia (`slint/renderer-skia`) stays
  an option if large repositories hit the limit; it adds ~10 MB and a
  prebuilt-binary download to the build. Interaction cost is CPU-side.
- **2026-11-06:** Key figures and presentation unified across all tabs
  (`feat/desktop-kpis`). Each figure is defined once in `src/kpi.rs`
  (label, scope note shown on hover, precision, unknown policy), numbers
  are formatted only in `src/format.rs` (Slint calls it through the `Fmt`
  global), statuses come from one `Status` (`src/status.rs`) and class
  ratings from one module (`src/rating.rs`, replacing the Quality and
  Start copies). Theme tokens gained a rating scale (`rate-*`), a
  severity scale (`sev-*`), `warning`, `danger`, `success` and
  `priority-text`, all with dark variants; `cycle` marks cycles only.
  Shared widgets in `ui/widgets.slint` (StatCard/StatInline, Badge,
  SeverityChip, RatingChip, Legend, StatusLine, EmptyState,
  SectionHeading, PageTitle, Disclosure, ZoomControls) replace the
  per-tab copies. Decisions by the user:
  1. **KPI levels.** A figure's name states the level it is counted on
     ("ACD (shown level)", "ACD (module level)", "Package cycles", "File
     cycle groups", "Cycle groups (shown level)", "Area cycle groups");
     values stay as computed. ACD has two decimals everywhere. arc42 text
     comes from the engine; the desktop names its level around it
     (chapter 10) and, in an English window, notes that the document is
     German.
  2. **Numbers follow the window language:** German `4,11` and `2.374`,
     English `4.11` and `2,374`, one formatter for the whole app.
     Unknown values read `—`, never `0`.
  3. **Projects are build units** (`documents::build_units`: manifest
     projects and local Compose build contexts below the opened folder,
     one per path). The header chip, its menu and Services count the
     same set; Services labels its manifest projects separately.
  4. **German UI uses the infinitive** ("Paket auswählen …"), no
     du-form; glossary: dependency = Abhängigkeit, call connection =
     Aufrufverbindung, service relation = Dienstbeziehung, cycle group =
     Zyklusgruppe. Counted nouns use plural forms (`@tr` plurals in Slint,
     `Texts::plural` in Rust); uppercase headings come from
     `SectionHeading`, not from the text.
