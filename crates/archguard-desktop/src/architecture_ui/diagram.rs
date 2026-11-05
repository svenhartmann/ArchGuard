//! Geometry of the web viewer's basic arc42 diagram (`arc42Graph`: cards in
//! a grid, edges routed above the cards) and its PlantUML export
//! (`arc42PlantUml`), both in viewer_architecture.js.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Card {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// SVG path of the line.
    pub path: String,
    /// SVG path of the arrow head at the target.
    pub head: String,
    /// Anchor of the count label: centre of the segment above the cards.
    pub label: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub cards: Vec<Card>,
    pub routes: Vec<Route>,
}

/// Lay out `nodes` cards and route `edges` (positions into the cards).
pub fn layout(nodes: usize, edges: &[(usize, usize)]) -> Layout {
    let columns = if nodes <= 4 { 2 } else { 3 };
    let rows = nodes.div_ceil(columns);
    let cards: Vec<Card> = (0..nodes)
        .map(|i| Card {
            x: 35.0 + (i % columns) as f32 * 270.0,
            y: 55.0 + (i / columns) as f32 * 148.0,
            w: 204.0,
            h: 98.0,
        })
        .collect();
    let routes = edges
        .iter()
        .enumerate()
        .map(|(i, &(from, to))| route(&cards[from], &cards[to], i))
        .collect();
    Layout {
        width: columns as f32 * 270.0 + 40.0,
        height: (rows as f32 * 148.0 + 64.0).max(230.0),
        cards,
        routes,
    }
}

fn route(a: &Card, b: &Card, i: usize) -> Route {
    let right = b.x >= a.x;
    let sign = if right { 1.0 } else { -1.0 };
    let x1 = if right { a.x + a.w } else { a.x };
    let x2 = if right { b.x } else { b.x + b.w };
    let y1 = a.y + a.h / 2.0 - 7.0;
    let y2 = b.y + b.h / 2.0 + 7.0;
    let lane = 16.0 + (i % 5) as f32 * 5.0;
    let (path, tip, dir, label) = if a.x == b.x {
        let out = a.x + a.w;
        (
            format!("M {out} {y1} H {} V {y2} H {}", out + lane, b.x + b.w),
            b.x + b.w,
            -1.0,
            // Lanes 5 px apart: stagger the labels vertically too.
            (out + lane + 6.0, (y1 + y2) / 2.0 + (lane - 16.0) * 2.4),
        )
    } else {
        let top = a.y.min(b.y) - lane;
        let (l, r) = (x1 + sign * lane, x2 - sign * lane);
        (
            format!("M {x1} {y1} H {l} V {top} H {r} V {y2} H {x2}"),
            x2,
            sign,
            ((l + r) / 2.0, top - 1.0),
        )
    };
    let back = tip - dir * 9.0;
    Route {
        path,
        head: format!("M {tip} {y2} L {back} {} L {back} {} Z", y2 - 4.5, y2 + 4.5),
        label,
    }
}

/// The card's name: the last path segment in lines of 24 characters, at
/// most two, the second ending in "…" when the name is longer.
pub fn name_lines(name: &str) -> (String, String) {
    let base = name
        .rsplit('/')
        .next()
        .filter(|b| !b.is_empty())
        .unwrap_or(name);
    let chars: Vec<char> = base.chars().collect();
    let line = |k: usize| -> String { chars.iter().skip(k * 24).take(24).collect() };
    let mut second = line(1);
    if chars.len() > 48 {
        second.push('…');
    }
    (line(0), second)
}

/// What a PlantUML node is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Package,
    Rectangle,
}

pub struct UmlNode<'a> {
    pub name: &'a str,
    pub shape: Shape,
    pub cycle: bool,
}

pub struct UmlEdge {
    pub from: usize,
    pub to: usize,
    pub count: usize,
    pub cycle: bool,
}

/// PlantUML source of a package diagram; `unit` names the counted things.
pub fn plantuml(nodes: &[UmlNode<'_>], edges: &[UmlEdge], unit: &str) -> String {
    let mut lines = vec![
        "@startuml".to_string(),
        "!pragma layout smetana".into(),
        "left to right direction".into(),
        "skinparam shadowing false".into(),
        "skinparam componentStyle uml2".into(),
    ];
    for (i, n) in nodes.iter().enumerate() {
        let shape = match n.shape {
            Shape::Package => "package",
            Shape::Rectangle => "rectangle",
        };
        let color = if n.cycle { " #FFE4E6" } else { "" };
        lines.push(format!("{shape} \"{}\" as n{i}{color}", uml_label(n.name)));
    }
    for e in edges
        .iter()
        .filter(|e| e.from < nodes.len() && e.to < nodes.len())
    {
        let color = if e.cycle { "#ee7d87" } else { "#607796" };
        lines.push(format!(
            "n{} -[{color}]-> n{} : {}",
            e.from,
            e.to,
            uml_label(&format!("{} {unit}", e.count))
        ));
    }
    lines.push("@enduml".into());
    lines.join("\n")
}

/// Delimiters and control characters as PlantUML Unicode escapes.
fn uml_label(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '"' | '\\' | '<' | '>' | '\u{0}'..='\u{1f}' => format!("<U+{:04x}>", c as u32),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_two_columns_up_to_four_cards_else_three() {
        let small = layout(3, &[]);
        assert_eq!((small.width, small.height), (580.0, 360.0));
        assert_eq!(
            small.cards[2],
            Card {
                x: 35.0,
                y: 203.0,
                w: 204.0,
                h: 98.0
            }
        );
        let big = layout(13, &[]);
        assert_eq!((big.width, big.height), (850.0, 804.0));
        assert_eq!(big.cards[4].x, 305.0);
        assert_eq!(layout(1, &[]).height, 230.0);
    }

    #[test]
    fn edges_run_above_the_cards_with_staggered_lanes() {
        let l = layout(3, &[(0, 1), (1, 0), (0, 2)]);
        // 0 → 1: right, lane 16, above row 0.
        assert_eq!(l.routes[0].path, "M 239 97 H 255 V 39 H 289 V 111 H 305");
        assert_eq!(l.routes[0].head, "M 305 111 L 296 106.5 L 296 115.5 Z");
        assert_eq!(l.routes[0].label, (272.0, 38.0));
        // 1 → 0: left, lane 21.
        assert_eq!(l.routes[1].path, "M 305 97 H 284 V 34 H 260 V 111 H 239");
        // Same column: out on the right, back in from the right.
        assert_eq!(l.routes[2].path, "M 239 97 H 265 V 259 H 239");
        assert_eq!(l.routes[2].head, "M 239 259 L 248 254.5 L 248 263.5 Z");
        assert_eq!(l.routes[2].label, (271.0, 202.0));
    }

    #[test]
    fn names_show_the_last_segment_in_two_lines() {
        assert_eq!(
            name_lines("crates/archguard-engine/src"),
            ("src".into(), String::new())
        );
        let long = "a".repeat(50);
        let (one, two) = name_lines(&long);
        assert_eq!((one.len(), two.chars().count()), (24, 25));
        assert!(two.ends_with('…'));
    }

    #[test]
    fn plantuml_escapes_labels_and_colors_cycles() {
        let nodes = [
            UmlNode {
                name: "a\"b",
                shape: Shape::Package,
                cycle: true,
            },
            UmlNode {
                name: "Other",
                shape: Shape::Rectangle,
                cycle: false,
            },
        ];
        let edges = [UmlEdge {
            from: 0,
            to: 1,
            count: 3,
            cycle: false,
        }];
        let src = plantuml(&nodes, &edges, "source connections");
        assert_eq!(
            src,
            "@startuml\n!pragma layout smetana\nleft to right direction\nskinparam shadowing false\nskinparam componentStyle uml2\npackage \"a<U+0022>b\" as n0 #FFE4E6\nrectangle \"Other\" as n1\nn0 -[#607796]-> n1 : 3 source connections\n@enduml"
        );
    }
}
