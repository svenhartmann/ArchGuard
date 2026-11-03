//! Decision register (R19).
//!
//! The register is the project-owned record of how the team handled
//! findings: `.archguard/reviews.json`, a serde document of
//! [`archguard_core::ReviewDecision`] entries. Writes are atomic (tmp
//! file in the same directory + rename) so a crash never leaves a
//! half-written register, and a corrupt file surfaces as an error with
//! a hint — archguard never overwrites a register it cannot read.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use archguard_core::{ArchguardError, ReviewDecision, ReviewRegister};

/// Register location relative to the project root.
pub const REGISTER_REL_PATH: &str = ".archguard/reviews.json";

/// Absolute path of the register file for `root`.
pub fn register_path(root: &Path) -> PathBuf {
    root.join(REGISTER_REL_PATH)
}

fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("review register: {message}"))
}

/// Read `.archguard/reviews.json`; a missing file means an empty
/// register, corrupt content is an error (never silently replaced).
pub fn load_register(root: &Path) -> Result<ReviewRegister, ArchguardError> {
    let path = register_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            error(format!(
                "{}: {e} — repair or move the file aside, archguard never overwrites a corrupt register",
                path.display()
            ))
        }),
        Err(io) if io.kind() == ErrorKind::NotFound => Ok(ReviewRegister::default()),
        Err(io) => Err(error(format!("reading {}: {io}", path.display()))),
    }
}

/// Write the register atomically: serialize to a tmp file next to the
/// target, then rename over it (single syscall swap on the same volume).
pub fn save_register(root: &Path, register: &ReviewRegister) -> Result<(), ArchguardError> {
    let path = register_path(root);
    let dir = path
        .parent()
        .ok_or_else(|| error("register path has no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(register)
        .map_err(|e| error(format!("serializing register: {e}")))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    use std::io::Write;
    writeln!(tmp, "{text}")?;
    tmp.flush()?;
    tmp.persist(&path)
        .map_err(|e| error(format!("renaming tmp file onto {}: {e}", path.display())))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

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
