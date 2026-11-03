# Jellyfin: a real C# analysis example

[English](jellyfin.md) | [Deutsch](jellyfin_DE.md)

[Jellyfin](https://github.com/jellyfin/jellyfin) is a free media server with an HTTP
API, media library, metadata providers, streaming and transcoding. This example
analyzes its actual C# sources without building or starting Jellyfin. Import
analysis requires neither .NET nor databases or media collections. With .NET SDK
8+ installed, Roslyn adds resolved internal calls and inheritance relationships
to the file/package graph. Missing compilers or references are reported as
limited coverage.

Uses **v10.11.11**, pinned to commit
[`1fbd8739292cce610231be93daf43368733edf63`](https://github.com/jellyfin/jellyfin/tree/1fbd8739292cce610231be93daf43368733edf63).
The GPL-2.0 `LICENSE` stays in the upstream checkout. Third-party source lives
only in the ignored `examples/checkouts/jellyfin` directory.

## Start

Run commands from the ArchGuard repository. Clone once for a fresh checkout:

```bash
git clone --depth 1 --single-branch --branch v10.11.11 \
  https://github.com/jellyfin/jellyfin.git examples/checkouts/jellyfin
git -C examples/checkouts/jellyfin rev-parse HEAD

mkdir -p examples/checkouts/jellyfin/.archguard
cp -n examples/jellyfin.toml examples/checkouts/jellyfin/.archguard/archguard.toml

cargo run -p archguard-cli --locked -- serve examples/checkouts/jellyfin \
  --port 8790 --depth 1
```

The commit ID must match the linked revision above. The viewer runs at
<http://127.0.0.1:8790/>; `Ctrl-C` stops it. Source changes update the analysis.
Other examples can run alongside it.

Setup copies the [configuration template](jellyfin.toml) into the analyzed
checkout; `cp -n` preserves an existing config. `.archguard/archguard.toml` is then
loaded automatically. The graph, effective config and standalone HTML report
also live under the checkout's `.archguard`, following the
[project convention](../README.md#configuration-and-project-data).
`--config examples/jellyfin.toml` remains an explicit alternative.

The configuration analyzes production C# sources across all project folders.
Tests, fuzzing, build outputs and 16 assembly/version-metadata-only files are
excluded. The logical hierarchy follows `namespace` declarations, including
those whose opening brace is on the next line. Depth 1 shows application and
infrastructure namespaces beneath `Emby`, `Jellyfin` and `MediaBrowser`.

## Recorded scope

Analysis on 2026-10-27 with the source revision and configuration above:

| Level | Result |
| --- | --- |
| Analyzed C# files | 1,767 |
| Directed import relationships | 4,942 with 4,942 source-line references |
| File/namespace graph including semantic dependencies | 7,569 relationships with 10,767 references |
| Additional semantic evidence | 5,046 internal call sites and 779 inheritance/interface relationships |
| Cyclic groups at file level | 9; largest: 17 files |
| Initial view, depth 1 | 20 nodes, 100 relationships |
| Cyclic groups in this view | 3; largest: 4 nodes |
| ACD in this view | 8.65 |
| Additional full-graph placeholders | 256 `namespace/*` nodes for namespace imports |

All analyzed files and saved import, call and declaration evidence were checked
against the sources. The file cycle includes `ExtraRuleResolver.cs:28` calling
`VideoResolver.IsVideoFile` and `VideoResolver.cs:81` calling back to
`ExtraRuleResolver.GetExtraInfo`. Reproduce the automated check with .NET SDK 8+:

```bash
cargo build -p archguard-cli --all-features --locked
python3 scripts/integration/test_jellyfin_example.py
```

This downloads nothing and requires the checkout above, so it runs separately
from offline CI. The actual viewer payload was also checked using the existing
DOM/SVG harness: initial KPIs, 100 directed arrows, namespace navigation and
selection. This was not a browser acceptance test.

## Explore the viewer

1. **Cycles → scc-3** shows `Jellyfin/MediaEncoding` and `MediaBrowser/Controller` as a small group at the initial depth.
2. Select a direction and read evidence in the inspector: `src/Jellyfin.MediaEncoding.Hls/Cache/CacheDecorator.cs:9` imports `MediaBrowser.Controller.Persistence`; in the other direction, `MediaBrowser.Controller/Library/IKeyframeManager.cs:5` imports `Jellyfin.MediaEncoding.Keyframes`.
3. Expand packages: `Hls` and `Keyframes` are different parts of the coarser `MediaEncoding` node. A cycle at this aggregated level therefore does not prove a file or assembly cycle. The outer package context stays visible during navigation and is labeled separately.
4. **Return to overview**, then explore `Jellyfin/Api/Controllers`, `MediaBrowser/Controller/Entities` or `MediaBrowser/Providers`. **Review priorities** shows evidence-backed candidates for closer inspection.

```bash
target/debug/archguard scan examples/checkouts/jellyfin \
  --level file
target/debug/archguard cycles examples/checkouts/jellyfin \
  --format json
target/debug/archguard hotspots examples/checkouts/jellyfin \
  --top 20 --format json
target/debug/archguard calls examples/checkouts/jellyfin \
  --format json
```

The scan saves its full dependency graph to
`examples/checkouts/jellyfin/.archguard/graph.json`.

## Interpretation limits

- The C# scanner captures explicit `using` directives and namespaces. A normal namespace import points to a `namespace/*` placeholder; it does not claim a call to a particular class. Implicit imports, type use within a namespace, preprocessor evaluation and `.csproj` relationships are not fully covered by this import scanner.
- **Roslyn supplements the file/package graph.** The local SDK 8.0.204 run yields 1,797 types, 15,055 methods and 64,683 call sites: 8,531 project targets, 14,938 external targets and 41,214 unresolved targets. Coverage is `partial`: Jellyfin requires SDK 9, while references and parts of its project configuration are missing from the statically evaluated context. This is not a complete call graph. Unambiguously resolved internal calls and known internal supertypes enter the graph before cycle detection, KPIs and rules. Same-file calls produce no file edge; external/unresolved calls are not reinterpreted as internal edges. Partial classes use the actual method file and inheritance declaration. Another 75 supertype relationships could not be assigned an unambiguous file edge and were omitted; no resolved project call failed file assignment. The Calls view remains original source evidence even during graph refactoring simulation. **With limited coverage, "0 file cycles" is not a complete all-clear, even after this integration.**
- The run leaves 1,081 unresolved imports, including external NuGet namespaces such as `Microsoft.Extensions.Logging` and `TagLib`. They are not analyzed internal edges. The CLI displays the first warnings.
- Project inventory detects 42 manifest projects and `Jellyfin.Server` as one executable application. The pinned checkout has no Compose files. Other libraries and tools are not presented as independent services; project or namespace names do not invent API edges.
- The checkout is shallow. Git churn counts are lower bounds, not historical risk rankings. For time-window analysis, explicitly fetch history with `git -C examples/checkouts/jellyfin fetch --unshallow`.
- The example rule flags package cycles for investigation. It does not represent Jellyfin's architecture policy, and no intended layers are invented.
