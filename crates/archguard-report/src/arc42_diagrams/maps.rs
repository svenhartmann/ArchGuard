//! The package maps of chapters 5, 6 and 11 (ADR 0006): the overview and,
//! for the ids in `[arc42] shown_diagrams`, its levels
//! (`<map>@<package>`), written from the [`crate::package_map`] model the
//! desktop draws, so a ticked level shows the boxes and names of the map.

use archguard_core::{
    Arc42Config, Diagram, DiagramElement, DiagramKind, DiagramRelation, DocLang, ElementKind,
    NodeKind, RelationKind,
};

use crate::arc42_blocks::Arc42Inputs;
use crate::package_map::{self as model, Group, Labels, PNode, Projection, SourceModel};

/// Boxes per diagram: nine, or eight and one for the rest. The readers
/// draw every box; the document stays readable.
const BOXES: usize = 9;

/// Title and note of the overview maps (ADR 0008).
const OVERVIEW_EN: (&str, &str) = (
    "Package overview",
    "Source: complete source graph; packages as building blocks, at most nine boxes. Arrows: source connections from the using to the used side.",
);
const OVERVIEW_DE: (&str, &str) = (
    "Paketübersicht",
    "Quelle: vollständiger Quellgraph; Pakete als Bausteine, höchstens neun Kästen. Pfeile: Quellverbindungen von der nutzenden zur genutzten Seite.",
);
const CALLS_EN: (&str, &str) = (
    "Static calls between building blocks",
    "Source: compiler-resolved call sites. Statically recorded; no runtime order and no frequency at runtime.",
);
const CALLS_DE: (&str, &str) = (
    "Statische Aufrufe zwischen Bausteinen",
    "Quelle: compileraufgelöste Aufrufstellen. Statisch erfasst; keine Laufzeitreihenfolge und keine Häufigkeit zur Laufzeit.",
);
const CYCLES_EN: (&str, &str) = (
    "Cyclic package relations",
    "Source: source connections between the boxes of the package overview; only relations in cycles are shown.",
);
const CYCLES_DE: (&str, &str) = (
    "Zyklische Paketbeziehungen",
    "Quelle: Quellverbindungen zwischen den Kästen der Paketübersicht; gezeigt sind nur Beziehungen in Zyklen.",
);

/// The source graph's package model and its relations, built once per
/// document.
pub struct Maps<'a> {
    input: &'a Arc42Inputs<'a>,
    model: SourceModel,
    deps: Vec<(usize, usize)>,
    calls: Vec<(usize, usize)>,
    labels: Labels,
}

impl<'a> Maps<'a> {
    pub fn new(input: &'a Arc42Inputs<'a>) -> Self {
        let graph = input.source_graph;
        let model = SourceModel::new(graph);
        Maps {
            deps: model::dependency_edges(&model, graph),
            calls: model::call_edges(graph, input.calls),
            labels: Labels::of(input.lang),
            model,
            input,
        }
    }

    /// The overview maps: packages, static calls and cycles, each only
    /// when it has something to show.
    pub fn overview(&self) -> Vec<Diagram> {
        let packages = self.diagram("building-blocks", None);
        if packages.as_ref().is_none_or(|d| d.elements.is_empty()) {
            return Vec::new();
        }
        let more = ["runtime-calls", "cycles"]
            .into_iter()
            .filter_map(|id| self.diagram(id, None))
            .filter(|d| !d.relations.is_empty());
        packages.into_iter().chain(more).collect()
    }

    /// The levels listed in `[arc42] shown_diagrams` whose package exists.
    pub fn levels(&self) -> Vec<Diagram> {
        let graph = self.input.source_graph;
        let config = &self.input.config.arc42;
        config
            .shown_diagrams
            .iter()
            .filter_map(|id| {
                let (_, name) = Arc42Config::level_of(id)?;
                let scope = graph
                    .nodes
                    .iter()
                    .position(|n| n.kind == NodeKind::Module && n.name == name)?;
                self.diagram(id, Some(scope))
            })
            .collect()
    }

    /// The map `id` (`<map>` or `<map>@<package>`) of `scope`, as the
    /// readers show it, at most [`BOXES`] boxes.
    fn diagram(&self, id: &str, scope: Option<usize>) -> Option<Diagram> {
        let l = self.input.lang;
        let map = Arc42Config::level_of(id).map_or(id, |(map, _)| map);
        let (chapter, kind, edges) = match map {
            "building-blocks" => (5, RelationKind::Dependency, &self.deps),
            "runtime-calls" => (6, RelationKind::Call, &self.calls),
            "cycles" => (11, RelationKind::Dependency, &self.deps),
            _ => return None,
        };
        let graph = self.input.source_graph;
        let mut shown = model::project(&self.model, graph, scope, edges, &self.labels);
        if map == "cycles" {
            shown = shown.cycles_only();
        }
        let more = l.pick("{} more packages", "Weitere {} Pakete");
        let folded = shown.folded(BOXES, more);
        let (title, mut note) = texts(l, map, scope.map(|s| graph.nodes[s].name.as_str()));
        if scope.is_some() && folded.nodes.len() < shown.nodes.len() {
            let rest = l.pick(
                " At most nine boxes: the smallest packages share the last one.",
                " Höchstens neun Kästen: die kleinsten Pakete teilen sich den letzten.",
            );
            note.push_str(rest);
        }
        Some(Diagram {
            id: id.into(),
            chapter,
            title,
            kind: DiagramKind::Packages,
            note,
            unit: if kind == RelationKind::Call {
                l.pick("call sites", "Aufrufstellen")
            } else {
                l.pick("source connections", "Quellverbindungen")
            }
            .into(),
            elements: self.elements(&folded, scope.is_none()),
            relations: relations(&folded, kind),
            shown: true,
        })
    }

    /// One element per box; overview packages are grouped by their
    /// declared layer.
    fn elements(&self, p: &Projection, layered: bool) -> Vec<DiagramElement> {
        let l = self.input.lang;
        let layer = |n: &PNode| {
            let Group::Package(at) = n.group else {
                return None;
            };
            let id = self.input.source_graph.nodes[at].id;
            let layer = self.input.blocks.layers.get(&id)?;
            let name = &self.input.config.layers.get(*layer)?.name;
            Some(format!("{} {name}", l.pick("Layer", "Schicht")))
        };
        p.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| DiagramElement {
                id: format!("e{i}"),
                name: n.label.clone(),
                kind: match n.group {
                    Group::Package(_) => ElementKind::Package,
                    _ => ElementKind::Remainder,
                },
                technology: None,
                detail: None,
                group: if layered { layer(n) } else { None },
                cycle: n.cycle,
            })
            .collect()
    }
}

/// Title and note of map `map`, for the overview or the package `scope`.
fn texts(l: DocLang, map: &str, scope: Option<&str>) -> (String, String) {
    let Some(scope) = scope else {
        let (title, note) = match map {
            "building-blocks" => l.pick(OVERVIEW_EN, OVERVIEW_DE),
            "runtime-calls" => l.pick(CALLS_EN, CALLS_DE),
            _ => l.pick(CYCLES_EN, CYCLES_DE),
        };
        return (title.into(), note.into());
    };
    let (title, note) = match map {
        "building-blocks" => (
            l.pick("Package overview", "Paketübersicht"),
            l.pick(
                "Source: source connections between the packages in",
                "Quelle: Quellverbindungen zwischen den Paketen in",
            ),
        ),
        "runtime-calls" => (
            l.pick("Static calls", "Statische Aufrufe"),
            l.pick(
                "Source: compiler-resolved call sites between the packages in",
                "Quelle: compileraufgelöste Aufrufstellen zwischen den Paketen in",
            ),
        ),
        _ => (
            l.pick("Cyclic package relations", "Zyklische Paketbeziehungen"),
            l.pick(
                "Source: cycles between the packages in",
                "Quelle: Zyklen zwischen den Paketen in",
            ),
        ),
    };
    let direct = l.pick(
        "files directly in the package as a box of their own, packages outside it as one box.",
        "Dateien direkt im Paket als eigener Kasten, Pakete außerhalb als ein Kasten.",
    );
    (
        format!("{title} · {scope}"),
        format!("{note} {scope}; {direct}"),
    )
}

fn relations(p: &Projection, kind: RelationKind) -> Vec<DiagramRelation> {
    p.edges
        .iter()
        .map(|e| DiagramRelation {
            from: format!("e{}", e.from),
            to: format!("e{}", e.to),
            kind,
            count: e.count,
            cycle: e.cycle,
            label: None,
            error: false,
            caller: None,
            reply: false,
        })
        .collect()
}
