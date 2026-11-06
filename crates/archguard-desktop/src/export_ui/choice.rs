//! Which export a view instance offers, as the web viewer's export menu:
//! Markdown and JSON for the views with an export snapshot, arc42 and PNG
//! for every view, SVG in Graph → Tree and Services only.

use crate::export::{Format, View};

/// An entry of File → Export (the `format` the Slint menu passes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Markdown,
    Json,
    Arc42,
    Svg,
    Png,
}

impl Choice {
    pub fn parse(format: &str) -> Option<Choice> {
        Some(match format.trim() {
            "markdown" | "md" => Choice::Markdown,
            "json" => Choice::Json,
            "arc42" => Choice::Arc42,
            "svg" => Choice::Svg,
            "png" => Choice::Png,
            _ => return None,
        })
    }

    pub fn format(self) -> Format {
        match self {
            Choice::Markdown | Choice::Arc42 => Format::Markdown,
            Choice::Json => Format::Json,
            Choice::Svg => Format::Svg,
            Choice::Png => Format::Png,
        }
    }
}

/// Why an entry is not offered; the window shows the matching hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// "SVG is available in Graph → Tree and Services."
    Svg,
    /// Markdown and JSON need a view with an export snapshot.
    Data,
}

/// The export view of a view kind (`panes::VIEWS`, "file").
pub fn view(kind: &str) -> Option<View> {
    Some(match kind {
        "graph" => View::Graph,
        "dsm" => View::Matrix,
        "services" => View::Services,
        "security" => View::Security,
        "quality" => View::Quality,
        "architecture" => View::Architecture,
        _ => return None,
    })
}

/// Whether a view of `kind` at Graph `display` (0 tree) offers `choice`.
pub fn check(kind: &str, display: i32, choice: Choice) -> Result<(), Hint> {
    match choice {
        Choice::Arc42 | Choice::Png => Ok(()),
        Choice::Markdown | Choice::Json if view(kind).is_some() => Ok(()),
        Choice::Markdown | Choice::Json => Err(Hint::Data),
        Choice::Svg if kind == "services" || (kind == "graph" && display == 0) => Ok(()),
        Choice::Svg => Err(Hint::Svg),
    }
}

/// The default file name: the web's download name, `arc42.md`, or
/// `archguard-<view>.png` for a view without an export snapshot.
pub fn file_name(kind: &str, choice: Choice) -> String {
    match (choice, view(kind)) {
        (Choice::Arc42, _) => crate::export::arc42_file_name(None),
        (_, Some(v)) => crate::export::file_name(v, choice.format()),
        (_, None) => format!("archguard-{kind}.{}", choice.format().extension()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As the web: data exports for the views with a snapshot, SVG only
    /// for Graph → Tree and Services, arc42 and PNG everywhere.
    #[test]
    fn each_view_offers_the_web_formats() {
        for kind in [
            "graph",
            "dsm",
            "services",
            "security",
            "quality",
            "architecture",
        ] {
            assert_eq!(check(kind, 0, Choice::Markdown), Ok(()), "{kind}");
            assert_eq!(check(kind, 0, Choice::Json), Ok(()), "{kind}");
        }
        for kind in ["start", "calls", "file"] {
            assert_eq!(check(kind, 0, Choice::Json), Err(Hint::Data), "{kind}");
        }
        assert_eq!(check("graph", 0, Choice::Svg), Ok(()));
        assert_eq!(check("graph", 1, Choice::Svg), Err(Hint::Svg), "network");
        assert_eq!(check("services", 0, Choice::Svg), Ok(()));
        assert_eq!(check("dsm", 0, Choice::Svg), Err(Hint::Svg));
        for kind in ["start", "file", "dsm", "security"] {
            assert_eq!(check(kind, 2, Choice::Png), Ok(()));
            assert_eq!(check(kind, 2, Choice::Arc42), Ok(()));
        }
        assert_eq!(Choice::parse("svg"), Some(Choice::Svg));
        assert_eq!(Choice::parse("pdf"), None);
    }

    #[test]
    fn default_names_follow_the_web() {
        assert_eq!(file_name("graph", Choice::Svg), "archguard-graph.svg");
        assert_eq!(file_name("dsm", Choice::Markdown), "archguard-dsm.md");
        assert_eq!(file_name("start", Choice::Png), "archguard-start.png");
        assert_eq!(file_name("file", Choice::Arc42), "arc42.md");
    }
}
