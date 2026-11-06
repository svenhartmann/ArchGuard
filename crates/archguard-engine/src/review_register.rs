//! Decision register (R19).
//!
//! The register is the project-owned record of how the team handled
//! findings: `.archguard/reviews.json`, a serde document of
//! [`archguard_core::ReviewDecision`] entries. `review-decision`, the MCP
//! `review_decisions` tool and `archguard review --decisions` all read and
//! write this one file. Writes are atomic (tmp file in the same directory +
//! rename) so a crash never leaves a half-written register, and a corrupt
//! file surfaces as an error with a hint — archguard never overwrites a
//! register it cannot read.
//!
//! Older versions kept review-session decisions in
//! `.archguard/review-decisions.json`. While that file exists, reads merge
//! its entries (register entries with the same id win); the next register
//! write stores them in `reviews.json` and deletes the old file. Read-only
//! paths never write.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use archguard_core::{ArchguardError, DecisionKind, ReviewDecision, ReviewRegister};
use serde::{Deserialize, Serialize};

/// Register location relative to the project root.
pub const REGISTER_REL_PATH: &str = ".archguard/reviews.json";

/// Former location of review-session decisions; migrated on the next write.
pub const LEGACY_DECISIONS_REL_PATH: &str = ".archguard/review-decisions.json";

/// Absolute path of the register file for `root`.
pub fn register_path(root: &Path) -> PathBuf {
    root.join(REGISTER_REL_PATH)
}

fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("review register: {message}"))
}

fn corrupt(path: &Path, message: impl std::fmt::Display) -> ArchguardError {
    error(format!(
        "{}: {message} — repair or move the file aside, archguard never overwrites a corrupt register",
        path.display()
    ))
}

/// The document `review-decisions.json` held (schema version 1).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDocument {
    schema_version: u32,
    #[allow(dead_code)]
    review_revision: String,
    #[allow(dead_code)]
    #[serde(default)]
    decision_revision: Option<String>,
    decisions: Vec<LegacyDecision>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDecision {
    finding_id: String,
    finding_fingerprint: String,
    status: String,
    owner: String,
    reason: String,
    expires_on: Option<String>,
    #[serde(default)]
    links: Vec<String>,
}

/// Decisions of a not yet migrated `review-decisions.json`; none when absent.
/// `open` becomes `defer`, `planned` becomes `fix`, `accepted_risk` becomes
/// `accept-risk`; the old file recorded no timestamp, so `ts` stays empty.
fn legacy_decisions(root: &Path) -> Result<Vec<ReviewDecision>, ArchguardError> {
    let path = root.join(LEGACY_DECISIONS_REL_PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(io) if io.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(io) => return Err(error(format!("reading {}: {io}", path.display()))),
    };
    let document: LegacyDocument = serde_json::from_str(&text).map_err(|e| corrupt(&path, e))?;
    if document.schema_version != 1 {
        return Err(corrupt(&path, "unsupported schema_version"));
    }
    document
        .decisions
        .into_iter()
        .map(|d| {
            let decision = match d.status.as_str() {
                "open" => DecisionKind::Defer,
                "planned" => DecisionKind::Fix,
                "accepted_risk" => DecisionKind::AcceptRisk,
                other => return Err(corrupt(&path, format!("unknown status {other:?}"))),
            };
            Ok(ReviewDecision {
                id: d.finding_id.clone(),
                finding_ref: d.finding_id,
                decision,
                adr_link: None,
                note: d.reason,
                author: d.owner,
                ts: String::new(),
                finding_fingerprint: Some(d.finding_fingerprint),
                expires_on: d.expires_on,
                links: d.links,
            })
        })
        .collect()
}

/// Read `.archguard/reviews.json` plus a not yet migrated
/// `review-decisions.json`; a missing file means an empty register,
/// corrupt content is an error (never silently replaced).
pub fn load_register(root: &Path) -> Result<ReviewRegister, ArchguardError> {
    let path = register_path(root);
    let mut register: ReviewRegister = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| corrupt(&path, e))?,
        Err(io) if io.kind() == ErrorKind::NotFound => ReviewRegister::default(),
        Err(io) => return Err(error(format!("reading {}: {io}", path.display()))),
    };
    for decision in legacy_decisions(root)? {
        if !register.decisions.iter().any(|d| d.id == decision.id) {
            register.decisions.push(decision);
        }
    }
    Ok(register)
}

/// Write the register atomically below `.archguard` (no symlinked
/// directories) and retire a migrated `review-decisions.json`.
pub fn save_register(root: &Path, register: &ReviewRegister) -> Result<(), ArchguardError> {
    let text = serde_json::to_string_pretty(register)
        .map_err(|e| error(format!("serializing register: {e}")))?;
    let relative = Path::new(REGISTER_REL_PATH)
        .strip_prefix(".archguard")
        .expect("register lives in .archguard");
    crate::project_files::write_project_file(root, relative, format!("{text}\n").as_bytes(), true)
        .map_err(|e| error(format!("writing {}: {e}", register_path(root).display())))?;
    match std::fs::remove_file(root.join(LEGACY_DECISIONS_REL_PATH)) {
        Ok(()) => Ok(()),
        Err(io) if io.kind() == ErrorKind::NotFound => Ok(()),
        Err(io) => Err(error(format!(
            "removing migrated {LEGACY_DECISIONS_REL_PATH}: {io}"
        ))),
    }
}

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ` (proleptic Gregorian, UTC) —
/// the civil-from-days conversion, no date crate needed.
pub fn iso_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Load, upsert the decision by id and persist; returns the register
/// as stored.
pub fn upsert_decision(
    root: &Path,
    decision: ReviewDecision,
) -> Result<ReviewRegister, ArchguardError> {
    let mut register = load_register(root)?;
    register.upsert(decision);
    save_register(root, &register)?;
    Ok(register)
}

/// Errors of the review import carry the review prefix, as before.
fn review_error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("architecture review: {message}"))
}
/// The decision import document (`archguard review --decisions`).
const IMPORT_SCHEMA: u32 = 2;
/// Bound for decision-state reads; matches the review session.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// The document `archguard review --decisions` imports and the viewer
/// exports: register entries bound to findings of one review revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecisionImport {
    pub schema_version: u32,
    pub review_revision: String,
    #[serde(default)]
    pub decision_revision: Option<String>,
    pub decisions: Vec<ReviewDecision>,
}
impl ReviewDecisionImport {
    /// The document belongs to this review and to its decision state.
    pub(crate) fn check(
        &self,
        revision: &str,
        decision_fingerprint: &str,
    ) -> Result<(), ArchguardError> {
        if self.schema_version != IMPORT_SCHEMA
            || !crate::evidence_import::valid_hash(&self.review_revision)
        {
            return Err(review_error(
                "invalid decision document version or review revision; export again from the current review",
            ));
        }
        if self.review_revision != revision {
            return Err(review_error(
                "decision document belongs to a different review revision; regenerate and review the current evidence",
            ));
        }
        if self.decision_revision.as_deref() != Some(decision_fingerprint)
            && (decision_fingerprint != NO_DECISION_FILE || self.decision_revision.is_some())
        {
            return Err(review_error(
                "decision export is based on an older decision revision; export again from the current review",
            ));
        }
        Ok(())
    }
}
/// Identity of the decision state: the register plus a not yet migrated
/// `review-decisions.json` (see `review_register`).
pub(crate) fn decisions_fingerprint(root: &Path) -> String {
    let register = crate::evidence_import::file_fingerprint(root, REGISTER_REL_PATH, MAX_BYTES);
    let legacy =
        crate::evidence_import::file_fingerprint(root, LEGACY_DECISIONS_REL_PATH, MAX_BYTES);
    if legacy == NO_DECISION_FILE {
        register
    } else {
        format!("{register}+{legacy}")
    }
}
pub(crate) const NO_DECISION_FILE: &str = "unavailable:NotFound";
/// The review disposition a decision kind stands for.
pub(crate) fn review_disposition(kind: DecisionKind) -> &'static str {
    match kind {
        DecisionKind::Fix => "planned",
        DecisionKind::Defer => "deferred",
        DecisionKind::AcceptRisk => "accepted_risk",
    }
}
/// The import replaces the review-bound entries; decisions recorded with
/// `review-decision` or MCP stay in the register.
pub(crate) fn replace_review_decisions(
    root: &Path,
    decisions: &[ReviewDecision],
) -> Result<(), ArchguardError> {
    let mut register = load_register(root)?;
    register
        .decisions
        .retain(|d| d.finding_fingerprint.is_none());
    for d in decisions {
        register.upsert(d.clone());
    }
    save_register(root, &register)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_civil_from_unix_is_exact() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(42_611), "1970-01-01T11:50:11Z");
    }

    #[test]
    fn register_path_points_into_archguard_dir() {
        assert_eq!(
            register_path(Path::new("/proj")),
            PathBuf::from("/proj/.archguard/reviews.json")
        );
    }

    #[test]
    fn signature_is_stable() {
        let _: fn(&Path) -> Result<ReviewRegister, ArchguardError> = load_register;
        let _: fn(&Path, &ReviewRegister) -> Result<(), ArchguardError> = save_register;
        let _: fn(&Path, ReviewDecision) -> Result<ReviewRegister, ArchguardError> =
            upsert_decision;
    }

    fn decision(id: &str, finding: &str) -> ReviewDecision {
        ReviewDecision {
            id: id.to_string(),
            finding_ref: finding.to_string(),
            decision: archguard_core::DecisionKind::Defer,
            adr_link: None,
            note: "later".to_string(),
            author: "sven".to_string(),
            ts: "2026-10-30T09:00:00Z".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn missing_file_reads_as_empty_register() {
        let dir = tempfile::tempdir().expect("tempdir");
        let register = load_register(dir.path()).expect("missing is empty, not an error");
        assert_eq!(register, ReviewRegister::default());
    }

    #[test]
    fn save_and_load_round_trip_in_tempdir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut register = ReviewRegister {
            version: 1,
            decisions: Vec::new(),
        };
        register.upsert(decision("d-1", "scc-3"));
        save_register(dir.path(), &register).expect("save");
        assert!(
            register_path(dir.path()).is_file(),
            "file lands in .archguard/"
        );
        assert_eq!(load_register(dir.path()).expect("load"), register);
    }

    #[test]
    fn upsert_of_same_id_replaces_instead_of_duplicating() {
        let dir = tempfile::tempdir().expect("tempdir");
        upsert_decision(dir.path(), decision("d-1", "scc-3")).expect("first record");
        let mut replacement = decision("d-1", "scc-3");
        replacement.decision = archguard_core::DecisionKind::AcceptRisk;
        let register = upsert_decision(dir.path(), replacement).expect("re-record");
        assert_eq!(register.decisions.len(), 1, "same id never duplicates");
        assert_eq!(
            register.decision_for("scc-3").expect("present").decision,
            archguard_core::DecisionKind::AcceptRisk
        );
        // And the file on disk agrees with the returned register.
        assert_eq!(load_register(dir.path()).expect("load"), register);
    }

    #[test]
    fn corrupt_file_is_an_error_with_hint_and_stays_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".archguard")).expect("dir");
        let path = register_path(dir.path());
        std::fs::write(&path, "{ not json").expect("write corrupt");
        let err = load_register(dir.path()).expect_err("corrupt must error");
        let message = err.to_string();
        assert!(
            message.contains("corrupt") || message.contains("repair"),
            "{message}"
        );
        // The load attempt must not have touched the file.
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            "{ not json"
        );
    }
}
