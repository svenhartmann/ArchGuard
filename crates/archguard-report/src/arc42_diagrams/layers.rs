//! Chapter 4: the layering as the solution strategy (ADR 0006). One box
//! per layer (declared or derived) with its building blocks, the source
//! connections between layers with their count; a connection into a layer
//! the source may not depend on (`may_depend_on`) is red.

use archguard_core::{
    Config, Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
};
use std::collections::BTreeMap;

use crate::arc42_blocks::{Arc42Blocks, Blocks};

/// Blocks named in a layer's box before "…".
const NAMED: usize = 4;

pub fn diagram(blocks: &Blocks<'_>, config: &Config, facts: &Arc42Blocks) -> Option<Diagram> {
    let of = blocks.block_layers(&facts.all_layers);
    let mut members: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for (i, layer) in of.iter().enumerate() {
        if let Some(l) = layer {
            members.entry(*l).or_default().push(&blocks.labels[i]);
        }
    }
    if members.len() < 2 {
        return None;
    }
    let order: Vec<usize> = members.keys().copied().collect();
    let at = |l: usize| order.iter().position(|&x| x == l).unwrap_or(0);
    let mut counts: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (&(a, b), &n) in blocks.uses() {
        if let (Some(la), Some(lb)) = (of[a], of[b])
            && la != lb
        {
            *counts.entry((la, lb)).or_default() += n;
        }
    }
    let allowed = |a: usize, b: usize| {
        let target = &config.layers[b].name;
        config.layers[a].may_depend_on.iter().any(|d| d == target)
    };
    let elements = order
        .iter()
        .map(|&l| element(at(l), &config.layers[l].name, &members[&l]))
        .collect();
    let relations = counts
        .into_iter()
        .map(|((a, b), count)| DiagramRelation {
            from: format!("e{}", at(a)),
            to: format!("e{}", at(b)),
            kind: RelationKind::Dependency,
            count,
            cycle: !allowed(a, b),
            label: None,
            error: false,
        })
        .collect();
    Some(Diagram {
        id: "layers".into(),
        chapter: 4,
        title: "Schichten".into(),
        kind: DiagramKind::Packages,
        note: "Quelle: Schichten aus `layers` (deklariert oder abgeleitet) mit ihren Bausteinen; Pfeile: Quellverbindungen zwischen den Schichten. Rot: Verbindung in eine Schicht, von der die Quelle laut `may_depend_on` nicht abhängen darf.".into(),
        unit: "Quellverbindungen".into(),
        elements,
        relations,
        shown: true,
    })
}

fn element(i: usize, layer: &str, members: &[&str]) -> DiagramElement {
    let mut named: Vec<&str> = members.iter().take(NAMED).copied().collect();
    if members.len() > NAMED {
        named.push("…");
    }
    DiagramElement {
        id: format!("e{i}"),
        name: format!("Schicht {layer}"),
        kind: ElementKind::Package,
        technology: None,
        detail: Some(format!(
            "{} {}: {}",
            members.len(),
            if members.len() == 1 {
                "Baustein"
            } else {
                "Bausteine"
            },
            named.join(", ")
        )),
        group: None,
        cycle: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{LayerConfig, ServiceTopology};

    #[test]
    fn layers_are_boxes_and_a_forbidden_direction_is_red() {
        let mut b = archguard_core::GraphBuilder::new();
        let files: Vec<_> = ["ui/S", "core/S", "util/S", "ui/T"]
            .iter()
            .map(|p| b.ensure_path(&format!("app/{p}.java"), Some("java")))
            .collect();
        let edge = |b: &mut archguard_core::GraphBuilder, x: usize, y: usize, line: u32| {
            let r = archguard_core::Ref {
                file: "x".into(),
                line,
                raw: "import".into(),
            };
            b.edge(files[x], files[y], r).unwrap();
        };
        edge(&mut b, 0, 1, 1);
        edge(&mut b, 3, 1, 2);
        edge(&mut b, 1, 0, 3);
        edge(&mut b, 2, 1, 4);
        let graph = b.build("p".into());
        let layer = |name: &str, deps: &[&str]| LayerConfig {
            name: name.into(),
            contains: vec![],
            may_depend_on: deps.iter().map(|d| d.to_string()).collect(),
            origin: Default::default(),
        };
        let config = Config {
            layers: vec![layer("ui", &["core"]), layer("core", &[])],
            ..Default::default()
        };
        let id = |name: &str| graph.nodes.iter().find(|n| n.name == name).unwrap().id;
        let facts = Arc42Blocks {
            all_layers: BTreeMap::from([(id("app/ui"), 0), (id("app/core"), 1)]),
            ..Default::default()
        };
        let services = ServiceTopology::default();
        let blocks = Blocks::new(&graph, &config, &services, &BTreeMap::new(), &facts);
        let d = diagram(&blocks, &config, &facts).unwrap();
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["Schicht ui", "Schicht core"],
            "util has no layer and stays out"
        );
        let rel: Vec<(&str, &str, usize, bool)> = d
            .relations
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str(), r.count, r.cycle))
            .collect();
        assert_eq!(rel, [("e0", "e1", 2, false), ("e1", "e0", 1, true)]);
        assert!(
            d.elements[0]
                .detail
                .as_deref()
                .unwrap()
                .starts_with("1 Baustein:")
        );
    }
}
