//! When the app looks for an update on its own: at most once per
//! [`INTERVAL`], unless `desktop.toml` turns it off.
//!
//! ```toml
//! [updates]
//! check = true        # false: never check at start-up (Help still can)
//! last_check = 1762560000  # Unix seconds of the last check, written by the app
//! ```

use toml_edit::{DocumentMut, Item, Table, value};

/// The shortest time between two automatic checks, in seconds.
pub const INTERVAL: i64 = 24 * 60 * 60;

/// Whether `check` is not `false`; other values read as the default, on.
pub fn enabled(doc: &DocumentMut) -> bool {
    updates(doc)
        .and_then(|t| t.get("check"))
        .and_then(Item::as_bool)
        != Some(false)
}

/// Whether the start-up check runs at `now` (Unix seconds): on, and no
/// check in the last [`INTERVAL`]. A last check in the future (clock set
/// back) does not block checks.
pub fn due(doc: &DocumentMut, now: i64) -> bool {
    if !enabled(doc) {
        return false;
    }
    let last = updates(doc)
        .and_then(|t| t.get("last_check"))
        .and_then(Item::as_integer);
    last.is_none_or(|last| now - last >= INTERVAL || last > now)
}

/// Record a check at `now`; writes `check = true` once, so the setting is
/// there to turn off.
pub fn record(doc: &mut DocumentMut, now: i64) {
    // A value of another type goes, key formatting included.
    if doc.get("updates").is_some_and(|i| !i.is_table()) {
        doc.remove("updates");
    }
    let t = doc
        .entry("updates")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("a table");
    if t.get("check").and_then(Item::as_bool).is_none() {
        t.insert("check", value(true));
    }
    t.insert("last_check", value(now));
}

/// The current time in Unix seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn updates(doc: &DocumentMut) -> Option<&dyn toml_edit::TableLike> {
    doc.get("updates").and_then(Item::as_table_like)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> DocumentMut {
        text.parse().expect("toml")
    }

    #[test]
    fn a_fresh_file_is_due_and_a_recorded_check_waits_a_day() {
        let mut d = doc("");
        assert!(due(&d, 1_000_000));
        record(&mut d, 1_000_000);
        assert!(!due(&d, 1_000_000));
        assert!(!due(&d, 1_000_000 + INTERVAL - 1));
        assert!(due(&d, 1_000_000 + INTERVAL));
        // The clock went back: check instead of waiting for years.
        assert!(due(&d, 999_000));
    }

    #[test]
    fn check_false_turns_the_start_up_check_off_and_record_keeps_it() {
        let mut d = doc("# mine\n[updates]\ncheck = false\n");
        assert!(!enabled(&d));
        assert!(!due(&d, 1_000_000));
        record(&mut d, 1_000_000);
        assert!(!enabled(&d));
        let text = d.to_string();
        assert!(text.starts_with("# mine"), "{text}");
        assert!(text.contains("check = false"), "{text}");
    }

    #[test]
    fn record_writes_the_setting_once_and_wrong_types_read_as_default() {
        let mut d = doc("updates = 3\n");
        assert!(enabled(&d));
        record(&mut d, 42);
        assert_eq!(d.to_string(), "[updates]\ncheck = true\nlast_check = 42\n");
        let d = doc("[updates]\ncheck = \"no\"\nlast_check = \"yesterday\"\n");
        assert!(enabled(&d));
        assert!(due(&d, 42));
    }

    #[test]
    fn the_settings_file_keeps_updates_when_the_settings_are_written() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(crate::settings_file::FILE);
        std::fs::write(&path, "language = \"de\"\n").expect("write");
        crate::settings_file::init(Some(path.clone()));
        crate::settings_file::update_doc(|d| record(d, 7));
        crate::settings_file::update(|s| s.language = "en".into());
        let text = std::fs::read_to_string(&path).expect("written");
        assert!(text.contains("last_check = 7"), "{text}");
        assert!(crate::settings_file::get_doc(|d| !due(d, 8)));
        crate::settings_file::init(None);
    }
}
