# ADR 0024: Project sets

- **Status:** accepted
- **Date:** 2026-11-24

## Context

The Elasticsearch map of [ADR 0023](0023-elasticsearch-artifacts.md) is built
per project. Often the declarations (Terraform: cluster, indices, pipelines)
live in an infrastructure repository and the code that uses the indices lives
in other repositories. Analysed alone, the infrastructure repository shows
declarations nobody uses and each adapter repository shows undeclared indices.
The user decided on 2026-11-24 to merge several projects into one map; this
ADR records how.

## Decision

1. **Set file.** TOML, extension `.archguard-set.toml`, like `desktop.toml`
   (ADR 0002 decision 7): `name = "..."` and `projects = [...]`, 2 to 16
   folders, the first is the primary project, each relative to the folder of
   the file or absolute. `deny_unknown_fields`, at most 64 KiB. The desktop
   app saves to `<config_dir>/sets/` by default; the user may pick any place,
   also inside a repository to share the set. Reading and writing live only in
   the engine (`project_set.rs`).
2. **Multi-root analysis.** `analyze_service_set(roots)` runs the unchanged
   `analyze_services(root)` once per project, so each analysis reads only
   inside its own root. The topology of the primary project is the base. Only
   the Elasticsearch model and the Elasticsearch gaps are merged; then the
   findings of ADR 0023 are computed over everything. Services, dependencies,
   datastores and the rest stay those of the primary project, and a diagnostic
   says so.
3. **Origin.** A project is named by its folder name (duplicates get `-2`,
   `-3`). `SearchArtifact.origins` lists the projects that declare an
   artifact, `ElasticsearchModel.projects` lists all projects of the set
   (first = primary); both are omitted from serialization when empty, so
   output without a set stays byte-identical. Usage services and evidence of
   other projects carry the prefix `<project>:`. Artifacts of the same `kind`
   and literal name from different projects become one artifact with the
   evidence of all; inside one project nothing changes (several Terraform
   roots stay separate). `SearchArtifact::foreign(primary)` is the one place
   that decides whether a node belongs to another project; the desktop marks
   such nodes with the glyph `↧` in the label.
4. **Entry points.** CLI: `archguard services [PATH...] [--set FILE]` (one path
   behaves as before). MCP: `service_topology` takes optional `paths`, each
   through `tools::resolve_root` (F02); there is no set-file parameter. Desktop:
   a set is a document whose root is the primary project; the dialog
   "Projekte zusammenstellen" writes the set file. Only the services and
   Elasticsearch views show the merged picture, all other tabs show the
   primary project.
5. **Trust boundary.** The user picks every folder; no analysis leaves its root
   (decision 2). A set file from elsewhere is operator input like a CLI path:
   size limit, `deny_unknown_fields`, errors without file contents. MCP paths
   pass the same root check as every other tool path.

## Alternatives

- **Export and import through `archguard.toml`** (the infrastructure repository
  exports its declarations, the others import them, like the API contracts).
  Rejected by the user on 2026-11-24 in favour of assembling the projects in a
  dialog.
- **A shared parent folder analysed as one project.** Mixes unrelated
  repositories into one root, breaks the rule that each analysis stays in its
  own root and cannot span folders in different places.
- **Merge the whole topology.** Services and datastores of different
  repositories need identity rules of their own; kept as future work.
- **Merge only in the desktop.** CLI and MCP would show different results; the
  merge lives in the engine instead.

## Consequences

- Services, dependencies and datastores remain those of the primary project.
- The desktop shows evidence of other projects but cannot open it by click.
- The desktop re-analyses only when the primary project changes, not when
  another member changes.
- The last two points and the merge of services and datastores are follow-up
  work.
- Output without a set is unchanged.

## History

- **2026-11-24:** Proposed and accepted.
- **2026-11-25:** The merge of project sets (decisions 2 and 3) moved with the
  Elasticsearch code to `archguard-search` (`merge_set`,
  [ADR 0025](0025-search-crate.md)); the engine passes `analyze_services` in.
