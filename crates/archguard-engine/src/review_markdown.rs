//! The architecture review ([`ReviewReport`]) as Markdown, as the web
//! viewer's assessment export (`assessmentMarkdown`). Pure: the desktop's
//! review dialog writes it, and `archguard review` could print it.

use serde_json::Value;

use crate::review_session::ReviewReport;

/// `value` safe in a Markdown table cell or paragraph: Markdown and HTML
/// syntax characters as entities, line breaks as `<br>`.
pub fn markdown_cell(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.replace("\r\n", "\n").chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '|' | '`' | '\\' | '[' | ']' | '*' | '_' | '!' | '~' | '#' => {
                out.push_str(&format!("&#{};", c as u32));
            }
            '\n' => out.push_str("<br>"),
            c => out.push(c),
        }
    }
    out
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn strings(v: &Value) -> impl Iterator<Item = &str> {
    v.as_array().into_iter().flatten().map(text)
}

/// Report `r` as a Markdown document: findings with their recorded
/// decisions, evidence gaps, impact and scope.
pub fn review_markdown(r: &ReviewReport) -> String {
    let cell = |s: &str| markdown_cell(s);
    let mut lines = vec![
        "# Architecture review".to_string(),
        String::new(),
        cell(&r.status),
        String::new(),
        format!("Revision: {}", cell(&r.revision)),
    ];
    if let Some(base) = &r.base_ref {
        lines.push(format!("Compared with: {}", cell(base)));
    }
    lines.extend([
        String::new(),
        "## Findings".into(),
        String::new(),
        "| Category | Severity | Change | Disposition | Finding | Owner | Reason | Expiry (UTC) |"
            .into(),
        "| --- | --- | --- | --- | --- | --- | --- | --- |".into(),
    ]);
    for f in &r.findings {
        let d = f.decision.as_ref();
        let cells = [
            f.category.as_str(),
            &f.severity,
            &f.change,
            &f.disposition,
            &f.title,
            d.map_or("", |d| &d.author),
            d.map_or("", |d| &d.note),
            d.and_then(|d| d.expires_on.as_deref()).unwrap_or(""),
        ];
        let row: Vec<String> = cells.iter().map(|c| cell(c)).collect();
        lines.push(format!("| {} |", row.join(" | ")));
    }
    lines.extend([String::new(), "## Evidence gaps".into(), String::new()]);
    for g in &r.gaps {
        let parts = [&g["source"], &g["status"], &g["reason"]].map(|v| cell(text(v)));
        lines.push(format!("- {}", parts.join(" · ")));
    }
    lines.extend([String::new(), "## Impact".into(), String::new()]);
    let impact = &r.impact;
    let named = strings(&impact["affected_components"]).chain(strings(&impact["affected_files"]));
    lines.extend(named.map(|s| format!("- {}", cell(s))));
    for t in impact["tests"]["selected"].as_array().into_iter().flatten() {
        let test = format!(
            "{} / {} · {}",
            text(&t["suite"]),
            text(&t["name"]),
            text(&t["outcome"])
        );
        lines.push(format!("- {}", cell(&test)));
    }
    lines.extend([String::new(), "## Scope".into(), String::new()]);
    lines.extend(r.limitations.iter().map(|l| format!("- {}", cell(l))));
    lines.extend([
        String::new(),
        "Draft decisions are unrecorded. JSON export contains the complete evidence and decision document.".into(),
        String::new(),
    ]);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_escape_markdown_html_and_line_breaks() {
        assert_eq!(
            markdown_cell("a|b <i> *x*\r\ny"),
            "a&#124;b &lt;i&gt; &#42;x&#42;<br>y"
        );
        assert_eq!(markdown_cell("plain text, ok."), "plain text, ok.");
    }

    /// A cyclic TypeScript project: its finding, the Markdown sections and
    /// a hostile title escaped.
    #[test]
    fn a_generated_review_renders_every_section() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        for (file, source) in [("a.ts", "import './b';\n"), ("b.ts", "import './a';\n")] {
            std::fs::write(root.path().join("src").join(file), source).unwrap();
        }
        std::fs::write(root.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='file'\nmax_scc_size=100\n").unwrap();
        let (mut report, _) =
            crate::review_session::generate_review(root.path(), None, None, 1_793_232_000).unwrap();
        assert!(!report.findings.is_empty());
        report.findings[0].title = "Cycle | <b>".into();
        let md = review_markdown(&report);
        for heading in [
            "# Architecture review",
            "## Findings",
            "## Evidence gaps",
            "## Impact",
            "## Scope",
        ] {
            assert!(
                md.contains(&format!("\n{heading}\n")) || md.starts_with(heading),
                "{heading}"
            );
        }
        assert!(md.contains(&format!("Revision: {}", report.revision)));
        assert!(md.contains("| Cycle &#124; &lt;b&gt; |"), "{md}");
        let rows = md
            .lines()
            .filter(|l| l.starts_with("| ") && !l.starts_with("| ---"))
            .count();
        assert_eq!(
            rows,
            report.findings.len() + 1,
            "header plus one row per finding"
        );
        assert!(
            md.contains("- src/a.ts") || md.contains("- a.ts"),
            "affected files: {md}"
        );
    }
}
