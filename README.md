# ArchGuard

[Deutsch](README_DE.md)

ArchGuard analyzes software dependencies from source code and project configuration. It provides a Rust CLI, a local browser viewer, and an MCP server for coding agents. Start without a configuration file; add explicit rules when you want findings to become CI gates.

ArchGuard is developed as a non-commercial open-source project. It is available under MIT OR Apache-2.0; both licenses also permit commercial use of the software.

## What it provides

- Package and file dependency graphs, cycle groups, layer checks, and coupling metrics.
- A World View with service, package, and source drilldown; tree graphs, a tree matrix, and contextual detail overlays.
- Compiler-backed types, methods, calls, and source metrics where a language adapter and its required tools are available.
- Service connections derived from supported configuration and source patterns, with separately imported OpenTelemetry trace evidence.
- Architecture documentation in arc42 form, declared target architecture, Git comparisons, and read-only refactoring simulations.
- Imports for coverage, quality findings, test results, duplication, mutation testing, API contracts, and runtime measurements.

The viewer keeps static observations, declared intent, imported measurements, and simulated changes distinct. It is available in English and German.

## Install

Install the latest release binary (macOS, Linux, Git Bash) into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.sh | sh
```

Windows (PowerShell), into `%LOCALAPPDATA%\Programs\archguard`, added to the user `PATH`:

```powershell
irm https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.ps1 | iex
```

Both verify the archive against `SHA256SUMS`. `ARCHGUARD_VERSION` pins a version, `ARCHGUARD_INSTALL_DIR` changes the target. Update later with `archguard update`.

The scripts also install the desktop app from the same release, checked against `SHA256SUMS-desktop`: `ArchGuard.app` in `~/Applications` on macOS (`ARCHGUARD_APP_DIR`), `archguard-desktop` (AppImage, x86_64 and aarch64) on Linux, `archguard-desktop.exe` on Windows. `ARCHGUARD_DESKTOP=0` skips it.

Or build from the repository with a Rust toolchain:

```sh
cargo install --path crates/archguard-cli --locked
```

The optional graph database requires CMake and a C++ toolchain:

```sh
cargo install --path crates/archguard-cli --features db --locked
```

Without `db`, the analysis and viewer remain available; database commands and Cypher queries are omitted. See [release packages and verification](RELEASES.md).

## Start with a project

```sh
archguard serve /path/to/project --port 8787
archguard check /path/to/project
archguard graph /path/to/project --format html
```

Open **http://127.0.0.1:8787/**. A detected multi-project repository starts with its project and service overview. Select a project to inspect its source scope. Use **Copy link** to retain the selected project and page across reloads or share them with a colleague running the same checkout locally.

In World View, use the wheel to zoom, drag to pan, and right-drag or Shift-drag to rotate. Touch supports pinch zoom and two-finger rotation. Select an element for context; use Details or Enter to open its overlay, and Esc to close it. The viewer help explains the remaining controls.

Service discovery respects `.gitignore` and `project.exclude`. Nested `examples/`, `fixtures/`, agent worktrees and generated build/report directories are excluded; open a sample directory directly to analyze it. A Rust binary is an application project, not automatically a service: deployment evidence is required.

Deployment evidence comes from Compose files, plain Kubernetes manifests and Terraform. For Kubernetes, ArchGuard reads Deployment, StatefulSet, DaemonSet, Job, CronJob and Pod workloads, matches Services to them by selector, and links literal endpoint variables such as `ORDERS_URL` to the selected workload. Values from `envFrom` and `configMapKeyRef` count when the ConfigMap is declared in the same manifest directory and namespace. Secrets are never read and no cluster is contacted; such references are reported as analysis gaps.

Helm charts and Kustomize overlays are rendered only after you set `ARCHGUARD_RENDER_MANIFESTS=1`, because rendering starts external tools. This switch is separate from `ARCHGUARD_TRUST_PROJECT_COMPILER`: that one allows a compiler that the repository itself names, while rendering uses your installed `helm` (or `ARCHGUARD_HELM`) and `kustomize`, or `kubectl kustomize` when `kustomize` is missing (or `ARCHGUARD_KUSTOMIZE`). Without the switch or the tool, every chart and overlay appears as an analysis gap with this hint (subcharts and bases with the chart or overlay that uses them). The tools run with an empty environment, temporary home and cache directories and no kubeconfig, within 120 seconds and 64 MiB of output; beyond that budget nothing from the render is used and the topology reports `complete: false`. Helm runs `helm template <chart name> <chart> --namespace default --skip-tests` with only the chart's own `values.yaml`; other values files, `--set` and `lookup` (empty without a cluster) stay unknown. ArchGuard never runs `helm dependency build` or `--dependency-update`: dependencies missing from `charts/` are reported and the chart is not rendered. A chart with a symlink leading out of the analyzed directory is not rendered. Kustomize runs `kustomize build` without plugins, exec functions or Helm; an overlay is not run when any referenced resource, base, component, patch or generator file is a URL, a git reference, missing or outside the analyzed directory, or when it declares `helmCharts`. Rendered Secrets are dropped unread. Rendered evidence points to the Helm template file or the `kustomization.yaml` and says "Rendered by helm template from …" or "Rendered by kustomize build from …"; the chart or overlay directory is its deployment scope. Files a successful render consumes (the whole chart directory; the overlay directory and the files and bases it references) are read only from the rendered output, not again as plain manifests, so a base shared by two overlays appears once per overlay and never on its own. If a render fails, its files are read as plain manifests as before.

Terraform is read statically from `*.tf` files ([ADR 0015](docs/adr/0015-terraform-deployment-evidence.md)): ArchGuard never runs `terraform`, never downloads providers or modules and never reads state, `.tfvars` or `.terraform`. Workloads are `kubernetes_deployment` and `kubernetes_stateful_set` (selected by `kubernetes_service` as above), `aws_ecs_service` with the `container_definitions` (from `jsonencode(...)` or literal JSON) of its task definition, `aws_lambda_function`, `google_cloud_run_v2_service`, `google_cloud_run_service`, `azurerm_container_app` and `azurerm_linux_web_app`/`azurerm_windows_web_app` (`app_settings`). Managed datastores such as `aws_db_instance`, `aws_rds_cluster`, `aws_elasticache_*`, `google_sql_database_instance`, `azurerm_postgresql_flexible_server` and `azurerm_mssql_server` become datastore nodes; only their engine setting is read. Literal environment values count as above; a reference such as `aws_db_instance.orders.address` links the workload to that datastore, and a reference to another workload becomes a configured reference, both within the same module. Local modules (`source = "./…"`) are followed with the address prefix `module.<name>.`; remote modules, variables, locals, module outputs and data sources stay unknown and appear as analysis gaps with the reason. Secret resources such as `aws_secretsmanager_*`, sensitive variables and variable defaults are never read. Evidence names the resource and position, e.g. "Declared in Terraform resource aws_ecs_task_definition.orders (main.tf:34)". `for_each`, `count`, `dynamic` blocks and `*.tf.json` are not expanded.

The service topology also lists datastores. Compose services and Kubernetes workloads with well-known database images and Terraform datastore resources (PostgreSQL, MySQL, MariaDB, MongoDB, Redis, SQL Server, Oracle, Cassandra, Elasticsearch and others) become datastore nodes. Connection URLs (`postgres://`, `mysql://`, `mongodb://`, `redis://`, `jdbc:…`), host/database settings such as `DB_HOST` with `DB_NAME` or `POSTGRES_DB`, and Spring `spring.datasource.url` link a service to such a node or to a configured external host. Only engine, host, port and database name are kept; credentials and URL options never appear. Two services with the same datastore and database name are reported as a **shared database**; the same server with different or unknown database names only as a **shared server**. Tables come from each service's own source ([ADR 0012](docs/adr/0012-datastore-tables.md)): EF Core (`DbSet<T>`, `[Table]`, `ToTable`, migrations with `CreateTable`), JPA and TypeORM `@Entity`/`@Table`, SQLAlchemy `__tablename__` and `CREATE TABLE`/`ALTER TABLE` in SQL files such as Flyway or Liquibase migrations. If two services on the same database name the same table, the finding becomes a **shared table** with those tables listed; if every service's tables are known and none repeats, the shared database is marked as having **disjoint tables**. Table names from constants, variables or placeholders stay unknown and appear as analysis gaps; queries are not analyzed, and a configured datastore does not prove runtime use. Values from Secrets stay unknown and appear as analysis gaps.

GraphQL schemas count as API contracts without an import ([ADR 0014](docs/adr/0014-graphql-contracts.md)). `*.graphql`, `*.graphqls` and `*.gql` files and `gql`/`graphql` tagged templates in TS/JS belong to the service whose build context or project directory contains them. ArchGuard reads types, fields, arguments, enum values and union members; directives such as federation `@key` are kept as text and not interpreted. An operation (`query`, `mutation`, `subscription`) in such a file or template links its service to the one service whose schema declares all its root fields; the link also becomes an HTTP connection of the topology. If no schema or several schemas match, or the document lies outside every service, the operation stays unresolved with the reason. A configured endpoint whose variable name or URL contains `graphql` links to the target service when that service has a schema. `archguard contracts . --graphql-baseline` saves the current schemas to `.archguard/graphql-contracts.json`; later runs report removed types and fields, removed arguments, new required arguments and arguments that became non-null as breaking, with the services whose linked operations use the changed root field. Nested selections are not checked against the schema.

## Languages and source metrics

| Language | Import graph | Additional source analysis | Required external tools |
| --- | --- | --- | --- |
| Java | Yes | Types, methods, calls and supported metrics | JDK 17+ and matching project references |
| C# | Yes | Types, methods, calls and supported metrics | .NET SDK 8+, target reference packs and existing dependency metadata |
| TypeScript / JavaScript | Yes | Types, calls and supported metrics; JavaScript requires `allowJs` | Node.js and an installed TypeScript compiler; `ARCHGUARD_TRUST_PROJECT_COMPILER=1` to run a compiler found through the repository |
| Go | Yes | Packages, structs, interfaces, receiver methods, function groups and source metrics | Go SDK on `PATH`, or `ARCHGUARD_GO` pointing to its executable |
| Rust | Yes | Complexity, Cognitive and Halstead per function, abstractness, implemented traits and supertraits from syntax; call targets, trait targets and type references from rust-analyzer after opt-in | None; rust-analyzer and `ARCHGUARD_TRUST_PROJECT_COMPILER=1` for calls and type references |
| Python | Yes | Classes, functions, Complexity, Cognitive and Halstead per function from syntax; call targets resolved statically from imports, module functions, classes, `self`/`cls` and `super()`; other calls and types stay unresolved | Python 3.9+ on `PATH`, or `ARCHGUARD_PYTHON` pointing to it; imports need nothing |
| PHP | Yes | Classes, functions, Complexity, Cognitive and Halstead per function from ArchGuard's token scanner; call targets resolved statically from namespaces, `use` imports, project classes and traits, `$this`, `self`, `static` and `parent`; other calls and types stay unresolved | None |
| Kotlin (`.kt`) | Yes, resolved together with Java | Classes, objects, functions, Complexity, Cognitive and Halstead per function from ArchGuard's token scanner; call targets resolved statically from the package, imports, Kotlin classes, objects, companion objects, top-level functions, `this`, `super` and the project class hierarchy; with a JDK also into Java classes of the project; other calls and types stay unresolved; Gradle Kotlin DSL scripts are read for subproject dependencies only | None |
| C / C++ (`.c`, `.h`, `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hh`, `.hxx`) | Yes, `#include` against the including directory and the include directories of `compile_commands.json` and literal CMake `include_directories` | Classes, structs, unions, functions, Complexity, Cognitive and Halstead per function from ArchGuard's token scanner; abstract when a class declares a pure virtual function; calls and types stay unresolved | None |
| Swift (`.swift`) | Yes, to the Swift Package Manager target | Types, extensions, functions, Complexity, Cognitive and Halstead per function from ArchGuard's token scanner; `Type(…)` and `Type.staticMethod(…)` of a unique project type resolved, other calls and types stay unresolved; Xcode projects are not read | None |
| Ruby (`.rb`) | Yes: `require_relative`, literal `require` against `lib/`, Zeitwerk constants in Rails apps | Classes, modules, methods, Complexity, Cognitive and Halstead per method from ArchGuard's token scanner; call targets resolved statically for `Const.method`/`Const.new` on unique project classes, `self.method`, bare method calls and `super` along included/prepended modules and superclasses; other receivers stay unresolved, `send`/`define_method`/`method_missing` as dynamic | None |
| Scala (`.scala`) | Yes, resolved together with Java and Kotlin | Classes, case classes, traits, objects, enums, givens and defs in Scala 2 and 3 syntax, Complexity, Cognitive and Halstead per def from ArchGuard's token scanner; call targets resolved statically from the package, imports, Scala classes, objects and their `apply`, top-level defs, `this`, `super` and the project class hierarchy; implicits, givens, extension methods, calls on values and calls into Java or Kotlin classes stay unresolved; `.sc` scripts are not scanned ([ADR 0020](docs/adr/0020-scala-support.md)) | None |

Dependencies written as fully qualified names without an import also count: PHP `\A\B\C`, Rust `crate::`, `self::` and `super::` paths, and Java/Kotlin/C# dotted names such as `com.a.B` or `A.B.C`. For Java, Kotlin and C#, the scanner links such a name only to a type file of a declared package or namespace; the compiler helpers additionally resolve names hidden behind variables.

Go uses the bundled `go/parser` and `go/types` helper without executing project code or downloading modules. Embedding is composition: DIT/NOC are not applicable. Dynamic calls, missing references, and combined build variants remain explicit analysis gaps.

Python uses a bundled helper on the standard `ast` module, run isolated (`-I -S`) so that no project code is imported or executed. It parses with the grammar of the local interpreter: a file using newer syntax than that Python (for example `match` on 3.9) is reported as a syntax diagnostic, not measured. A class counts as abstract when it lists `ABC` or `Protocol` as a base, uses `metaclass=ABCMeta` or declares an `@abstractmethod`. A call is resolved only where the source binds its target: an import (including relative imports and re-exports through `__init__.py`), a module-level function, a class, `self`/`cls` or `super()` along the C3 order of project classes. Calls on local variables, call results, rebound names and star imports are listed as `unresolved`; builtins and other libraries as `external`. `archguard calls` reports the counts; the report stays `partial` because types are not resolved.

Rust declarations come from `syn` without any opt-in. Traits count as abstract; structs, enums and unions as concrete, so Martin A/I zones are measurable for Rust modules. `impl Trait for Type` and supertraits (`trait A: B`) become supertypes, resolved from syntax through `crate::`, `self::` and `super::` paths, `use` bindings, re-exports and globs of local modules. Traits under `std`, `core` and `alloc` are external; prelude names such as `Iterator`, other crates' traits (a workspace member cannot be told apart from a registry crate) and blanket impls (`impl<T> Trait for T`) stay unresolved. A file with an item macro invocation (for example `lazy_static!`) or a syntax error keeps its types unknown, so the abstractness of its module stays unmeasured; types that derive or attribute macros generate are not counted. Rust has no implementation inheritance: DIT/NOC are not applicable. Call targets, trait targets and the project types each type references in its fields, signatures and impl bodies come from `rust-analyzer scip` once `ARCHGUARD_TRUST_PROJECT_COMPILER=1` is set outside the repository; one index run serves calls and types. ArchGuard replaces the build-script command, so no `build.rs` or proc macro runs; cargo stays offline and ignores `RUSTC_WRAPPER`. Calls and types in code generated by build scripts or proc macros therefore stay unresolved, and which implementation a generic bound or trait object dispatches to is never resolved. CBO and RFC stay partial.

PHP and Kotlin are measured by ArchGuard's own token scanner without a PHP installation or Kotlin compiler; interfaces and `abstract` classes count as abstract, in Kotlin also `sealed` classes. PHP call targets are resolved as PHP resolves names at compile time: `namespace`, `use` imports (including aliases, group use and `use function`) and fully qualified `\A\B` names. `new C`, `C::m()`, `$this->m()`, `self::m()` and `parent::m()` are looked up in the declared project classes: own methods, then methods of used project traits, then the `extends` chain; `static::` is read as `self::`, so late static binding to a subclass is not followed. An unqualified function call tries the current namespace first and then the global project function. Calls on other variables, dynamic names (`$f()`, `$o->$m()`), `__call`, `$this` inside closures (which `Closure::bind` can rebind), abstract methods, unknown or external base classes, trait `insteadof`/`as` rules and names declared twice are listed as `unresolved`; a callable passed to `call_user_func` is not followed. Classes and functions declared nowhere in the project are `external`, but only when every PHP file could be scanned. Calls inside strings, nested functions and anonymous classes are not counted. `archguard calls` reports the counts; the report stays `partial` because types are not resolved.

Kotlin call targets are resolved in Kotlin's name order: explicit imports (including `import a.B as C`), then the file's package, then star imports, which resolve a name only when exactly one imported package declares it. `C(…)`, `C.m()`, `C.Nested(…)`, `C.Companion.m()`, `Obj.m()` and fully qualified `a.b.C(…)` are looked up in the scanned Kotlin classes, objects and companion objects; `this.m()`, a bare `m()` inside a class and `super.m()`/`super<I>.m()` along the class's own functions, its companion, nested and outer classes and its project supertypes, including interface default methods; top-level functions through the package and imports. Calls on variables, parameters, properties and call results, safe calls, extension functions, scope functions (`let`, `run`, `apply` …), labeled `this@L`, abstract or overloaded functions, the generated `copy`/`componentN` of data classes, names declared twice, `invoke` on objects and every call inside lambdas, anonymous, local and extension functions (where an implicit receiver of unknown type may be in scope) are listed as `unresolved`. A class with a supertype outside the scanned Kotlin and Java code (a library class, a Java class without javac declarations, and every enum class) resolves only its own functions; everything inherited from there, and names its unknown nested classes could shadow, stays `unresolved`. Overloads are not told apart by argument types: a function declared once in the class itself wins. With a JDK, Java classes of the project are known from the declarations of the javac helper: `J(…)` (not for abstract classes, interfaces and enums), `J.staticMethod()`, `J.Nested(…)`, a static method imported with `import a.J.m`, and functions a Kotlin class inherits from a Java superclass resolve to the Java class or method, with the Java identity. Annotation processors such as Lombok may generate members that javac without processors does not report, and every Java class inherits from `java.lang.Object`; so a name declared neither in the Java class nor in its project supertypes stays `unresolved`, as do instance methods called on the class name, abstract Java methods and Java overloads: a call links one method signature, and overloads are not told apart by argument types. Without a JDK, and for a Java file javac reported no class for, Java classes are known only by package and file name; calls to them stay `unresolved`, never `external`. Names declared nowhere in the project (Kotlin or Java), such as `println` or `listOf`, are `external`, but only when every Kotlin and Java file could be read. Infix and operator calls are not counted as call sites, nor are calls in string templates, object expressions, local classes, property initializers, accessors, constructors and `init` blocks, which are not measured either. Method-limit quality gates stay incomplete for Python, PHP and Kotlin, as for Rust, because syntax alone is no complete compiler analysis.

Gradle Kotlin DSL scripts are neither measured nor run. `archguard workspace` reads `<configuration>(project(":a:b"))` in a `build.gradle.kts` as a dependency on the subproject whose build script lies in `a/b` below the nearest `settings.gradle.kts`, when a literal `include(…)` there names the project. `test…` configurations get the scope `test`, `runtimeOnly` `runtime`, `compileOnly` `provided`, `kapt`, `ksp` and `annotationProcessor` `build`, all others `compile`. Computed and relative paths (`project(name)`, `project("a")`), projects whose directory the settings move, and included projects without a build script are listed as unresolved; typesafe `projects.a` accessors and Groovy `build.gradle`/`settings.gradle` are not read.

C and C++ share one language (`cpp` in `project.languages`), so a `.h` header needs no guess ([ADR 0017](docs/adr/0017-c-cpp-support.md)). A quoted `#include "x.h"` resolves against the including file's directory first, then against the include directories (`-I`, `-iquote`, `-isystem` from `compile_commands.json` in the root or `build/`, and `include_directories`/`target_include_directories` arguments without variables other than `${CMAKE_CURRENT_SOURCE_DIR}` in the `CMakeLists.txt` files above the sources); an unresolved quoted include is reported as an unresolved import. An angled `#include <x.h>` resolves only against the include directories; otherwise it is a system or library header and stays external without a warning. Include directories are pooled for the whole repository, so a header found in two of them stays unresolved. The package is the directory. No compiler or preprocessor runs: directive lines are skipped and the code of every `#if`/`#else` branch is measured as written, both branches at once; a file whose branches leave brackets unbalanced is reported and not measured. Out-of-class definitions `A::f` belong to the class `A` the project declares once (also through `using namespace`), otherwise to the file's module. Calls and types are not resolved, so the report stays `partial`.

Swift is measured by the same kind of token scanner without a Swift toolchain ([ADR 0018](docs/adr/0018-swift-support.md)). A Swift module is a Swift Package Manager target: every `.target`, `.executableTarget`, `.testTarget` and `.macro` with a literal `name:` in a walked `Package.swift` (directory `path:` when literal, else `Sources/<name>` or `Tests/<name>`); the manifest is read as text, never run. `import X` links a file to the directory of target `X`; platform modules such as `Foundation` are ignored, other imports are reported as external. Xcode projects are not read: their files form one module and a diagnostic says how many there are. Classes, structs, enums, actors and protocols are measured (protocols count as abstract); `extension T` hands its functions and conformances to the one project type `T` of that name, otherwise it stays a class of kind `extension`. `func`, `init`, `deinit` and `subscript` with a body are measured; computed properties, accessors, property initializers and top-level statements are not. Complexity counts `if`, `guard`, loops, `catch`, each `switch` `case`, `&&`, `||`, `??` and the ternary `?`; `if let`/`guard let` count like `if`/`guard`, the commas of a condition list and `?.` do not, and the `else` of a `guard` adds no cognitive load. Only `Type(…)`, `Type.init(…)`, `Type.Nested(…)` and `Type.staticMethod(…)` of a unique top-level project type of the calling target or an imported one are resolved; calls on values, `self`, `super`, bare functions, trailing closures, generic `Type<T>(…)` and overloads stay `unresolved`, and nothing is called `external`. Regex literals are not recognized.

Ruby is measured by ArchGuard's own token scanner without a Ruby installation ([ADR 0019](docs/adr/0019-ruby-support.md)); heredocs, `%w`/`%i` and other percent literals, regexes, symbols and `#{…}` interpolation do not break it. The package of a file is its directory. `require_relative` resolves against the requiring file and a literal `require 'a/b'` against every `lib/` directory of the project; gems, the standard library and computed paths stay external. Constant references such as `Billing::Invoice` become import edges only in a Rails app (a `config/application.rb`) or a gem that references `Zeitwerk::Loader`: Zeitwerk naming maps them to `billing/invoice.rb` below `app/*`, `app/*/concerns` or the loader's directory, innermost lexical scope first; elsewhere they stay unresolved. Ruby has no abstract classes, so abstractness and the Martin zones do not apply (not "unknown"). `include`, `prepend` and `extend` count as supertypes. Calls in class bodies and top-level code are not counted, locals are every name assigned anywhere in the method, and calls inside `#{…}` are not counted.

**Duplicates.** `archguard duplicates . --report cpd.xml` imports PMD CPD XML. Without PMD, `archguard duplicates . --scan` runs ArchGuard's built-in token scanner over the walked sources (`.gitignore` and `project.exclude` apply). It reports identical token runs of at least `--min-tokens` (default 100; each punctuation character is one token, so counts are finer than CPD's). `--normalize` also matches copies whose identifiers and literals differ. Groups are token matches, not semantic equivalence. Files over 1 MiB, a budget of 5 million tokens and a 120 s limit mark the result `partial` with a diagnostic. A scan replaces an imported CPD report only with `--replace`; the `producer` field says which one is stored.

**KPI help** in the viewer lists setup, supported measurements, and import examples for each language. Coverage and runtime measurements require actual reports; missing data is not treated as zero.

Martin A/I zones need abstractness from compiler evidence (Java, C#) or Rust declarations, so `archguard scan` and `archguard check` report them as not measured by default. Add `--measure-zones` (MCP: `review_kpis` with `measure_zones=true`) to run the slower call analysis and fill the zones row; see [Review metrics and their limits](ARCHITECTURE.md#review-metrics-and-their-limits).

## Common workflows

| Task | Command |
| --- | --- |
| Explore interactively | `archguard serve .` |
| Check architecture rules | `archguard check .` |
| Inspect cycles | `archguard cycles . --format json` |
| Inspect compiler evidence | `archguard calls . --format json` |
| Inspect service connections | `archguard services . --format json` |
| Inspect API contracts (imported OpenAPI, GraphQL from the sources) | `archguard contracts . --format json` |
| Inspect recorded runtime (requests, DB calls per request, slow and repeated queries, profiler samples per request; [ADR 0004](docs/adr/0004-runtime-probes.md)) | `archguard runtime . --format json` |
| Generate or update arc42 evidence blocks | `archguard doc . --arc42` |
| Export arc42 as AsciiDoc or Word (for a Confluence import) | `archguard doc . --arc42 --format asciidoc` or `--format docx` |
| List or draw the arc42 diagrams in the terminal | `archguard diagram .` or `archguard diagram . building-blocks` |
| The arc42 document and its diagram catalogue as JSON | `archguard doc . --arc42 --format json` |
| Export the service topology as a Structurizr (C4) workspace | `archguard graph . --format structurizr [--service orders]` |
| Compare with a Git revision | `archguard review . --base main --format html` |
| Inspect effective configuration | `archguard config . --format json` |
| SBOM of third-party dependencies (CycloneDX 1.6, offline) | `archguard sbom . --format cyclonedx` |

Use `archguard --help` and `archguard <command> --help` for the complete command reference. The viewer also includes CLI help and natural-language MCP examples.

Arc42 generation updates explicitly marked evidence blocks. Your text outside those blocks remains unchanged. Generated architecture observations do not replace team decisions or establish a complete runtime architecture.

Without `--out`, `doc --arc42` uses the arc42 text the desktop app edits: the shallowest `arc42.md` or `arc42/` folder up to four folder levels deep (not in hidden or build folders such as `target`, `node_modules`). A folder holds one file per chapter, named by its number (`05-building-blocks.md`, `arc42-05.md`); each is checked and updated on its own. Only when the project has neither, `doc --arc42` creates `arc42.md` in the project folder. Diagram blocks that the desktop's "Insert into chapter" put into a chapter of your own text (`<!-- archguard:diagram:ID:start/end -->`) are refreshed like the evidence blocks and a hidden one's block is removed. A chapter of your own text gets only the diagrams you chose for it: a diagram listed in `[arc42] shown_diagrams` (the desktop's "In documentation" switch writes it) and missing from such a chapter is added at its end; the other diagrams are only offered in the desktop's "Diagrams" tab. Chapters with generated blocks contain every diagram not listed in `[arc42] hidden_diagrams`. `--check` names chapters these rules would change as outdated.

`--format asciidoc` writes `arc42.adoc` and `--format docx` writes `arc42.docx` (or `--out`). Both convert the arc42 text `doc --arc42` finds (file or chapter folder) with its evidence and diagram blocks brought up to date, which they only read; without one they convert the generated document. `--chapter N` exports that chapter. AsciiDoc diagrams are `[mermaid]`/`[plantuml]` blocks for Asciidoctor Diagram. Word headings use the Heading 1–3 styles, so Confluence's "Import Word document" builds its table of contents from them; tables are Word tables, and diagrams stay Mermaid or PlantUML source, because the document has no rendered images. `graph --format structurizr` writes `.archguard/reports/graph.dsl`: one software system, the deployment services as containers, datastores outside the configuration as external systems, relations with protocol and configuration evidence, and with `--service` that service's packages as components. Mapping and limits: [ADR 0013](docs/adr/0013-structurizr-and-asciidoc-export.md). The desktop app offers the same arc42 files under File → Export, the MCP tool `architecture_document` returns AsciiDoc (`format=asciidoc`) and the workspace (`template=structurizr`).

### Diagrams in the terminal

`archguard diagram [PATH]` lists the arc42 diagram catalogue: every diagram ArchGuard derives, with its id, chapter, kind, whether the document shows it (`IN DOC`) and whether the project's arc42 text contains it (`IN TEXT`, by the rule the desktop uses). `archguard diagram [PATH] ID` draws one diagram; a single argument that is no folder is taken as the id. `--level PACKAGE` draws a package map level (`building-blocks@PACKAGE`, or `cycles --level PACKAGE`), the level the desktop's map opens, also when `[arc42] shown_diagrams` does not list it. Box graphs (context, building blocks, package maps, deployment, services, cycles, layers) are layered top-down from the using to the used side; a cycle is broken at one edge, which is drawn upwards, and `!` marks cyclic boxes and edges. Sequences show participants with lifelines, numbered messages, dashed returns, calls to oneself as a loop, events with a double head (`>>`) and errors with `✗`; the scenarios have no alt/loop frames, so none are drawn. Output uses Unicode box-drawing characters, `--ascii` plain ASCII; colour (cycles red, HTTP blue) only on a terminal without `NO_COLOR`; the width follows `COLUMNS` (default 100), wide rows wrap. `--format json` prints the diagram's data (elements and relations) or the whole catalogue instead. Example from `fixtures/py-layers`:

```text
$ archguard diagram fixtures/py-layers
ID                                                                CHAPTER  KIND      IN DOC  IN TEXT  TITLE
building-blocks                                                   5        packages  yes     yes      Package overview
runtime-calls                                                     6        packages  yes     yes      Static calls between building blocks
runtime-scenario-python-api-http-py-module-handle-4               6        sequence  yes     yes      Scenario · handle · api (static)
...
cycles                                                            11       packages  yes     yes      Cyclic package relations

$ archguard diagram fixtures/py-layers building-blocks
Package overview [building-blocks, chapter 5, packages]

┌─────────┐
│ api     │
│ package │
└────┬────┘
     │
     ▼ 2
┌─────────┐
│ ! app   │
│ package │
└──┬───┬──┘
   │   ▲ !1
!3 ▼   │
┌──────┴───┐
│ ! domain │
│ package  │
└──────────┘

counts: source connections

$ archguard diagram fixtures/py-layers runtime-scenario-python-api-http-py-module-handle-4
Scenario · handle · api (static) [runtime-scenario-python-api-http-py-module-handle-4, chapter 6, sequence]

      ┌─────┐           ┌─────┐         ┌────────┐
      │ api │           │ app │         │ domain │
      └──┬──┘           └──┬──┘         └────┬───┘
         │ 1. estimate     │                 │
         ├────────────────>│                 │
         │                 │ 2. price_cents  │
         │                 ├────────────────>│
         │                 │<╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
         │<╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤                 │
         │                 │                 │
```

`archguard doc --arc42 --format json [--chapter N]` prints the arc42 document as data and writes nothing: where the text lives (`target`: file or folder, exists), each chapter with number, title, the file that holds it (`null`: only generated) and its Markdown as the desktop reads it, and the catalogue as above, each diagram with `shown`, `chosen` (listed in `[arc42] shown_diagrams`), `in_text`, elements and relations. The MCP tool `architecture_document` returns the same catalogue with `template=arc42, format=catalogue`.

### Compare findings with an expert assessment

`archguard evaluate --init .` writes `.archguard/assessment.toml` with every current finding as an unjudged stub; it never overwrites an existing file. An expert records the problems they see (`[[expected]]`, with kind and node globs) and judges findings (`verdict = "true_positive"` or `"false_positive"`; empty means not judged). `archguard evaluate . [--assessment FILE] [--format json]` then reports recall (matched expected problems / expected problems), precision (true positives / judged current findings), unjudged findings, missed expected problems with reason and stale verdicts, and warns when the assessment revision is not `HEAD`. A metric with nothing counted is "not measured", never 0. ArchGuard never writes verdicts itself, and `evaluate` is not a gate. The MCP tool `evaluation` returns the same result. Format and matching rules: [ADR 0005](docs/adr/0005-evaluation-against-expert-assessment.md).

### SBOM and licence policy

`archguard sbom .` lists the third-party dependencies that the lockfiles and manifests in the repository state: `Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `packages.lock.json` (else `*.csproj` `PackageReference`), `pom.xml`, `gradle.lockfile`, `go.mod`, `poetry.lock`, `uv.lock` (else `requirements*.txt` / `pyproject.toml`) and `composer.lock` (else `composer.json`). It reads no registry and needs no network. `--format cyclonedx` writes CycloneDX 1.6 JSON with purl, version, scope and the manifests (projects/services) of each component; `--format json` and text give a summary. Licences come only from files that state them (`package-lock.json`, `composer.lock`); every other licence is `unknown`, never fetched or guessed. Workspace members and path/editable/project references are first-party and not listed; `fixtures/` is test data and skipped.

A licence policy in `.archguard/archguard.toml` makes `archguard check` fail (exit 1, one `license` finding per component, also in SARIF):

```toml
[licenses]
allow = ["MIT", "Apache-2.0", "BSD-3-Clause"]  # SPDX ids; non-empty = everything else violates
deny = ["GPL-3.0-only"]                         # always violates
unknown = "allow"                               # "deny": a dependency without stated licence violates
```

SPDX expressions are evaluated (`OR`: one allowed branch is enough, `AND`: all branches must pass). `unknown` is its own state and only a violation with `unknown = "deny"`. The MCP tool `sbom` returns the same data in pages. Decision: [ADR 0016](docs/adr/0016-sbom-and-licence-policy.md).

## Configuration and project data

Configuration precedence is: explicit `--config`, `.archguard/archguard.toml`, legacy `archguard.toml`, then derived values and defaults. Relative source paths are resolved against the project root. An invalid configuration is reported as an error rather than silently replaced.

`project.exclude` holds globs over paths relative to the project root, anchored at the root: `vendor` means `./vendor`, not `a/vendor`; use `**/vendor` for any depth. As in `.gitignore`, a pattern that matches a directory excludes everything below it and the directory is not entered: `vendor`, `vendor/`, `vendor/**` and `**/generated` all exclude whole directories. A trailing `/` matches directories only, never a file of that name. Negation (`!`) is not supported. The same rule applies to the source walk, service discovery and dependency discovery.

For a project without an existing configuration:

```sh
archguard init /path/to/project
archguard config /path/to/project --format json
```

Review the generated rules before using them as gates. Derived layers are informational; explicitly declared rules can fail a check. `init` does not overwrite an existing configuration.

Project-local output lives under `.archguard/`: configuration, imported evidence, reports, caches, and recorded history. Generated reports and caches can be regenerated; retain intentional configuration, baselines, scenarios, and recorded measurements. Details and examples are in [ARCHITECTURE.md](ARCHITECTURE.md).

### Quality goals

A quality goal can name the ISO/IEC 25010 characteristic it addresses, the part of the system it covers and a quality scenario. All three fields are optional:

```toml
[[architecture.goals]]
name = "Changeable checkout"
priority = "first"                       # your own wording; never ranked
rationale = "Weekly price changes"
attribute = "maintainability"            # functional_suitability, performance_efficiency, compatibility,
                                         # usability, reliability, security, maintainability, portability
scope = ["checkout/**"]                  # node-name globs; empty = whole system
scenario = "A price rule change ships within one day"
```

`architecture_context` (MCP and `archguard review`), `archguard doc` and arc42 chapters 1 and 10 then show per goal the matching finding ids, KPIs, the evidence sources and a status: `unclassified`, `not_assessable`, `no_evidence`, `findings` or `no_findings` (no findings from the available sources, not "goal met"). The mapping from evidence to characteristics is fixed: structural findings and review KPIs count for maintainability, a saved `archguard security` scan for security, imported measurements, test results, coverage and mutation reports for performance efficiency and reliability. ArchGuard has no evidence for the other four characteristics. Goals do not change `[review] health_weights`. Decision: [ADR 0003](docs/adr/0003-quality-goal-assessment.md).

## MCP for coding agents

Register ArchGuard as a stdio MCP server in your client, using an absolute project path:

```json
{
  "mcpServers": {
    "archguard": {
      "command": "archguard",
      "args": ["mcp", "--root", "/absolute/path/to/project"]
    }
  }
}
```

Example requests to the agent:

- “Show the services and their connections, and distinguish configuration from observed runtime evidence.”
- “Explain this package cycle using its source locations, then simulate a possible cut.”
- “Find the most complex methods in the checkout service and show the supporting measurements.”
- “Check my changes against the architecture rules and explain any remaining analysis gaps.”

The server exposes evidence-oriented tools and pagination where supported. Follow returned cursors and coverage diagnostics; a truncated response is not the complete result. Simulations do not modify source files.

## CI and development

Use `archguard check .` as the architecture gate. Exit codes are `0` for success, `1` for gate violations, `2` for usage errors, and `3` for analysis, I/O, or configuration errors.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node scripts/viewer/test_viewer_world.js
```

The workspace includes the native desktop viewer `archguard-desktop` ([ADR 0002](docs/adr/0002-native-desktop-ui.md)). On Linux it builds against the system fontconfig, so install its headers first (Debian/Ubuntu: `libfontconfig1-dev`).

The [`scripts/`](scripts/README.md) directory contains integration and browser tests, benchmarks, and release verification. These are development tools; normal CLI and viewer use does not require running them manually. Go integration tests use `ARCHGUARD_GO`; language-specific and browser tests may need additional installed tools.

The CI workflows also cover optional features, dependency advisories, secret scanning, native release checks, and malware scanning. Release bundles include verification reports and available scan logs; a clean scan is not a guarantee that software is harmless. See [SECURITY.md](SECURITY.md) and [RELEASES.md](RELEASES.md).

### GitHub Action

The repository ships a composite action. It installs a pinned or the latest release (checksums verified), runs `archguard check` as SARIF and, on pull requests, creates or updates one review comment. Until the action is part of a release, reference `@main` or a commit SHA.

```yaml
name: ArchGuard
on: pull_request
permissions:
  contents: read
  pull-requests: write    # review comment
  security-events: write  # SARIF upload
jobs:
  archguard:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - id: archguard
        uses: svenhartmann/ArchGuard@main
        with:
          version: latest
      - if: success() || failure()
        uses: github/codeql-action/upload-sarif@v4
        with:
          sarif_file: ${{ steps.archguard.outputs.sarif-file }}
          category: archguard
```

| Input | Default | Meaning |
| --- | --- | --- |
| `version` | `latest` | Release to install, e.g. `0.2.1`. |
| `path` | `.` | Project to analyze. Keep the repository root when you upload SARIF: file locations are relative to it. |
| `base` | PR base commit | Ref for `review --base`. |
| `comment` | `true` | Create or update one PR comment. |
| `fail-on-violation` | `true` | Fail on gate violations (exit 1). Exit 2 and 3 always fail. |
| `sarif-file` | `$RUNNER_TEMP/archguard.sarif` | SARIF output path. |
| `token` | `github.token` | Needs `pull-requests: write` for the comment. |

Outputs are `sarif-file`, `exit-code` and `comment-file`; the comment also goes to the job summary. If the checkout lacks the PR base commit, the action fetches it; if that fails, check out with `fetch-depth: 0`. Pull requests from forks get a read-only token, so the comment is skipped there. Only a comment written by `github-actions[bot]` is updated; with another token, each run adds a new comment. `check` gates the whole current state; to fail only on new findings, run `archguard check . --base <ref>` yourself. Linux and macOS runners are supported; on Windows the action stops with an error.

## Interpretation and limits

- Import and source graphs are not complete runtime call graphs. Reflection, dynamic dispatch, framework registration, generated code, and missing dependencies can leave gaps.
- A package cycle is not necessarily a file cycle. Cycle samples and suggested cuts are bounded examples, not exhaustive proofs of every possible cycle.
- Declared service endpoints do not prove that a request ran. Trace imports describe only the captured execution and their matched services.
- Unknown, partial, stale, and not-applicable measurements remain distinct. Compare metrics only across compatible language definitions, revisions, and scopes.
- Imported findings, coverage, and runtime reports retain their provenance. A report for another source revision is not current evidence.
- Refactoring simulations change the displayed graph only. Validate actual code changes with the project's own tests and a fresh analysis.

## Further documentation

- [Architecture and configuration reference](ARCHITECTURE.md)
- [Reproducible analysis examples](examples/README.md)
- [Service Shop demo](examples/service-shop/README.md)
- [Kafka example](examples/kafka.md) and [Jellyfin example](examples/jellyfin.md)
- [Context benchmark methodology](examples/context-token-benchmark.md)
- [Release packages](RELEASES.md) and [security policy](SECURITY.md)

Licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE). Third-party components and example projects retain their own licenses.
