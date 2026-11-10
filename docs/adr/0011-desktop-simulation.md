# ADR 0011: Simulation, cycle repair and scenarios in the desktop app

- **Status:** accepted
- **Date:** 2026-11-10

## Context

The web viewer lets users try changes on the dependency graph before
touching code. You can turn on "Manual simulation", cut a dependency or
move a package, see what happens to cycles and ACD, then undo, clear or
export the changes. "Resolve a cycle" suggests plans for removing
dependencies, and "Scenarios" saves named experiments. The desktop app
(ADR 0002) had none of this. Of the features it still lacked from the web
viewer, these were the largest, and the cycle planner and scenarios need
simulation first.

The logic already exists in Rust. `archguard-analysis` has `apply_ops`
and `simulate` with the fixed SimOp JSON format, which the CLI
(`simulate --ops`), MCP and the web export (`archguard-simops.json`)
also use. It also has `suggest_cuts`. The desktop app already depends on
it through `archguard-engine`.

## Decision

1. **The simulated changes belong to the document.** The desktop keeps
   one `Simulation` per document (`graph_ui/simulation.rs`) holding the
   SimOps, the change waiting for confirmation and the "Manual
   simulation" toggle. When a newer analysis arrives, the changes are not
   discarded. They are applied again, because ops refer to nodes by name.
   If they no longer apply, the app shows the original graph and the
   reason. Nothing is deleted.
2. **Simulation changes only the package tree.** `tab_support::tree`
   builds the tree from `apply_ops(graph, ops)`, so all Graph and Matrix
   tabs of the document show the simulated state. Measurements,
   findings, security, quality, start KPIs and source evidence stay those
   of the analysis, as in the web viewer.
3. **Graphs keep their view state.** When the changes change, every
   Graph moves onto the new tree and keeps its open packages, selection
   and scope by name. Matrix and File views are rebuilt.
4. **Nothing changes without confirmation.** As in the web viewer, every
   proposal (cutting a dependency from the details panel, moving a
   package under another module) waits for "Apply". The ops strip under
   the graph shows the cycle groups and ACD before and after (`simulate`
   at `modules.depth`) and offers Undo, Clear and Export. Export writes
   the same SimOp JSON as the web viewer.
5. **Scenarios are files in the project:** `.archguard/scenarios/*.json`,
   in the web viewer's scenario format. The web viewer keeps them in
   browser storage. Files in the project survive a reinstall, can go
   through git and work with both apps.
6. **Plans for resolving a cycle use the existing Rust analysis**
   (`suggest_cuts`). Previewing a plan adds its ops as simulated changes,
   just like a manual cut.

## Alternatives

- **Port the viewer's JS simulation.** Rejected. It would be a second
  implementation of a format that is already fixed and tested in
  `archguard-analysis`.
- **Keep changes per Graph tab.** Rejected. Graph and Matrix side by side
  would then show different graphs. The web viewer has a single
  simulation too.
- **Keep scenarios in the desktop's own settings.** Rejected. They would
  be tied to one machine and invisible to the web viewer and to git.

## Consequences

- With simulated changes, the cycles listed in Graph's explorer come from
  the simulated graph at `modules.depth`. For a `--level file` run, this
  is a different level from the report's findings. Marked as a shortcut
  in the code until `AnalysisRun` records the level.
- Matrix and File views lose their scroll and expansion state when the
  changes change.
- Merging packages (`SimOp::Merge`) is shown and exported, but there is
  no button to propose it, as in the web viewer.

## History

- 2026-11-10: Step 1 (simulation) implemented.
- 2026-11-10: Step 2 (cycle planner) implemented: Explorer → Cycles lists
  up to five single-edge cuts of the chosen group (`suggest_cuts` on the
  shown graph at `modules.depth`, with sampled cycles broken and
  references); "Preview" turns simulation on and proposes the cut. The
  web viewer's multi-edge plan search (`cycle_planner.js`: cheapest edge
  sets by affected files, evidence and plan export) is not ported; the
  confirmation moved from the details panel to the ops strip so it shows
  without a selection.
- 2026-11-10: Step 3 (scenarios) implemented: Graph toolbar "Scenarios"
  opens a panel in the ops strip that saves the current changes as
  `.archguard/scenarios/<name>.json`, lists, loads and deletes them, and
  imports a scenario file from elsewhere. Saving under a used name,
  loading and deleting ask first. Files use the web viewer's
  `archguard-refactoring-scenario/v1` format and fingerprint (checked
  against the web algorithm). Its scope holds the top-level nodes after
  the changes, since the desktop keeps no zoom level of its own; there is
  no description field yet (saved empty).
