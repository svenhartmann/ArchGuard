//! Imported quality findings (SARIF from linters and analyzers): the web
//! viewer's `renderQualityFindings` list and `appendQualityFinding` card.
//! The Graph's badges and inspector section use the same rules
//! (`graph_ui/quality.rs`).

use std::collections::BTreeSet;

use archguard_engine::{QualityFinding, QualityReport};

use super::card::{Builder, Card, Kind};
use super::rows::{Scope, matches};
use crate::tab_support::Texts;

/// The level picker after "All levels": SARIF's levels.
pub const LEVELS: [&str; 4] = ["error", "warning", "note", "none"];

/// `qualityActive`: a fresh, mapped, unsuppressed finding of a usable
/// import that the producer still reports and that has a level.
pub fn active(report: &QualityReport, f: &QualityFinding) -> bool {
    matches!(report.status.as_str(), "ready" | "partial" | "stale")
        && f.node.is_some()
        && f.status == "unchanged_since_import"
        && !f.suppressed
        && f.baseline_state.as_deref() != Some("absent")
        && f.level != "none"
}

fn level_rank(level: &str) -> usize {
    LEVELS
        .iter()
        .position(|&l| l == level)
        .unwrap_or(LEVELS.len())
}

/// All findings, active first, then by level and id (as the web).
pub fn sorted(report: &QualityReport) -> Vec<&QualityFinding> {
    let mut out: Vec<_> = report.findings.iter().collect();
    out.sort_by_cached_key(|f| (!active(report, f), level_rank(&f.level), f.id.clone()));
    out
}

/// The listed findings: `level` (index into [`LEVELS`], `None` all), the
/// scope (mapped node or path) and the search over rule, message and path.
pub fn rows<'a>(
    report: &'a QualityReport,
    scope: &Scope,
    level: Option<usize>,
    query: &str,
) -> Vec<&'a QualityFinding> {
    let level = level.and_then(|i| LEVELS.get(i));
    sorted(report)
        .into_iter()
        .filter(|f| level.is_none_or(|l| f.level == *l))
        .filter(|f| {
            scope.all()
                || f.node.as_ref().is_some_and(|n| scope.names.contains(n))
                || f.path.as_ref().is_some_and(|p| scope.paths.contains(p))
        })
        .filter(|f| {
            let path = f.path.as_deref().unwrap_or_default();
            let text = [f.tool.as_str(), &f.rule_id, &f.message, path].join(" ");
            matches(query, &text)
        })
        .collect()
}

/// "source unchanged since import", with suppression and baseline notes.
fn freshness(lang: &Texts, f: &QualityFinding) -> String {
    let mut out = match f.status.as_str() {
        "unchanged_since_import" => lang.t("Source unchanged since import"),
        "changed_since_import" => lang.t("Source changed since import · historical finding"),
        _ => lang.t("Source freshness unknown"),
    }
    .to_string();
    if f.baseline_state.as_deref() == Some("absent") {
        out.push_str(" · ");
        out.push_str(lang.t("Absent in the producer's baseline comparison"));
    }
    if f.suppressed {
        out.push_str(" · ");
        out.push_str(lang.t("suppressed"));
    }
    out
}

/// One finding: rule and level, message, file:line, freshness, the rule's
/// documentation and, with `locate`, "Locate in graph".
pub fn card(lang: &Texts, nodes: &BTreeSet<String>, f: &QualityFinding, locate: bool) -> Card {
    // The engine accepts only SARIF's levels, each a template.
    let level = lang.t(&f.level);
    let mut b = Builder::new(lang, nodes, format!("{} · {} · {level}", f.rule_id, f.tool));
    b.card.key = f.id.clone();
    let location = match &f.path {
        Some(path) => {
            let at = [f.line, f.column].into_iter().flatten();
            std::iter::once(path.clone())
                .chain(at.map(|n| n.to_string()))
                .collect::<Vec<_>>()
                .join(":")
        }
        None => lang.t("No local source location").into(),
    };
    b.push(Kind::Text, &f.message)
        .push(Kind::Path, location)
        .push(Kind::Small, freshness(lang, f));
    let url = crate::security_ui::snapshot::safe_url(f.help_uri.as_deref());
    if let Some(url) = url {
        b.link(lang.t("Rule documentation"), url);
    }
    if locate {
        b.locate(f.node.as_deref());
    }
    b.done()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn finding(id: &str, level: &str, node: Option<&str>) -> QualityFinding {
        QualityFinding {
            id: id.into(),
            tool: "Lint".into(),
            rule_id: format!("R-{id}"),
            level: level.into(),
            message: format!("message {id}"),
            path: node.map(|n| format!("src/{n}")),
            node: node.map(Into::into),
            line: Some(3),
            column: None,
            source_sha256: None,
            status: if node.is_some() {
                "unchanged_since_import"
            } else {
                "unmapped"
            }
            .into(),
            help_uri: Some("https://example.test/rules/R".into()),
            suppressed: false,
            baseline_state: None,
        }
    }

    pub fn report(findings: Vec<QualityFinding>) -> QualityReport {
        QualityReport {
            schema_version: 1,
            status: "ready".into(),
            report_path: None,
            imported_at_unix: None,
            tools: vec!["Lint".into()],
            findings,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn active_findings_come_first_then_by_level() {
        let mut suppressed = finding("a", "error", Some("x.py"));
        suppressed.suppressed = true;
        let r = report(vec![
            suppressed,
            finding("b", "note", Some("x.py")),
            finding("c", "error", None),
            finding("d", "warning", Some("x.py")),
        ]);
        let ids: Vec<_> = sorted(&r).iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["d", "b", "a", "c"]);
        let all = Scope::default();
        let warnings = rows(&r, &all, Some(1), "");
        assert_eq!(warnings.len(), 1);
        assert_eq!(rows(&r, &all, None, "message c").len(), 1);
        assert_eq!(rows(&r, &all, None, "src/x.py").len(), 3);
    }

    #[test]
    fn a_card_names_rule_location_freshness_and_its_links() {
        let lang = Texts::default();
        let nodes = BTreeSet::from(["x.py".to_string()]);
        let mut f = finding("a", "warning", Some("x.py"));
        f.column = Some(7);
        let c = card(&lang, &nodes, &f, true);
        assert_eq!(c.title, "R-a · Lint · warning");
        let texts: Vec<_> = c.items.iter().map(|i| (i.kind, i.text.as_str())).collect();
        assert_eq!(texts[0], (Kind::Text, "message a"));
        assert_eq!(texts[1], (Kind::Path, "src/x.py:3:7"));
        assert_eq!(texts[2], (Kind::Small, "Source unchanged since import"));
        assert_eq!(texts[3], (Kind::Link, "Rule documentation"));
        assert_eq!(c.items[3].node, "https://example.test/rules/R");
        assert_eq!(c.items[4].kind, Kind::Locate);
        f.help_uri = Some("javascript:alert(1)".into());
        f.status = "changed_since_import".into();
        let c = card(&Texts::german(), &nodes, &f, false);
        assert!(
            c.items
                .iter()
                .all(|i| i.kind != Kind::Link && i.kind != Kind::Locate)
        );
        assert_eq!(c.title, "R-a · Lint · Warnung");
    }

    /// A temp copy of fixture `py-layers` with an imported SARIF report of
    /// three findings: an error and a note in `src/app/service.py`, a
    /// warning outside the project. Returns the root and its analysis.
    pub fn imported_fixture(name: &str) -> (std::path::PathBuf, archguard_engine::AnalysisRun) {
        use std::path::Path;
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap().flatten() {
                let (p, t) = (e.path(), to.join(e.file_name()));
                if p.is_dir() {
                    copy(&p, &t);
                } else {
                    std::fs::copy(&p, &t).unwrap();
                }
            }
        }
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/py-layers");
        let root = std::env::temp_dir().join(format!("ag-findings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        copy(&fixture, &root);
        let root = root.canonicalize().unwrap();
        let result = |rule: &str, uri: &str, line: u32, level: &str| {
            serde_json::json!({"ruleId": rule, "level": level, "message": {"text": format!("{rule} found")},
                "locations": [{"physicalLocation": {"artifactLocation": {"uri": uri}, "region": {"startLine": line}}}]})
        };
        let sarif = serde_json::json!({"version": "2.1.0", "runs": [{
            "tool": {"driver": {"name": "Lint", "rules": [{"id": "E1", "helpUri": "https://example.test/rules/E1"}]}},
            "invocations": [{"executionSuccessful": true}],
            "results": [result("E1", "src/app/service.py", 4, "error"), result("N1", "src/app/service.py", 5, "note"),
                result("W1", "vendor/other.py", 1, "warning")]}]});
        std::fs::write(root.join("lint.sarif"), sarif.to_string()).unwrap();
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        archguard_engine::import_quality_findings(
            &root,
            &run.config,
            &run.graph,
            Path::new("lint.sarif"),
            None,
        )
        .expect("import");
        (root, run)
    }
}
