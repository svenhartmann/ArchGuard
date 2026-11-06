# ADR 0003: Quality goals linked to evidence

- **Status:** accepted
- **Date:** 2026-11-06

## Context

ArchGuard reports structural findings (cycles, layer, dependency, ownership
and target-architecture violations, metric thresholds, orphans), review KPIs
and imported evidence (dependency advisories, runtime and test measurements,
test results, coverage, mutation tests). Organizations declare their quality
goals as free text in `[[architecture.goals]]` (`name`, `priority`,
`rationale`). The goals are only listed: in `architecture_context`, the
architecture document and arc42 chapter 1. No output says which declared
goal a finding endangers, or which goal ArchGuard cannot judge at all.

Deriving that link from free text would be guessing. A goal needs a
machine-readable quality characteristic, and the link from evidence to a
characteristic must be the same for every project.

## Decision

1. **Optional goal fields.** `ArchitectureGoal` gains three optional fields.
   Old goal files load unchanged (`serde(default)`), unknown keys stay
   rejected (`deny_unknown_fields`).
   - `attribute`: one ISO/IEC 25010 product quality characteristic:
     `functional_suitability`, `performance_efficiency`, `compatibility`,
     `usability`, `reliability`, `security`, `maintainability`,
     `portability`. Other values are rejected when the configuration loads.
   - `scope`: node-name globs; empty means the whole system. They use the
     same `globset` matching as `layers[].contains` and
     `architecture.model.components[].include`. Invalid globs are rejected
     when the configuration loads.
   - `scenario`: free text quality scenario (stimulus, response, measure).
     ArchGuard shows it and does not evaluate it.
2. **Fixed mapping from evidence to characteristics.** The table below is
   the whole mapping (`archguard-analysis/src/goals.rs`). It does not change
   per project.

   | Characteristic | Evidence sources | What is used |
   |---|---|---|
   | maintainability | `static_analysis` (report findings), `review_kpis` | Findings of kind `cycle`, `layer_violation`, `layer_unranked`, `dependency_rule`, `ownership_rule`, `target_architecture`, `metric_threshold`, `orphan`; KPIs: highest propagation cost, tangledness, SDP violation count, zone of pain and zone of uselessness counts when zones are measured |
   | security | `dependency_advisories` (`.archguard/security.json`) | Saved advisories as findings (`<advisory id>@<package>`), counted by the advisory's own severity |
   | performance_efficiency | `measurement_runs` (`.archguard/measurement-runs.json`) | Presence and counts the reader already reports (runs, groups, test groups with mixed results, their share) |
   | reliability | `test_results` (`.archguard/tests.json`), `coverage` (`.archguard/coverage.json`), `mutation` (`.archguard/mutation.json`), `measurement_runs` | Presence and counts the readers already report (tests, failed tests, coverage files, mutants, killed share, runs) |
   | functional_suitability, compatibility, usability, portability | none | ArchGuard has no evidence for them |

   A saved snapshot counts as present when its reader reports `ready`,
   `partial` or `stale`; `partial` and `stale` add a gap line.
3. **Per goal assessment.** Each goal yields a `GoalAssessment` with `goal`,
   `priority`, `attribute`, `scope`, `status`, `findings` (finding ids),
   `finding_count_by_severity`, `kpis` (name and value), `evidence_sources`
   (source and `present`) and `gaps`. The status is one of:
   - `unclassified`: no `attribute`. The assessment and the architecture
     questions ask "Which ISO 25010 characteristic does X address?".
   - `not_assessable`: ArchGuard has no evidence source for the
     characteristic.
   - `no_evidence`: the characteristic is assessable, but the needed source
     is absent (for example no saved security scan).
   - `findings`: at least one finding matches.
   - `no_findings`: sources are present and nothing matched. Outputs say
     "no findings from the available sources", never "goal met".
4. **Matching.** A finding matches a goal when its kind maps to the goal's
   characteristic and the scope is empty or one of the finding's nodes
   matches a scope glob. KPIs and imported counts are project-wide; a scoped
   goal names this in its gaps. Dependency advisories name manifests, not
   modules, so a scoped security goal gets `no_evidence` with a gap instead
   of a guessed assignment.
5. **Order and priority.** Goals keep their file order. `priority` stays the
   organization's own string; ArchGuard invents no numeric ranking.
6. **No derived health weights.** `[review] health_weights` are not derived
   from goals. A maintainability goal with priority "high" does not change
   the health index; the weights stay what the configuration states.
7. **Surfaces.** `ArchitectureContext.goal_assessment` (MCP
   `architecture_context`, `archguard review`), the architecture document,
   arc42 chapter 1 (characteristic, scope and status columns) and arc42
   chapter 10 (per goal: scenario, status, linked finding ids, KPIs, gaps).

## Alternatives

- **Infer the characteristic from goal names or rationale text.** Rejected:
  "Fast delivery" can mean maintainability or performance; a keyword guess
  would attach findings to the wrong goal without the organization noticing.
- **Per-project mapping in the configuration** (for example
  `cycle -> reliability`). Rejected for now: findings would mean different
  things in different projects, and every project would have to maintain a
  table. The fixed table can grow when ArchGuard gains new evidence.
- **Derive `health_weights` from goal priorities.** Rejected: priorities are
  free text without an order ArchGuard may interpret, and a derived weight
  would be a hidden default that changes the health index when someone
  edits a goal's wording.
- **Report "goal met" when nothing matched.** Rejected: absent findings from
  static analysis and imported reports do not show that a quality goal is
  reached.
- **Use the ISO/IEC 25010:2023 characteristic list** (adds safety, renames
  usability to interaction capability and portability to flexibility).
  Not chosen for this change, which specified the eight characteristics of
  the 2011 edition; new values can be added later without breaking
  existing files.

## Consequences

- Goals without `attribute` keep working and produce a question instead of
  an assessment.
- An assessment depends on which reports are saved under `.archguard/`.
  The same goal can move from `no_evidence` to `findings` after an explicit
  `archguard security` scan.
- Every new finding kind must be placed in the mapping; the match in
  `goals.rs` is exhaustive, so the compiler enforces this.
- Statuses report evidence, not conformance. `no_findings` for a
  performance or reliability goal only says that imported measurements
  exist; ArchGuard derives no findings from them.
