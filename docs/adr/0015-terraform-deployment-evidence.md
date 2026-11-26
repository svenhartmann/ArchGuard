# ADR 0015: Terraform as static deployment evidence

- **Status:** accepted
- **Date:** 2026-11-18

## Context

The service topology takes its deployment evidence from Compose files, plain
Kubernetes manifests and, after opt-in, rendered Helm charts and Kustomize
overlays. Many systems declare their workloads and managed databases only in
Terraform: ECS services, Lambda functions, Cloud Run services, Azure web apps,
RDS instances, Cloud SQL and so on. Without Terraform these systems have no
deployment evidence and no datastore nodes.

Running `terraform plan` or reading state would evaluate variables and module
outputs, but it needs providers, credentials and often cloud access, and state
contains secrets. ArchGuard must not do any of that.

## Decision

1. ArchGuard parses `*.tf` files statically. It never runs `terraform`, never
   downloads providers or modules, and never reads state, `.tfvars` or the
   `.terraform` directory.
2. Each directory with `.tf` files is a module. A module that another module
   calls with a local `source = "./…"` or `"../…"` is read only as part of that
   call, with the address prefix `module.<name>.`; the deployment scope is the
   calling root module (`terraform:<dir>`). Remote sources, local sources that
   leave the analyzed directory and non-literal sources are analysis gaps.
3. Workloads: `kubernetes_deployment` and `kubernetes_stateful_set` (also
   `_v1`), `aws_ecs_service` with the containers of the `aws_ecs_task_definition`
   it references (a task definition without a service is its own workload),
   `aws_lambda_function`, `google_cloud_run_v2_service`,
   `google_cloud_run_service`, `azurerm_container_app`, `azurerm_linux_web_app`
   and `azurerm_windows_web_app`. ECS `container_definitions` are read from
   `jsonencode(...)` or literal JSON; other forms are gaps. Environment comes
   from `env` blocks, Lambda `environment.variables` and `app_settings`.
   `kubernetes_service` selectors give the selected deployment its cluster
   host names, as for plain manifests.
4. Datastores: `aws_db_instance`, `aws_rds_cluster` (engine from `engine`),
   `aws_elasticache_*`, `aws_dynamodb_table`, `google_sql_database_instance`
   (engine from `database_version`), `google_redis_instance`,
   `azurerm_postgresql_flexible_server`, `azurerm_postgresql_server`,
   `azurerm_mysql_flexible_server`, `azurerm_mssql_server` and
   `azurerm_redis_cache` become datastore nodes `terraform:<dir>:<address>`.
   Only the engine setting is read; a non-literal engine stays unknown.
5. Values: literal environment values go through the existing endpoint and
   datastore-setting rules. A reference such as `aws_db_instance.orders.address`
   links the workload to that datastore, a reference to a workload or a
   `kubernetes_service` becomes a configured reference, both only within the
   same module instance. Variables, locals, module outputs, data sources and
   other computed values are not evaluated; for endpoint or datastore settings
   they are analysis gaps with the reason. References to secret resources
   (for example `aws_secretsmanager_*`) are gaps; secret resources, sensitive
   variables and variable defaults are never read.
6. Evidence names the declaring resource and its position, e.g. "Declared in
   Terraform resource aws_ecs_task_definition.orders (main.tf:34)", with the
   line in `ServiceEvidence::line`.
7. Parsing uses the crate `hcl-edit` 0.10 (MIT OR Apache-2.0, part of the
   maintained `hcl-rs` project). It keeps byte spans of blocks and expressions,
   which give the line numbers. Its new transitive crates are `hcl-primitives`
   (MIT OR Apache-2.0), `pratt` (MIT) and `vecmap-rs` (MIT OR Apache-2.0);
   `winnow` was already in the tree.

## Alternatives

- **`hcl-rs` (`hcl` crate) with serde.** Same project and parser, but its
  `Body` has no spans, so evidence could not carry a line. It also pulls in
  serde machinery and an expression evaluator that ArchGuard must not use.
- **Own HCL parser.** HCL has heredocs, templates, nested objects and
  `jsonencode` calls; a hand-written parser would be more code and less
  reliable than a maintained crate.
- **`terraform show -json` / plan JSON.** Needs `terraform`, providers and
  usually credentials; plan output can contain secrets. Rejected for the same
  reasons that keep Helm and Kustomize opt-in.
- **Evaluating variables and locals.** Defaults and `.tfvars` describe one
  environment of many and may hold secrets. Unknown stays unknown.

## Consequences

- Systems deployed with Terraform get workloads, managed datastore nodes and
  datastore usages without any tool or cloud access.
- Cross-module references (module outputs) and values passed as variables are
  reported as gaps, so `complete` is often `false` for real Terraform code.
- `*.tf.json`, `for_each`/`count` instances and `dynamic` blocks are not
  expanded; a resource counts once.

## History

- **2026-11-23:** [ADR 0023](0023-elasticsearch-artifacts.md) reads
  Elasticsearch names from non-sensitive `variable` defaults and literal module
  arguments. This applies only to those names; the decision above (variables
  and locals stay unknown for workloads and datastores) is unchanged.
- **2026-11-25:** Only for Elasticsearch index detail (mappings, settings,
  analysis), [ADR 0023](0023-elasticsearch-artifacts.md) reads literal
  `local.*` values and `elasticstack_elasticsearch_index.<r>.<attr>` of the
  same module instance, one level deep. Workloads, datastores, names, aliases
  and pipelines still do not read locals.
- **2026-11-25:** For Elasticsearch index detail the lookup of `local.*` and
  sibling index attributes is extended to depth 3 (index, then up to two
  references) with cycle protection, and `merge()` of literal objects and
  `concat()` of literal lists are evaluated ([ADR 0023](0023-elasticsearch-artifacts.md)).
  `var.*`, conditionals and `each.*` stay unknown.
- **2026-11-25:** For Elasticsearch index detail, `for` expressions over
  literal collections and access on resolved literal values are evaluated too
  ([ADR 0023](0023-elasticsearch-artifacts.md)); nothing else in this ADR changes.
- **2026-11-26:** For Elasticsearch, the evaluator also reads names through `local.*` and
  `jsondecode(file())` under the project root, with reference depth 5
  ([ADR 0023](0023-elasticsearch-artifacts.md)); nothing else in this ADR changes.
