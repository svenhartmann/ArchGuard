//! Agreement of reported findings with an expert assessment (ADR 0004).
//! Recall and precision come only from what a human recorded in the
//! assessment file; nothing here invents an expected problem or a verdict.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use archguard_core::{
    ArchguardError, Assessment, AssessmentMeta, ExpectedKind, Finding, FindingKind, Verdict,
};
use globset::{Glob, GlobMatcher};
use serde::Serialize;

/// Default assessment location, relative to the project root.
pub const DEFAULT_ASSESSMENT_PATH: &str = ".archguard/assessment.toml";
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// `numerator / denominator`; `value` is `None` ("not measured") when
/// nothing was counted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ratio {
    pub numerator: usize,
    pub denominator: usize,
    pub value: Option<f64>,
}

impl Ratio {
    fn new(numerator: usize, denominator: usize) -> Self {
        let value = (denominator > 0).then(|| numerator as f64 / denominator as f64);
        Ratio {
            numerator,
            denominator,
            value,
        }
    }
}

/// A current finding, named by its stable id and its report id.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvaluatedFinding {
    pub id: String,
    pub report_id: String,
    pub kind: FindingKind,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedExpectation {
    pub id: String,
    pub kind: ExpectedKind,
    /// Stable ids of every finding that satisfies the matching rule.
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MissedExpectation {
    pub id: String,
    pub kind: ExpectedKind,
    pub reason: String,
}

/// Result of `archguard evaluate`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evaluation {
    pub assessment_path: String,
    pub assessment: AssessmentMeta,
    /// `HEAD` of the analysed project; `None` without Git.
    pub head: Option<String>,
    pub warnings: Vec<String>,
    /// Expected problems matched / expected problems.
    pub recall: Ratio,
    /// Judged true positives / judged current findings.
    pub precision: Ratio,
    pub findings_total: usize,
    pub matched: Vec<MatchedExpectation>,
    pub missed: Vec<MissedExpectation>,
    /// Current findings without a verdict; judge these next.
    pub unjudged: Vec<EvaluatedFinding>,
    /// Judged finding ids no current finding has.
    pub stale: Vec<String>,
}

/// Stable SHA-256 id of an architecture finding: the review `id`. It
/// survives `scc-N` renumbering; the evidence `fingerprint` does not.
pub fn finding_id(f: &Finding) -> String {
    let identity = serde_json::to_vec(&crate::comparison::identity(f))
        .expect("a finding identity is a plain string");
    crate::evidence_import::hash(&identity)
}

/// Parse and validate an assessment file's text.
pub fn parse_assessment(text: &str) -> Result<Assessment, ArchguardError> {
    let assessment: Assessment =
        toml::from_str(text).map_err(|e| ArchguardError::Config(format!("assessment: {e}")))?;
    assessment.validate()?;
    Ok(assessment)
}

fn current(findings: &[Finding]) -> BTreeMap<String, EvaluatedFinding> {
    findings
        .iter()
        .map(|f| {
            let id = finding_id(f);
            let row = EvaluatedFinding {
                id: id.clone(),
                report_id: f.id.clone(),
                kind: f.kind,
                nodes: f.nodes.clone(),
            };
            (id, row)
        })
        .collect()
}

fn matchers(globs: &[String]) -> Result<Vec<GlobMatcher>, ArchguardError> {
    globs
        .iter()
        .map(|g| {
            Glob::new(g)
                .map(|g| g.compile_matcher())
                .map_err(|e| ArchguardError::Config(format!("assessment: node glob {g:?}: {e}")))
        })
        .collect()
}

/// Compare `findings` (one analysis run) with `assessment`.
///
/// Matching rule: an expected problem is matched when at least one finding
/// has the same kind and every one of its node globs matches at least one
/// node of that finding. Further finding nodes are allowed. `other` never
/// matches.
pub fn evaluate(
    assessment: &Assessment,
    assessment_path: &str,
    findings: &[Finding],
    head: Option<&str>,
) -> Result<Evaluation, ArchguardError> {
    let current = current(findings);
    let (mut matched, mut missed) = (Vec::new(), Vec::new());
    for expected in &assessment.expected {
        let ExpectedKind::Finding(kind) = expected.kind else {
            missed.push(MissedExpectation {
                id: expected.id.clone(),
                kind: expected.kind,
                reason: "no ArchGuard finding kind".into(),
            });
            continue;
        };
        let globs = matchers(&expected.nodes)?;
        let same_kind: Vec<_> = current.values().filter(|f| f.kind == kind).collect();
        let hits: Vec<String> = same_kind
            .iter()
            .filter(|f| globs.iter().all(|g| f.nodes.iter().any(|n| g.is_match(n))))
            .map(|f| f.id.clone())
            .collect();
        if hits.is_empty() {
            let reason = if same_kind.is_empty() {
                format!("no {} finding reported", expected.kind)
            } else {
                format!("no {} finding covers every node glob", expected.kind)
            };
            missed.push(MissedExpectation {
                id: expected.id.clone(),
                kind: expected.kind,
                reason,
            });
        } else {
            matched.push(MatchedExpectation {
                id: expected.id.clone(),
                kind: expected.kind,
                findings: hits,
            });
        }
    }
    let (mut true_positives, mut judged) = (0, 0);
    let mut stale = Vec::new();
    let mut with_verdict = BTreeSet::new();
    for j in &assessment.judged {
        if !current.contains_key(&j.finding) {
            stale.push(j.finding.clone());
            continue;
        }
        if let Some(verdict) = j.verdict {
            judged += 1;
            true_positives += usize::from(verdict == Verdict::TruePositive);
            with_verdict.insert(j.finding.as_str());
        }
    }
    let unjudged = current
        .values()
        .filter(|f| !with_verdict.contains(f.id.as_str()))
        .cloned()
        .collect();
    Ok(Evaluation {
        assessment_path: assessment_path.into(),
        assessment: assessment.assessment.clone(),
        head: head.map(str::to_string),
        warnings: revision_warnings(&assessment.assessment.revision, head),
        recall: Ratio::new(matched.len(), assessment.expected.len()),
        precision: Ratio::new(true_positives, judged),
        findings_total: current.len(),
        matched,
        missed,
        unjudged,
        stale,
    })
}

/// The assessment is checked against `HEAD` by commit-hash prefix.
fn revision_warnings(revision: &str, head: Option<&str>) -> Vec<String> {
    let revision = revision.trim();
    match head {
        _ if revision.is_empty() => {
            vec!["assessment names no revision; cannot tell which commit it refers to".into()]
        }
        None => vec![format!(
            "HEAD unknown (no Git repository); cannot check assessment revision {revision}"
        )],
        Some(head) if head.starts_with(revision) => vec![],
        Some(head) => vec![format!(
            "assessment revision {revision} differs from HEAD {head}; findings may have changed since the assessment"
        )],
    }
}

fn head_of(root: &Path) -> Option<String> {
    let out = crate::review_sources::git_query(root, &["rev-parse", "HEAD"])?;
    let head = String::from_utf8(out).ok()?.trim().to_string();
    (!head.is_empty()).then_some(head)
}

fn assessment_file(root: &Path, assessment: Option<&Path>) -> PathBuf {
    assessment
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(DEFAULT_ASSESSMENT_PATH))
}

fn analyzed_findings(
    root: &Path,
    config_path: Option<&Path>,
) -> Result<Vec<Finding>, ArchguardError> {
    let loaded = crate::config::load_config(config_path, root)?;
    let run = crate::effective::analyze_run_quiet(root, &loaded, crate::pipeline::Level::Module)?;
    Ok(run.report.findings)
}

/// Run the normal analysis of `root` and evaluate it against the
/// assessment (default `.archguard/assessment.toml`).
pub fn evaluate_project(
    root: &Path,
    config_path: Option<&Path>,
    assessment: Option<&Path>,
) -> Result<Evaluation, ArchguardError> {
    let path = assessment_file(root, assessment);
    let len = std::fs::metadata(&path)
        .map_err(|e| ArchguardError::Config(format!("assessment {}: {e}", path.display())))?
        .len();
    if len > MAX_BYTES {
        return Err(ArchguardError::Config(format!(
            "assessment {} exceeds {MAX_BYTES} bytes",
            path.display()
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ArchguardError::Config(format!("assessment {}: {e}", path.display())))?;
    let parsed = parse_assessment(&text)
        .map_err(|e| ArchguardError::Config(format!("{}: {e}", path.display())))?;
    let findings = analyzed_findings(root, config_path)?;
    evaluate(
        &parsed,
        &path.display().to_string(),
        &findings,
        head_of(root).as_deref(),
    )
}

fn toml_string(text: &str) -> String {
    toml::Value::String(text.into()).to_string()
}

/// A commented assessment listing every current finding as an unjudged
/// `[[judged]]` stub. Verdicts stay empty: only the expert sets them.
pub fn assessment_template(findings: &[Finding], head: Option<&str>) -> String {
    let mut out = String::from(
        "# ArchGuard expert assessment (docs/adr/0004-evaluation-against-expert-assessment.md).\n\
         # Fill in by hand; ArchGuard never writes expected problems or verdicts.\n\
         # Evaluate with: archguard evaluate\n\n\
         [assessment]\n\
         assessor = \"\"\n\
         date = \"\"            # e.g. \"2026-11-06\"\n",
    );
    out.push_str(&format!(
        "revision = {}   # Git commit this assessment refers to\n",
        toml_string(head.unwrap_or(""))
    ));
    out.push_str(
        "scope = \"\"\n\n\
         # Problems you see in the system. kind: cycle, layer_violation, metric_threshold,\n\
         # layer_unranked, dependency_rule, ownership_rule, orphan, target_architecture,\n\
         # or other (ArchGuard has no finding kind for it; never matched, counts as a miss).\n\
         # nodes: globs over finding node names; every glob must match a node of one\n\
         # finding of that kind.\n\
         #\n\
         # [[expected]]\n\
         # id = \"E1\"\n\
         # kind = \"cycle\"\n\
         # nodes = [\"src/orders/**\", \"src/billing/**\"]\n\
         # note = \"\"\n\n\
         # Findings reported at this revision. verdict: \"true_positive\", \"false_positive\",\n\
         # or \"\" (not judged yet).\n",
    );
    for f in current(findings).values() {
        let summary: String = format!(
            "{} {}: {}",
            archguard_core::finding_kind_name(f.kind),
            f.report_id,
            f.nodes.join(", ")
        )
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
        out.push_str(&format!(
            "\n[[judged]]\n# {summary}\nfinding = {}\nverdict = \"\"\nnote = \"\"\n",
            toml_string(&f.id)
        ));
    }
    out
}

/// Write the template to the assessment path; an existing file is kept.
pub fn init_assessment(
    root: &Path,
    config_path: Option<&Path>,
    assessment: Option<&Path>,
) -> Result<PathBuf, ArchguardError> {
    let path = assessment_file(root, assessment);
    if path.exists() {
        return Err(ArchguardError::Config(format!(
            "assessment {} already exists; not overwritten",
            path.display()
        )));
    }
    let findings = analyzed_findings(root, config_path)?;
    let text = assessment_template(&findings, head_of(root).as_deref());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::Severity;

    fn finding(kind: FindingKind, id: &str, nodes: &[&str]) -> Finding {
        Finding {
            kind,
            id: id.into(),
            severity: Severity::Error,
            message: None,
            evidence: vec![],
            nodes: nodes.iter().map(|n| n.to_string()).collect(),
            edge_count: 0,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: vec![],
        }
    }

    fn findings() -> Vec<Finding> {
        vec![
            finding(FindingKind::Cycle, "scc-1", &["a/alpha.ts", "b/beta.ts"]),
            finding(FindingKind::Cycle, "scc-2", &["c/gamma.ts", "d/delta.ts"]),
            finding(
                FindingKind::LayerUnranked,
                "layers-unranked",
                &["a/alpha.ts"],
            ),
        ]
    }

    fn assessment(body: &str) -> Assessment {
        parse_assessment(&format!(
            "[assessment]\nassessor='A'\ndate='2026-11-06'\nrevision='abc'\n{body}"
        ))
        .unwrap()
    }

    fn run(body: &str) -> Evaluation {
        evaluate(&assessment(body), "a.toml", &findings(), Some("abcdef")).unwrap()
    }

    #[test]
    fn empty_assessment_measures_nothing() {
        let e = run("");
        assert_eq!(
            e.recall,
            Ratio {
                numerator: 0,
                denominator: 0,
                value: None
            }
        );
        assert_eq!(
            e.precision,
            Ratio {
                numerator: 0,
                denominator: 0,
                value: None
            }
        );
        assert_eq!(e.unjudged.len(), 3);
        assert_eq!(e.findings_total, 3);
    }

    #[test]
    fn match_needs_same_kind_and_every_glob_on_one_finding() {
        let e = run("[[expected]]\nid='hit'\nkind='cycle'\nnodes=['a/*']\n\
             [[expected]]\nid='split'\nkind='cycle'\nnodes=['a/*','c/*']\n\
             [[expected]]\nid='kind'\nkind='layer_violation'\nnodes=['a/*']\n\
             [[expected]]\nid='both'\nkind='cycle'\nnodes=['**/*.ts']\n");
        assert_eq!(e.recall.numerator, 2);
        assert_eq!(e.recall.denominator, 4);
        assert_eq!(e.recall.value, Some(0.5));
        assert_eq!(e.matched[0].id, "hit");
        assert_eq!(e.matched[0].findings, vec![finding_id(&findings()[0])]);
        assert_eq!(e.matched[1].id, "both");
        assert_eq!(e.matched[1].findings.len(), 2);
        assert_eq!(e.missed[0].id, "split");
        assert_eq!(
            e.missed[0].reason,
            "no cycle finding covers every node glob"
        );
        assert_eq!(e.missed[1].reason, "no layer_violation finding reported");
    }

    #[test]
    fn other_kind_is_always_a_recall_miss() {
        let e = run("[[expected]]\nid='E1'\nkind='other'\nnodes=['a/*']\n");
        assert_eq!(e.recall.value, Some(0.0));
        assert_eq!(e.missed[0].reason, "no ArchGuard finding kind");
    }

    #[test]
    fn precision_counts_verdicts_of_current_findings_only() {
        let ids: Vec<String> = findings().iter().map(finding_id).collect();
        let e = run(&format!(
            "[[judged]]\nfinding='{}'\nverdict='true_positive'\n\
             [[judged]]\nfinding='{}'\nverdict='false_positive'\n\
             [[judged]]\nfinding='{}'\nverdict=''\n\
             [[judged]]\nfinding='gone'\nverdict='true_positive'\n",
            ids[0], ids[1], ids[2]
        ));
        assert_eq!(e.precision.numerator, 1);
        assert_eq!(e.precision.denominator, 2);
        assert_eq!(e.precision.value, Some(0.5));
        assert_eq!(e.stale, vec!["gone".to_string()]);
        assert_eq!(e.unjudged.len(), 1);
        assert_eq!(e.unjudged[0].id, ids[2]);
    }

    #[test]
    fn finding_id_survives_renumbering_and_is_not_the_report_id() {
        let a = finding(FindingKind::Cycle, "scc-1", &["x", "y"]);
        let b = finding(FindingKind::Cycle, "scc-7", &["y", "x"]);
        assert_eq!(finding_id(&a), finding_id(&b));
        assert_eq!(finding_id(&a).len(), 64);
    }

    #[test]
    fn revision_mismatch_and_unknown_head_warn() {
        let a = assessment("");
        assert!(
            evaluate(&a, "a", &[], Some("abcdef"))
                .unwrap()
                .warnings
                .is_empty()
        );
        let other = evaluate(&a, "a", &[], Some("ffff")).unwrap();
        assert!(other.warnings[0].contains("differs from HEAD ffff"));
        let none = evaluate(&a, "a", &[], None).unwrap();
        assert!(none.warnings[0].contains("HEAD unknown"));
    }

    #[test]
    fn invalid_glob_is_a_config_error() {
        let a = assessment("[[expected]]\nid='E'\nkind='cycle'\nnodes=['a/[']\n");
        assert!(matches!(
            evaluate(&a, "a", &findings(), None),
            Err(ArchguardError::Config(_))
        ));
    }

    #[test]
    fn template_parses_and_leaves_every_finding_unjudged() {
        let text = assessment_template(&findings(), Some("abcdef"));
        let parsed = parse_assessment(&text).unwrap();
        assert_eq!(parsed.assessment.revision, "abcdef");
        assert_eq!(parsed.judged.len(), 3);
        assert!(parsed.judged.iter().all(|j| j.verdict.is_none()));
        let e = evaluate(&parsed, "t", &findings(), Some("abcdef")).unwrap();
        assert_eq!(e.unjudged.len(), 3);
        assert_eq!(e.precision.value, None);
    }
}
