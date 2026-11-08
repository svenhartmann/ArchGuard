# ADR 0006: arc42 diagram catalogue

- **Status:** accepted
- **Date:** 2026-11-07

## Context

The arc42 document (`archguard doc --arc42`, the web viewer, the desktop
Architecture tab) shows derived diagrams in some chapters: package maps
in chapters 5, 6 and 11, a C4 deployment view from Compose files in the
web viewer's chapter 7. Only chapter 5 writes its diagram into
`arc42.md` (a Mermaid block); the others exist only in the viewers.

The user asked for (2026-11-07):

1. runtime diagrams in chapter 6, from traces where they exist;
2. C4 deployment and container diagrams in chapter 7 from `compose.yml`
   and similar files;
3. choosing which diagrams the documentation shows (hide, tick);
4. a view that lists all diagrams with a preview, from which a hidden
   diagram can be taken back into the document;
5. the choice stored in `archguard.toml`; diagrams written into the
   document as Mermaid or PlantUML.

What exists:

- `archguard_engine::arc42_document` already analyses calls and services
  (`analyze_services`) for the document; `archguard-report` writes the
  Markdown and the generated blocks
  (`<!-- archguard:arc42:NN:start/end -->`).
- `service_traces.rs` reads `.archguard/otel-traces.json` and derives
  service edges with call counts, errors and durations; it keeps no
  per-request structure, so there is no order of calls yet.
- The desktop draws package maps natively with a layered layout
  (`architecture_ui/diagram/`, ADR 0002 entry of 2026-11-07).

## Decision

1. **One catalogue in the engine.** The engine builds a list of
   diagrams for a project, each with a stable id, chapter, title, kind
   and a neutral model (elements with kind and technology, relations
   with kind, label and evidence). CLI, web and desktop use this list;
   no client derives its own set.

   | id | chapter | kind | source |
   | --- | --- | --- | --- |
   | `building-blocks` | 5 | package map | source graph |
   | `runtime-calls` | 6 | package map | static call sites |
   | `runtime-services` | 6 | C4 container | configured and detected HTTP, gRPC, event links, datastore use |
   | `runtime-trace-<n>` | 6 | sequence | one recorded trace (OTLP), real order |
   | `deployment-<file>` | 7 | C4 deployment | one Compose or manifest file: containers, datastores, relations |
   | `cycles` | 11 | package map | cyclic package relations |

   Static diagrams say that they show configuration or source, not
   measured behaviour. Trace diagrams are only offered when traces exist.

2. **Choice in `archguard.toml`.**

   ```toml
   [arc42]
   diagram_format = "mermaid"          # or "plantuml"
   hidden_diagrams = ["runtime-calls"] # ids; all others are shown
   ```

   A list of hidden ids (not of shown ones) keeps new diagram kinds
   visible by default. Unknown ids are ignored, so a diagram that comes
   back later stays hidden.

3. **Mermaid or PlantUML in `arc42.md`.** Every shown diagram is written
   into its chapter's generated block in the chosen format: Mermaid
   `flowchart`, `sequenceDiagram`, `C4Deployment`, `C4Container`; PlantUML
   with the C4-PlantUML standard library (`!include <C4/C4_Deployment>`,
   as the web's export). Hidden diagrams are left out of the block.

4. **Desktop.** The reader draws the shown diagrams natively (layered
   layout, C4 shapes, sequence lanes) and has a "show in documentation"
   tick on each. A diagram view lists the whole catalogue with a
   preview, the generated Mermaid/PlantUML source and the tick; ticking
   writes `archguard.toml` (format-preserving) and refreshes the chapter.

5. **Traces with order.** `service_traces.rs` additionally keeps a
   bounded number of request trees (root span, child spans in start
   order, service, operation, duration, error) for the sequence
   diagrams.

## Alternatives

- **Store the choice in `arc42.md`** (a diagram is shown when its block
  is there): one file less, but a block the user deleted cannot be told
  apart from one never generated, and the CLI would need the old file to
  know the choice. Rejected by the user in favour of `archguard.toml`.
- **Desktop-only settings:** the CLI and the web would show other
  diagrams than the app.
- **SVG files beside `arc42.md`:** same look everywhere, but binary-ish
  files in the repository and no text diff. Rejected by the user.
- **Each client derives its diagrams** (today's state): the sets already
  differ (chapter 7 is only in the web).

## Consequences

- New config keys need the settings catalogue of the desktop and the
  documentation of `archguard.toml`.
- Mermaid's C4 support is marked experimental; diagrams may look
  different in GitHub than in the app. PlantUML needs a renderer.
- The generated blocks of chapters 6, 7 and 11 change; saved `arc42.md`
  files get the new diagrams on their next merge.
- Sequence diagrams from traces depend on the trace sample; the chapter
  names the trace and its time.

## History

- **2026-11-07:** Accepted by the user. Step 1 implemented: the
  catalogue (`archguard_core::Diagram`, `archguard-report`
  `arc42_diagrams`), `[arc42]` in `archguard.toml`, Mermaid
  (`flowchart`, `C4Deployment`, `C4Container`) and PlantUML (packages,
  C4-PlantUML) blocks at the start of chapters 5, 6, 7 and 11, the
  catalogue as `diagrams` in the arc42 document (CLI JSON, MCP, desktop).
  Chapter 6 shows the communication between services as a C4 container
  diagram (not C4 dynamic: without traces there is no order to number).
  Checked with `archguard doc --arc42 --stdout` on
  `fixtures/compose-services` and a copy of `examples/service-shop` with
  `diagram_format = "plantuml"` and a hidden deployment. Not yet: the
  desktop draws these blocks only as source; native C4 drawing, ticks and
  the diagram view (step 2); trace sequences (step 3).
- **2026-11-07:** Step 2 implemented in the desktop app. The reader draws
  the catalogue's C4 diagrams natively with the layered layout
  (`architecture_ui/figure.rs`, `ui/architecture_figure.slint`): containers,
  databases (cylinder lid), external containers, a frame per deployment
  node (only when no foreign card lies inside it, else the group is named
  on the card), one colour per relation kind with a legend. The generated
  Mermaid/PlantUML sections are cut from the chapter text where the app
  draws them; the package maps of chapters 5, 6 and 11 get a tick. Hidden
  diagrams stay as one line with their tick. A "Diagrams" entry below the
  chapters lists the whole catalogue with preview, source and tick. A tick
  writes `[arc42] hidden_diagrams` through the Settings view's
  `ConfigEditor` (other keys and comments stay) and rewrites the open
  document as the CLI would; `arc42.md` changes on the next save or
  `archguard doc --arc42`. Checked in headless snapshots of a copy of
  `fixtures/compose-services` (chapters 5, 6, 7, diagram view, hide and
  show via `ARCHGUARD_DESKTOP_ARC_TOGGLE`). Not tried in a real window.
- **2026-11-07:** Step 3 implemented. The trace import keeps the matched
  CLIENT/SERVER and producer/consumer pairs per trace
  (`trace_sequences.rs`): at most 5 traces (most steps first), 40 steps
  each, as `ServiceTopology.traces` with offset, duration, error and an
  operation from `http.route` (with method) or `rpc.service`/`rpc.method`
  only; URLs, queries, span names and message destinations are never
  copied (topics stay private, as in the dependency evidence). Each trace
  is a catalogue entry `runtime-trace-<first 8 hex of the trace id>` in
  chapter 6, written as Mermaid `sequenceDiagram` or PlantUML sequence
  (errors marked, events asynchronous) and drawn natively in the desktop
  (participants, lifelines, messages in measured order). Checked with a
  hand-written OTLP file in a copy of `fixtures/compose-services`
  (`archguard doc --arc42 --stdout` and a desktop snapshot).
- **2026-11-07:** Web viewer. The HTML report now carries the catalogue
  (`insights.arc42.diagrams`, `review_cmd.rs`). The reader
  (`viewer_arc42_catalogue.js`) cuts the generated sections of shown
  diagrams from the chapter text and draws C4 diagrams with Graphviz:
  containers, databases as cylinders, dashed external containers, a dashed
  cluster per group, one colour and label per relation kind. Chapter 7
  shows one figure per deployment file from the catalogue; reports without
  `diagrams` keep the projection from `data.services`. Every catalogue
  diagram and the package maps of chapters 5, 6 and 11 get a tick. The
  page cannot write `archguard.toml`: a tick changes only the view (hidden
  diagrams collapse to one line) and shows the `[arc42] hidden_diagrams`
  snippet to copy. A "Diagrams" entry below the chapters lists the whole
  catalogue with preview, tick and the section source the document
  contains; hidden diagrams say that their source appears after enabling.
  Checked with `scripts/viewer/test_viewer_arc42_catalogue.js` and in
  headless Chrome on copies of `fixtures/compose-services` and
  `examples/service-shop`.
- **2026-11-07:** The web viewer draws `sequence` diagrams as SVG lanes
  (participants, lifelines, numbered messages, errors red, events
  dashed) instead of C4 boxes. Checked in headless Chrome on a report of
  the copy of `fixtures/compose-services` with its OTLP sample.
- **2026-11-07:** Package map levels, asked for by the user ("every
  diagram in a chapter"). Each level of the chapter 5, 6 and 11 maps (the
  packages inside one package) is a catalogue diagram `<map>@<package>`,
  e.g. `building-blocks@app/core`. Levels are opt-in, so the document
  does not grow with every package: `[arc42] shown_diagrams` lists the
  ones written into the document (next to `hidden_diagrams` for the
  default diagrams). The engine writes them (`arc42_diagrams/scoped.rs`:
  the level's packages, the package's direct files as one box, at most
  eleven packages and a remainder box); the desktop's tick in the map
  header writes `shown_diagrams` and has the document written again off
  the UI thread; the map stays visible when its level is off. Not in the
  web viewer yet. Checked with a copy of `fixtures/php-psr4` (level
  `App`): `archguard.toml`, `archguard doc --arc42 --stdout`, snapshot.
- **2026-11-07:** Runtime scenarios from the code, asked for by the user
  (chapter 6 stayed empty without traces or Compose files; "should work
  for all languages"). `arc42_diagrams/scenarios.rs` works on the
  language-neutral call model: entry points are methods that call but
  are not called (`main` first, then by blocks reached; at most 3); from
  each, resolved project calls are followed depth first in source order
  (depth 6, at most 25 messages), and every call between building blocks
  of the package overview is a message of a `sequence` diagram
  `runtime-scenario-<entry>`, titled "(statisch)" with a note that order,
  branches and loops are not measured. Shown by default like the other
  default diagrams. Checked with `archguard doc --arc42 --stdout` on
  copies of `fixtures/py-layers`, `java-packages`, `php-psr4` and
  `rs-clean` (Rust with `ARCHGUARD_TRUST_PROJECT_COMPILER=1` and a
  Cargo manifest; without rust-analyzer Rust has no resolved calls and
  so no scenarios), and in a desktop snapshot.
- **2026-11-08:** The web viewer ticks package map levels too: the map's
  tick follows the opened level (`<map>@<package>`, off unless listed),
  the map stays when a level is off, and the snippet writes
  `shown_diagrams` whenever levels are involved (also as an empty list,
  so removing the last one is copied). The desktop maps also tuck cards
  without edges into free row space (this repository's chapter 5: 79 %
  instead of 61 % fit). Checked in headless Chrome on a report of the
  copy of `fixtures/php-psr4` (level `App`).
- **2026-11-08:** Scenarios name every building block on its own (no
  remainder box of the package overview), and long sequences fit their
  width in the desktop and scroll instead of shrinking.
- **2026-11-08:** Scenarios per entry, after the user found chapter 6
  still unfitting. A root (calling, not called) or a method up to three
  calls below it that calls three or more methods is a dispatcher (a CLI
  `main` calling `run`, which matches on the command; a router); each of
  its handlers becomes a scenario of its own. Scenarios from a `main` (in
  any language) come first, then by blocks reached; at most six. The
  same call between the same blocks is one message with its count
  ("(3×)"); titles name the handler and its block. This repository now
  gets `init`, `analyze_with_zones`, `cycles`, `graph_export` (CLI),
  `wire` (desktop) and `compare_branches`.
