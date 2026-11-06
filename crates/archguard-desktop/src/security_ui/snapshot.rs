//! The web viewer's security snapshot logic (viewer_security.js): severity
//! order, totals, filters and the empty-state choice. Pure functions over
//! the saved [`SecurityReport`].

use std::collections::BTreeSet;

use archguard_engine::{SecurityFinding, SecurityReport};

/// `SECURITY_SEVERITIES`, in display and sort order.
pub(crate) const SEVERITIES: [&str; 5] = ["critical", "high", "medium", "low", "unknown"];

/// Position in [`SEVERITIES`]; unrecognized values count as "unknown".
pub(crate) fn severity_rank(value: &str) -> usize {
    SEVERITIES
        .iter()
        .position(|s| *s == value)
        .unwrap_or(SEVERITIES.len() - 1)
}

/// `securityCounts`: original snapshot totals, before display filters.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub packages_scanned: usize,
    pub affected_packages: usize,
    pub findings: usize,
    pub targets: usize,
    pub by_severity: [usize; 5],
}

pub(crate) fn counts(report: &SecurityReport) -> Counts {
    let mut by_severity = [0; 5];
    for f in &report.findings {
        by_severity[severity_rank(&f.severity)] += 1;
    }
    let affected: BTreeSet<_> = report
        .findings
        .iter()
        .map(|f| (&f.ecosystem, &f.package, &f.installed_version))
        .collect();
    Counts {
        packages_scanned: report.packages_scanned,
        affected_packages: affected.len(),
        findings: report.findings.len(),
        targets: report.targets.len(),
        by_severity,
    }
}

/// Totals are shown only for a usable scan (`available` in `renderSecurity`).
pub(crate) fn available(report: &SecurityReport, counts: &Counts) -> bool {
    !matches!(report.status.as_str(), "not_scanned" | "error") && counts.packages_scanned > 0
}

/// `filteredSecurityFindings`: `severity` 0 is "all", else 1 + rank.
pub(crate) fn filtered<'a>(
    report: &'a SecurityReport,
    query: &str,
    severity: usize,
) -> Vec<&'a SecurityFinding> {
    let query = query.trim().to_lowercase();
    let key = |f: &'a SecurityFinding| {
        (
            severity_rank(&f.severity),
            &f.package,
            &f.id,
            &f.installed_version,
        )
    };
    let mut rows: Vec<_> = report
        .findings
        .iter()
        .filter(|f| severity == 0 || severity_rank(&f.severity) + 1 == severity)
        .filter(|f| {
            f.package.to_lowercase().contains(&query) || f.id.to_lowercase().contains(&query)
        })
        .collect();
    rows.sort_by(|a, b| key(a).cmp(&key(b)));
    rows
}

/// Which text the empty findings list shows (`renderSecurity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Empty {
    /// The list has findings.
    None = 0,
    Filtered = 1,
    NotScanned = 2,
    Error = 3,
    NoPackages = 4,
    Clean = 5,
    Incomplete = 6,
}

pub(crate) fn empty(report: &SecurityReport, counts: &Counts, matching: usize) -> Empty {
    match report.status.as_str() {
        _ if matching > 0 => Empty::None,
        _ if counts.findings > 0 => Empty::Filtered,
        "not_scanned" => Empty::NotScanned,
        "error" => Empty::Error,
        _ if counts.packages_scanned == 0 => Empty::NoPackages,
        "ready" => Empty::Clean,
        _ => Empty::Incomplete,
    }
}

/// `safeSecurityUrl`: only absolute http(s) links open.
pub(crate) fn safe_url(value: Option<&str>) -> Option<&str> {
    let value = value?;
    let lower = value.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let clean = !value.chars().any(|c| c.is_whitespace() || c.is_control());
    (!host.is_empty() && clean).then_some(value)
}

/// `securityScanCommand`: the root in POSIX single quotes.
pub(crate) fn scan_command(root: &str) -> String {
    format!("archguard security -- '{}'", root.replace('\'', "'\\''"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use archguard_engine::SecurityTarget;

    pub(crate) fn finding(
        id: &str,
        package: &str,
        version: &str,
        severity: &str,
    ) -> SecurityFinding {
        SecurityFinding {
            id: id.into(),
            package: package.into(),
            installed_version: version.into(),
            fixed_version: None,
            ecosystem: "cargo".into(),
            severity: severity.into(),
            title: format!("{id} title"),
            url: Some(format!("https://example.invalid/{id}")),
            targets: vec!["Cargo.lock".into()],
        }
    }

    pub(crate) fn report(status: &str, findings: Vec<SecurityFinding>) -> SecurityReport {
        SecurityReport {
            schema_version: 1,
            status: status.into(),
            scanner: "trivy".into(),
            scanner_version: Some("0.76.0".into()),
            scanned_at: Some("2026-10-27T09:00:00Z".into()),
            database_updated_at: None,
            fingerprint: "0".repeat(64),
            packages_scanned: 7,
            targets: vec![
                SecurityTarget {
                    path: "Cargo.lock".into(),
                    ecosystem: "cargo".into(),
                    packages: 5,
                },
                SecurityTarget {
                    path: "web/package-lock.json".into(),
                    ecosystem: "npm".into(),
                    packages: 2,
                },
            ],
            findings,
            diagnostics: Vec::new(),
        }
    }

    fn sample() -> SecurityReport {
        report(
            "ready",
            vec![
                finding("CVE-2", "zeta", "1.0.0", "low"),
                finding("CVE-1", "alpha", "1.0.0", "high"),
                finding("GHSA-x", "alpha", "1.0.0", "critical"),
                finding("CVE-3", "alpha", "2.0.0", "HIGH"),
                finding("CVE-4", "Beta", "0.1.0", "high"),
            ],
        )
    }

    #[test]
    fn severities_are_exact_lowercase_names_else_unknown() {
        assert_eq!(severity_rank("critical"), 0);
        assert_eq!(severity_rank("low"), 3);
        assert_eq!(severity_rank("HIGH"), 4);
        assert_eq!(severity_rank(""), 4);
    }

    #[test]
    fn counts_distinct_affected_packages_and_severities() {
        let c = counts(&sample());
        assert_eq!(
            c,
            Counts {
                packages_scanned: 7,
                affected_packages: 4,
                findings: 5,
                targets: 2,
                by_severity: [1, 2, 0, 1, 1],
            }
        );
        assert!(available(&sample(), &c));
        let mut r = sample();
        r.status = "error".into();
        assert!(!available(&r, &counts(&r)));
        r.status = "stale".into();
        r.packages_scanned = 0;
        assert!(!available(&r, &counts(&r)));
    }

    #[test]
    fn filter_sorts_by_severity_then_package_then_id() {
        let r = sample();
        let ids =
            |rows: Vec<&SecurityFinding>| rows.iter().map(|f| f.id.clone()).collect::<Vec<_>>();
        assert_eq!(
            ids(filtered(&r, "", 0)),
            ["GHSA-x", "CVE-4", "CVE-1", "CVE-2", "CVE-3"]
        );
        assert_eq!(
            ids(filtered(&r, "  ALPHA ", 0)),
            ["GHSA-x", "CVE-1", "CVE-3"]
        );
        assert_eq!(ids(filtered(&r, "ghsa", 0)), ["GHSA-x"]);
        assert_eq!(ids(filtered(&r, "", 2)), ["CVE-4", "CVE-1"]);
        assert_eq!(ids(filtered(&r, "", 5)), ["CVE-3"]);
    }

    #[test]
    fn empty_state_follows_the_web_order() {
        let r = sample();
        assert_eq!(empty(&r, &counts(&r), 5), Empty::None);
        assert_eq!(empty(&r, &counts(&r), 0), Empty::Filtered);
        let mut r = report("not_scanned", Vec::new());
        r.packages_scanned = 0;
        assert_eq!(empty(&r, &counts(&r), 0), Empty::NotScanned);
        r.status = "error".into();
        assert_eq!(empty(&r, &counts(&r), 0), Empty::Error);
        r.status = "ready".into();
        assert_eq!(empty(&r, &counts(&r), 0), Empty::NoPackages);
        r.packages_scanned = 3;
        assert_eq!(empty(&r, &counts(&r), 0), Empty::Clean);
        r.status = "stale".into();
        assert_eq!(empty(&r, &counts(&r), 0), Empty::Incomplete);
    }

    #[test]
    fn only_http_links_and_quoted_roots() {
        assert_eq!(safe_url(Some("https://a.b/x")), Some("https://a.b/x"));
        assert_eq!(safe_url(Some("HTTP://a.b")), Some("HTTP://a.b"));
        assert_eq!(safe_url(Some("javascript:alert(1)")), None);
        assert_eq!(safe_url(Some("https://")), None);
        assert_eq!(safe_url(Some("https://a b")), None);
        assert_eq!(safe_url(None), None);
        assert_eq!(
            scan_command("/p/it's"),
            "archguard security -- '/p/it'\\''s'"
        );
    }
}
