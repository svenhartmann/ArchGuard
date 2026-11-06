//! Standalone SVG of a drawn diagram, as `exportSvg` in the web viewer:
//! title and description (coverage, filters, legend), a background in
//! the theme's colour, every style inlined, source → target arrows.
//!
//! The input is plain geometry, so the views' layout code stays free to
//! change. Mapping for the integrator:
//!
//! - Graph → Tree (`tree.rs`, `ui/tree_view.slint`): one [`DiagramBox`] per
//!   row at `y = TOP + row * ROW_H`, height `ROW_H - 4`, x after the left
//!   lane (`LANE_W`) and the indent; one [`DiagramEdge`] per `Drawn`:
//!   `path`, `kind` from `LinkKind`, `dx` = `LANE_W + tree width` when
//!   `right` (right-lane paths are relative to the tree's right edge),
//!   else 0. The paths carry their own arrow heads: `arrow: false`.
//! - Services (`services_ui/layout.rs`): one box per `Layout::rows` entry
//!   (`x - w/2`, `y - h/2`, `w`, `h`), one edge per route path
//!   (`paths::curve` / `paths::dashed` plus `paths::head`), `arrow: false`.
//! - Network (`network.rs`): one box per `Card` (`CARD_W` × `CARD_H`), one
//!   edge per `Arrow` with `width` = `Arrow::width`.
//!
//! `description` is [`super::Snapshot::description`] of the same view plus
//! the legend text; `title` is "<project> · Import dependencies" (Graph)
//! or "<project> · Service configuration" (Services), as in the web.

use std::fmt::Write as _;

/// Colour class of an edge (`TreeLink.kind` in `tree_view.slint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Neutral,
    Incoming,
    Outgoing,
    Cyclic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagramBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
    pub selected: bool,
    pub cyclic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagramEdge {
    /// SVG path data, as the views draw it.
    pub path: String,
    pub kind: EdgeKind,
    /// Offset of the path's coordinate system.
    pub dx: f32,
    pub dy: f32,
    /// Stroke width; 0 uses the tree's width for `kind`.
    pub width: f32,
    /// Add an arrow head at the end (for paths without their own).
    pub arrow: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagram {
    pub title: String,
    pub description: String,
    pub width: f32,
    pub height: f32,
    pub boxes: Vec<DiagramBox>,
    pub edges: Vec<DiagramEdge>,
}

/// The colour tokens of `ui/theme.slint` (the web viewer's `:root` and
/// dark override) that a diagram uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: &'static str,
    pub panel: &'static str,
    pub text: &'static str,
    pub border: &'static str,
    pub accent: &'static str,
    pub edge: &'static str,
    pub incoming: &'static str,
    pub outgoing: &'static str,
    pub cycle: &'static str,
}

impl Theme {
    pub const LIGHT: Theme = Theme {
        bg: "#F5F7FB",
        panel: "#EEF2F8",
        text: "#172033",
        border: "#DDE3ED",
        accent: "#2563EB",
        edge: "#64748B",
        incoming: "#0369A1",
        outgoing: "#7C3AED",
        cycle: "#D92D20",
    };
    pub const DARK: Theme = Theme {
        bg: "#101722",
        panel: "#131C29",
        text: "#E6EDF7",
        border: "#2B3A4E",
        accent: "#5B9CF6",
        edge: "#8FA0B5",
        incoming: "#38BDF8",
        outgoing: "#C4B5FD",
        cycle: "#FF6B61",
    };

    /// The theme the window shows (`Theme.dark` in Slint).
    pub fn of(dark: bool) -> Theme {
        if dark { Theme::DARK } else { Theme::LIGHT }
    }

    fn stroke(&self, kind: EdgeKind) -> &'static str {
        match kind {
            EdgeKind::Neutral => self.edge,
            EdgeKind::Incoming => self.incoming,
            EdgeKind::Outgoing => self.outgoing,
            EdgeKind::Cyclic => self.cycle,
        }
    }
}

const KINDS: [EdgeKind; 4] = [
    EdgeKind::Neutral,
    EdgeKind::Incoming,
    EdgeKind::Outgoing,
    EdgeKind::Cyclic,
];
const FONT: &str =
    "font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;font-size:13px";

/// Escape text for XML content and attribute values.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The diagram as a standalone SVG document.
pub fn svg(d: &Diagram, theme: Theme) -> String {
    let (w, h) = (d.width.max(1.0), d.height.max(1.0));
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" role=\"img\">\
         <title>{}</title><desc>{}</desc><defs>",
        xml(&d.title),
        xml(&d.description)
    );
    for (i, kind) in KINDS.iter().enumerate() {
        let _ = write!(
            out,
            "<marker id=\"arrow-{i}\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M 0 0 L 10 5 L 0 10 z\" style=\"fill:{}\"/></marker>",
            theme.stroke(*kind)
        );
    }
    let _ = write!(
        out,
        "</defs><rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" style=\"fill:{}\"/>",
        theme.bg
    );
    for b in &d.boxes {
        node(&mut out, b, theme);
    }
    for e in &d.edges {
        edge(&mut out, e, theme);
    }
    out.push_str("</svg>\n");
    out
}

fn node(out: &mut String, b: &DiagramBox, t: Theme) {
    let stroke = match (b.selected, b.cyclic) {
        (true, _) => t.accent,
        (false, true) => t.cycle,
        _ => t.border,
    };
    let _ = write!(
        out,
        "<g><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6\" style=\"fill:{};stroke:{stroke};stroke-width:1\"/>\
         <text x=\"{}\" y=\"{}\" style=\"fill:{};{FONT};dominant-baseline:middle\">{}</text></g>",
        b.x,
        b.y,
        b.w,
        b.h,
        t.panel,
        b.x + 10.0,
        b.y + b.h / 2.0,
        t.text,
        xml(&b.label)
    );
}

fn edge(out: &mut String, e: &DiagramEdge, t: Theme) {
    let width = match (e.width, e.kind) {
        (w, _) if w > 0.0 => w,
        (_, EdgeKind::Neutral) => 1.3,
        _ => 1.8,
    };
    let marker = if e.arrow {
        let i = KINDS.iter().position(|k| *k == e.kind).unwrap_or(0);
        format!(" marker-end=\"url(#arrow-{i})\"")
    } else {
        String::new()
    };
    let _ = write!(
        out,
        "<path d=\"{}\" transform=\"translate({} {})\" style=\"fill:none;stroke:{};stroke-width:{width};stroke-linecap:round;stroke-linejoin:round\"{marker}/>",
        xml(&e.path),
        e.dx,
        e.dy,
        t.stroke(e.kind)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagram() -> Diagram {
        let row = |y: f32, label: &str| DiagramBox {
            x: 300.0,
            y,
            w: 200.0,
            h: 26.0,
            label: label.into(),
            selected: false,
            cyclic: true,
        };
        Diagram {
            title: "ts-cyclic · Import dependencies".into(),
            description: "Shown graph nodes & \"filters\" <all>.".into(),
            width: 800.0,
            height: 120.0,
            boxes: vec![row(34.0, "src/a"), row(64.0, "src/<b>")],
            edges: vec![
                DiagramEdge { path: "M 22.0 47.0 C 45.0 47.0 45.0 77.0 22.0 77.0 M 29.0 72.5 L 22.0 77.0 L 29.0 81.5".into(), kind: EdgeKind::Cyclic, dx: 500.0, dy: 0.0, width: 0.0, arrow: false },
                DiagramEdge { path: "M 10 10 L 90 90".into(), kind: EdgeKind::Neutral, dx: 0.0, dy: 0.0, width: 3.0, arrow: true },
            ],
        }
    }

    #[test]
    fn svg_is_well_formed_with_title_desc_and_background() {
        let text = svg(&diagram(), Theme::DARK);
        let doc = roxmltree::Document::parse(&text).expect("well-formed SVG");
        let root = doc.root_element();
        assert_eq!(root.tag_name().name(), "svg");
        assert_eq!(
            root.tag_name().namespace(),
            Some("http://www.w3.org/2000/svg")
        );
        assert_eq!(root.attribute("width"), Some("800"));
        let child = |name: &str| root.children().find(|n| n.has_tag_name(name)).expect(name);
        assert_eq!(
            child("title").text(),
            Some("ts-cyclic · Import dependencies")
        );
        assert_eq!(
            child("desc").text(),
            Some("Shown graph nodes & \"filters\" <all>.")
        );
        let background = child("rect");
        assert_eq!(background.attribute("width"), Some("800"));
        assert_eq!(background.attribute("style"), Some("fill:#101722"));
        let labels: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("text"))
            .filter_map(|n| n.text())
            .collect();
        assert_eq!(labels, ["src/a", "src/<b>"]);
        let paths: Vec<_> = root.children().filter(|n| n.has_tag_name("path")).collect();
        assert_eq!(paths.len(), 2);
        assert!(
            paths[0]
                .attribute("style")
                .unwrap()
                .contains("stroke:#FF6B61")
        );
        assert_eq!(paths[1].attribute("marker-end"), Some("url(#arrow-0)"));
        assert!(!text.contains("class="), "styles are inlined");
    }

    #[test]
    fn light_theme_changes_the_background() {
        let text = svg(&diagram(), Theme::of(false));
        assert!(text.contains("style=\"fill:#F5F7FB\""), "{text}");
    }
}
