//! The Services tab as the export's input: its settings as
//! [`ServiceFilters`] and the drawn tree as a [`Diagram`] (the rows and
//! relations of [`build`], at the canvas geometry).

use slint::Model;

use super::state::{KINDS, Mode, ServicesState};
use super::view::build;
use crate::export::{Diagram, DiagramBox, DiagramEdge, EdgeKind, ServiceFilters};

/// The tab's `serviceUi` under the web's names.
pub fn filters(s: &ServicesState) -> ServiceFilters {
    let t = s.topology();
    ServiceFilters {
        kind: KINDS[s.kind],
        direction: match s.mode {
            Mode::All => "all",
            Mode::Incoming => "incoming",
            Mode::Outgoing => "outgoing",
            Mode::Both => "both",
        }
        .into(),
        reference: s
            .selected
            .and_then(|i| Some(t?.services.get(i)?.id.clone())),
        selected_project: s.project.and_then(|i| Some(t?.projects.get(i)?.id.clone())),
        cycles_only: s.cycles_only,
        focused_cycle: s.cycle.clone(),
    }
}

/// The drawn service tree: a box per row, an edge with an arrow head per
/// shown relation, coloured as in the view.
pub fn diagram(s: &ServicesState, unmapped: &str) -> Diagram {
    let m = build(s, unmapped);
    let boxes = m
        .rows
        .iter()
        .map(|r| DiagramBox {
            x: r.left,
            y: r.y - r.h / 2.0,
            w: r.w,
            h: r.h,
            label: r.name.to_string(),
            selected: r.selected,
            cyclic: r.in_cycle,
        })
        .collect();
    let edges = m
        .edges
        .iter()
        .map(|e| DiagramEdge {
            path: e.line.to_string(),
            kind: match e.color {
                1 => EdgeKind::Incoming,
                2 => EdgeKind::Outgoing,
                3 => EdgeKind::Cyclic,
                _ => EdgeKind::Neutral,
            },
            dx: 0.0,
            dy: 0.0,
            width: 2.0,
            arrow: true,
        })
        .collect();
    Diagram {
        width: m.width,
        height: m.height,
        boxes,
        edges,
        ..Diagram::default()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::state::build_from;
    use super::*;

    fn shop() -> ServicesState {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
        let mut s = build_from(std::sync::Arc::new(
            archguard_engine::analyze_services(&root).map_err(|e| e.to_string()),
        ));
        s.viewport = 1480.0;
        s
    }

    /// The selection and direction become the web's filter values; the
    /// diagram has the view's rows and relations.
    #[test]
    fn services_export_their_filters_and_drawn_tree() {
        let mut s = shop();
        let t = s.topology().expect("topology");
        let first = t.services[0].id.clone();
        let rows = build(&s, "Unmapped service").rows.row_count();
        let edges = build(&s, "Unmapped service").edges.row_count();
        s.select_service(0);
        s.mode = Mode::Outgoing;
        let f = filters(&s);
        assert_eq!(f.reference.as_deref(), Some(first.as_str()));
        assert_eq!(f.direction, "outgoing");
        assert_eq!(f.kind, KINDS[s.kind]);
        s.mode = Mode::All;
        s.selected = None;
        let d = diagram(&s, "Unmapped service");
        assert_eq!(d.boxes.len(), rows);
        assert_eq!(d.edges.len(), edges);
        assert!(edges > 0, "the example has relations");
        assert!(d.width > 0.0 && d.height > 0.0);
    }
}
