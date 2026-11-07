//! The arc42 package map: a layered layout of cards and curved edges, as
//! the web viewer draws it with Graphviz `dot` (`arc42Graph`), the card
//! labels and the PlantUML export (`arc42PlantUml`), both in
//! viewer_architecture.js.

mod layered;
mod place;

pub use place::{Card, Layout};

/// Lay out `nodes` cards and route `edges` (positions into the cards); one
/// route per edge, in order.
pub fn layout(nodes: usize, edges: &[(usize, usize)]) -> Layout {
    place::layout(&layered::grid(nodes, edges))
}

/// The card's label in at most two lines of 24 characters, as
/// `arc42NameLines`: the first ends after its last "/" when it has one,
/// the second ends in "…" when the label is longer.
pub fn name_lines(label: &str) -> (String, String) {
    let chars: Vec<char> = label.chars().collect();
    if chars.len() <= 24 {
        return (label.to_string(), String::new());
    }
    let cut = chars[..24]
        .iter()
        .rposition(|&c| c == '/')
        .filter(|&i| i > 0)
        .map_or(24, |i| i + 1);
    let skip = chars[cut..]
        .iter()
        .take_while(|c| c.is_whitespace())
        .count();
    let rest = &chars[cut + skip..];
    let mut second: String = rest.iter().take(24).collect();
    if rest.len() > 24 {
        second.push('…');
    }
    (chars[..cut].iter().collect(), second)
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

    fn centre(c: &Card) -> f32 {
        c.x + c.w / 2.0
    }

    fn overlaps(a: &Card, b: &Card) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    /// The point after the first command letter `cmd` of an SVG path.
    fn points(path: &str, cmd: char) -> Vec<(f32, f32)> {
        let tokens: Vec<&str> = path.split_whitespace().collect();
        tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == cmd.to_string())
            .map(|(i, _)| {
                (
                    tokens[i + 1].parse().unwrap(),
                    tokens[i + 2].parse().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn dependencies_go_down_in_a_straight_line() {
        let l = layout(3, &[(0, 1), (1, 2)]);
        let [a, b, c] = [l.cards[0], l.cards[1], l.cards[2]];
        assert!(a.y + a.h < b.y && b.y + b.h < c.y);
        assert!((centre(&a) - centre(&b)).abs() < 1.0);
        assert!((centre(&b) - centre(&c)).abs() < 1.0);
        // Out of the bottom of the source, arrow tip on top of the target.
        assert_eq!(points(&l.routes[0].path, 'M')[0].1, a.y + a.h);
        assert_eq!(points(&l.routes[0].head, 'M')[0].1, b.y);
        assert!(l.width >= c.x + c.w && l.height >= c.y + c.h);
    }

    #[test]
    fn a_cycle_puts_one_edge_upwards() {
        let l = layout(2, &[(0, 1), (1, 0)]);
        let [a, b] = [l.cards[0], l.cards[1]];
        assert!(a.y + a.h < b.y);
        assert_eq!(points(&l.routes[1].path, 'M')[0].1, b.y);
        assert_eq!(points(&l.routes[1].head, 'M')[0].1, a.y + a.h);
        // Both lines between the same cards use different ports.
        let starts: Vec<f32> = l.routes.iter().map(|r| points(&r.path, 'M')[0].0).collect();
        assert!((starts[0] - starts[1]).abs() > 10.0);
    }

    #[test]
    fn rows_hold_at_most_four_cards_without_overlap() {
        let l = layout(13, &[]);
        for c in &l.cards {
            assert!(l.cards.iter().filter(|o| o.y == c.y).count() <= 4);
            assert!(l.cards.iter().filter(|o| overlaps(c, o)).count() == 1);
        }
    }

    #[test]
    fn long_edges_pass_beside_the_cards_in_between() {
        // 0 → 1 → 2 and 0 → 2: the long edge bends around card 1.
        let l = layout(3, &[(0, 1), (1, 2), (0, 2)]);
        let b = l.cards[1];
        let bend: Vec<(f32, f32)> = points(&l.routes[2].path, 'L')
            .into_iter()
            .filter(|p| p.1 >= b.y && p.1 <= b.y + b.h)
            .collect();
        assert!(!bend.is_empty());
        for (x, _) in bend {
            assert!(x < b.x || x > b.x + b.w, "{x} inside {b:?}");
        }
    }

    #[test]
    fn order_avoids_crossings() {
        // Input order would cross 0 → 3 with 1 → 2.
        let l = layout(4, &[(0, 3), (1, 2)]);
        let c = &l.cards;
        assert_eq!(c[0].x < c[1].x, c[3].x < c[2].x);
    }

    #[test]
    fn labels_do_not_overlap_and_every_edge_has_a_route() {
        let mut edges = vec![(2, 2)];
        for a in 0..6 {
            for b in 0..6 {
                if a != b {
                    edges.push((a, b));
                }
            }
        }
        let l = layout(6, &edges);
        assert_eq!(l.routes.len(), edges.len());
        assert!(l.routes[0].path.is_empty());
        let labels: Vec<(f32, f32)> = l.routes[1..].iter().map(|r| r.label).collect();
        for (i, a) in labels.iter().enumerate() {
            for b in &labels[i + 1..] {
                assert!(
                    (a.0 - b.0).abs() >= 20.0 || (a.1 - b.1).abs() >= 12.0,
                    "{a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn layout_is_deterministic() {
        let edges = [(0, 1), (1, 2), (2, 0), (3, 1), (4, 2)];
        assert_eq!(layout(5, &edges), layout(5, &edges));
    }

    #[test]
    fn labels_break_after_a_slash_in_two_lines() {
        assert_eq!(
            name_lines("archguard-engine"),
            ("archguard-engine".into(), String::new())
        );
        assert_eq!(
            name_lines("archguard-engine/semantic_calls"),
            ("archguard-engine/".into(), "semantic_calls".into())
        );
        assert_eq!(
            name_lines("archguard-desktop / direct files"),
            ("archguard-desktop /".into(), "direct files".into())
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
