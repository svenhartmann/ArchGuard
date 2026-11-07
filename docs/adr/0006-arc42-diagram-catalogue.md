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
