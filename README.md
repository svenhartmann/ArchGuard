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

Deployment evidence comes from Compose files and plain Kubernetes manifests. For Kubernetes, ArchGuard reads Deployment, StatefulSet, DaemonSet, Job, CronJob and Pod workloads, matches Services to them by selector, and links literal endpoint variables such as `ORDERS_URL` to the selected workload. Values from `envFrom` and `configMapKeyRef` count when the ConfigMap is declared in the same manifest directory and namespace. Secrets are never read, Helm templates are not rendered, Kustomize overlays are not applied, and no cluster is contacted; such references are reported as analysis gaps.

## Languages and source metrics

| Language | Import graph | Additional source analysis | Required external tools |
| --- | --- | --- | --- |
| Java | Yes | Types, methods, calls and supported metrics | JDK 17+ and matching project references |
| C# | Yes | Types, methods, calls and supported metrics | .NET SDK 8+, target reference packs and existing dependency metadata |
| TypeScript / JavaScript | Yes | Types, calls and supported metrics; JavaScript requires `allowJs` | Node.js and an installed TypeScript compiler; `ARCHGUARD_TRUST_PROJECT_COMPILER=1` to run a compiler found through the repository |
| Go | Yes | Packages, structs, interfaces, receiver methods, function groups and source metrics | Go SDK on `PATH`, or `ARCHGUARD_GO` pointing to its executable |
| Rust | Yes | Complexity, Cognitive and Halstead per function from syntax; call targets from rust-analyzer after opt-in; types stay unresolved | None; rust-analyzer and `ARCHGUARD_TRUST_PROJECT_COMPILER=1` for calls |
| Python | Yes | Classes, functions, Complexity, Cognitive and Halstead per function from syntax; call targets resolved statically from imports, module functions, classes, `self`/`cls` and `super()`; other calls and types stay unresolved | Python 3.9+ on `PATH`, or `ARCHGUARD_PYTHON` pointing to it; imports need nothing |
| PHP | Yes | Classes, functions, Complexity, Cognitive and Halstead per function from ArchGuard's token scanner; calls and types stay unresolved | None |
| Kotlin (`.kt`) | Yes, resolved together with Java | Classes, objects, functions, Complexity, Cognitive and Halstead per function from ArchGuard's token scanner; calls and types stay unresolved; Gradle `.kts` scripts are not scanned | None |

Dependencies written as fully qualified names without an import also count: PHP `\A\B\C`, Rust `crate::`, `self::` and `super::` paths, and Java/Kotlin/C# dotted names such as `com.a.B` or `A.B.C`. For Java, Kotlin and C#, the scanner links such a name only to a type file of a declared package or namespace; the compiler helpers additionally resolve names hidden behind variables.

Go uses the bundled `go/parser` and `go/types` helper without executing project code or downloading modules. Embedding is composition: DIT/NOC are not applicable. Dynamic calls, missing references, and combined build variants remain explicit analysis gaps.

Python uses a bundled helper on the standard `ast` module, run isolated (`-I -S`) so that no project code is imported or executed. It parses with the grammar of the local interpreter: a file using newer syntax than that Python (for example `match` on 3.9) is reported as a syntax diagnostic, not measured. A class counts as abstract when it lists `ABC` or `Protocol` as a base, uses `metaclass=ABCMeta` or declares an `@abstractmethod`. A call is resolved only where the source binds its target: an import (including relative imports and re-exports through `__init__.py`), a module-level function, a class, `self`/`cls` or `super()` along the C3 order of project classes. Calls on local variables, call results, rebound names and star imports are listed as `unresolved`; builtins and other libraries as `external`. `archguard calls` reports the counts; the report stays `partial` because types are not resolved. Rust call targets come from `rust-analyzer scip` once `ARCHGUARD_TRUST_PROJECT_COMPILER=1` is set outside the repository. ArchGuard replaces the build-script command, so no `build.rs` or proc macro runs; cargo stays offline and ignores `RUSTC_WRAPPER`. Calls in code generated by build scripts or proc macros therefore stay unresolved.

PHP and Kotlin are measured by ArchGuard's own token scanner without a PHP installation or Kotlin compiler; interfaces and `abstract` classes count as abstract, in Kotlin also `sealed` classes. Kotlin property accessors, constructors and `init` blocks are not measured. Method-limit quality gates stay incomplete for Python, PHP and Kotlin, as for Rust, because syntax alone is no complete compiler analysis.

**Duplicates.** `archguard duplicates . --report cpd.xml` imports PMD CPD XML. Without PMD, `archguard duplicates . --scan` runs ArchGuard's built-in token scanner over the walked sources (`.gitignore` and `project.exclude` apply). It reports identical token runs of at least `--min-tokens` (default 100; each punctuation character is one token, so counts are finer than CPD's). `--normalize` also matches copies whose identifiers and literals differ. Groups are token matches, not semantic equivalence. Files over 1 MiB, a budget of 5 million tokens and a 120 s limit mark the result `partial` with a diagnostic. A scan replaces an imported CPD report only with `--replace`; the `producer` field says which one is stored.

**KPI help** in the viewer lists setup, supported measurements, and import examples for each language. Coverage and runtime measurements require actual reports; missing data is not treated as zero.

## Common workflows

| Task | Command |
| --- | --- |
| Explore interactively | `archguard serve .` |
| Check architecture rules | `archguard check .` |
| Inspect cycles | `archguard cycles . --format json` |
| Inspect compiler evidence | `archguard calls . --format json` |
| Inspect service connections | `archguard services . --format json` |
| Generate or update arc42 evidence blocks | `archguard doc . --arc42` |
| Compare with a Git revision | `archguard review . --base main --format html` |
| Inspect effective configuration | `archguard config . --format json` |

Use `archguard --help` and `archguard <command> --help` for the complete command reference. The viewer also includes CLI help and natural-language MCP examples.

Arc42 generation updates explicitly marked evidence blocks. Your text outside those blocks remains unchanged. Generated architecture observations do not replace team decisions or establish a complete runtime architecture.

## Configuration and project data

Configuration precedence is: explicit `--config`, `.archguard/archguard.toml`, legacy `archguard.toml`, then derived values and defaults. Relative source paths are resolved against the project root. An invalid configuration is reported as an error rather than silently replaced.

For a project without an existing configuration:

```sh
archguard init /path/to/project
archguard config /path/to/project --format json
```

Review the generated rules before using them as gates. Derived layers are informational; explicitly declared rules can fail a check. `init` does not overwrite an existing configuration.

Project-local output lives under `.archguard/`: configuration, imported evidence, reports, caches, and recorded history. Generated reports and caches can be regenerated; retain intentional configuration, baselines, scenarios, and recorded measurements. Details and examples are in [ARCHITECTURE.md](ARCHITECTURE.md).

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
