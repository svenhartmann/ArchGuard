//! Review decisions (View → Review decisions…): the findings of a review
//! generated now, decision drafts per finding, and the two ways out. Both
//! go through the engine's decision document: "Record" imports it as
//! `archguard review --decisions … --save` does (same validation, register
//! lock and fingerprint checks), "Export" writes it for someone who
//! imports it later. No window here; `review_decisions_ui` shows it.

use std::path::{Path, PathBuf};

use archguard_engine::core::{DecisionKind, ReviewDecision};
use archguard_engine::{ReviewReport, apply_review_decisions, review_decision_document};

/// The kinds in the dialog's order (its combo box index).
pub const KINDS: [DecisionKind; 3] = [
    DecisionKind::Fix,
    DecisionKind::Defer,
    DecisionKind::AcceptRisk,
];

/// What the form holds for one finding.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Form {
    pub kind: DecisionKind,
    pub note: String,
    pub adr: String,
    pub expires_on: String,
    pub author: String,
}

pub struct Session {
    pub root: PathBuf,
    pub report: ReviewReport,
    /// Decisions not yet recorded or exported, one per finding.
    pub drafts: Vec<ReviewDecision>,
    /// The author a new decision starts with: `git config user.name`.
    pub author: String,
}

fn some(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

impl Session {
    /// Generate the review of `root` as `archguard review` does (blocking).
    pub fn generate(root: &Path, now: u64) -> Result<Session, String> {
        let (report, _) =
            archguard_engine::generate_review(root, None, None, now).map_err(|e| e.to_string())?;
        Ok(Session {
            root: root.to_path_buf(),
            report,
            drafts: Vec::new(),
            author: git_author(root),
        })
    }

    /// The draft of finding `i`, else its recorded decision.
    pub fn decision(&self, i: usize) -> Option<&ReviewDecision> {
        let f = self.report.findings.get(i)?;
        self.drafts
            .iter()
            .find(|d| d.finding_ref == f.id)
            .or(f.decision.as_ref())
    }

    /// The form for finding `i`: its draft or recorded decision, else
    /// empty with the default author.
    pub fn form(&self, i: usize) -> Form {
        match self.decision(i) {
            Some(d) => Form {
                kind: d.decision,
                note: d.note.clone(),
                adr: d.adr_link.clone().unwrap_or_default(),
                expires_on: d.expires_on.clone().unwrap_or_default(),
                author: d.author.clone(),
            },
            None => Form {
                author: self.author.clone(),
                ..Form::default()
            },
        }
    }

    /// Keep `form` as the draft of finding `i`, replacing an earlier one.
    /// The import validates it (author, note, expiry for accepted risks).
    pub fn add(&mut self, i: usize, form: &Form) {
        let Some(f) = self.report.findings.get(i) else {
            return;
        };
        let mut d = f.draft_decision(form.kind);
        d.note = form.note.trim().to_string();
        d.author = form.author.trim().to_string();
        d.adr_link = some(&form.adr);
        d.expires_on = some(&form.expires_on);
        self.drafts.retain(|x| x.finding_ref != d.finding_ref);
        self.drafts.push(d);
    }

    fn document(&self) -> Vec<u8> {
        let doc = review_decision_document(&self.report, &self.drafts);
        serde_json::to_vec_pretty(&doc).expect("decision document serializes")
    }

    /// Import the drafts into the register and save the review, as
    /// `archguard review --decisions <export> --save`; returns how many
    /// were recorded. On an error nothing is written and the drafts stay.
    pub fn record(&mut self, now: u64) -> Result<usize, String> {
        let bytes = self.document();
        apply_review_decisions(&self.root, &mut self.report, &bytes, now)
            .and_then(|()| archguard_engine::save_review(&self.root, &self.report))
            .map_err(|e| e.to_string())?;
        Ok(std::mem::take(&mut self.drafts).len())
    }

    /// Write the decision document (recorded decisions plus drafts) to
    /// `path` for `archguard review --decisions <path>`.
    pub fn export(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.document()).map_err(|e| e.to_string())
    }

    /// Where Export suggests to save, as the import command names it.
    pub fn export_path(&self) -> PathBuf {
        self.root.join(".archguard/review-decisions-draft.json")
    }
}

/// `git config user.name` in `root`; empty without Git or a name.
pub fn git_author(root: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_793_232_000;

    /// A three-file import cycle: one finding.
    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(root.path().join("src")).expect("src");
        for (file, source) in [("a.ts", "import './b';\n"), ("b.ts", "import './a';\n")] {
            std::fs::write(root.path().join("src").join(file), source).expect("source");
        }
        std::fs::write(root.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='file'\nmax_scc_size=100\n").expect("config");
        root
    }

    fn form(kind: DecisionKind) -> Form {
        Form {
            kind,
            note: " After the release ".into(),
            adr: "docs/adr/0007.md".into(),
            expires_on: String::new(),
            author: "Reviewer".into(),
        }
    }

    #[test]
    fn a_recorded_draft_lands_in_the_register_and_the_saved_review() {
        let root = project();
        let mut s = Session::generate(root.path(), NOW).expect("review");
        assert!(!s.report.findings.is_empty(), "the cycle is a finding");
        assert_eq!(s.form(0).author, s.author);
        s.add(0, &form(DecisionKind::Defer));
        s.add(0, &form(DecisionKind::Fix));
        assert_eq!(s.drafts.len(), 1, "one draft per finding");
        assert_eq!(s.record(NOW), Ok(1));
        assert!(s.drafts.is_empty());
        let register = archguard_engine::load_register(root.path()).expect("register");
        let d = register
            .decision_for(&s.report.findings[0].id)
            .expect("recorded");
        assert_eq!(
            (d.decision, d.note.as_str()),
            (DecisionKind::Fix, "After the release")
        );
        assert_eq!(d.adr_link.as_deref(), Some("docs/adr/0007.md"));
        assert_eq!(s.report.findings[0].disposition, "planned");
        assert!(root.path().join(".archguard/review.json").is_file());
        // The form shows the recorded decision now.
        assert_eq!(s.form(0).author, "Reviewer");
    }

    /// The engine refuses an accepted risk without expiry; nothing is
    /// written and the draft stays for correction.
    #[test]
    fn an_invalid_draft_is_refused_and_kept() {
        let root = project();
        let mut s = Session::generate(root.path(), NOW).expect("review");
        s.add(0, &form(DecisionKind::AcceptRisk));
        let err = s.record(NOW).expect_err("needs expires_on");
        assert!(err.contains("expires_on"), "{err}");
        assert_eq!(s.drafts.len(), 1);
        assert!(!root.path().join(".archguard/reviews.json").exists());
    }

    #[test]
    fn an_export_is_what_review_decisions_imports() {
        let root = project();
        let mut s = Session::generate(root.path(), NOW).expect("review");
        s.add(0, &form(DecisionKind::Defer));
        let path = s.export_path();
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        s.export(&path).expect("export");
        assert!(
            !root.path().join(".archguard/reviews.json").exists(),
            "export records nothing"
        );
        let (mut fresh, _) =
            archguard_engine::generate_review(root.path(), None, None, NOW).expect("review");
        archguard_engine::import_review_decisions(root.path(), &mut fresh, &path, NOW)
            .expect("the import accepts the export");
        assert_eq!(fresh.findings[0].disposition, "deferred");
    }
}
