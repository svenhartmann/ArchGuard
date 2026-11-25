# ADR 0023: Elasticsearch artifacts and the index map

- **Status:** accepted
- **Date:** 2026-11-23

## Context

Systems that run on Elasticsearch declare indices, ingest pipelines, transforms,
watches and Kibana assets in Terraform (provider `elastic/elasticstack`,
documentation v0.11.13) and use the indices from service code. The service
topology ([ADR 0015](0015-terraform-deployment-evidence.md),
[ADR 0012](0012-datastore-tables.md)) shows neither the declared artifacts nor
which service reads or writes which index, so a missing pipeline, an index
nobody declared or an index with two writers stays invisible. Painless scripts
are code, but no analyzer measures them.

## Decision

1. **Static only.** No cluster access, no `terraform` binary, no state, no
   `.tfvars`, no `.terraform/` (ADR 0015 stays). The result is a new field
   `ServiceTopology.elasticsearch` (`artifacts`, `usages`, `findings`), omitted
   from serialization when empty so existing output stays byte-identical. It is
   shown by `archguard services` (text, JSON) and MCP `service_topology`, with
   findings in the topology like `datastores.shared`, not in
   `architecture_findings` or the `check` gate.
2. **Declared artifacts.** Module resolution, address prefix `module.<name>.`
   and scope `terraform:<dir>` are those of ADR 0015. Read per kind:
   - `elasticstack_elasticsearch_index`: `name`, `alias { name }` (own artifact
     `alias`, referencing the index), `default_pipeline`, `final_pipeline`.
   - `_index_template`: `name`, `index_patterns` (as patterns), `composed_of`,
     `template.alias`. `_component_template`, `_index_lifecycle`: `name`.
   - `_ingest_pipeline`: `name`; a processor `pipeline.name` inside
     `jsonencode({...})` is a reference.
   - `_transform`: `name`, `source.indices`, `destination.index`,
     `destination.pipeline`. `_watch`: `watch_id`.
   - `_enrich_policy`, `_snapshot_lifecycle`: `name`, `indices`.
   - `_script`: `script_id`; `lang = "mustache"` makes a `search_template`,
     otherwise a `stored_script`.
   - `elasticstack_kibana_data_view`: `data_view.name`, `data_view.title` as
     patterns (comma list split, entries with a leading `-` dropped).
   - `elasticstack_kibana_alerting_rule`: `name`, patterns from the key `index`
     (string or list) of `params` (`jsonencode({...})` or literal JSON);
     `rule_type_id` goes into the evidence text.
   - `elasticstack_kibana_import_saved_objects` is not analyzed; each such
     resource only produces a diagnostic.
   - Security roles and users, `ec_deployment` and the link from an index to a
     `datastores.nodes` entry are out of scope.
3. **Search templates** are also JSON files with `"lang": "mustache"` (text
   search `"lang"\s*:\s*"mustache"`, no JSON parser) under directories with
   `.tf` files or their subdirectories, excluding test paths and `.terraform/`.
   The name is the file name without extension; this is an assumption and the
   evidence says "name assumed from file name".
4. **References.** Edges carry target kind and target name: index to pipeline
   (`default_pipeline`, `final_pipeline`), pipeline to pipeline, transform to
   source index, destination index and pipeline, alias to index, snapshot
   lifecycle and enrich policy to indices, template to component templates.
5. **Literals and gaps.** A name is read from a literal string or a
   `jsonencode` value. `local.`, `each.`, `module.` outputs, data sources and
   function calls give no name; each yields the diagnostic "Elasticsearch name
   is not a literal" with file, and `complete` becomes `false`. `var.x` is read
   only from the literal module argument of the call, or, without one, from a
   non-sensitive literal `default` of type `string` or `list(string)` (at most
   one hop per module level, depth as `MAX_MODULE_DEPTH`); `sensitive = true`
   variables and `.tfvars` are never read; `locals` stay gaps. This applies only
   to Elasticsearch name attributes, not to the workload and datastore
   resolution of ADR 0015. The evidence names the source of the value.
6. **Index usage in code.** For each service (as in ADR 0012: owner walk, test
   paths excluded, 1 MiB per file, 5 000 files) files that import an Elasticsearch
   client are scanned for literal index names:
   - Java `co.elastic.clients` (`org.elasticsearch`): `.index("x")`,
     `.index(List.of("x"))`, `new IndexRequest("x")`, `IndexRequest.of(...)`,
     `.header(h -> h.index("x"))`.
   - C# NEST and `Elastic.Clients.Elasticsearch`: `.Index("x")`,
     `Indices.Index("x")`, `IndexMany(docs, "x")`, `new IndexName("x")`,
     `.DefaultIndex("x")`.
   - Python `elasticsearch-py`: `index="x"` on `index`, `create`, `update`,
     `delete`, `bulk`, `search`, `count`, `get`, `msearch`; `"_index": "x"` in
     `helpers.bulk`.
   - TypeScript `@elastic/elasticsearch`: `{ index: 'x' }` on the same methods;
     template strings with `${}` are not literal.
   - A string constant with a literal value in the same service
     (`Class.NAME` or an unqualified `NAME` with exactly one declaration) is
     resolved; ambiguous, computed (`"a" + x`) or foreign-service constants are
     gaps. Evidence then lists the call site and the declaration.
7. **Access vocabulary.** The nearest preceding identifier of the same statement
   decides, from a fixed table. `write`: Java `IndexRequest`, `IndexOperation`,
   `BulkRequest`, `BulkOperation`, `CreateOperation`, `UpdateRequest`,
   `UpdateOperation`, `DeleteRequest`, `DeleteOperation`, `DeleteByQueryRequest`,
   `UpdateByQueryRequest`; C# `Index`, `IndexMany`, `Bulk`; Python and TS
   `index`, `create`, `update`, `delete`, `bulk`. `read`: `SearchRequest`,
   `MsearchRequest`, `MsearchTemplateRequest`, `SearchTemplateRequest`,
   `CountRequest`, `GetRequest`; C# `Search`, `Count`, `Get`; Python and TS
   `search`, `count`, `get`, `msearch`. Java `header` (`header(h -> h.index(X))`,
   which exists only in Msearch requests) is `read`. Anything else is `unknown`. One entry
   per (service, index, access).
8. **Findings** (computed from artifacts and usages, one per subject):
   - (a) `undeclared_pipeline`: a pipeline reference whose name is not a
     declared `ingest_pipeline`.
   - (b) `undeclared_index`: a code usage whose index matches no declared
     index, alias or template pattern; only if the repository has at least one
     `elasticstack_*` declaration.
   - (c) `shared_write_index`: two or more services with `write` on the same
     index; an alias with exactly one declared target counts as that target;
     `unknown` is not a writer.
   - (d) `uncovered_pattern`: a data view pattern matching no declared index,
     alias or template pattern.
   - (e) `alert_undeclared_index`: an alerting rule index entry with no such
     match.
   - Artifacts without a literal name do not suppress a finding; the finding
     then carries "declarations incomplete" in its evidence.
9. **Glob rule.** Only `*` is a wildcard, matching any run of characters
   (including none). A pattern against a name is a plain glob match. Pattern
   against pattern counts as covered when the pattern with its `*` characters
   removed is matched by the other glob (`logs-*` covers `logs-app-*`, because
   `logs-app-` matches `logs-*`). Other special characters are literal.
10. **Painless.** Each `.painless` file and each literal inline `source` in a
    `.tf` file (heredoc inside `jsonencode({ script = { source = ... } })`)
    becomes one method of a new call analyzer `painless`, with
    `complexity.definition = "ast-decisions-v1"`. A `source` built with
    `file(...)` or `join(...)` yields no method; the referenced files count
    themselves; other non-literals give a diagnostic with a count. The analyzer
    reports a fingerprint of all `.painless` and `.tf` files (outside
    `.terraform/`) in `semantic_fingerprints`, so a changed script invalidates
    the memo. The counting rule `ast-decisions-v1` for Painless is listed only in the
    module documentation of the counter,
    [`painless.rs`](../../crates/archguard-extract/src/painless.rs).
11. **Answers to the open questions.**
    - Q1: constants with a literal value in the same service are resolved
      (decision 6).
    - Q2: names are read from non-sensitive `variable` defaults and literal
      module arguments (decision 5); `locals` stay gaps.
    - Q3: Painless is its own call analyzer `painless`, one method per script
      (decision 10).
    - Q4: search templates are also read from JSON files with `"lang":
      "mustache"`, named after the file (decision 3).

## Alternatives

- **Evaluate `terraform plan` or read state.** Needs providers and credentials;
  rejected for the reasons of ADR 0015.
- **Reuse `datastores.nodes` for indices.** Would tie an index to a server
  although most index names are not literal; kept apart as future work.
- **Report the findings in `architecture_findings` or the `check` gate.**
  Findings here are hypotheses from configuration, like shared datastores in
  ADR 0012, so they stay in the topology.
- **"Index without covering template" as finding (b).** Repositories without
  templates would flag every index; "not declared" is the usable condition.
- **Read `locals` and `.tfvars`.** Describe one environment and may hold
  secrets; unknown stays unknown.
- **Full parsers for the client libraries.** The call patterns are literal; a
  text scan needs no new dependency (as in ADR 0012).

## Consequences

- Painless methods have no graph node. They do not appear in
  `architecture_world` (it hides classes without graph nodes), only in
  `calls` and `semantic_model`.
- A declaration or a code usage is configuration and source evidence, not proof
  of runtime use; a declared artifact may be unused.
- `unknown` access never counts as a writer, so a shared index can be missed
  when the client call is not in the vocabulary.
- Index names behind `locals`, `each`, module outputs or non-constant
  expressions stay gaps and make `complete` `false`.
- Security roles, saved-object contents, mappings, fields and query contents
  are not analyzed.

## History

- **2026-11-23:** Proposed and accepted.
- **2026-11-23:** Decision 10 no longer lists the counting rules; the module
  documentation of `painless.rs` is their only place (the `while` of a do-while
  does not count again).
- **2026-11-24:** Decision 7: Java `header(...)` in a Msearch request is `read`
  (table entry in the code reader). An `alias` block of an index whose own name
  is unknown is still an alias artifact, with no target; Java operation objects
  made by a factory (`XOperation.of(...)`) are objects, not index names.
- **2026-11-24:** Several projects can be merged into one Elasticsearch map; see [ADR 0024](0024-project-sets.md).
- **2026-11-25:** Mapping/Settings im Scope (R12). `SearchArtifact.detail:
  Option<SearchIndexDetail>` (skipped when `None`) carries `fields` (dotted
  path, parents before children, multi-fields as `a.raw`, an object without
  `type` is `object`; `kind`, `analyzer`, `search_analyzer`, `normalizer`),
  `settings` (keys without the `index.` prefix, values as written) and
  `analysis` (`kind`, `name`, compact JSON `definition`); empty lists and maps
  are skipped. A project-set merge keeps the first project's detail. Reading is
  all or nothing per attribute: if one attribute of a resource is not literal,
  that resource gets no detail and one gap `DetailNotLiteral`. `file()` is
  allowed under the project root but not built; this is the limit for later.
  MCP `service_topology` drops `detail` (4 KiB pages; the mapping is desktop
  content); CLI text is unchanged and CLI JSON carries `detail`.
- **2026-11-25:** Templates, ILM and watches (ticket 38). The `template {}`
  block of `index_template` and `component_template` carries `detail` like an
  index: `mappings` as before, `settings` (JSON) flattened to dotted keys
  without `index.`; `analysis` inside it becomes analysis entries. The
  deprecated `settings { setting { name value } }` block of an index is read
  into `settings` as well. The setting `lifecycle.name` is the reference
  `ilm_policy` (graph: edge, a ghost policy carries no warning). A watch gives
  the references `index` from a literal `input.search.request.indices`; entries
  with `*` become the watch's `patterns`. A non-literal `input` is one
  `DetailNotLiteral` gap; other inputs (`http`) and `indices` elsewhere (e.g.
  in `actions`) give nothing.
