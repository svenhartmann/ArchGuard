//! Markdown cells, table rows and JavaScript-like value text shared by
//! the export renderers (`exportMarkdownCell` in `viewer_export.js`).

use serde_json::Value;

/// Lines of a Markdown document under construction.
pub(super) type Lines = Vec<String>;

/// Key order of `filters` and `metrics` per kind, as the web viewer
/// builds the objects (serde_json keeps keys sorted).
fn key_order(kind: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match kind {
        "services" => (
            &[
                "workspace",
                "kind",
                "direction",
                "reference",
                "selected_project",
                "cycles_only",
                "focused_cycle",
            ],
            &[
                "scope_nodes",
                "scope_dependencies",
                "shown_nodes",
                "shown_dependencies",
                "cyclic_groups",
            ],
        ),
        "architecture" => (
            &["workspace", "only_violations", "query"],
            &[
                "source_files",
                "mapped_files",
                "forbidden_directions",
                "interface_violations",
                "coupling_violations",
                "unclassified_connections",
            ],
        ),
        "security" => (
            &["workspace", "query", "severity"],
            &[
                "packages_scanned",
                "affected_packages",
                "findings",
                "targets",
                "critical",
                "high",
                "medium",
                "low",
                "unknown",
                "matching_findings",
            ],
        ),
        "quality" => (&["workspace", "section", "target", "query"], &[]),
        _ => (
            &[
                "workspace",
                "diagram",
                "reference",
                "direction",
                "show_files",
                "cycles_only",
                "search",
                "search_behavior",
            ],
            &[
                "scope_nodes",
                "scope_dependencies",
                "shown_nodes",
                "shown_dependencies",
                "cyclic_groups",
                "acd",
            ],
        ),
    }
}

/// The entries of `object` in `order`, then the remaining ones.
pub(super) fn ordered<'a>(
    object: &'a Value,
    kind: &str,
    metrics: bool,
) -> Vec<(&'a str, &'a Value)> {
    let Some(map) = object.as_object() else {
        return Vec::new();
    };
    let (filters, metric_keys) = key_order(kind);
    let order = if metrics { metric_keys } else { filters };
    let known = order.iter().filter_map(|k| map.get_key_value(*k));
    let rest = map.iter().filter(|(k, _)| !order.contains(&k.as_str()));
    known.chain(rest).map(|(k, v)| (k.as_str(), v)).collect()
}

/// `exportMarkdownCell`: escape Markdown and HTML, keep line breaks.
pub(super) fn cell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.replace("\r\n", "\n").chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("<br>"),
            '|' | '`' | '\\' | '[' | ']' | '*' | '_' | '!' | '~' | '#' => {
                out.push_str(&format!("&#{};", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// `String(value ?? '')` as JavaScript writes it.
pub(super) fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// `cell(String(value ?? ''))`.
pub(super) fn vcell(value: &Value) -> String {
    cell(&text(value))
}

/// A table row of escaped cells.
pub(super) fn row(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

/// `file:line:column · pointer` of one evidence record.
pub(super) fn location(ev: &Value) -> String {
    let mut out = text(&ev["file"]);
    for key in ["line", "column"] {
        if ev[key].as_u64().is_some_and(|n| n > 0) {
            out.push_str(&format!(":{}", text(&ev[key])));
        }
    }
    if let Some(p) = ev["pointer"].as_str().filter(|p| !p.is_empty()) {
        out.push_str(&format!(" · {p}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_escape_markdown_and_html() {
        assert_eq!(cell("a|b_c <x>\nnext"), "a&#124;b&#95;c &lt;x&gt;<br>next");
        assert_eq!(text(&serde_json::json!(2.0)), "2");
        assert_eq!(text(&serde_json::json!(2.5)), "2.5");
    }
}
