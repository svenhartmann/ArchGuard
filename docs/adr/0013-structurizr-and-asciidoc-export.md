# ADR 0013: Structurizr, AsciiDoc and Word export

- **Status:** accepted
- **Date:** 2026-11-18

## Context

Teams keep architecture models and documents in other tools. C4 models
often live in Structurizr workspaces. arc42 documents are often written in
AsciiDoc, and many teams publish them in Confluence. ArchGuard produced
the arc42 document only as Markdown (`doc --arc42`) and the service
topology only as diagrams inside that document (ADR 0006).

A Confluence API push needs a login and sends project data to a server.
Confluence can also import a Word file ("Import Word document"), which
needs neither.

## Decision

1. **Structurizr DSL from the service topology.** `graph --format
   structurizr` (MCP: `architecture_document` with
   `template=structurizr`) writes one workspace:
   - one software system, named after the project directory;
   - one **container** per service in `ServiceTopology::services`
     (description: the Compose or manifest file, technology: the
     datastore engine or the project runtime/ecosystem, else empty). A
     service that runs a datastore is tagged `Database`;
   - a datastore without a service of its own (`external:<host>`) is an
     external **software system** tagged `External,Database`;
   - one **relation** per (from, to, label), because the DSL refuses
     duplicates. Labels and technologies: `HTTP, configured`/`HTTP`,
     `HTTP call`/`HTTP`, `gRPC call`/`gRPC`, `event`/`messaging`,
     `starts after`/`depends_on`, `configured reference`, and `uses
     datastore[, database X]`/engine. Up to three pieces of evidence
     (`file:line pointer`) are kept in an `evidence` property, the rest
     is counted;
   - with `--service <id or unique name>`, that service's packages become
     **components**: every analysed file whose physical path lies under
     the service's project path (else its build context) is assigned to
     its module in the module cut (`modules.depth`), or to its parent
     directory where the cut already shows files. Relations between
     components sum the file-level source references. A service without
     a source directory, or with no analysed file in it, is an error: its
     components are unknown, not empty;
   - views: a container view and, with `--service`, a component view,
     both `include *` with auto layout.

   Element identifiers are generated (`c0`, `x0`, `p0`), so names never
   need escaping beyond DSL strings. Nothing here is measured traffic; the
   workspace description says so.

2. **AsciiDoc and Word are conversions of the arc42 Markdown.** The arc42
   Markdown stays the one source (`arc42.md`, ADR 0006/0008); the
   `arc42.rs` model is not changed. `doc --arc42 --format asciidoc|docx`
   converts the Markdown of `arc42_document`, merged with the own text of
   an existing `<root>/arc42.md` (read only). With `--chapter N` only the
   generated chapter is exported. `--check` compares the bytes;
   `--stdout` is refused for the binary `.docx`. Desktop (File → Export)
   and MCP (`format=asciidoc`, no docx, since MCP replies are text)
   convert the generated Markdown the same way.
   - **AsciiDoc:** `#` to `=` headings, `**`/`__` emphasis, `link:`,
     `*`/`.` lists, `|===` tables with alignment, HTML comments as `//`
     comments (the `archguard:arc42` block markers stay visible as
     comments). Mermaid and PlantUML blocks become `[mermaid]`/
     `[plantuml]` literal blocks (`....`) for Asciidoctor Diagram; other
     code becomes `[source,lang]`. Plain text containing AsciiDoc markup
     characters is wrapped in `++…++` so evidence like `SHIPPING_API` or
     `{id}` stays literal.
   - **Word:** headings use the built-in styles Heading 1–4 (style ids
     `Heading1`…, names `heading 1`…, outline levels), so Word and
     Confluence build the outline from them. Tables are `w:tbl` with a
     repeated header row, lists use `numbering.xml` (every numbered list
     restarts at 1), links are external hyperlinks. The document model
     has no rendered diagrams, so Mermaid and PlantUML stay as source in
     a monospace `Code` paragraph, labelled `mermaid diagram source`.
     HTML comments are dropped. The zip entries carry a fixed time, so
     equal documents are byte-identical.

3. **Libraries.** Markdown is parsed with `pulldown-cmark` 0.13 (MIT,
   already in `Cargo.lock` through Slint) without default features. The
   `.docx` package is written by hand (six XML parts) into a `zip` 9
   archive (MIT, already used by `archguard-update`). No new crate enters
   the lock file.

4. **No Confluence push.** Uploading needs credentials and sends data out;
   the Word file covers the import without both.

## Alternatives

- **`docx-rs`** (MIT): a full Word writer, but a new dependency tree for
  the few elements we need (paragraph styles, tables, numbering,
  hyperlinks).
- **A hand-written Markdown line converter** for AsciiDoc and Word: no
  dependency, but tables, nested lists, escapes and HTML blocks would
  need their own parser; `pulldown-cmark` is already compiled for the
  desktop.
- **AsciiDoc as a second source** (round trip, merging own AsciiDoc
  text): the protected-block merge works on Markdown; a second merge
  format would double that logic. AsciiDoc is export only.
- **Rendering diagrams to PNG for Word:** needs Mermaid/PlantUML
  renderers (Node or Java) at export time; out of scope.
- **Containers from manifest projects without a deployment service:**
  would mix "deployed" and "buildable"; only services become containers.

## Consequences

- AsciiDoc and Word files are generated output; edits to them are not
  read back. Teams that want their own text in them write it in
  `arc42.md`.
- Diagrams in Word need a Mermaid or PlantUML macro after the import.
- A real Confluence import, Structurizr CLI/Lite parsing and an
  Asciidoctor render were not run when this was written (the tools were
  not installed). pandoc 3.13 (docx and AsciiDoc readers) and LibreOffice
  (headless docx → HTML) read the exports of this repository: headings at
  their levels, all six tables, all five diagram blocks.
- Components depend on the module cut: a deeper `modules.depth` gives
  finer components.

## History

- **2026-11-18, review fixes.**
  - `doc --arc42 --format asciidoc|docx` refuses (exit 2) a target that is
    the same file as `<root>/arc42.md` (same device and inode, so also
    `ARC42.MD` on a case-insensitive file system), since `arc42.md` is
    only read. With `--chapter N` the default file is `arc42-NN.adoc`/
    `arc42-NN.docx`, as in the desktop, so a chapter no longer replaces
    the full export and `--check` compares like with like.
  - Structurizr: a service name that occurs more than once becomes its
    id (`compose.yaml::api`), a repeated external datastore host becomes
    `host:port`, since the DSL refuses two elements of one name. Evidence
    is sorted and deduplicated before the first three are shown.
  - AsciiDoc: a line that starts with a non-alphanumeric character or
    whose first word ends in `.`, `)`, `:` or `;` gets a leading
    `{empty}` (no block title, comment, attribute entry, admonition or
    list); text containing `++` is wrapped in `pass:c[…]`; code spans are
    unconstrained (``` ``+x+`` ```); in link text `=` is passed through
    too, so `[docs, version=2]` is not read as attributes (quoting the
    text, `["…"]`, shows the quotes in Asciidoctor 2.0.26 when it has no
    `=`). A second paragraph or a block inside a list item is attached
    with a list continuation (`+`). Checked by rendering with
    Asciidoctor 2.0.26.
  - Word: run and paragraph properties follow the schema order (`rStyle`,
    `rFonts`, `b`, `i`; `shd` before `spacing`); a second paragraph in a
    list item is indented without its own bullet.
