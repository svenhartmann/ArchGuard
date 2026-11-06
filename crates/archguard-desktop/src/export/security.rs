//! Security snapshot and `exportSecurityMarkdown` (English texts of
//! `viewer_export.js`). The severity order, filter and totals follow
//! `viewer_security.js`, shared with the tab (`security_ui/snapshot.rs`).

use archguard_engine::{SecurityFinding, SecurityReport};
use serde_json::{Map, Value, json};

use super::cells::{Lines, cell, ordered, row, text, vcell};
use super::snapshot::{Snapshot, add_filters, base, extend};
use crate::security_ui::snapshot::{self as shared, SEVERITIES, severity_rank};

/// The Security tab's list filters (`securityUi`).
#[derive(Debug, Clone, PartialEq)]
pub struct SecurityFilters {
    pub query: String,
    /// "all" or one of critical, high, medium, low, unknown.
    pub severity: String,
}

impl Default for SecurityFilters {
    fn default() -> Self {
        SecurityFilters {
            query: String::new(),
            severity: "all".into(),
        }
    }
}

/// `filteredSecurityFindings` with the web's severity value.
fn filtered<'a>(r: &'a SecurityReport, f: &SecurityFilters) -> Vec<&'a SecurityFinding> {
    let severity = SEVERITIES
        .iter()
        .position(|s| *s == f.severity)
        .map_or(0, |i| i + 1);
    shared::filtered(r, &f.query, severity)
}

/// `securityCounts` plus `matching_findings`.
fn metrics(r: &SecurityReport, matching: usize) -> Value {
    let c = shared::counts(r);
    let mut m = Map::new();
    m.insert("packages_scanned".into(), json!(c.packages_scanned));
    m.insert("affected_packages".into(), json!(c.affected_packages));
    m.insert("findings".into(), json!(c.findings));
    m.insert("targets".into(), json!(c.targets));
    for (s, n) in SEVERITIES.iter().zip(c.by_severity) {
        m.insert((*s).into(), json!(n));
    }
    m.insert("matching_findings".into(), json!(matching));
    Value::Object(m)
}

/// The snapshot of the Security tab: every matching finding, not only
/// the shown page.
pub fn security_snapshot(project: &str, r: &SecurityReport, f: &SecurityFilters) -> Snapshot {
    let findings = filtered(r, f);
    let mut map = base(project, "security");
    extend(
        &mut map,
        json!({
            "kind": "security",
            "status": r.status,
            "coverage": "Original dependency scanner snapshot. All matching findings are exported, including additional list pages. Dependency-graph simulation does not change security findings.",
        }),
    );
    add_filters(&mut map, json!({"query": f.query, "severity": f.severity}));
    extend(
        &mut map,
        json!({
            "snapshot": r,
            "findings": findings,
            "metrics": metrics(r, findings.len()),
            "notes": [
                "Packages are distinct ecosystem, name and installed-version combinations; a package can have several findings. Per-target package totals can overlap.",
                "Missing, failed, partial, outdated or zero-package scans do not establish dependency safety. Scanner advisories do not measure runtime exploitability. Fixed-version strings are scanner evidence, not automatic changes.",
            ],
            "diagnostics": r.diagnostics,
        }),
    );
    Snapshot(Value::Object(map))
}

fn status_text(status: &str) -> &'static str {
    match status {
        "not_scanned" => "Not scanned — run the CLI command to create a dependency scan.",
        "ready" => "Scan completed",
        "partial" => "Partial scan — findings do not cover all dependencies.",
        "stale" => "Outdated snapshot — dependency inputs have changed; run a fresh scan.",
        "error" => "Scan failed — dependency coverage is unknown.",
        _ => "Unknown scan status",
    }
}

/// `safeSecurityUrl`: only absolute http(s) URLs.
fn safe_url(value: &Value) -> Option<String> {
    let url = value.as_str()?;
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let clean = !url.chars().any(|c| c.is_whitespace() || c.is_control());
    (!host.is_empty() && clean).then(|| {
        url.replace('<', "%3C")
            .replace('>', "%3E")
            .replace('|', "%7C")
    })
}

/// `exportSecurityMarkdown`.
pub(super) fn markdown(s: &Value) -> Lines {
    let snap = &s["snapshot"];
    let mut lines: Lines = vec![
        format!("# Dependency security: {}", vcell(&s["project"])),
        String::new(),
        cell(
            "Original dependency scan; graph simulations do not change these findings. This report includes every matching finding, beyond the displayed page.",
        ),
        String::new(),
        cell(status_text(snap["status"].as_str().unwrap_or_default())),
        String::new(),
    ];
    lines.extend(
        [
            "## Scan and coverage",
            "",
            "| Field | Value |",
            "| --- | --- |",
        ]
        .map(String::from),
    );
    let or_unknown = |v: &Value| {
        if v.is_null() || v == "" {
            "Not reported".to_string()
        } else {
            text(v)
        }
    };
    let m = &s["metrics"];
    let fields = [
        ("Status", text(&snap["status"])),
        ("Scanner", text(&snap["scanner"])),
        ("Scanner version", or_unknown(&snap["scanner_version"])),
        ("Scanned at", or_unknown(&snap["scanned_at"])),
        (
            "Database updated at",
            or_unknown(&snap["database_updated_at"]),
        ),
        ("Input fingerprint", or_unknown(&snap["fingerprint"])),
        ("Packages scanned", text(&m["packages_scanned"])),
        ("Affected packages", text(&m["affected_packages"])),
        ("Findings in snapshot", text(&m["findings"])),
        (
            "Matching findings",
            s["findings"].as_array().map_or(0, Vec::len).to_string(),
        ),
    ];
    for (label, value) in fields {
        lines.push(row(&[cell(label), cell(&value)]));
    }
    lines.extend([String::new(), cell("Packages are distinct ecosystem, name and installed-version combinations. Per-target counts can overlap. Missing, failed, incomplete, outdated or zero-package scans do not establish dependency safety."), String::new()]);
    lines.extend(["## Filters", "", "| Setting | Value |", "| --- | --- |"].map(String::from));
    for (key, value) in ordered(&s["filters"], "security", false) {
        lines.push(row(&[cell(key), vcell(value)]));
    }
    lines.extend(
        [
            "",
            "## Manifest / lockfile targets",
            "",
            "| Path | Ecosystem | Packages |",
            "| --- | --- | ---: |",
        ]
        .map(String::from),
    );
    for t in snap["targets"].as_array().into_iter().flatten() {
        lines.push(row(&["path", "ecosystem", "packages"].map(|k| vcell(&t[k]))));
    }
    findings(&mut lines, s);
    lines
}

fn findings(lines: &mut Lines, s: &Value) {
    lines.extend(["", "## Findings", "", "| Advisory | Package | Ecosystem | Installed | Fixed versions | Severity | Title | Evidence |", "| --- | --- | --- | --- | --- | --- | --- | --- |"].map(String::from));
    let list = s["findings"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for f in list {
        let advisory = match safe_url(&f["url"]) {
            Some(url) => format!("{} · <{url}>", vcell(&f["id"])),
            None => vcell(&f["id"]),
        };
        let fixed = f["fixed_version"]
            .as_str()
            .filter(|v| !v.is_empty())
            .unwrap_or("No fixed version reported");
        let severity = ["Critical", "High", "Medium", "Low", "Unknown"]
            [severity_rank(f["severity"].as_str().unwrap_or_default())];
        let targets: Vec<_> = f["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(vcell)
            .collect();
        lines.push(row(&[
            advisory,
            vcell(&f["package"]),
            vcell(&f["ecosystem"]),
            vcell(&f["installed_version"]),
            cell(fixed),
            cell(severity),
            vcell(&f["title"]),
            targets.join("<br>"),
        ]));
    }
    if list.is_empty() {
        lines.extend([String::new(), cell("No matching findings are recorded in this export. See the scan status and coverage above; this is not a safety guarantee.")]);
    }
    let diagnostics = s["diagnostics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !diagnostics.is_empty() {
        lines.extend(["", "## Scanner diagnostics (original text)", ""].map(String::from));
        lines.extend(diagnostics.iter().map(|d| format!("- {}", vcell(d))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic scan as the Security tab's tests use it.
    fn report() -> SecurityReport {
        serde_json::from_value(json!({"schema_version":1,"status":"ready","scanner":"trivy","scanner_version":"0.76.0","scanned_at":"2026-10-27T09:00:00Z","database_updated_at":null,"fingerprint":"abc","packages_scanned":2,
            "targets":[{"path":"Cargo.lock","ecosystem":"cargo","packages":2}],
            "findings":[
                {"id":"CVE-2026-1","package":"demo-dep","installed_version":"1.0.0","fixed_version":"1.0.1","ecosystem":"cargo","severity":"high","title":"Synthetic | advisory","url":"https://example.invalid/a","targets":["Cargo.lock"]},
                {"id":"CVE-2026-2","package":"other","installed_version":"2.0.0","fixed_version":null,"ecosystem":"cargo","severity":"low","title":"Second","url":"javascript:alert(1)","targets":["Cargo.lock"]}],
            "diagnostics":["note from trivy"]})).expect("report")
    }

    #[test]
    fn security_json_exports_every_matching_finding() {
        let f = SecurityFilters {
            severity: "high".into(),
            ..Default::default()
        };
        let v = security_snapshot("demo", &report(), &f).0;
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["kind"], "security");
        assert_eq!(v["filters"]["severity"], "high");
        assert_eq!(v["findings"].as_array().unwrap().len(), 1);
        assert_eq!(v["metrics"]["findings"], 2);
        assert_eq!(v["metrics"]["matching_findings"], 1);
        assert_eq!(v["metrics"]["high"], 1);
        assert_eq!(v["metrics"]["affected_packages"], 2);
        assert_eq!(v["diagnostics"][0], "note from trivy");
    }

    #[test]
    fn security_markdown_escapes_and_drops_unsafe_links() {
        let md = security_snapshot("demo", &report(), &SecurityFilters::default()).markdown();
        assert!(md.starts_with("# Dependency security: demo\n"), "{md}");
        assert!(md.contains("| Matching findings | 2 |"), "{md}");
        assert!(
            md.contains("CVE-2026-1 · <https://example.invalid/a>"),
            "{md}"
        );
        assert!(md.contains("Synthetic &#124; advisory"), "{md}");
        assert!(!md.contains("javascript:"), "{md}");
        assert!(md.contains("No fixed version reported"), "{md}");
        assert!(
            md.contains("## Scanner diagnostics (original text)\n\n- note from trivy"),
            "{md}"
        );
    }
}
