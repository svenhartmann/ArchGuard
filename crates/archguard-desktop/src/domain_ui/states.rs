//! The state diagram of one state machine: states in columns by distance
//! from the initial state, transitions as numbered arrows (the number of
//! the table row), a transition from any state starts at an "any state"
//! node.

use std::collections::HashMap;

use archguard_engine::core::domain_model::StateMachine;

use crate::diagram;

pub const NODE_W: f32 = 150.0;
pub const NODE_H: f32 = 36.0;
const GAP_X: f32 = 80.0;
const GAP_Y: f32 = 34.0;
const LEFT: f32 = 44.0;
const TOP: f32 = 40.0;
const BOTTOM: f32 = 60.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The state; `None` for the "any state" source.
    pub state: Option<String>,
    pub initial: bool,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub path: String,
    /// 1-based, the transition's row in the table.
    pub number: usize,
    pub at: diagram::Point,
}

#[derive(Debug, Default)]
pub struct StateDiagram {
    pub nodes: Vec<Node>,
    pub arrows: Vec<Arrow>,
    pub width: f32,
    pub height: f32,
}

/// Column of each state: breadth first from the initial state (else the
/// first), a transition from any state leaving every reached one;
/// unreached states after the last column.
fn columns(m: &StateMachine) -> HashMap<&str, usize> {
    let mut col: HashMap<&str, usize> = HashMap::new();
    let first = m
        .initial
        .as_deref()
        .or(m.states.first().map(String::as_str));
    let mut queue: Vec<&str> = first.into_iter().collect();
    if let Some(f) = first {
        col.insert(f, 0);
    }
    let mut at = 0;
    while at < queue.len() {
        let s = queue[at];
        at += 1;
        let leaving =
            (m.transitions.iter()).filter(|t| t.from.is_empty() || t.from.iter().any(|f| f == s));
        for t in leaving {
            if !col.contains_key(t.to.as_str()) {
                col.insert(t.to.as_str(), col[s] + 1);
                queue.push(t.to.as_str());
            }
        }
    }
    let last = col.values().max().map_or(0, |c| c + 1);
    for s in m.states.iter().chain(m.transitions.iter().map(|t| &t.to)) {
        col.entry(s.as_str()).or_insert(last);
    }
    col
}

pub fn layout<'a>(m: &'a StateMachine) -> StateDiagram {
    let col = columns(m);
    let mut d = StateDiagram::default();
    let mut rows: HashMap<usize, usize> = HashMap::new();
    let mut at: HashMap<Option<&'a str>, (f32, f32)> = HashMap::new();
    let mut place = |state: Option<&'a str>, c: usize| {
        let r = rows.entry(c).or_insert(0);
        let p = (
            LEFT + c as f32 * (NODE_W + GAP_X),
            TOP + *r as f32 * (NODE_H + GAP_Y),
        );
        *r += 1;
        at.insert(state, p);
        p
    };
    let mut states: Vec<&'a str> = m.states.iter().map(String::as_str).collect();
    for t in &m.transitions {
        if !states.contains(&t.to.as_str()) {
            states.push(t.to.as_str());
        }
    }
    states.sort_by_key(|s| col[s]);
    for s in states {
        let (x, y) = place(Some(s), col[s]);
        let initial = m.initial.as_deref() == Some(s);
        d.nodes.push(Node {
            state: Some(s.to_string()),
            initial,
            x,
            y,
        });
    }
    if m.transitions.iter().any(|t| t.from.is_empty()) {
        let (x, y) = place(None, 0);
        d.nodes.push(Node {
            state: None,
            initial: false,
            x,
            y,
        });
    }
    for (i, t) in m.transitions.iter().enumerate() {
        let froms: Vec<Option<&str>> = if t.from.is_empty() {
            vec![None]
        } else {
            t.from.iter().map(|f| Some(f.as_str())).collect()
        };
        for f in froms {
            let (Some(&a), Some(&b)) = (at.get(&f), at.get(&Some(t.to.as_str()))) else {
                continue;
            };
            let (path, mid) = arrow(a, b);
            d.arrows.push(Arrow {
                path,
                number: i + 1,
                at: mid,
            });
        }
    }
    d.width = (d.nodes.iter())
        .map(|n| n.x + NODE_W + 24.0)
        .fold(0.0, f32::max);
    d.height = (d.nodes.iter())
        .map(|n| n.y + NODE_H + BOTTOM)
        .fold(0.0, f32::max);
    d
}

fn arrow(a: (f32, f32), b: (f32, f32)) -> (String, diagram::Point) {
    if a == b {
        diagram::self_loop(a.0 + NODE_W / 2.0, a.1, 40.0)
    } else {
        let (p, q) = diagram::ends(a, b, NODE_W, NODE_H);
        diagram::curve(p, q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::StateTransition;

    fn go(from: &[&str], to: &str) -> StateTransition {
        StateTransition {
            from: from.iter().map(|s| s.to_string()).collect(),
            to: to.into(),
            method: format!("Order.Set{to}"),
            guard: None,
            events: vec![],
            file: "Order.cs".into(),
            line: 1,
        }
    }

    fn order() -> StateMachine {
        StateMachine {
            entity: "Order".into(),
            field: "Status".into(),
            enum_type: "Status".into(),
            states: ["Submitted", "Paid", "Shipped", "Cancelled", "Archived"]
                .map(String::from)
                .to_vec(),
            initial: Some("Submitted".into()),
            transitions: vec![
                go(&["Submitted"], "Paid"),
                go(&["Paid"], "Shipped"),
                go(&["Submitted", "Paid", "Cancelled"], "Cancelled"),
            ],
        }
    }

    fn node<'a>(d: &'a StateDiagram, s: &str) -> &'a Node {
        d.nodes
            .iter()
            .find(|n| n.state.as_deref() == Some(s))
            .unwrap()
    }

    #[test]
    fn states_follow_the_transitions_from_the_initial_one() {
        let d = layout(&order());
        let col = |s| ((node(&d, s).x - LEFT) / (NODE_W + GAP_X)).round() as usize;
        assert!(node(&d, "Submitted").initial);
        assert_eq!(
            [
                col("Submitted"),
                col("Paid"),
                col("Shipped"),
                col("Cancelled")
            ],
            [0, 1, 2, 1]
        );
        assert_eq!(col("Archived"), 3, "unreached after the last column");
    }

    #[test]
    fn every_source_state_gets_an_arrow_with_the_row_number() {
        let d = layout(&order());
        let numbers: Vec<usize> = d.arrows.iter().map(|a| a.number).collect();
        assert_eq!(numbers, [1, 2, 3, 3, 3]);
        assert!(d.arrows[4].path.contains(" C "), "a self loop");
        let mut any = order();
        any.transitions.push(go(&[], "Archived"));
        let d = layout(&any);
        assert!(
            d.nodes.iter().any(|n| n.state.is_none()),
            "an any-state node"
        );
        assert_eq!(d.arrows.last().unwrap().number, 4);
    }
}
