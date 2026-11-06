# ADR 0004: Evaluation against an expert assessment

- **Status:** accepted
- **Date:** 2026-11-06

## Context

ArchGuard reports findings (cycles, layer violations, metric thresholds,
rule violations, orphans, target-architecture deviations). Nothing
measured whether they agree with an expert's view of a system: does
ArchGuard find the problems an architect names (recall), and how many
reported findings are real problems (precision)? The case studies in
`examples/` (Kafka, Jellyfin, Commons Collections) describe findings but
compare them with no expert assessment.

ArchGuard cannot produce that assessment itself: any "truth" it derived
from its own analysis would measure the tool against itself. The tool
can only make it easy for a human expert to record an assessment and to
compute the agreement reproducibly.

## Decision

1. **Truth file.** The expert writes a TOML file, by default
   `.archguard/assessment.toml` (`--assessment FILE` overrides it):

   ```toml
   [assessment]
   assessor = "Jane Doe"
   date = "2026-11-06"
   revision = "f04b91f"     # Git commit the assessment refers to
   scope = "server modules only"   # optional

   [[expected]]             # a problem the expert sees
   id = "E1"
   kind = "cycle"
   nodes = ["src/orders/**", "src/billing/**"]
   note = "orders and billing call each other"   # optional

   [[judged]]               # a verdict on a reported finding
   finding = "8624604a…"    # stable finding id (SHA-256)
   verdict = "true_positive"   # or "false_positive"; "" = not judged yet
   note = ""                   # optional
   ```

   Unknown keys, unknown kinds and unknown verdicts are rejected.
   Expected ids and judged finding ids must be unique, and an expected
   item of a finding kind needs at least one node glob.
2. **Kinds.** `kind` is an ArchGuard finding kind (`cycle`,
   `layer_violation`, `metric_threshold`, `layer_unranked`,
   `dependency_rule`, `ownership_rule`, `orphan`, `target_architecture`)
   or `other`. `shared_datastore` and `hotspot` are not kinds: main has
   no shared-datastore output, and hotspots are a ranking, not findings
   with an id. Such problems are recorded as `other`. `other` is never
   matched and counts as a recall miss with the reason
   "no ArchGuard finding kind", so the gap stays visible.
3. **Finding id.** Verdicts name findings by the stable SHA-256 `id`
   that `archguard review` already uses for architecture findings
   (`finding_id` in `crates/archguard-engine/src/evaluation.rs`). It is
   derived from the finding's identity (kind/rule and sorted member
   nodes), so it survives `scc-N` renumbering. The review `fingerprint`
   is not used: it changes with edges and source contents.
4. **Matching rule.** An expected item is matched when at least one
   current finding has the same kind and every node glob of the item
   matches at least one node of that finding. The finding may have
   further nodes. Globs use `globset` syntax over finding node names, as
   elsewhere in the configuration.
5. **Metrics.**
   - recall = matched expected items / all expected items (`other`
     included);
   - precision = judged `true_positive` / judged findings, counting only
     verdicts on findings that exist in the current analysis.

   A metric whose denominator is 0 has no value (JSON `null`, text
   "not measured"); it is never reported as 0 or 1.
6. **Lists.** The result also lists unjudged current findings (no
   verdict, or verdict `""`), expected items without match and why, and
   stale judged ids (no current finding has them). It warns when the
   assessment names no revision, when `HEAD` is unknown, or when the
   revision is not a prefix of `HEAD`.
7. **Interfaces.** `archguard evaluate [PATH] [--assessment FILE]
   [--format text|json]` runs the normal analysis and prints the result;
   `archguard evaluate --init` writes a commented template with every
   current finding as a `[[judged]]` stub with an empty verdict and
   `revision` set to `HEAD`. It never overwrites an existing file.
   Exit codes: 0 ok, 2 usage, 3 analysis or assessment error. evaluate is
   not a gate. The MCP tool `evaluation` returns the same result,
   read-only, in flattened pages within 4 KiB.
8. **ArchGuard never writes verdicts or expected problems.** The
   template leaves every verdict empty; an empty verdict is never
   defaulted.

## Alternatives

- **Derive the truth from ArchGuard's own findings or a second tool.**
  Rejected: it measures agreement between tools, not with an expert.
- **Judge by report id (`scc-3`).** Rejected: report ids are renumbered
  when other cycles appear or disappear.
- **Judge by review fingerprint.** Rejected: any edge or source change
  reopens the verdict although the problem is the same.
- **Exact node-set matching.** Rejected: an expert rarely knows every
  member of a cycle; requiring the named parts to be covered is enough
  and easier to write.
- **Count verdicts on stale findings in precision.** Rejected: precision
  would then describe findings the current run no longer reports.

## Consequences

- Recall and precision describe agreement with one recorded assessment,
  not ground truth. Different experts may record different assessments.
- Assessments of the existing case studies still have to be written by a
  human expert; ArchGuard ships none.
- Only the architecture findings of the report take part. Imported
  review findings (quality, security, tests, mutations, duplicates,
  contracts) have no finding kind here and cannot be judged yet.
- When the identity rules of findings change, verdicts become stale and
  are listed as such instead of being silently reassigned.

## History

- **2026-11-06:** Proposed and accepted; implemented on `feat/evaluate`
  (core types, engine metrics, CLI `evaluate`, MCP `evaluation`).
