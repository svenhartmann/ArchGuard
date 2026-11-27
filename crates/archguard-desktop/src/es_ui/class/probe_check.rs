//! Corpus probe, ticket 13: counts only, no names. Run: see spec.md `## Abnahme-Probe`.
use super::boxes::content;
use super::diagram::diagram;
use super::layout::layout;
use super::list::{GroupTitle, list, members};
use super::types::{Group, Key, Switches};
use archguard_engine::{analyze_services, search_graph};
use std::path::Path;

fn median(v: &mut [usize]) -> usize {
    v.sort_unstable();
    v.get(v.len() / 2).copied().unwrap_or(0)
}

#[test]
#[ignore = "reads the corpus named by ARCHGUARD_ES_PROBE"]
fn probe() {
    let Ok(root) = std::env::var("ARCHGUARD_ES_PROBE") else {
        return;
    };
    let t = analyze_services(Path::new(&root)).unwrap();
    let g = search_graph(&t);
    let sw = Switches::default();
    let groups = list(&t, &g, "", Group::All);
    for gr in &groups {
        let kind = match &gr.title {
            GroupTitle::Indices(_) => "indices",
            other => &format!("{other:?}").to_lowercase(),
        };
        println!("list group {kind}: {} rows", gr.rows.len());
    }
    let flat = members(&groups);
    let (mut boxes, mut edges) = (Vec::new(), Vec::new());
    let mut best = (0, None);
    for gr in &groups {
        if !matches!(gr.title, GroupTitle::Indices(_)) {
            continue;
        }
        for r in &gr.rows {
            let d = diagram(&t, &g, &r.key, None, &sw);
            boxes.push(d.boxes.len());
            edges.push(d.edges.len());
            let parts = d
                .boxes
                .iter()
                .filter(|b| matches!(b.key, Key::Part { .. }))
                .count();
            if parts > best.0 {
                best = (parts, flat.iter().position(|k| *k == r.key));
            }
        }
    }
    let mm = |v: &mut Vec<usize>| {
        let (lo, hi) = (*v.iter().min().unwrap_or(&0), *v.iter().max().unwrap_or(&0));
        format!("{lo}/{}/{hi}", median(v))
    };
    println!(
        "index neighbourhood boxes min/median/max: {}",
        mm(&mut boxes)
    );
    println!(
        "index neighbourhood edges min/median/max: {}",
        mm(&mut edges)
    );
    println!("most analysis parts in one neighbourhood: {}", best.0);
    // Snapshot hooks take these numbers (`ARCHGUARD_DESKTOP_ES_FOCUS=#<n>`, `_ES_FIELD=#<k>`).
    let analyzer = flat
        .iter()
        .position(|k| matches!(k, Key::Part { kind, .. } if kind == "analyzer"));
    let field = flat.iter().enumerate().find_map(|(n, k)| {
        let Key::Node(i) = k else { return None };
        let d = g.nodes[*i]
            .artifact
            .and_then(|a| t.elasticsearch.artifacts[a].detail.as_ref())?;
        let f = d.fields.iter().position(|f| f.analyzer.is_some())?;
        Some((n, f))
    });
    println!(
        "row number of that index / of the first analyzer: {:?} / {analyzer:?}; first index row / field number with an analyzer: {field:?}",
        best.1
    );
    let rows = members(&groups);
    let d = diagram(&t, &g, &rows[0], Some(&rows), &sw);
    let cs: Vec<_> = d
        .boxes
        .iter()
        .map(|b| content(&t, &g, &b.key, &sw))
        .collect();
    let s = layout(&d, &cs);
    println!(
        "cluster boxes: {}; edges: {}; canvas: {:.0} x {:.0}",
        d.boxes.len(),
        d.edges.len(),
        s.width,
        s.height
    );
    let lines = cs
        .iter()
        .map(|c| c.compartments.iter().map(|k| k.lines.len()).sum::<usize>())
        .max()
        .unwrap_or(0);
    let chips = cs
        .iter()
        .flat_map(|c| c.compartments.iter())
        .map(|k| k.chips.len())
        .max()
        .unwrap_or(0);
    println!("most rows in one box: {lines}; longest chip row: {chips}");
}
