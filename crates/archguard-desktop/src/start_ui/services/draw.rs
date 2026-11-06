//! The connections of the services World in canvas pixels, as the
//! repository branch of `worldDraw`: the first 60 visible connections,
//! one path per tone, events dashed, and the curves the data flow dots
//! run along (`worldFlowPackets`; the dots move in Slint).

use archguard_engine::core::ServiceDependencyKind as Kind;

use super::super::arrows::{self, Point};
use super::links;
use super::world::{EdgeTone, ServicesWorld};

/// At most this many arrows, as the web.
pub const MAX_ARROWS: usize = 60;

/// Curves and heads of one tone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Strokes {
    pub curves: String,
    pub heads: String,
}

/// The curve of one call or event connection for its moving dots.
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub p: Point,
    pub c: Point,
    pub q: Point,
    pub length: f64,
    /// Start of the first dot along the curve, 0..1.
    pub offset: f64,
    pub count: i32,
    pub tone: EdgeTone,
}

/// Strokes per tone in [`EdgeTone`] order, and the flows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawn {
    pub strokes: [Strokes; 3],
    pub flows: Vec<Flow>,
}

pub fn tone_index(t: EdgeTone) -> usize {
    match t {
        EdgeTone::Cycle => 0,
        EdgeTone::Into => 1,
        EdgeTone::Other => 2,
    }
}

/// Draw the `visible` connections between the island centres `center`
/// gives per layout area.
pub fn draw(w: &ServicesWorld, visible: &[usize], center: impl Fn(usize) -> Point) -> Drawn {
    let mut out = Drawn::default();
    for (index, &i) in visible.iter().take(MAX_ARROWS).enumerate() {
        let l = &w.links[i];
        let (Some(a), Some(b)) = (w.area(l.from), w.area(l.to)) else {
            continue;
        };
        let (p, q) = (center(a), center(b));
        let tone = w.tone(l);
        let s = &mut out.strokes[tone_index(tone)];
        if l.kind == Kind::Event {
            arrows::dashed(p, q, 7.0, 5.0, &mut s.curves, &mut s.heads);
        } else {
            arrows::arrow(p, q, &mut s.curves, &mut s.heads);
        }
        if w.filter.flow && links::carrier(l.kind) {
            let length = (q.0 - p.0).hypot(q.1 - p.1).max(1.0);
            out.flows.push(Flow {
                p,
                c: arrows::control(p, q),
                q,
                length,
                offset: (index as f64 * 0.37).fract(),
                count: (length / 90.0).round().clamp(1.0, 4.0) as i32,
                tone,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::inventory::tests as inv;
    use super::super::links::Link;
    use super::super::world::ServicesWorld;
    use super::*;

    fn world() -> ServicesWorld {
        let projects = ["a", "b", "c"].map(|p| inv::project(p, "application"));
        let t = inv::topology(projects.to_vec(), Vec::new());
        let mut w = ServicesWorld::new(std::sync::Arc::new(Ok(t))).expect("three projects");
        let l = |from, to, kind, cyclic| Link {
            from,
            to,
            kind,
            evidence: Vec::new(),
            cyclic,
        };
        w.links = vec![
            l(0, 1, Kind::HttpCall, false),
            l(1, 2, Kind::Event, false),
            l(2, 0, Kind::ConfiguredReference, true),
        ];
        w
    }

    fn centre(area: usize) -> Point {
        (area as f64 * 300.0, 0.0)
    }

    /// Green without a selection, the accent into it, red for cycles;
    /// events dashed; dots only with the data flow on and only for
    /// calls and events, one per 90 px, at most four.
    #[test]
    fn tones_dashes_and_flows() {
        let mut w = world();
        let d = draw(&w, &[0, 1, 2], centre);
        assert!(d.flows.is_empty(), "data flow is off by default");
        assert_eq!(
            d.strokes[0].curves.matches('Q').count(),
            1,
            "the cyclic reference"
        );
        assert!(d.strokes[1].curves.is_empty());
        let other = &d.strokes[2];
        assert_eq!(other.curves.matches('Q').count(), 1, "the call");
        assert!(
            other.curves.matches('M').count() > 10,
            "the event, in dashes"
        );
        assert_eq!(other.heads.matches('Z').count(), 2);
        w.select_area(Some(1));
        w.filter.flow = true;
        let d = draw(&w, &[0, 1, 2], centre);
        assert_eq!(
            d.strokes[1].heads.matches('Z').count(),
            1,
            "the call into b"
        );
        let flows: Vec<_> = d
            .flows
            .iter()
            .map(|f| (f.count, f.tone, f.offset))
            .collect();
        assert_eq!(
            flows,
            [(3, EdgeTone::Into, 0.0), (3, EdgeTone::Other, 0.37)]
        );
        let long = draw(&w, &[0], |a| (a as f64 * 1000.0, 0.0));
        assert_eq!(long.flows[0].count, 4);
    }

    #[test]
    fn at_most_sixty_arrows() {
        let mut w = world();
        w.links = (0..70).map(|_| w.links[0].clone()).collect();
        let all: Vec<usize> = (0..70).collect();
        let d = draw(&w, &all, centre);
        assert_eq!(d.strokes[2].heads.matches('Z').count(), MAX_ARROWS);
    }
}
