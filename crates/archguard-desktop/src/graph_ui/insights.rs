//! Explorer → Review → "Change coupling & ownership", as the web viewer's
//! review insights (`renderReviewInsights`): Git co-change pairs without an
//! import edge (`archguard co-change`), CODEOWNERS team boundaries
//! (`archguard ownership`) and the architecture intent of the config. One
//! per document, built on a worker thread (Git log and CODEOWNERS take
//! seconds); every Graph of the document shows it.

use archguard_engine::core::{ArchitectureConfig, GitStatus};
use archguard_engine::{CoChangeOptions, CoChangeReport, OwnershipReport, OwnershipStatus};
use slint::ComponentHandle;

use crate::context::Loaded;
use crate::model_sync::sync_rows;
use crate::tab_support::{self, TabState};
use crate::ui::{AppWindow, CoChangeItem, InsightsData, IntentItem, OwnerDepItem};

/// Pairs and dependencies the section lists, as the web viewer.
const SHOWN: usize = 20;

pub struct Insights {
    co_change: Option<CoChangeReport>,
    ownership: Option<OwnershipReport>,
    architecture: ArchitectureConfig,
    /// Whether the section is open, for every Graph of the document.
    pub open: bool,
}

impl Insights {
    /// Read Git history and CODEOWNERS of `data` (blocking).
    pub fn compute(data: &Loaded) -> Self {
        let config = &data.run.config;
        let options = CoChangeOptions {
            since: config.review.git_since.clone(),
            ..Default::default()
        };
        let named = (&data.run.graph, &data.run.source_names);
        Insights {
            co_change: archguard_engine::co_change(data.root, config, named, &options).ok(),
            ownership: archguard_engine::analyze_ownership(
                data.root,
                &data.run.graph,
                data.sources,
                &config.ownership_rules,
            )
            .ok(),
            architecture: config.architecture.clone(),
            open: false,
        }
    }
}

impl TabState for Insights {
    /// The Graphs read it while they are shown, which re-enters this
    /// state; so they are shown again after it is released.
    fn push(&mut self, ui: &AppWindow) {
        let w = ui.as_weak();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                crate::panes_ui::sync(&ui);
            }
        });
    }
}

/// Start reading the active document's insights once.
pub fn show(ui: &AppWindow) {
    let Some(analysis) = crate::documents::with_store(|s| s.active()?.analysis.clone()) else {
        return;
    };
    tab_support::show_async(ui, || move || Insights::compute(&analysis.loaded()));
}

/// Open or close the section in every Graph of the active document
/// (snapshots: `ARCHGUARD_DESKTOP_SIM=explorer:2,review-insights`).
pub fn toggle(ui: &AppWindow) {
    tab_support::update_or_queue(ui, |s: &mut Insights| s.open = !s.open);
}

/// The active document's insights in the models of `old`; not ready while
/// they are read.
pub fn data(old: &InsightsData) -> InsightsData {
    crate::documents::with_active_state(|s: &mut Insights| insights_data(s, old))
        .unwrap_or_default()
}

fn insights_data(s: &Insights, old: &InsightsData) -> InsightsData {
    let mut d = InsightsData {
        ready: true,
        open: s.open,
        intent: sync_rows(&old.intent, intent(&s.architecture)),
        ..Default::default()
    };
    co_change(&mut d, s.co_change.as_ref(), old);
    ownership(&mut d, s.ownership.as_ref(), old);
    d
}

fn percent(ratio: f64) -> i32 {
    (ratio * 100.0).round() as i32
}

fn co_change(d: &mut InsightsData, report: Option<&CoChangeReport>, old: &InsightsData) {
    let Some(r) = report else {
        d.co_message = "co-change analysis failed".into();
        return;
    };
    if r.git.status != GitStatus::Available {
        d.co_message = r.git.message.clone().unwrap_or_default().into();
        return;
    }
    d.co_available = true;
    d.co_commits = r.considered_commits as i32;
    d.co_bulk = r.skipped_bulk_commits as i32;
    d.co_candidates = r.total_candidates as i32;
    d.co_shallow = r.git.shallow;
    d.co_truncated = r.git.truncated || r.pair_search_truncated || r.results_truncated;
    d.co_more = r.pairs.len().saturating_sub(SHOWN) as i32;
    let pairs = r.pairs.iter().take(SHOWN).map(|p| CoChangeItem {
        left: p.left.name.as_str().into(),
        right: p.right.name.as_str().into(),
        shared: p.shared_commits as i32,
        left_to_right: percent(p.left_to_right),
        right_to_left: percent(p.right_to_left),
    });
    d.co_pairs = sync_rows(&old.co_pairs, pairs);
}

fn ownership(d: &mut InsightsData, report: Option<&OwnershipReport>, old: &InsightsData) {
    let Some(r) = report.filter(|r| r.status == OwnershipStatus::Available) else {
        return;
    };
    d.own_available = true;
    d.own_path = r.codeowners_path.clone().unwrap_or_default().into();
    d.own_owned = r.owned_files as i32;
    d.own_total = r.total_files as i32;
    d.own_unowned = r.unowned_files as i32;
    d.own_complete = r.complete;
    d.own_cross = r.cross_owner_dependencies as i32;
    let cross: Vec<_> = r.dependencies.iter().filter(|x| x.cross_owner).collect();
    d.own_more = cross.len().saturating_sub(SHOWN) as i32;
    let rows = cross.iter().take(SHOWN).map(|x| OwnerDepItem {
        from: x.from_name.as_str().into(),
        to: x.to_name.as_str().into(),
        from_owners: x.from_owners.join(", ").into(),
        to_owners: x.to_owners.join(", ").into(),
        count: format!(
            "{}{}",
            x.count,
            if x.count_is_lower_bound { "+" } else { "" }
        )
        .into(),
    });
    d.own_deps = sync_rows(&old.own_deps, rows);
}

/// Goals (kind 1), constraints (2) and decisions (3) as declared.
fn intent(a: &ArchitectureConfig) -> Vec<IntentItem> {
    let goals = a.goals.iter().map(|g| IntentItem {
        kind: 1,
        title: g.name.as_str().into(),
        status: g.priority.as_str().into(),
        detail: g.rationale.as_str().into(),
        ..Default::default()
    });
    let constraints = a.constraints.iter().map(|c| IntentItem {
        kind: 2,
        title: c.name.as_str().into(),
        detail: c.rationale.as_str().into(),
        ..Default::default()
    });
    let decisions = a.decisions.iter().map(|x| IntentItem {
        kind: 3,
        title: format!("{} · {}", x.id, x.title).into(),
        status: x.status.as_str().into(),
        detail: x.chosen_option.as_str().into(),
        alternatives: x.alternatives.join("; ").into(),
        trade_offs: x.trade_offs.join("; ").into(),
    });
    goals.chain(constraints).chain(decisions).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;
    use std::path::Path;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("dir");
        for e in std::fs::read_dir(from).expect("fixture") {
            let e = e.expect("entry");
            let target = to.join(e.file_name());
            if e.path().is_dir() {
                copy(&e.path(), &target);
            } else {
                std::fs::copy(e.path(), target).expect("copy");
            }
        }
    }

    const CONFIG: &str = "[[architecture.goals]]\nname = \"Changeable checkout\"\npriority = \"first\"\nrationale = \"Weekly releases\"\n[[architecture.constraints]]\nname = \"On premises\"\nrationale = \"Customer contracts\"\n[[architecture.decisions]]\nid = \"ADR-1\"\ntitle = \"Layered core\"\nstatus = \"accepted\"\nchosen_option = \"Layers\"\nalternatives = [\"Hexagonal\"]\ntrade_offs = [\"More indirection\"]\nlinks = []\n";

    /// A temp copy of fixtures/ts-derive as a Git repository: cli.ts and
    /// catalog.ts (no import between them) change together four times;
    /// CODEOWNERS gives each layer its own team.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts-derive");
        copy(&fixture, dir.path());
        std::fs::write(dir.path().join("archguard.toml"), CONFIG).expect("config");
        std::fs::write(
            dir.path().join("CODEOWNERS"),
            "/src/api/ @team-api\n/src/app/ @team-app\n/src/domain/ @team-domain\n",
        )
        .expect("codeowners");
        git(dir.path(), &["init", "--quiet"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        git(dir.path(), &["add", "--all"]);
        git(dir.path(), &["commit", "--quiet", "-m", "start"]);
        for i in 0..4 {
            for f in ["src/api/cli.ts", "src/app/catalog.ts"] {
                let path = dir.path().join(f);
                let text = std::fs::read_to_string(&path).expect("source");
                std::fs::write(&path, format!("{text}// change {i}\n")).expect("write");
            }
            git(
                dir.path(),
                &["commit", "--quiet", "-am", &format!("change {i}")],
            );
        }
        dir
    }

    fn analysis(root: &Path) -> crate::context::Analysis {
        let loaded = archguard_engine::load_config(None, root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let named = (&run.graph, &run.source_names);
        let sources = archguard_engine::collect_review_sources(
            root,
            &run.config,
            named,
            &run.config.review.git_since,
        )
        .expect("sources");
        crate::context::Analysis {
            root: root.to_path_buf(),
            run,
            sources,
        }
    }

    #[test]
    fn co_change_ownership_and_intent_reach_the_section() {
        let dir = project();
        let a = analysis(dir.path());
        let s = Insights::compute(&a.loaded());
        let d = insights_data(&s, &InsightsData::default());
        assert!(d.ready && d.co_available, "{}", d.co_message);
        assert!(d.co_commits >= 4, "commits used: {}", d.co_commits);
        let pairs: Vec<CoChangeItem> = d.co_pairs.iter().collect();
        let pair = pairs
            .iter()
            .find(|p| [&p.left, &p.right].iter().any(|n| n.contains("cli")))
            .unwrap_or_else(|| panic!("cli.ts ↔ catalog.ts in {pairs:?}"));
        assert!(pair.left.contains("catalog") || pair.right.contains("catalog"));
        assert_eq!(
            (pair.shared, pair.left_to_right, pair.right_to_left),
            (5, 100, 100),
            "the first commit and four changes"
        );
        assert!(!d.co_shallow && !d.co_truncated);

        assert!(d.own_available, "CODEOWNERS is read");
        assert_eq!(d.own_path.as_str(), "CODEOWNERS");
        assert_eq!((d.own_owned, d.own_unowned), (d.own_total, 0));
        assert!(d.own_cross > 0, "app → domain crosses teams");
        let dep = d.own_deps.row_data(0).expect("a cross-owner dependency");
        assert_ne!(dep.from_owners, dep.to_owners);

        let intent: Vec<IntentItem> = d.intent.iter().collect();
        assert_eq!(intent.iter().map(|i| i.kind).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(intent[2].title.as_str(), "ADR-1 · Layered core");
        assert_eq!(intent[2].trade_offs.as_str(), "More indirection");
    }

    /// Outside Git: the reason instead of pairs; no CODEOWNERS, no section.
    #[test]
    fn without_git_or_codeowners_the_section_says_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts-derive");
        copy(&fixture, dir.path());
        let a = analysis(dir.path());
        let d = insights_data(&Insights::compute(&a.loaded()), &InsightsData::default());
        assert!(!d.co_available && !d.co_message.is_empty());
        assert!(!d.own_available);
        assert_eq!(d.intent.row_count(), 0);
    }
}
