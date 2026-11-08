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
- **2026-11-06:** Decision 6 (panes) implemented on `feat/desktop-panes`,
  left/right only. Each document has a layout of one or two panes with
  view tabs (`src/panes.rs`). The view bar shows a view in the focused
  pane; its context menu and View > "Open in new tab" add a view tab;
  "Open to the side", View > "Split right" (⌘\ / Ctrl+\) and the pane
  strip's button open a right pane; a divider sets the ratio (each pane
  at least 380 px); closing a pane or its last tab returns to one pane.
  A node selected in the Graph or in a Matrix cell is revealed in the
  view beside it if that view can reveal nodes (Graph, Matrix); one link
  runs at a time, so a reveal never links back (`src/panes/link.rs`).
  "Open in graph" from Architecture, Start and Quality reveals in the
  pane that shows the Graph. The layout is stored per project in
  `desktop.toml` (`panes`, `pane_active`, `pane_focus`, `pane_ratio`
  under `[projects."<path>"]`, `src/pane_store.rs`) and restored with the
  session. Decision taken in the implementation, instead of state per
  pane: **a view is open at most once per document**, and its state
  stays one struct per view kind and document. The Slint views read
  their data from window-wide properties and globals (`StartData`, …),
  so two visible instances of one view would show, and write to, one
  state (for example the Start world's camera size). Opening an open
  view selects its tab; opening it to the side moves its tab. Two
  independent instances of one view first need the views' data as
  per-pane model structs (follow-up). One background analysis per
  document stays (`Shared`, `show_async`): panes only choose which built
  view is shown. Known limits: views with a fixed minimum width (Matrix
  640 px, the Graph's explorer and details panels) are clipped in a
  narrow pane; checked with headless snapshot builds on macOS only.
- **2026-11-06:** View instances (`feat/desktop-view-instances`), asked
  for by the user to compare views side by side. The rule "a view is
  open at most once per document" is replaced for Graph, Matrix and a
  new File view: a view tab is an instance with an id (`panes::Tab`),
  and "Open in new tab" / "Open to the side" always add an instance.
  Each instance keeps its own state (scope, expanded nodes, filters,
  display, selection) keyed by its tab id (`src/instances.rs`); the
  Slint views take their data as one struct per pane (`GraphData`,
  `MatrixData`, `FileData` in per-pane models) and report with the
  instance id, instead of window-wide properties. The package tree and
  the one background analysis per document stay shared. The **File
  view** shows one tree node (file or package): path, counts, incoming
  and outgoing dependencies grouped by the file on the other end with
  their evidence (`file:line  code`, `src/node_view.rs`), and "Open in
  graph"; it opens from the context menu of Graph explorer and tree rows
  and of Matrix rows (`ui/node_menu.slint`, which also offers a Graph
  opened on a package beside) and from the Graph's details panel. Tab
  titles name the instance: "Graph · graph_ui" (the package it was
  opened on), "Matrix · " and its scope, the File view's node. Linked
  selection works between instances (also two Graphs); a File view is
  no reveal target, clicking one of its dependencies reveals it beside.
  `desktop.toml` stores a tab as its view id or as
  `{ view = "graph", node = "…", display = 1 }`; restoring drops tabs
  of unknown views, File tabs whose node the analysis no longer has,
  and forgets a missing Graph or Matrix scope. **Not migrated:** Start,
  Architecture, Calls, Services, Quality and Security still read
  window-wide globals and stay open once per document (opening one
  selects its tab, opening it to the side moves it); moving them to
  per-pane structs is a follow-up. Scroll positions are kept per Slint
  view, not per instance: switching between two instances in one pane
  keeps the view's scroll offset. Checked with unit tests and headless
  snapshots on macOS only.
- **2026-11-06:** Export wired (`feat/desktop-export-follow`). File →
  Export (native menu bar) and a view tab's context menu ("Export") write
  the focused pane's view instance with its scope and filters through
  the UI-free export core (`src/export.rs`): Markdown + Mermaid and JSON
  data + evidence for Graph, Matrix, Services, Security, Quality and
  Architecture; arc42 documentation (the CLI's document) and a PNG of
  the pane (cropped from `Window::take_snapshot`, logical × scale) for
  every view; an SVG diagram for Graph → Tree and Services only, as in
  the web, which the menu disables with the web's hint otherwise. Start,
  Calls and the File view have no export snapshot yet, so Markdown and
  JSON are disabled there with a hint. The views hand over plain input
  structs (`view_export.rs` for Graph and Matrix, `services_ui/export.rs`,
  `quality_ui::export`, `security_ui::export_state`); the security filter
  and totals are shared with the tab (`security_ui/snapshot.rs`) instead
  of a copy in `export/`. The save dialog is `rfd`'s blocking
  `FileDialog::save_file`, as the existing "Open project" dialog
  (`documents_ui::pick_folder`), started in the project folder with the
  web's file name; the result shows in the notice bar in the web's
  wording ("Exported archguard-graph.svg with the current scope and
  filter settings."). Snapshot builds export without a dialog with
  `ARCHGUARD_DESKTOP_EXPORT=<format>:<path>[,…]`. Checked with unit tests
  and headless snapshot runs on macOS (fixtures/ts-cyclic,
  examples/service-shop); the save dialog itself was not opened.
- **2026-11-06:** "Follow left" (`feat/desktop-export-follow`), asked
  for by the user: in a split layout the right pane follows the left
  pane's navigation where the two views share a context, mapped by tree
  node name (`src/follow.rs`, `src/follow_ui.rs`). A chain toggle
  on the right pane's strip switches it; it is disabled, with its reason
  on hover, when the pair has no common context. Pairs: Start/World →
  Graph (entering a package, also by zooming in, opens and expands it in
  the Graph; leaving it, also by zooming out, collapses the Graph; a
  selected area or entity is revealed), Graph → Graph and Graph → Matrix
  (scope, expanded and collapsed packages), Graph or Matrix → File view
  (the selected node, a Matrix cell's row, becomes the File view's node),
  Architecture → Graph (the building-block map's scope and selected
  package). Selections between Graph and Matrix stay with the existing
  linked selection. Not supported: Matrix → Graph, anything → Calls,
  Services, Quality, Security, Start or Architecture. One direction only:
  views report their navigations (Graph and Matrix as the difference of
  scope, open packages and selection per callback; World and
  Architecture per wrapped callback); a navigation reported by the right
  pane switches Follow off, and the follow runs inside the link guard, so
  the right pane's own reports while it follows are ignored. Switching
  Follow on does not copy the left pane's current state; the next left
  navigation is followed. The toggle is stored with the layout
  (`pane_follow` in `desktop.toml`) and dropped when the split closes.
  Checked with unit tests and headless snapshots on macOS
  (fixtures/py-layers, fixtures/ts-cyclic; snapshot hooks
  `ARCHGUARD_DESKTOP_FOLLOW`, `…_LEFT_OPEN`, `…_LEFT_EXPAND`,
  `…_WORLD_OPEN`, `…_WORLD_BACK`, `…_ARC_OPEN` in `src/snapshot.rs`).
- **2026-11-06:** Settings view (`feat/desktop-settings-view`), the
  first view on the configuration editor core (`src/config_editor/`).
  View → Settings opens one tab per document ("Settings ·
  archguard.toml", view kind `settings`, not in the view bar). Left: the
  catalogue's 11 groups with a dot for edits (compared with the file as
  loaded or saved) or errors, and a search over field paths and
  descriptions that lists matches of all groups. Right: the group's form,
  one row per field with a widget by kind (switch, line input, choice,
  one entry per line for lists and `name = number` maps), the provenance
  (File, Default, Derived), Reset for written values, derived values
  read-only; table arrays as entry cards with add, remove and move,
  derived layers read-only after the declared ones. Input that is no value
  of its field stays as typed with its error; the engine's validation
  problem shows on the row it names (also when the engine puts the path
  only at the start of its message), else under the top bar. Top bar:
  status, Text (the raw TOML with the parse error and its line), Preview
  (`preview::run` on a worker thread; pass/fail and counts before and
  after, added and removed findings, the note that baseline and the
  `[review]` gates are not included), Revert and Save (disabled with
  errors). The editor is bound to the document, so edits mark it dirty
  and the existing "Save changes?" prompt saves it; a file changed on
  disk shows Reload and Overwrite. Text inputs are debounced and applied
  before any other action. Field descriptions and group names come from
  the catalogue and stay English for now. Snapshot hooks
  `ARCHGUARD_DESKTOP_SETTINGS=<group>[,text][,preview]` and
  `ARCHGUARD_DESKTOP_SETTINGS_EDIT`. Checked with unit tests and headless
  snapshots on macOS (temp copies of fixtures/ts-cyclic,
  fixtures/py-layers, examples/service-shop; light and dark, en and de);
  the view needs its import, route and menu entry in `ui/app.slint` and
  its label in `ui/panes.slint`, which the integration adds.
- **2026-11-06:** arc42 chapter editor (`feat/arc42-editor`), asked for by
  the user: the Architecture tab reads and edits the project's own arc42
  text instead of only showing the generated document. The text is found
  in the project (`src/arc42_file.rs`): the shallowest `arc42.md`, or an
  `arc42/` folder with one file per chapter named by its number
  (`05_bausteine.md`, `arc42-05.md`; a folder holding `arc42.md` is that
  file); four folder levels deep, hidden and build folders skipped. The
  reader shows the file's chapter with its generated blocks refreshed,
  and the generated chapter where the file has none. "Edit chapter" opens
  the chapter's Markdown beside a preview drawn as the reader draws it,
  updated on every edit. Saving writes the chapter atomically: text
  outside the `archguard:arc42` blocks stays as typed, the blocks are
  refreshed with `Arc42Document::merge` as `archguard doc --arc42` does
  (edits inside them are dropped), a broken marker refuses the save
  without touching the file, CRLF files stay CRLF; in a folder a missing
  chapter becomes `arc42-NN.md`. A project with neither asks on save:
  `arc42.md` in the project folder (the CLI's default), another place in
  the project (save dialog), or cancel. The choice is not stored in the
  settings; the search finds the file next time, which is why a place
  outside the project is refused. Snapshot hook `ARCHGUARD_DESKTOP_ARC_EDIT`
  (`1` opens the editor, `save` also saves). Checked with unit tests and
  headless snapshots on macOS (temp copy of fixtures/py-layers, with and
  without an `arc42.md` written by the CLI; en and de; a split pane, where
  editor and preview stack; a `docs/arc42/` folder, where a new chapter
  became `arc42-09.md`); not checked yet: typing in the editor and the
  save dialog. The editor names the file the chapter is saved to (in a
  folder the chapter file, also one a save would create).
- **2026-11-06:** Platform look (`feat/desktop-typography`), asked for by
  the user ("the type does not match macOS, so not Windows or Linux
  either"). Widget style per target in `build.rs`: Cupertino on macOS,
  Fluent elsewhere (`SLINT_STYLE` overrides). UI font per platform in
  `src/platform.rs`: "System Font" (San Francisco) on macOS, "Segoe UI"
  on Windows, fontconfig's sans-serif on Linux; monospace ".SF NS Mono"
  (SF Mono, checked by snapshot against Menlo and an unknown family),
  Consolas, "monospace". The window renders with Skia on macOS, which
  draws SF's weights (FemtoVG fell back to Helvetica); this replaces the
  FemtoVG decision of the measurement above for macOS only. One type scale
  in `ui/theme.slint`, chosen by `Theme.platform` at start, sizes in px /
  weights:

  | Role (token) | macOS (HIG) | Windows (Fluent 2) | Linux (Adwaita) |
  |---|---|---|---|
  | micro: matrix and diagram labels | 10 / 400 | 11 / 400 | 11 / 400 |
  | caption: notes, legends, badges | 11 / 400 | 12 / 400 | 12 / 400 |
  | small: table cells, field labels | 12 / 400 | 13 / 400 | 13 / 400 |
  | body: text and controls | 13 / 400 | 14 / 400 | 14 / 400 |
  | mono: code, paths | 12 / 400 | 13 / 400 | 13 / 400 |
  | heading: uppercase group label | 11 / 600 | 12 / 600 | 12 / 700 |
  | section: panel and card titles | 15 / 700 | 16 / 600 | 16 / 700 |
  | page: a tab's title | 17 / 700 | 20 / 600 | 20 / 700 |
  | display: key figures | 22 / 600 | 24 / 600 | 24 / 700 |

  Emphasis in running text uses `weight-strong` (600, Adwaita 700),
  buttons regular. Fluent's subtitle (20) and title (28) are scaled down
  to 16/20/24 for this dense UI; Adwaita's 11 pt body (≈14.7 px) is 14.
  SF's optical tracking (its `trak` table) is not applied by Skia or the
  software renderer; no letter spacing is added to imitate it. Control
  metrics follow the platform too: control height 28/32/34 px (compact
  24/28/28), control radius 6/4/6, card radius 10/8/12, focus ring 3 px
  soft accent on macOS, 2 px solid elsewhere. The style's ComboBox kept
  its own colours (Cupertino: a grey box with a bright blue stepper in
  the dark theme), as did LineEdit and TextEdit (grey fields), so
  `ui/combo.slint` and `ui/fields.slint` draw them like the app's fields
  in both themes, with the same properties and keyboard handling (the
  text fields have no right-click menu yet; the shortcuts work).
  The window title names the open project; the app name stays in menu bar
  and Dock. Traffic-light placement is left to the system (Slint exposes
  no title bar style without the unstable winit API). A unit test keeps
  literal font sizes and weights out of the views (scaled diagram fonts
  excepted); the Start tab (`ui/start_*.slint`, reworked in parallel)
  is exempt until it moves to the tokens and the own ComboBox. Checked with headless snapshots on macOS only (Cupertino, and
  Fluent with `ARCHGUARD_DESKTOP_PLATFORM=windows|linux` for the scales);
  the real Segoe UI, Consolas, Cantarell and the Linux fontconfig aliases
  were not seen.
- **2026-11-06:** Services World on the Start tab, the web's repository
  overview ("Your services"). A document whose topology has build units
  and at least two islands opens on it, as `archguard serve` opens a
  monorepo; Services / Architecture switch between the two Worlds.
  Islands, connections (merged per from, to and kind, cycles per kind
  with `tree_order::scc`), the deployment, evidence, kind, direction
  (incl. every hop up or down the data flow), cycles-only and
  all-projects filters are ports of `viewer_repository.js`; a JS-vs-Rust
  test reads the same cases (`scripts/viewer/services_world_cases.json`,
  19 cases on service-shop and compose-services). Arrows are canvas
  paths, one per tone; events are dashed as segments, since Slint paths
  have no dash pattern. The data flow dots move in Slint from
  `animation-tick()` along the curves Rust pushes, so the animation
  rebuilds nothing per frame. Decision: "Zoom into service" shows the
  project's packages in this document's architecture World (the desktop
  analyses the whole repository; the web reloads with `?project=`), with
  a crumb and Back to the services; "Open project" opens the project as
  a document of its own, as the Projects menu. Not ported: collapsible
  connection entries, the "Analysis & traces" help, a reduced-motion
  setting (the desktop has none), persisting mode and filters.
- **2026-11-06:** Second World round. Rows synced in place
  (`model_sync::sync_rows`) could stay stale: Slint 1.18's repeater
  (`RepeaterTracker::row_changed`, unchanged upstream) only marks an item
  dirty, without scheduling its update, when the row changes while the
  repeater's model binding is dirty itself, i.e. after the view's struct
  was set again since the last frame. Two pushes per frame or pushes to a
  hidden tab did that in real windows too, not only in snapshots.
  `sync_rows` now schedules the update with an insert and removal at the
  first changed row (items are updated, not rebuilt); the services
  World's arrows sync in place again. The World gained the web's legend
  and KPI help, a hover tooltip with outline, collapsible connection
  entries with Select neighbor in both Worlds, the detail actions
  (Cycles, Matrix, Calls, Class metrics, Quality scope, Copy path, AI
  context; the other views are reached through their own callbacks once
  their state exists), source dependencies in the details, keys (+ / =,
  -, 0, Q / E, arrows, Enter, Esc), Ctrl-drag, pinch and twist (Slint's
  `ScaleRotateGestureHandler`), 320 ms cubic camera glides for buttons
  and keys (drags, wheel and gestures stay immediate) and a lit selected
  roof. Decision: the World view (mode, scope, lens, KPI, services
  filters, selected area or island; not the camera) is kept per project
  as a `world` table of the Start tab's entry in the pane layout, so it
  is stored with the layout and only when it changes. A Graph on the left
  now leads a World on the right ("Follow left"). Not done: the Start
  KPI section, the Explore and Test gaps details depth, fullscreen; Calls
  and Class metrics search rather than select the class; gestures and
  the clipboard were not tried on real hardware (headless snapshots only).
- **2026-11-07:** Layered arc42 package maps, asked for by the user
  ("not happy with the diagrams"). Chapters 5, 6 and 11 no longer use the
  web's fallback grid, whose edges all ran in lanes above the cards and
  tangled on larger projects. Decision: a layered layout after Graphviz
  `dot` (which the web viewer uses), written in Rust in
  `architecture_ui/diagram/` instead of a layout crate, so cards keep
  their selection, zoom and model. Cycles are broken by turning back
  edges upwards, cards get the row below their users (at most 4 per row),
  long edges get bend points in the rows they cross, barycentre sweeps
  reduce crossings, and rows move towards their neighbours. Edges leave
  the bottom and enter the top as curves, from ports spread over the
  card; counts no longer overlap. Checked in headless snapshots of this
  repository (chapters 5 and 11) and `fixtures/py-layers`. Open: wide
  maps still fit by width (about 65 % on this repository), long edges
  can bend in S-curves, Mermaid blocks in the chapter text are still
  shown as source.
- **2026-11-08:** Monospace on macOS is Menlo instead of ".SF NS Mono"
  (`fix/desktop-mono-italic`). The hidden SF Mono family holds
  SFNSMono.ttf and SFNSMonoItalic.ttf; fontique, which every Slint
  renderer uses for font matching, reads both as upright weight 295 and
  picked the italic file, so paths, commands and the project chip were
  drawn in italic (in snapshots and, since they share the code, in the
  window). Slint cannot register a font file by path, so the family
  changes. `platform::tests::the_monospace_font_is_upright` measures the
  slant of a "|" in the configured family (".SF NS Mono": 7.9 px).
- **2026-11-08:** The desktop app trusts project compilers by default,
  decided by the user ("always trust"), so Rust and TypeScript calls
  resolve and arc42 chapter 6 has runtime scenarios without starting the
  app with `ARCHGUARD_TRUST_PROJECT_COMPILER=1`. The engine got a
  process switch (`archguard_engine::trust_project_compiler`, read next
  to the environment variable); the app sets it at start from
  `desktop.toml`, where `trust_project_compiler = false` turns it off
  (the key is read, never written). Accepted risk: rust-analyzer and a
  project's TypeScript compiler read configuration from the repository,
  which can start commands; opening an untrusted repository in the app
  runs them. The CLI and the MCP server stay off unless the variable is
  set. Checked: chapter 6 of this repository in a snapshot without the
  variable (static call map with 9,614 call sites, scenarios from
  `main`).
- **2026-11-08:** The default trust of project compilers now holds only
  where the tooling runs confined (macOS, ADR 0007); on Linux and Windows
  the app needs `ARCHGUARD_TRUST_PROJECT_COMPILER=1` again.
