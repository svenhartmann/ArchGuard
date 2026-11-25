# ADR 0025: The crate archguard-search

- **Status:** accepted
- **Date:** 2026-11-25

## Context

The Elasticsearch map of [ADR 0023](0023-elasticsearch-artifacts.md) grew to
about 3 200 lines in nine files next to `services.rs`, which already has 20
functions. Each new input format (the user plans more than Terraform, search
templates and code) would add a module to the engine and a call to
`analyze_services_with`. The user decided on 2026-11-25 to give the
Elasticsearch code its own crate with a table of sources and one merge.

## Decision

1. **Own crate.** `archguard-search` (layer `search`, may depend on `core`,
   `extract`, `flow-common`, `terraform`; the engine may depend on it) holds the
   sources, the merge of their output, the findings, the graph and the merge of
   project sets ([ADR 0024](0024-project-sets.md)). The move changes no
   behaviour: goldens and `archguard services` output stay byte-identical.
2. **Source table.** `sources.rs` has `SOURCES: &[fn(&SearchContext) ->
   SourceOutput]` (Terraform, search templates, code), the pattern of
   `domain_model.rs`. A source returns `SourceOutput { artifacts, usages,
   diagnostics }` and never writes the report. `merge::apply` concatenates the
   outputs in table order, sorts the artifacts by (scope, address), derives
   `complete` (every diagnostic but the saved-object note clears it) and
   computes the findings.
3. **`SearchContext`, not `ExtractContext`.** The sources need the `Discovery`
   and the service list (code usages are per owning service), which
   `ExtractContext` does not carry. The context is `{ root, discovery,
   topology, read }`; `read` is the engine's no-follow file reader, which
   cannot move into this crate.
4. **Call site.** `archguard_search::analyze(root, discovery, read, report)`
   runs after `service_tables`; no earlier step reads `report.elasticsearch`.
   The Terraform source walks the modules itself with `archguard_terraform::walk`
   ([ADR 0015](0015-terraform-deployment-evidence.md)), so the `.tf` files are
   parsed a second time next to the engine's deployment walk. The duplicate
   parse diagnostics vanish in the final sort and dedup of the report.
5. **Public API.** The engine re-exports `SearchEdge`, `SearchGraph`,
   `SearchNode`, `is_detail_gap`, `is_elasticsearch_gap`,
   `search_artifact_covers`, `search_graph` and `short_scope`; the CLI, MCP and
   desktop still import `archguard_engine::...`. Painless stays in the engine
   (call analyzer, no Elasticsearch model).
6. **Tests.** Module tests moved with their files. End-to-end tests over
   `analyze_services` stay in the engine test folder because this crate must
   not know the engine; format tickets test crate-locally.

## Alternatives

- **Runtime plugins (WASM, subprocess).** Rejected: no second implementer
  exists (YAGNI), a plugin crosses the trust boundary of the analysis
  (ADR 0007 confines project tooling, not extensions), and loading, versioning
  and releasing plugins costs more than a compiled module.
- **Stay in the engine.** Rejected: nine files and 3 200 lines beside
  `services.rs`; every format lengthens `analyze_services_with`.

## Consequences

- A format is one module and one table row; the row is the one known merge
  point.
- `archguard-terraform` is the shared base of the engine's deployments and the
  search sources.
- Cost: one additional parse of the Terraform files per analysis (measured in
  the review of ticket 00).
- Deviation from the ticket wording: `SearchContext` carries `read`, and
  `analyze` takes the report instead of a context with the report inside,
  because the context borrows the report while the sources run.

## History

- **2026-11-25:** Proposed and accepted.
