//! The arc42 document as data, for `doc --arc42 --format json`,
//! `archguard diagram` and the MCP: the chapters as the project's text
//! has them and the diagram catalogue with what that text does with each
//! diagram, by the rules the desktop shows.

use std::collections::BTreeMap;

use archguard_report::Arc42Document;
use archguard_report::arc42_file::in_text;
use serde_json::{Value, json};

use crate::arc42_file::{Arc42Files, Target};

/// Each diagram of `doc` (of chapter `only`) with `chosen` (named in
/// `[arc42] shown_diagrams`) and `in_text` (in its chapter as
/// [`Arc42Files::chapter`] reads it).
pub fn catalogue(files: &Arc42Files, doc: &Arc42Document, only: Option<u8>) -> Vec<Value> {
    let mut chapters = BTreeMap::new();
    doc.diagrams
        .iter()
        .filter(|d| only.is_none_or(|n| n == d.chapter))
        .map(|d| {
            let text = chapters
                .entry(d.chapter)
                .or_insert_with(|| files.chapter(doc, d.chapter));
            let mut v = serde_json::to_value(d).expect("diagrams serialize");
            v["chosen"] = doc.chosen.contains(&d.id).into();
            v["in_text"] = in_text(text, d).into();
            v
        })
        .collect()
}

/// The document: where its text lives, the chapters (the file holding
/// each, `null` when it is only generated) and the catalogue.
pub fn document(files: &Arc42Files, doc: &Arc42Document, only: Option<u8>) -> Value {
    let (kind, path) = match &files.target {
        Target::File(p) => ("file", p),
        Target::Folder(p) => ("folder", p),
    };
    let chapters: Vec<Value> = doc
        .chapters
        .iter()
        .filter(|c| only.is_none_or(|n| n == c.number))
        .map(|c| {
            let file = files.has_chapter(c.number).then(|| files.path_of(c.number));
            json!({"number":c.number,"title":c.title,"file":file,"markdown":files.chapter(doc, c.number)})
        })
        .collect();
    json!({
        "schema_version": doc.schema_version,
        "target": {"kind": kind, "path": path, "exists": files.exists()},
        "chapters": chapters,
        "diagrams": catalogue(files, doc, only),
    })
}
