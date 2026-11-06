//! `exportMarkdown(snapshot)` of the web viewer for the graph-like kinds
//! (imports, dependencies, services, architecture).

use serde_json::Value;

use super::cells::{Lines, cell, location, ordered, row, text, vcell};

/// `exportMermaidLabel`.
fn mermaid_label(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' | '<' | '>' | '"' | '`' | '#' | '\r' | '\n' => format!("#{};", c as u32),
            c => c.to_string(),
        })
        .collect()
}

/// The Markdown lines of a graph-like snapshot.
pub(super) fn graph_like(s: &Value) -> Lines {
    let kind = s["kind"].as_str().unwrap_or_default();
    let mut lines = vec![
        format!("# Architecture: {}", vcell(&s["project"])),
        String::new(),
        vcell(&s["coverage"]),
        String::new(),
    ];
    if let Some(status) = s["status"].as_str().filter(|t| !t.is_empty()) {
        lines.extend([
            format!("Analysis status: **{}**.", cell(status)),
            String::new(),
        ]);
    }
    lines.extend(
        [
            "## Scope and filters",
            "",
            "| Setting | Value |",
            "| --- | --- |",
        ]
        .map(String::from),
    );
    for (key, value) in ordered(&s["filters"], kind, false) {
        let shown = match value {
            Value::Object(_) | Value::Array(_) | Value::Null => value.to_string(),
            v => text(v),
        };
        lines.push(row(&[cell(&key.replace('_', " ")), cell(&shown)]));
    }
    metrics(&mut lines, s, kind);
    for note in s["notes"].as_array().into_iter().flatten() {
        lines.extend([String::new(), vcell(note)]);
    }
    if kind == "architecture" {
        super::architecture::append_model(&mut lines, &s["model"]);
    }
    if kind == "services" {
        super::services::append_projects(&mut lines, s);
    }
    mermaid(&mut lines, s);
    relationships(&mut lines, s);
    diagnostics(&mut lines, s);
    lines
}

fn metrics(lines: &mut Lines, s: &Value, kind: &str) {
    let title = |key: &str| {
        match key {
            "scope_nodes" => "Analysis scope nodes",
            "scope_dependencies" => "Analysis scope dependencies",
            "shown_nodes" => "Shown nodes",
            "shown_dependencies" => "Shown dependencies",
            "cyclic_groups" => "Cyclic groups in analysis scope",
            "acd" => "ACD in analysis scope",
            "call_sites" => "Matching static call sites",
            other => other,
        }
        .to_string()
    };
    lines.extend(["", "## Metrics", "", "| Metric | Value |", "| --- | --- |"].map(String::from));
    for (key, value) in ordered(&s["metrics"], kind, true) {
        let shown = match (key, value.as_f64()) {
            ("acd", Some(f)) => format!("{f:.2}"),
            _ => text(value),
        };
        lines.push(row(&[cell(&title(key)), cell(&shown)]));
    }
}

fn edges(s: &Value) -> &[Value] {
    s["edges"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn count(edge: &Value) -> String {
    let exact = edge["count_exact"].as_bool().unwrap_or(false);
    format!("{}{}", text(&edge["count"]), if exact { "" } else { "+" })
}

fn mermaid(lines: &mut Lines, s: &Value) {
    lines.extend(["", "## Directed graph", "", "Mermaid shows the exported relationships. SVG export preserves the displayed tree layout and colors.", "", "```mermaid", "flowchart LR"].map(String::from));
    let nodes = s["nodes"].as_array().map(Vec::as_slice).unwrap_or_default();
    let id = |name: &Value| {
        nodes
            .iter()
            .position(|n| n["id"] == *name)
            .map_or("undefined".to_string(), |i| format!("n{i}"))
    };
    for (i, node) in nodes.iter().enumerate() {
        lines.push(format!(
            "  n{i}[\"{}\"]",
            mermaid_label(&text(&node["name"]))
        ));
    }
    for edge in edges(s) {
        let kind = text(&edge["kind"]);
        let label = if ["import", "dependency", "call"].contains(&kind.as_str()) {
            count(edge)
        } else {
            kind
        };
        lines.push(format!(
            "  {} -->|\"{}\"| {}",
            id(&edge["from"]),
            mermaid_label(&label),
            id(&edge["to"])
        ));
    }
    lines.push("```".into());
}

fn relationships(lines: &mut Lines, s: &Value) {
    lines.extend(["", "## Relationships", "", &format!("Counts: {}.", vcell(&s["count_unit"])), "", "| Source | Target | Kind / resolution | Count | Current-scope cycle | Package-cycle context |", "| --- | --- | --- | ---: | --- | --- |"].map(String::from));
    for e in edges(s) {
        let kind = match e["resolution"].as_str() {
            Some(r) if !r.is_empty() => format!("{} / {r}", text(&e["kind"])),
            _ => text(&e["kind"]),
        };
        let cyclic = match e["cyclic_current_scope"].as_bool() {
            None => "not evaluated",
            Some(true) => "yes",
            Some(false) => "no",
        };
        let context = if e["package_cycle_context"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        };
        lines.push(row(&[
            vcell(&e["from"]),
            vcell(&e["to"]),
            cell(&kind),
            cell(&count(e)),
            cyclic.into(),
            context.into(),
        ]));
    }
    if edges(s).is_empty() {
        lines.extend([
            String::new(),
            "No relationships match this export scope.".into(),
        ]);
    }
    lines.extend(
        [
            "",
            "## Source evidence",
            "",
            "| Source → target | Location | Evidence |",
            "| --- | --- | --- |",
        ]
        .map(String::from),
    );
    for e in edges(s) {
        for ev in e["evidence"].as_array().into_iter().flatten() {
            lines.push(row(&[
                cell(&format!("{} → {}", text(&e["from"]), text(&e["to"]))),
                cell(&location(ev)),
                cell(&detail(ev)),
            ]));
        }
    }
}

/// `evidence.raw ?? evidence.code ?? evidence.description ?? ''`.
fn detail(ev: &Value) -> String {
    ["raw", "code", "description"]
        .iter()
        .map(|k| &ev[*k])
        .find(|v| !v.is_null())
        .map(text)
        .unwrap_or_default()
}

fn diagnostics(lines: &mut Lines, s: &Value) {
    let list = s["diagnostics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if list.is_empty() {
        return;
    }
    lines.extend(["", "## Coverage diagnostics", ""].map(String::from));
    for d in list {
        let shown = match d {
            Value::String(t) => t.clone(),
            d => ["file", "service", "message"]
                .iter()
                .map(|k| text(&d[*k]))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
        };
        lines.push(format!("- {}", cell(&shown)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_labels_escape_quotes() {
        assert_eq!(mermaid_label("a\"b#"), "a#34;b#35;");
    }
}
