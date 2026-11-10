# ADR 0010: Declared packages hang under their project directory

- **Status:** accepted
- **Date:** 2026-11-10

## Context

Java, Kotlin, C# and PHP files are named after their declared package or
namespace (ARCHITECTURE.md Section 5.1), not after their directory. In a
monorepo whose projects share a package, this merged the projects.

Example: ElasticSearchIK has four Azure Functions under `functions/*`, each with
its own `pom.xml`, and all four declare `de.union.investment.ikonline`. The graph
had one tree `de/union/investment/ikonline`. Three different
`common/LogHelper.java` files became one node, and so did eight other classes
whose names occur in more than one function. Imports between them pointed
to a package pseudo node. The service topology had found the four projects
correctly. When the desktop app zoomed into one service, it found no module
`functions/cms-import` and showed the merged overview instead.

## Decision

1. While walking the files, the walker records the directories of build
   manifests: `pom.xml`, `build.gradle`, `build.gradle.kts`, `*.csproj`,
   `*.fsproj`, `*.vbproj` and `composer.json`. Each source file gets the
   nearest such directory as `SourceFile::project`. A manifest at the
   repository root does not count.
2. A file with a declared package gets this directory as a prefix. The
   directory is spelled as the stripped paths spell it, so a configured
   source root is cut off first. `functions/cms-import/src/main/java/de/x/App.java`
   becomes `functions/cms-import/de/x/App.java`. Files without a declared
   package keep their physical path, as before.
3. Imports of Java/Kotlin, C# and PHP resolve against the importing file's
   own project first, then against the whole repository. A package pseudo
   node hangs under the project that declares the package. If several
   projects declare it, there is no single target, and the import is
   reported as unresolved.
4. The cache fingerprint includes each file's project directory, and the
   cache schema is 9. Adding or moving a manifest therefore rebuilds the graph.

## Alternatives

- **Prefix only in repositories with at least two projects.** Rejected. A
  second project would rename every file of the first one, and a single
  rule is easier to predict.
- **Prefix only packages that several projects declare.** Rejected. This
  needs a view of the whole repository in every place that names one file:
  the call analyses, performance, review and change context.
- **Use the project names from the service topology** (for example
  `ui-ikonline-cms-import`). Rejected. The topology parses every manifest
  and Compose file, which is too slow to run on every graph build. The
  directory is also what the desktop app passes when it zooms into a service.
- **Fix it only in the desktop app.** Not possible. The merged nodes already
  come out of the engine.

## Consequences

- Repositories with build manifests below the root get new node names for
  Java, Kotlin, C# and PHP. This includes multi-module Maven and Gradle builds
  and .NET solutions with `src/<Project>/<Project>.csproj`. Baselines, review
  entries and dependency rules that name such modules must be updated.
- In a .NET solution the project and namespace often repeat each other:
  `Shop.Api/Shop/Api/Controller.cs`. That repetition is accepted; it is the
  price of keeping projects apart.
- Gradle subprojects without their own `build.gradle` stay in the root
  project, as before.
- The desktop app's "zoom into service" now finds the service's module tree.
  It cuts the analysis' effective source roots off the project path first,
  because derived roots can cut a parent directory (`functions/cms-import`
  becomes module `cms-import`).
- In the services view (desktop and web), projects with the runtime
  `azure-functions` count as deployed. Before this change, a local Compose
  stack beside them hid them unless "All projects" was on.
