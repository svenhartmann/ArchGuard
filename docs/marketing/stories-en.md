# Case-study and tutorial drafts — English

Editorial drafts for weeks 4–9 and 11. These are working manuscripts, not measured results. Run commands in fresh example checkouts, preserve the upstream licence/notices and record `git rev-parse HEAD`, `archguard --version`, configuration and tool versions. Check command availability with the release used for the recording: the locally installed 0.3.2 binary predates SBOM and newer export formats described in this repository's README.

## Kafka

### A cycle map is the beginning of a review

A large dependency map makes it easy to mistake size for a diagnosis. The more useful question is which source dependency closes a cycle and what responsibility it represents. With Apache Kafka, begin at the package level, choose a cycle group and follow one edge to its source. Then inspect the same area in the matrix.

```sh
archguard cycles examples/checkouts/kafka --format json
archguard metrics examples/checkouts/kafka --format json
archguard hotspots examples/checkouts/kafka --format json
```

The [existing walkthrough](../../site/examples/kafka.html) supplies a pinned source and configuration. For a new capture, clone it afresh. A shallow clone supports the static example but not a meaningful long-history churn claim; fetch the intended history range and record it before using hotspots or co-change in the story.

The article should end with a bounded conclusion: which dependency deserves a review, which source locations support that judgement, and what context a Kafka maintainer would still need. A large cycle is not evidence that the project is badly designed.

**Editorial TODO:** capture cycle and architecture-metric views from the current analysis. Record exact counts and scope from JSON. Do not recycle the marketing-plan headline “3,369 files” without a matching fresh run. Add a 30-second clip that moves from the selected group to one source edge.

## Shared tables

### Same server, same database, same table?

Two services can point to one server while using separate databases. A shared database may still contain disjoint known tables. The distinction matters because each observation supports a different review question.

```sh
archguard services examples/checkouts/eshop --format json
```

ArchGuard derives table evidence from supported source and migration patterns; it does not infer every runtime query. The case study should trace one reported usage to its configuration and then to its table declaration. If the pinned eShop checkout does not contain a shared table, report that honestly. Use Sock Shop only after confirming its current deployment files and licence; do not manufacture a shared-table finding merely to fit the title.

**Editorial TODO:** save the exact datastore identity, database-name evidence and table evidence; capture a genuine detail view. Keep credentials out of the published artifact. Explain missing names as unknown. The distinction is documented in the [README](../../README.md#start-with-a-project).

## MCP

### Ask for the edge before asking for the fix

A coding agent can make a plausible refactoring proposal before understanding the dependency it is trying to remove. Start with a smaller request: identify the source evidence for one cycle edge and state the coverage limits.

```sh
archguard mcp --root /absolute/path/to/commons-collections
```

Use the client's documented stdio configuration, as in the [MCP guide](../../site/docs/mcp.html). The command starts a server and waits for the client; it is not a one-shot report.

Recording script:

1. Ask: “Find one package cycle. Show every edge's source evidence and the exact project revision.”
2. Ask: “Why does `map` depend on `bag`? If that edge is absent in this revision, say so.”
3. Ask: “Simulate cutting the evidenced edge. Compare cycle groups and explain what this does not prove. Do not edit source.”
4. Review the result. Only then propose an implementation and its tests in a separate branch.

**Editorial TODO:** record the actual prompts, tool names, arguments and responses; retain unresolved and truncated-result notes. No scripted assistant answer presented as a transcript. Use a genuinely observed edge for the before/after; do not claim a repair from simulation alone.

## Documentation

### Keep the evidence in arc42; write the decisions yourself

Context and building blocks can be derived from supported evidence. The reasons for a boundary, a risk acceptance or an organisational constraint still need an author. Generated documentation is useful when that division is visible.

From the eShop checkout, using a release that advertises these formats:

```sh
archguard doc . --arc42 --stdout
archguard doc . --arc42 --format docx --out arc42-review.docx
archguard graph . --format structurizr
```

The first command previews Markdown. The second writes a Word file; the last writes the Structurizr workspace under `.archguard/reports/graph.dsl`. Word export preserves headings and tables; diagrams remain Mermaid/PlantUML source rather than rendered images. Confluence import is a separate user action, not automatic synchronisation.

**Editorial TODO:** capture the actual export and import result, validate heading navigation, and show one manually authored decision beside generated evidence. Confirm installed CLI support before recording. Source: [export semantics](../../README.md#common-workflows) and [ADR 0013](../adr/0013-structurizr-and-asciidoc-export.md).

## Deployment

### Read the deployment before reaching for the cluster

A repository can already say a lot about how its services are intended to connect. Kubernetes selectors, Compose endpoints and supported Terraform resources provide different pieces of that picture. None proves that the declared deployment is currently running.

```sh
archguard services examples/checkouts/online-boutique --format json
ARCHGUARD_RENDER_MANIFESTS=1 archguard services examples/checkouts/online-boutique --format json
```

The second command opts into locally installed Helm/Kustomize rendering. It is a separate run with external tools, not an invisible default. Record their versions and compare diagnostics. Remote Terraform modules, variable-dependent values and unsupported constructs can remain unknown.

**Editorial TODO:** fresh-clone Online Boutique, record commit and licence, inspect whether the candidate still contains the expected K8s/Helm/Kustomize/Terraform files, and capture one configured edge with its original file location. Do not claim all eleven services or all deployment mechanisms were recovered unless the actual output establishes it.

## CI

### Make the boundary a reviewable gate

An architecture check becomes useful when the team agrees what it should reject. Begin with one boundary in a temporary Spring PetClinic branch, reproduce the violation locally, then let the same check run in CI.

```sh
archguard check . --format sarif > archguard.sarif
archguard review . --base main --format pr-comment
```

For existing cycles, `archguard baseline write .` records a deliberate adoption baseline; review and commit it once, then use `archguard check . --baseline`. Rewriting it in every CI run accepts new cycles.

Use the repository's [Action definition](../../action.yml) as the source of truth for inputs. It produces SARIF, can write a sticky PR comment, and needs the comparison base to be fetched. Fork PR comments are skipped when the token is read-only. Pin the selected action commit in the final tutorial.

**Editorial TODO:** create a real failing/passing example, capture its PR comment and KPI deltas, and time the walkthrough before using “five minutes” in the headline. Preserve nonzero exit status when uploading the report. Do not create a deliberately failing public PR merely for a screenshot without the maintainer's intent.

## Supply chain

### An offline inventory should say what it cannot identify

A lockfile can establish a package and version without providing a licence. An offline tool should report that gap. ArchGuard's SBOM reads supported manifests and lockfiles; it does not query package registries to fill missing licence fields.

```sh
archguard sbom . --format cyclonedx > sbom.cdx.json
archguard check .
```

A configured licence policy can reject named licences or unknown values. That is a project policy result, not a legal determination about the application. A repository's own licence is not automatically a dependency licence.

**Editorial TODO:** choose a current Mastodon revision only after checking which supported JavaScript lockfiles it contains. **Ruby/Gemfile.lock is not in the README's supported SBOM inputs**, so do not promise a combined Gems/npm inventory. Also, Yarn dependencies can have unknown licences; use an actually supported lockfile with explicit licence fields for a licence-policy demonstration if necessary. Capture the real result and label omitted ecosystems. Recheck `archguard sbom --help` on the release being demonstrated.
