//! The one status vocabulary of analyses, imports and scans. Every tab
//! maps its source's status code here and shows it with the `StatusLine`
//! widget: one label per status, one colour token per status.

use crate::tab_support::Texts;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    Partial,
    Stale,
    Unavailable,
    NotImported,
    Error,
}

impl Status {
    pub const ALL: [Status; 6] = [
        Status::Ready,
        Status::Partial,
        Status::Stale,
        Status::Unavailable,
        Status::NotImported,
        Status::Error,
    ];

    /// The status of a source's status code; `None` for codes outside
    /// this vocabulary (history comparisons and the like).
    pub fn from_code(code: &str) -> Option<Status> {
        Some(match code {
            "ready" | "complete" => Status::Ready,
            "partial" | "incomplete" => Status::Partial,
            "stale" => Status::Stale,
            "unavailable" | "not_applicable" | "imports_only" => Status::Unavailable,
            "" | "not_imported" | "not_scanned" => Status::NotImported,
            "error" | "failed" => Status::Error,
            _ => return None,
        })
    }

    /// [`Status::from_code`], unknown codes read as unavailable.
    pub fn of(code: &str) -> Status {
        Status::from_code(code).unwrap_or(Status::Unavailable)
    }

    /// The `StatusLine.status` slot, which picks the colour token.
    pub fn slot(self) -> i32 {
        Status::ALL.iter().position(|&s| s == self).unwrap_or(3) as i32
    }

    /// Template key of the status word.
    pub fn label(self) -> &'static str {
        match self {
            Status::Ready => "ready",
            Status::Partial => "partial",
            Status::Stale => "stale",
            Status::Unavailable => "unavailable",
            Status::NotImported => "not imported",
            Status::Error => "error",
        }
    }

    /// `subject: status`, e.g. "Analysis: partial" (`subject` a template key).
    pub fn line(self, texts: &Texts, subject: &str) -> String {
        texts.f(
            "{subject}: {status}",
            &[
                ("subject", texts.t(subject).to_string()),
                ("status", texts.t(self.label()).to_string()),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_map_onto_one_vocabulary() {
        assert_eq!(Status::of("ready"), Status::Ready);
        assert_eq!(Status::of("partial"), Status::Partial);
        assert_eq!(Status::of("stale"), Status::Stale);
        assert_eq!(Status::of("not_scanned"), Status::NotImported);
        assert_eq!(Status::of(""), Status::NotImported);
        assert_eq!(Status::of("error"), Status::Error);
        assert_eq!(Status::of("weird"), Status::Unavailable);
        assert_eq!(Status::from_code("compared"), None);
    }

    /// Slots match `Statuses.color` in `ui/widgets.slint`.
    #[test]
    fn slots_are_stable() {
        let slots: Vec<_> = Status::ALL.iter().map(|s| s.slot()).collect();
        assert_eq!(slots, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn every_status_has_one_translated_word() {
        let de = Texts::german();
        for s in Status::ALL {
            assert_ne!(de.t(s.label()), s.label(), "{s:?}");
        }
        assert_eq!(
            Status::Ready.line(&Texts::default(), "Analysis"),
            "Analysis: ready"
        );
        assert_eq!(Status::Partial.line(&de, "Analysis"), "Analyse: teilweise");
        assert_eq!(Status::Ready.line(&de, "Import"), "Import: vollständig");
    }
}
