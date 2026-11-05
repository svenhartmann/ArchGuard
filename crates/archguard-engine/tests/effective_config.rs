//! T46 integration: the effective-config derivation over real project
//! trees (ARCHITECTURE.md Kap. 6 steps 2 and 7, Kap. 11) — provenance
//! in `Report::config`, layer merge per name and the unranked hint.

use std::fs;
use std::path::Path;

use archguard_core::{ConfigSource, FindingKind, Severity, Status};
use archguard_engine::{Level, analyze_effective, effective_config_string, load_config};

/// Write `files` (path, content) into a fresh temp dir.
fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(path, content).expect("write file");
    }
    dir
}

/// `api` → `app` → `domain`, acyclic. The module nodes visible at the
/// derived view depth are `shop/api`, `shop/app`, `shop/domain`, so their
/// trailing segments hit the naming affinity vocabulary (Kap. 7) and the
/// derived layers come out named `domain`, `app`, `api`.
const LAYERED: &[(&str, &str)] = &[
    (
        "src/shop/api/one.ts",
        "import { a } from \"../app/one\";\nexport const one = a + 1;\n",
    ),
    (
        "src/shop/app/one.ts",
        "import { d } from \"../domain/one\";\nexport const a = d + 1;\n",
    ),
    ("src/shop/domain/one.ts", "export const d = 1;\n"),
];

/// `app` ⇄ `api` form a cycle above the acyclic `domain`.
const CYCLING: &[(&str, &str)] = &[
    (
        "src/shop/api/one.ts",
        "import { a } from \"../app/one\";\nexport const one = a + 1;\n",
    ),
    (
        "src/shop/app/one.ts",
        "import { d } from \"../domain/one\";\nimport { p } from \"../api/one\";\nexport const a = d;\nexport const b = p + 1;\n",
    ),
    ("src/shop/domain/one.ts", "export const d = 1;\n"),
];

/// load + analyze exactly as the CLI would.
fn run(root: &Path) -> (archguard_core::Graph, archguard_core::Report) {
    let loaded = load_config(None, root).expect("load config");
    analyze_effective(root, &loaded, Level::Module).expect("analyze")
}

#[test]
fn project_without_config_derives_project_modules_and_layers() {
    let dir = fixture(LAYERED);
    let (_graph, report) = run(dir.path());
    assert_eq!(report.config["project"], ConfigSource::Derived);
    assert_eq!(report.config["modules"], ConfigSource::Derived);
    assert_eq!(report.config["layers"], ConfigSource::Derived);
    assert_eq!(report.config["rules"], ConfigSource::Default);
    assert_eq!(report.config["store"], ConfigSource::Default);
    assert_eq!(report.summary.status, Status::Pass);

    // The same project rendered by `archguard init` names the three layers
    // after their affinity words and covers the three module dirs.
    let text = effective_config_string(dir.path());
    assert!(text.contains("# abgeleitet"), "text: {text}");
    assert!(text.contains("# gemessen"), "text: {text}");
    for name in ["domain", "app", "api"] {
        assert!(text.contains(&format!("name = \"{name}\"")), "text: {text}");
        assert!(
            text.contains(&format!("contains = [\"shop/{name}/**\"]")),
            "text: {text}"
        );
    }
}

#[test]
fn three_declared_layers_stay_declared_and_keep_their_names() {
    let mut files: Vec<(&str, &str)> = LAYERED.to_vec();
    files.push((
        "archguard.toml",
        "[[layers]]\n\
         name = \"domain\"\n\
         contains = [\"domain/**\"]\n\
         may_depend_on = []\n\n\
         [[layers]]\n\
         name = \"app\"\n\
         contains = [\"app/**\"]\n\
         may_depend_on = [\"domain\"]\n\n\
         [[layers]]\n\
         name = \"api\"\n\
         contains = [\"api/**\"]\n\
         may_depend_on = [\"domain\", \"app\"]\n",
    ));
    let dir = fixture(&files);
    let (_graph, report) = run(dir.path());
    // Every derived entry lost its name to a declared one → the whole
    // section stays Declared, with no derived duplicates.
    assert_eq!(report.config["layers"], ConfigSource::Declared);
    assert_eq!(report.summary.status, Status::Pass);
}

#[test]
fn declared_override_of_one_derived_name_yields_mixed() {
    let mut files: Vec<(&str, &str)> = LAYERED.to_vec();
    files.push((
        "archguard.toml",
        "[[layers]]\n\
         name = \"app\"\n\
         contains = [\"app/**\"]\n\
         may_depend_on = [\"domain\"]\n",
    ));
    let dir = fixture(&files);
    let (_graph, report) = run(dir.path());
    // `app` kept by the declared entry, `domain` and `api` appended
    // derived → both origins survive.
    assert_eq!(report.config["layers"], ConfigSource::Mixed);
    assert_eq!(report.summary.status, Status::Pass);
}

#[test]
fn cycle_that_blocks_ranking_yields_the_unranked_info_hint() {
    let dir = fixture(CYCLING);
    let (_graph, report) = run(dir.path());
    let hints: Vec<&archguard_core::Finding> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::LayerUnranked)
        .collect();
    assert_eq!(hints.len(), 1, "findings: {:?}", report.findings);
    let hint = hints[0];
    assert_eq!(hint.id, "layers-unranked");
    assert_eq!(hint.severity, Severity::Info);
    assert_eq!(hint.nodes, ["shop/api", "shop/app"]);
    assert_eq!(hint.edge_count, 0);
    assert_eq!(hint.cycles_sampled, 0);
    assert!(hint.suggested_cuts.is_empty());
    // The hint is sorted behind the gate findings (Info last).
    assert_eq!(
        report.findings.last().expect("findings exist").kind,
        FindingKind::LayerUnranked
    );
    assert_eq!(report.config["layers"], ConfigSource::Derived);
}

#[test]
fn rules_and_store_stay_declared_when_written() {
    let mut files: Vec<(&str, &str)> = LAYERED.to_vec();
    files.push((
        "archguard.toml",
        "[rules]\ndeny_cycles = \"off\"\n\n[store]\ndb = \".archguard/x.db\"\n",
    ));
    let dir = fixture(&files);
    let (_graph, report) = run(dir.path());
    assert_eq!(report.config["rules"], ConfigSource::Declared);
    assert_eq!(report.config["store"], ConfigSource::Declared);
    // Derived sections are unaffected by the declared ones.
    assert_eq!(report.config["project"], ConfigSource::Derived);
    assert_eq!(report.config["modules"], ConfigSource::Derived);
}

#[test]
fn header_only_project_and_modules_sections_are_still_derived() {
    // `[project]` and `[modules]` without values declare nothing, so the
    // resolution `declared ⇐ derived ⇐ default` must still derive them.
    let mut files: Vec<(&str, &str)> = LAYERED.to_vec();
    files.push(("archguard.toml", "[project]\n[modules]\n[rules]\n"));
    let dir = fixture(&files);
    let (_graph, report) = run(dir.path());
    assert_eq!(report.config["project"], ConfigSource::Derived);
    assert_eq!(report.config["modules"], ConfigSource::Derived);
    assert_eq!(report.config["rules"], ConfigSource::Default);
}

#[test]
fn defaults_apply_with_default_provenance_and_never_fail_check() {
    // No archguard.toml: every section that is neither derived nor
    // declared carries its documented default with provenance `Default`.
    // The off-by-default gates stay off although the project has measurable
    // coupling, and derived layers produce no Error finding.
    let dir = fixture(LAYERED);
    let loaded = load_config(None, dir.path()).expect("load config");
    let run =
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, Level::Module).expect("analyze");
    let defaults = archguard_core::Config::default();
    for key in [
        "rules",
        "store",
        "review",
        "performance",
        "architecture",
        "semantic",
        "dependency_rules",
        "ownership_rules",
    ] {
        assert_eq!(
            run.report.config[key],
            ConfigSource::Default,
            "section {key}"
        );
    }
    assert_eq!(run.config.rules, defaults.rules);
    assert_eq!(run.config.store, defaults.store);
    assert_eq!(run.config.review, defaults.review);
    assert_eq!(run.config.performance, defaults.performance);
    assert_eq!(run.config.semantic, defaults.semantic);
    assert_eq!(run.config.project.exclude, defaults.project.exclude);
    assert_eq!(run.report.summary.status, Status::Pass);
    assert!(
        run.report
            .findings
            .iter()
            .all(|finding| finding.severity != Severity::Error),
        "findings: {:?}",
        run.report.findings
    );
}
