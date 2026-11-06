//! Package cohesion card: `renderCohesionCard` of
//! `viewer_engineering_quality.js`. The selected package shows its groups,
//! the active group's files and its external dependencies.

use std::collections::BTreeSet;

use archguard_engine::{CohesionGroup, CohesionPackage};

use super::card::{Builder, Card, Group, Kind, Line, locate};
use super::lang::percent;
use crate::tab_support::Texts;

/// Which package and group the cohesion section inspects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub package: Option<String>,
    /// `None`: the first group in display order.
    pub group: Option<usize>,
    pub limit: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Selection {
            package: None,
            group: None,
            limit: 20,
        }
    }
}

const FILES: (&str, &str) = ("{count} file", "{count} files");

/// Group indices in display order: connected groups first, then larger.
pub fn ordered(groups: &[CohesionGroup]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&groups[a], &groups[b]);
        (b.internal_dependencies > 0)
            .cmp(&(a.internal_dependencies > 0))
            .then(b.files.len().cmp(&a.files.len()))
    });
    order
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The package's title; the project root (".") by name.
fn title(lang: &Texts, path: &str) -> String {
    match path {
        "." | "" => lang.t("(project root)").to_string(),
        _ => base_name(path).to_string(),
    }
}

pub fn card(lang: &Texts, nodes: &BTreeSet<String>, p: &CohesionPackage, sel: &Selection) -> Card {
    let mut b = Builder::new(lang, nodes, title(lang, &p.node));
    if b.card.title.is_empty() {
        b.card.title = p.node.clone();
    }
    b.card.full = p.node.clone();
    b.card.key = p.node.clone();
    let selected = sel.package.as_deref() == Some(p.node.as_str());
    b.card.pressed = selected;
    b.card.action = if selected {
        lang.t("Selected")
    } else {
        lang.t("Inspect groups")
    }
    .into();
    summary(&mut b, p);
    if selected {
        inspect(&mut b, p, sel);
    }
    b.done()
}

fn summary(b: &mut Builder, p: &CohesionPackage) {
    let lang = b.lang;
    let connected = p
        .groups
        .iter()
        .filter(|g| g.internal_dependencies > 0)
        .count();
    let isolated = p
        .groups
        .iter()
        .filter(|g| g.files.len() == 1 && g.internal_dependencies == 0)
        .count();
    let parts = [
        lang.plural(p.files, FILES.0, FILES.1),
        lang.plural(
            connected,
            "{count} connected group",
            "{count} connected groups",
        ),
        lang.plural(isolated, "{count} isolated file", "{count} isolated files"),
    ];
    b.push(Kind::Text, parts.join(" · "));
    if p.split_candidate {
        b.push(
            Kind::Verdict,
            lang.t("Review package split: check the responsibilities of these groups."),
        );
    } else if p.internal_dependencies == 0 {
        b.push(
            Kind::Muted,
            lang.t("Too few internal connections to assess a package split."),
        );
    }
}

fn inspect(b: &mut Builder, p: &CohesionPackage, sel: &Selection) {
    let lang = b.lang;
    let co_change = if p.co_change_status == "not_checked" {
        lang.t("Co-change not checked.").to_string()
    } else {
        let status = if p.co_change_status == "available" {
            lang.t("history available")
        } else {
            lang.t("incomplete history")
        };
        lang.f(
            "{count} cross-group co-change pairs · {status}",
            &[
                ("count", p.cross_group_co_changes.to_string()),
                ("status", status.into()),
            ],
        )
    };
    b.push(Kind::Muted, co_change);
    if !p.groups.is_empty() {
        let order = ordered(&p.groups);
        let active = sel.group.unwrap_or(order[0]).min(p.groups.len() - 1);
        groups(b, p, &order, active, sel.limit);
        group_detail(b, &p.groups[active], active);
    }
    context(b, p);
}

fn groups(b: &mut Builder, p: &CohesionPackage, order: &[usize], active: usize, limit: usize) {
    let lang = b.lang;
    let mut shown: Vec<usize> = order.iter().copied().take(limit).collect();
    if !shown.contains(&active) {
        shown.push(active);
    }
    b.card.groups = shown
        .into_iter()
        .map(|index| {
            let g = &p.groups[index];
            let names: Vec<&str> = g.files.iter().take(2).map(|f| base_name(f)).collect();
            let rest = g.files.len().saturating_sub(2);
            Group {
                index,
                label: format!(
                    "{} · {}",
                    lang.f("Group {number}", &[("number", (index + 1).to_string())]),
                    lang.plural(g.files.len(), FILES.0, FILES.1)
                ),
                files: names.join(", ")
                    + &if rest > 0 {
                        format!(" +{rest}")
                    } else {
                        String::new()
                    },
                active: index == active,
            }
        })
        .collect();
    b.card.more_groups = p.groups.len() > limit;
    b.push(Kind::Groups, "");
}

fn group_detail(b: &mut Builder, g: &CohesionGroup, active: usize) {
    let lang = b.lang;
    b.push(
        Kind::Heading,
        lang.f("Group {number}", &[("number", (active + 1).to_string())]),
    );
    if g.files.iter().any(|f| !b.nodes.contains(f)) {
        b.push(
            Kind::Muted,
            lang.t("Some group files are absent from the loaded source graph."),
        );
    }
    let mut files = Vec::new();
    for f in &g.files {
        files.push(Line::new(Kind::Path, f));
        files.extend(locate(lang, b.nodes, Some(f)));
    }
    b.details(
        format!("{} · {}", lang.t("All files"), g.files.len()),
        files,
    );
    let mut lines = Vec::new();
    for edge in &g.boundaries {
        lines.push(Line::new(
            Kind::Path,
            format!("{} → {}", edge.from, edge.to),
        ));
        lines.extend(
            edge.evidence
                .iter()
                .map(|r| Line::new(Kind::Pre, format!("{}:{} {}", r.file, r.line, r.raw))),
        );
    }
    if g.boundaries_truncated {
        lines.push(Line::new(
            Kind::Muted,
            lang.t("Boundary samples truncated by analysis."),
        ));
    }
    let summary = lang.f(
        "External dependencies · {incoming} in / {outgoing} out",
        &[
            ("incoming", g.incoming_dependencies.to_string()),
            ("outgoing", g.outgoing_dependencies.to_string()),
        ],
    );
    b.details(summary, lines);
}

fn context(b: &mut Builder, p: &CohesionPackage) {
    let lang = b.lang;
    b.push(Kind::Path, &p.node).push(Kind::Muted, lang.t("Direct files only; nested packages are separate. Isolation refers to connections inside this package.",
    ));
    let lines = vec![
        Line::new(
            Kind::Text,
            lang.f(
                "Internal share: {share} · internal / (internal + outgoing)",
                &[("share", percent(lang, p.internal_share.map(|s| s * 100.0)))],
            ),
        ),
        Line::new(
            Kind::Text,
            lang.f(
                "{internal} internal · {outgoing} outgoing · {incoming} incoming file dependencies",
                &[
                    ("internal", p.internal_dependencies.to_string()),
                    ("outgoing", p.outgoing_dependencies.to_string()),
                    ("incoming", p.incoming_dependencies.to_string()),
                ],
            ),
        ),
        Line::new(Kind::Text, &p.reason),
    ];
    b.details(lang.t("Measurements and interpretation"), lines);
    b.locate(Some(&p.node));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(files: &[&str], internal: usize) -> CohesionGroup {
        CohesionGroup {
            files: files.iter().map(|f| f.to_string()).collect(),
            internal_dependencies: internal,
            incoming_dependencies: 2,
            outgoing_dependencies: 3,
            boundaries: vec![archguard_engine::CohesionBoundary {
                from: files[0].into(),
                to: "other/z.rs".into(),
                evidence: vec![archguard_engine::core::Ref {
                    file: files[0].into(),
                    line: 4,
                    raw: "use other::Z;".into(),
                }],
            }],
            boundaries_truncated: false,
        }
    }

    fn package() -> CohesionPackage {
        CohesionPackage {
            node: "crate/src".into(),
            files: 4,
            internal_dependencies: 3,
            incoming_dependencies: 1,
            outgoing_dependencies: 2,
            internal_share: Some(0.6),
            groups: vec![
                group(&["crate/src/lone.rs"], 0),
                group(&["crate/src/a.rs", "crate/src/b.rs", "crate/src/c.rs"], 3),
            ],
            split_candidate: false,
            co_change_status: "available".into(),
            cross_group_co_changes: 0,
            reason: "One connected group.".into(),
        }
    }

    #[test]
    fn the_root_package_is_named_not_a_dot() {
        let nodes = BTreeSet::new();
        let root = CohesionPackage {
            node: ".".into(),
            ..package()
        };
        let card = card(&Texts::default(), &nodes, &root, &Selection::default());
        assert_eq!(card.title, "(project root)");
        assert_eq!(card.full, ".");
        let de = card_de(&root);
        assert_eq!(de.title, "(Projektwurzel)");
    }

    fn card_de(p: &CohesionPackage) -> Card {
        card(&Texts::german(), &BTreeSet::new(), p, &Selection::default())
    }

    #[test]
    fn groups_order_connected_then_larger() {
        assert_eq!(ordered(&package().groups), [1, 0]);
    }

    #[test]
    fn unselected_card_summarises_the_package() {
        let nodes = BTreeSet::new();
        let card = card(&Texts::default(), &nodes, &package(), &Selection::default());
        assert_eq!(card.title, "src");
        assert_eq!(card.full, "crate/src");
        assert_eq!(card.action, "Inspect groups");
        assert_eq!(
            card.items[0].text,
            "4 files · 1 connected group · 1 isolated file"
        );
        assert!(card.groups.is_empty());
    }

    #[test]
    fn selected_card_shows_the_active_group() {
        let nodes = BTreeSet::from(["crate/src".to_string(), "crate/src/a.rs".to_string()]);
        let sel = Selection {
            package: Some("crate/src".into()),
            ..Selection::default()
        };
        let card = card(&Texts::default(), &nodes, &package(), &sel);
        assert_eq!(card.action, "Selected");
        assert!(card.pressed);
        let labels: Vec<_> = card
            .groups
            .iter()
            .map(|g| (g.label.as_str(), g.files.as_str(), g.active))
            .collect();
        assert_eq!(
            labels,
            [
                ("Group 2 · 3 files", "a.rs, b.rs +1", true),
                ("Group 1 · 1 file", "lone.rs", false)
            ]
        );
        let texts: Vec<_> = card.items.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.contains(&"0 cross-group co-change pairs · history available"));
        assert!(texts.contains(&"All files · 3"));
        assert!(texts.contains(&"External dependencies · 2 in / 3 out"));
        let measured = card
            .items
            .iter()
            .find(|i| i.text == "Measurements and interpretation")
            .unwrap();
        assert_eq!(
            measured.lines[0].text,
            "Internal share: 60.0% · internal / (internal + outgoing)"
        );
        let files = card
            .items
            .iter()
            .find(|i| i.text == "All files · 3")
            .unwrap();
        assert_eq!(
            files
                .lines
                .iter()
                .filter(|l| l.kind == Kind::Locate)
                .count(),
            1
        );
    }
}
