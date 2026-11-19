//! `archguard-report` — L1: serialization of `archguard_core::Report` and
//! `archguard_core::Graph` into all output formats. The canonical JSON
//! format (T18) lives in [`json`], the compact plain-text format (T19) in
//! [`text`], and the graph visualizations (T20) — a mermaid `flowchart`
//! and a Graphviz `digraph` — in [`mermaid`] and [`dot`]; the dependency
//! structure matrix (T38) lives in [`dsm`]; the interchange
//! formats (T34) — GraphML and GEXF — in [`graphml`] and [`gexf`]; the
//! architecture PR comment
//! renders in [`pr_comment`]; the remaining
//! formats land in later tasks.

mod arc42;
mod arc42_blocks;
mod arc42_chapters;
mod arc42_diagrams;
mod arc42_errors;
pub mod arc42_file;
mod arc42_goals;
mod arc42_quality;
mod arc42_text;
mod architecture_doc;
pub mod ascii;
mod asciidoc;
mod docx;
mod dot;
mod dsm;
mod gexf;
mod graphml;
mod json;
mod mermaid;
pub mod package_map;
mod pr_comment;
mod render;
mod share_report;
mod structurizr;
mod text;

pub use arc42::{
    Arc42Blocks, Arc42Chapter, Arc42Context, Arc42Document, Arc42Inputs, render_arc42,
};
pub use arc42_diagrams::{
    class_sequence, mermaid as diagram_mermaid, place as diagram_place,
    plantuml as diagram_plantuml, section as diagram_section, strip as diagram_strip,
};
pub use architecture_doc::render_architecture_doc;
pub use asciidoc::markdown_to_asciidoc;
pub use docx::markdown_to_docx;
pub use dot::to_dot;
pub use dsm::{to_dsm_json, to_dsm_text};
pub use gexf::to_gexf;
pub use graphml::to_graphml;
pub use json::to_json;
pub use mermaid::to_mermaid;
pub use pr_comment::render_pr_comment;
pub use share_report::{ShareReport, render_share_report};
pub use structurizr::{C4Components, to_structurizr};
pub use text::to_text;
