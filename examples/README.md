# Analysis examples

[English](README.md) | [Deutsch](README_DE.md)

- [Service shop](service-shop/README.md): runnable demo monorepo with 14 services, three Compose stacks, two different API cycles and infrastructure.
- Apache Commons Collections: real open-source project for package/class analysis; see below.
- [Jellyfin](jellyfin.md): real C# media server with 1,767 analyzed source files, namespace hierarchy and three cyclic groups at the initial depth.
- [Apache Kafka](kafka.md): larger example with 3,369 production Java files, independently checked file cycles and a rotatable architecture landscape.

## Apache Commons Collections

[Apache Commons Collections](https://commons.apache.org/proper/commons-collections/)
extends Java collections with maps, bags, iterators, tries and Bloom filters.
Its package structure and explicit Java imports suit ArchGuard's current scanner.

Uses **4.6.0**, tag `rel/commons-collections-4.6.0`, pinned to commit
[`4107624de41aeed37985d2a069fad41992fd71a2`](https://github.com/apache/commons-collections/tree/4107624de41aeed37985d2a069fad41992fd71a2).
Upstream license: Apache-2.0; `LICENSE.txt` and `NOTICE.txt` stay in the checkout.
Third-party source lives under `checkouts/`, ignored by the ArchGuard repository.
Import analysis requires no build or installed Java dependencies.

## Start

Run all commands from the ArchGuard repository root. Clone once for a fresh checkout:

```bash
git clone --depth 1 --single-branch --branch rel/commons-collections-4.6.0 \
  https://github.com/apache/commons-collections.git examples/checkouts/commons-collections
git -C examples/checkouts/commons-collections rev-parse HEAD

mkdir -p examples/checkouts/commons-collections/.archguard
cp -n examples/commons-collections.toml \
  examples/checkouts/commons-collections/.archguard/archguard.toml

cargo run -p archguard-cli --locked -- serve examples/checkouts/commons-collections \
  --port 8788 --depth 4
```

The printed commit ID must match the linked revision. Viewer:
<http://127.0.0.1:8788/>. Port 8787 can simultaneously show the small example.
`Ctrl-C` stops the server.

Setup copies the [configuration template](commons-collections.toml) into the
analyzed checkout; `cp -n` preserves existing config. Then
`.archguard/archguard.toml` loads automatically. Graph, effective config and
standalone HTML report also live under the checkout's `.archguard`, following
the [project convention](../README.md#configuration-and-project-data).
`--config examples/commons-collections.toml` remains an explicit alternative.

The configuration excludes `src/test/**`. Depth 4 shows the children of
`org/apache/commons/collections4`: subpackages and files directly in that package.
The server needs explicit `--depth 4` because its default initial depth is 0.
The graph initially shows 18 direct subpackages; **Show files** reveals 55 direct
files. Analysis still includes all 73 nodes.

## Recorded scope

Analysis on 2026-10-27 with ArchGuard 0.1.0 and the configuration above:

| Level | Result |
| --- | --- |
| Production sources | 367 Java files, including `package-info.java` |
| Declared Java packages | 20 |
| Hierarchy | 23 module nodes, including `org`, `org/apache`, `org/apache/commons` |
| File relationships | 700 directed edges with 701 import references |
| Depth-4 view | 73 nodes, 192 edges, 3 cyclic groups; largest: 16 nodes |
| Excluded | 310 Java test files |
| Calls with Java 17 baseline | 601 classes/types including nested and anonymous types; 5,040 method/constructor declarations |
| Call sites | 10,291: 5,867 internal, 4,424 external; no unresolved targets or compiler diagnostics in this run |

All 367 file nodes match production sources. The 701 saved import references
were checked against source lines. The live payload and startup with the actual
pinned graph library and DOM/Canvas test doubles were checked; no visual browser
acceptance test was performed. All 10,291 call references were also checked
against file, line, column and source text. These call data are independent of
import edges.

## Explore the viewer

1. **Cycles → scc-2** focuses the small cycle between `ComparatorUtils.java` and `comparators`, a manageable starting point.
2. Select a directed edge and inspect import lines. One concrete file pair is `ComparatorUtils.java:26 → NullComparator` and `comparators/NullComparator.java:23 → ComparatorUtils`.
3. **Return to overview** restores the package view. Find and expand `map`, `list`, `trie` or `bloomfilter` in the explorer.
4. Confirm a package suggestion with **Simulate suggested move**, then use **Undo**. Moving does not remove import relationships.

For classes and methods, open **Calls** and select `org.apache.commons.collections4`.
Search for `CollectionUtils` and open `CollectionUtils → AbstractMultiSet`: source
references such as `CollectionUtils.java:1830` support an interface review plan.
Check existing interfaces first. `BagUtils → CollectionBag` shows a candidate
move toward `bag`, supported in part by `BagUtils.java:44`. **Copy review plan**
copies the suggestion; source code is not changed automatically. Generating this
data requires JDK 17+ on PATH; static-analysis limits are in the
[viewer guide](../README.md).

CLI queries for the same revision:

```bash
target/debug/archguard scan examples/checkouts/commons-collections \
  --level file
target/debug/archguard cycles examples/checkouts/commons-collections \
  --format json
target/debug/archguard hotspots examples/checkouts/commons-collections \
  --top 20 --format json
target/debug/archguard performance examples/checkouts/commons-collections \
  --format json
target/debug/archguard co-change examples/checkouts/commons-collections \
  --since 6.months --format json
```

The scan saves its full import graph to
`examples/checkouts/commons-collections/.archguard/graph.json`.

**Review priorities** opens an evidence-backed ranking. Yellow P1/P2 markers show
candidates; the inspector explains coupling, cycle contribution, transitive users
and available Git file touches. Expandable KPIs add CCD/NCCD, largest cyclic group
and source coverage. **Source & build indicators** includes Java abstractness
and normalized main-sequence distance when compiler data is complete. This source
evidence belongs to the original report; structural KPIs and review ranking
follow current navigation and simulation.

The documented clone is **shallow**. Git counts are lower bounds; an empty
co-change report does not prove the absence of historical coupling. For a longer
analysis, explicitly fetch history:

```bash
git -C examples/checkouts/commons-collections fetch --unshallow
```

The new run reports its actual history/source coverage in JSON and the viewer.
No new measurements are claimed here; the figures above belong to the separately
documented run. Score, configuration rules and limits are in the
[review guide](../README.md#common-workflows).

## Interpretation limits

This is import analysis, not complete Java type analysis: implicit relationships
within a package and fully qualified type accesses without imports are not
captured. Seven import warnings remain: four concern `org.w3c.dom`, three concern
nested types (`TreeBidiMap.DataElement` and `AbstractHashedMap.HashEntry`). The two
`DataElement` imports are within the same source file. Upstream code is not
modified to conceal these limits.

The strict example rule reports cyclic groups as findings; it does not claim
that upstream violates its own architecture policy. Cycle samples and cut
suggestions are bounded. No intended layers are invented; this example disables
`derive_layers`.
