//! The export document of one view, as `exportSnapshot()` returns it in
//! the web viewer: `schema_version`, `project`, `kind`, `coverage`,
//! `filters`, `metrics`, `notes`, `diagnostics` and the view's data.

use serde_json::{Map, Value, json};

/// One view's export; JSON and Markdown are two renderings of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot(pub Value);

impl Snapshot {
    /// `JSON.stringify(snapshot, null, 2)` plus a final newline.
    pub fn json(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.0).expect("JSON values serialize");
        text.push('\n');
        text
    }

    pub fn kind(&self) -> &str {
        self.0["kind"].as_str().unwrap_or_default()
    }

    /// The SVG `<desc>` of `exportSvg`: coverage and filters.
    pub fn description(&self) -> String {
        format!(
            "{} Filters: {}. Source → target arrows.",
            self.0["coverage"].as_str().unwrap_or_default(),
            serde_json::to_string(&self.0["filters"]).unwrap_or_default()
        )
    }
}

/// The fields every snapshot starts with (`base` in `exportSnapshot`).
pub(super) fn base(project: &str, workspace: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("schema_version".into(), json!(1));
    map.insert("project".into(), json!(project));
    map.insert("filters".into(), json!({ "workspace": workspace }));
    map
}

/// Add `extra` to the `filters` object of `map`, after `workspace`.
pub(super) fn add_filters(map: &mut Map<String, Value>, extra: Value) {
    if let (Some(filters), Value::Object(extra)) =
        (map.get_mut("filters").and_then(Value::as_object_mut), extra)
    {
        filters.extend(extra);
    }
}

/// Insert every field of `fields` (an object) into `map`, in order.
pub(super) fn extend(map: &mut Map<String, Value>, fields: Value) {
    if let Value::Object(fields) = fields {
        map.extend(fields);
    }
}
