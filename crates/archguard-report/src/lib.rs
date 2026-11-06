//! `archguard-report` — L1: serialization of `archguard_core::Report` and
//! `archguard_core::Graph` into all output formats. The canonical JSON
//! format (T18) lives in [`json`], the compact plain-text format (T19) in
//! [`text`], and the graph visualizations (T20) — a mermaid `flowchart`
//! and a Graphviz `digraph` — in [`mermaid`] and [`dot`]; the dependency
//! structure matrix (T38) lives in [`dsm`]; the interchange and viewer
//! formats (T34) — GraphML, GEXF and the self-contained HTML viewer — in
//! [`graphml`], [`gexf`] and [`html`]; the architecture PR comment
//! renders in [`pr_comment`]; the remaining
//! formats land in later tasks.

mod arc42;
mod arc42_blocks;
mod arc42_goals;
mod architecture_doc;
mod dot;
mod dsm;
mod gexf;
mod graphml;
mod html;
mod html_app;
mod json;
mod mermaid;
mod pr_comment;
mod render;
mod staged;
mod text;
pub use staged::{branch_diff_html, staged_diff_html};

pub use arc42::{Arc42Blocks, Arc42Chapter, Arc42Document, Arc42Inputs, render_arc42};
pub use architecture_doc::render_architecture_doc;
pub use dot::to_dot;
pub use dsm::{to_dsm_json, to_dsm_text};
pub use gexf::to_gexf;
pub use graphml::to_graphml;
pub use html::{
    HtmlContext, HtmlPage, to_html, to_html_page, to_html_with_calls, to_html_with_context,
};
pub use json::to_json;
pub use mermaid::to_mermaid;
pub use pr_comment::render_pr_comment;
pub use text::to_text;
