//! Export of the views, as the web viewer's "Share & export" menu
//! (`viewer_export.js`): JSON data with evidence, Markdown with a Mermaid
//! graph, the arc42 document, an SVG diagram, a PNG image and the file
//! that holds them. Independent of Slint and of the views' state: every
//! function takes engine reports or the plain input structs below, so the
//! views can change without changing the export.
//!
//! ```ignore
//! let snapshot = export::graph_snapshot(&input);          // or services_snapshot, …
//! let name = export::file_name(View::Graph, Format::Markdown); // "archguard-graph.md"
//! export::write(&dir.join(name), snapshot.markdown().as_bytes())?;
//! ```
//!
//! The data a view shows maps onto the inputs as follows (by the
//! integrator, in the view's `*_ui.rs`):
//!
//! - Graph tab ([`GraphInput`]): one [`GraphNode`] per drawn row of the
//!   `TreeView` (`tree.nodes[row.node].name`, `kind` "file" or "module" from
//!   `TreeNode::file`), one [`GraphEdge`] per shown link (`view.shown(..)`:
//!   `weight` → `count`, `cyclic`, `refs` → `evidence`). The scope figures
//!   are those of the KPI row: `scope_rows().len()`, `links.len()`,
//!   `cycle_groups`, `acd(..)`. For the SVG see [`svg`].
//! - Matrix tab: the same input with `matrix: true` and every scope row
//!   and link (the web exports the complete matrix scope).
//! - Services tab: the document's `ServiceTopology` and [`ServiceFilters`].
//! - Security tab: the saved `SecurityReport` and [`SecurityFilters`].
//! - Quality tab: the reports of `quality_ui::inputs::Inputs` ([`QualityInput`]).
//! - Architecture tab: `archguard_engine::analyze_target_architecture`
//!   ([`architecture_snapshot`]) and the arc42 document ([`arc42`]).

mod architecture;
mod cells;
mod file;
mod graph;
mod image;
mod markdown;
mod quality;
mod security;
mod services;
mod snapshot;
mod svg;

pub use architecture::architecture_snapshot;
pub use file::write;
pub use graph::{GraphEdge, GraphFilters, GraphInput, GraphNode, graph_snapshot};
pub use image::{Rect, png};
pub use quality::{QualityFilters, QualityInput, quality_snapshot};
pub use security::{SecurityFilters, security_snapshot};
pub use services::{ServiceFilters, services_snapshot};
pub use snapshot::Snapshot;
pub use svg::{Diagram, DiagramBox, DiagramEdge, EdgeKind, Theme, svg};

use std::path::Path;

use archguard_engine::AnalysisRun;

impl Snapshot {
    /// `exportMarkdown(snapshot)`: by kind, as the web viewer. It lives
    /// here because it dispatches to every view's renderer.
    pub fn markdown(&self) -> String {
        let lines = match self.kind() {
            "security" => security::markdown(&self.0),
            "quality" => quality::markdown(&self.0),
            _ => markdown::graph_like(&self.0),
        };
        lines.join("\n") + "\n"
    }
}

/// A view that can be exported; its name is the web viewer's workspace id
/// (`ui.workspace`), which the file name and the `workspace` filter use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Graph,
    Matrix,
    Services,
    Security,
    Quality,
    Architecture,
}

impl View {
    pub fn workspace(self) -> &'static str {
        match self {
            View::Graph => "graph",
            View::Matrix => "dsm",
            View::Services => "services",
            View::Security => "security",
            View::Quality => "quality",
            View::Architecture => "architecture",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Json,
    Svg,
    Png,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Markdown => "md",
            Format::Json => "json",
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }
}

/// `archguard-<workspace>.<ext>`, the web viewer's download name.
pub fn file_name(view: View, format: Format) -> String {
    format!("archguard-{}.{}", view.workspace(), format.extension())
}

/// The arc42 file name: `arc42.md`, or `arc42-05.md` for one chapter.
pub fn arc42_file_name(chapter: Option<u8>) -> String {
    match chapter {
        Some(n) => format!("arc42-{n:02}.md"),
        None => "arc42.md".into(),
    }
}

/// The arc42 document as `archguard doc --arc42 --stdout [--chapter N]`
/// prints it: the same engine call on the same analysis run.
pub fn arc42(root: &Path, run: &AnalysisRun, chapter: Option<u8>) -> Result<String, String> {
    archguard_engine::arc42_document(root, &run.config, &run.graph, &run.report)
        .map(|doc| doc.markdown(chapter))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_follow_the_web_viewer() {
        assert_eq!(
            file_name(View::Graph, Format::Markdown),
            "archguard-graph.md"
        );
        assert_eq!(file_name(View::Matrix, Format::Json), "archguard-dsm.json");
        assert_eq!(
            file_name(View::Services, Format::Svg),
            "archguard-services.svg"
        );
        assert_eq!(
            file_name(View::Quality, Format::Png),
            "archguard-quality.png"
        );
        assert_eq!(arc42_file_name(None), "arc42.md");
        assert_eq!(arc42_file_name(Some(5)), "arc42-05.md");
    }

    /// The same text as the CLI: `arc42_document(..).markdown(None)` on
    /// the run of the repo's own fixture.
    #[test]
    fn arc42_matches_the_cli_document() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts-cyclic");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let text = arc42(&root, &run, None).expect("arc42");
        let cli = archguard_engine::arc42_document(&root, &run.config, &run.graph, &run.report)
            .expect("document")
            .markdown(None);
        assert_eq!(text, cli);
        assert!(text.contains("arc42"), "{text}");
        let chapter = arc42(&root, &run, Some(5)).expect("chapter 5");
        assert!(chapter.len() < text.len());
    }
}
