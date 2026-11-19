//! Diagrams of the arc42 catalogue as terminal text (`archguard diagram`):
//! box graphs layered top-down, sequences with lifelines. Pure functions;
//! the caller decides on Unicode, colour and width ([`Style`]).

mod canvas;
mod graph;
mod layers;
mod sequence;

use archguard_core::{Diagram, DiagramKind};

pub use canvas::Style;

/// `d` drawn for a terminal.
pub fn draw(d: &Diagram, style: Style) -> String {
    match d.kind {
        DiagramKind::Sequence => sequence::draw(d, style),
        _ => graph::draw(d, style),
    }
}

/// Golden drawings of small diagrams.
#[cfg(test)]
mod tests {
    use archguard_core::{
        Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
    };

    use super::*;

    fn element(id: &str, cycle: bool) -> DiagramElement {
        DiagramElement {
            id: id.into(),
            name: id.into(),
            kind: ElementKind::Package,
            technology: None,
            detail: None,
            group: None,
            cycle,
        }
    }

    fn rel(from: &str, to: &str, count: usize) -> DiagramRelation {
        DiagramRelation {
            from: from.into(),
            to: to.into(),
            kind: RelationKind::Dependency,
            count,
            cycle: false,
            label: None,
            error: false,
            caller: None,
            reply: false,
        }
    }

    fn diagram(
        kind: DiagramKind,
        elements: Vec<DiagramElement>,
        relations: Vec<DiagramRelation>,
    ) -> Diagram {
        Diagram {
            id: "t".into(),
            chapter: 5,
            title: "T".into(),
            kind,
            note: String::new(),
            unit: "source connections".into(),
            elements,
            relations,
            shown: true,
        }
    }

    fn graph(names: &[&str], edges: &[(&str, &str, usize)]) -> Diagram {
        diagram(
            DiagramKind::Packages,
            names.iter().map(|n| element(n, false)).collect(),
            edges.iter().map(|&(a, b, c)| rel(a, b, c)).collect(),
        )
    }

    /// The drawing as written in the source: after a first line break.
    fn golden(text: &str) -> &str {
        text.strip_prefix('\n').unwrap()
    }

    fn ascii(width: usize) -> Style {
        Style {
            unicode: false,
            color: false,
            width,
        }
    }

    #[test]
    fn a_chain_runs_top_down() {
        let d = graph(
            &["web", "core", "db"],
            &[("web", "core", 3), ("core", "db", 1)],
        );
        let got = draw(&d, Style::default());
        assert_eq!(
            got,
            golden(
                r"
┌─────────┐
│ web     │
│ package │
└────┬────┘
     │
     ▼ 3
┌─────────┐
│ core    │
│ package │
└────┬────┘
     │
     ▼ 1
┌─────────┐
│ db      │
│ package │
└─────────┘
",
            )
        );
    }

    #[test]
    fn a_fan_out_turns_on_tracks() {
        let d = graph(
            &["app", "a", "b", "c"],
            &[("app", "a", 1), ("app", "b", 2), ("app", "c", 12)],
        );
        assert_eq!(
            draw(&d, Style::default()),
            golden(
                r"
             ┌─────────┐
             │ app     │
             │ package │
             └─┬──┬──┬─┘
               │  │  │
     ┌────1────┘  │  └───12────┐
     ▼            ▼ 2          ▼
┌─────────┐  ┌─────────┐  ┌─────────┐
│ a       │  │ b       │  │ c       │
│ package │  │ package │  │ package │
└─────────┘  └─────────┘  └─────────┘
",
            )
        );
    }

    #[test]
    fn a_cycle_is_broken_and_marked() {
        let mut d = graph(&["a", "b"], &[("a", "b", 2), ("b", "a", 1)]);
        d.elements.iter_mut().for_each(|e| e.cycle = true);
        d.relations.iter_mut().for_each(|r| r.cycle = true);
        assert_eq!(
            draw(&d, ascii(100)),
            golden(
                r"
+---------+
| ! a     |
| package |
+--+---+--+
   |   ^ !1
!2 v   |
+------+--+
| ! b     |
| package |
+---------+
",
            )
        );
    }

    #[test]
    fn a_wide_layer_wraps_and_long_edges_pass_rows() {
        let d = graph(
            &["root", "alpha-module", "beta-module", "gamma-module"],
            &[
                ("root", "alpha-module", 1),
                ("root", "beta-module", 1),
                ("root", "gamma-module", 1),
            ],
        );
        assert_eq!(
            draw(&d, ascii(40)),
            golden(
                r"
            +---------+
            | root    |
            | package |
            +-+--+--+-+
              |  |  |
        +--1--+  |  +--------------+
        |        +---1---+         |
        v                v         |
+--------------+  +-------------+  |
| alpha-module |  | beta-module |  |
| package      |  | package     |  |
+--------------+  +-------------+  |
                                   |
                  +-------1--------+
                  v
          +--------------+
          | gamma-module |
          | package      |
          +--------------+
",
            )
        );
    }

    #[test]
    fn a_sequence_has_returns_and_a_call_to_itself() {
        let call = |from: &str, to: &str, caller: Option<usize>, label: &str| DiagramRelation {
            caller,
            kind: RelationKind::Http,
            label: Some(label.into()),
            ..rel(from, to, 1)
        };
        let d = diagram(
            DiagramKind::Sequence,
            vec![
                element("user", false),
                element("api", false),
                element("db", false),
            ],
            vec![
                call("user", "api", None, "1. GET /orders"),
                call("api", "api", Some(0), "2. validate"),
                call("api", "db", Some(0), "3. select"),
            ],
        );
        assert_eq!(
            draw(&d, Style::default()),
            golden(
                r"
     ┌──────┐           ┌─────┐           ┌────┐
     │ user │           │ api │           │ db │
     └───┬──┘           └──┬──┘           └──┬─┘
         │ 1. GET /orders  │                 │
         ├────────────────>│                 │
         │                 ├──┐ 2. validate  │
         │                 │  │              │
         │                 ├<─┘              │
         │                 │ 3. select       │
         │                 ├────────────────>│
         │                 │<╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
         │<╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤                 │
         │                 │                 │
",
            )
        );
    }

    #[test]
    fn colour_appears_only_when_asked() {
        let mut d = graph(&["a", "b"], &[("a", "b", 2), ("b", "a", 1)]);
        d.relations[1].cycle = true;
        assert!(!draw(&d, Style::default()).contains('\x1b'));
        let coloured = Style {
            color: true,
            ..Style::default()
        };
        assert!(draw(&d, coloured).contains("\x1b[31m"));
    }

    #[test]
    fn an_event_has_a_double_head_and_an_error_a_mark() {
        let message = |kind, error, label: &str| DiagramRelation {
            kind,
            error,
            label: Some(label.into()),
            ..rel("a", "b", 1)
        };
        let d = diagram(
            DiagramKind::Sequence,
            vec![element("a", false), element("b", false)],
            vec![
                message(RelationKind::Event, false, "1. publish"),
                message(RelationKind::Http, true, "2. save"),
            ],
        );
        let got = draw(&d, ascii(60));
        assert!(got.contains("->>|"), "{got}");
        assert!(got.contains("x 2. save"), "{got}");
        // The event gets no return, the failed call does.
        assert_eq!(got.matches("<....").count(), 1, "{got}");
    }
}
