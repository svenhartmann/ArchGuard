//! The event focus: published by → event → handled by, left to right, one
//! lane per service, so an arrow that leaves its lane crosses a service
//! boundary. Drawn on the Screens view's canvas (`FlowCanvas`).

use archguard_engine::core::domain_model::DomainModel;

use super::services::Services;
use super::tree::short;
use crate::diagram;
use crate::tab_support::Texts;
use crate::ui::{FlowArrow, FlowBand, FlowCard};

pub const W: f32 = 340.0;
pub const H: f32 = 120.0;
/// Characters of a title line at the card's width.
const LINE: usize = 32;
const GAP_X: f32 = 90.0;
const GAP_Y: f32 = 12.0;
const HEAD: f32 = 34.0;
const PAD: f32 = 24.0;
const LANE_GAP: f32 = 12.0;

/// A publisher or handler card.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub title: String,
    pub service: String,
    pub what: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Default)]
pub struct Lanes {
    pub cards: Vec<FlowCard>,
    pub arrows: Vec<FlowArrow>,
    pub bands: Vec<FlowBand>,
    pub width: f32,
    pub height: f32,
}

fn card(n: &Node, x: f32, y: f32, state: i32) -> FlowCard {
    FlowCard {
        kind: 0,
        state,
        title: two_lines(&n.title).into(),
        file: if n.file.is_empty() {
            Default::default()
        } else {
            format!("{}:{}", n.file, n.line).into()
        },
        detail: n.what.as_str().into(),
        x,
        y,
        w: W,
        h: H,
        screen: -1,
        group: -1,
        path: n.file.as_str().into(),
        line: n.line as i32,
        ..Default::default()
    }
}

/// `name` on at most two lines of [`LINE`] characters, split at a capital
/// near the middle (`OrderStatusChangedToPaid` / `IntegrationEventHandler`);
/// a longer name loses its middle, so both ends stay readable.
pub fn two_lines(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let chars: Vec<char> = if chars.len() > 2 * LINE {
        let tail = &chars[chars.len() - LINE..];
        (chars[..LINE - 1].iter().chain(['…'].iter()).chain(tail))
            .copied()
            .collect()
    } else {
        chars
    };
    let n = chars.len();
    if n <= LINE {
        return chars.into_iter().collect();
    }
    let at = (1..n)
        .filter(|&i| chars[i].is_uppercase() && i <= LINE && n - i <= LINE)
        .min_by_key(|&i| i.abs_diff(n - i))
        .unwrap_or(n / 2);
    let (a, b) = chars.split_at(at);
    format!(
        "{}\n{}",
        a.iter().collect::<String>(),
        b.iter().collect::<String>()
    )
}

/// The class of `Class.method`.
fn class(by: &str) -> &str {
    by.split_once('.').map_or(by, |(c, _)| c)
}

/// Lanes in order of first use: publishers, the event (in its first
/// publisher's lane), handlers. `""` is the lane of `unknown`.
pub fn layout(event: &Node, publishers: &[Node], handlers: &[Node], unknown: &str) -> Lanes {
    let mut lanes: Vec<&str> = Vec::new();
    for n in publishers
        .iter()
        .chain(std::iter::once(event))
        .chain(handlers)
    {
        if !lanes.contains(&n.service.as_str()) {
            lanes.push(&n.service);
        }
    }
    let x = [
        PAD + 16.0,
        PAD + 16.0 + W + GAP_X,
        PAD + 16.0 + 2.0 * (W + GAP_X),
    ];
    let mut l = Lanes {
        width: x[2] + W + PAD + 16.0,
        ..Lanes::default()
    };
    let mut top = PAD;
    let (mut from, mut to, mut at) = (vec![], vec![], (0.0, 0.0));
    for lane in lanes {
        let mut rows = [0usize; 3];
        let mut put = |col: usize, n: &Node, state: i32| {
            let y = top + HEAD + rows[col] as f32 * (H + GAP_Y);
            rows[col] += 1;
            l.cards.push(card(n, x[col], y, state));
            (x[col], y)
        };
        from.extend(
            publishers
                .iter()
                .filter(|n| n.service == lane)
                .map(|n| put(0, n, 4)),
        );
        if event.service == lane {
            at = put(1, event, 0);
        }
        to.extend(
            handlers
                .iter()
                .filter(|n| n.service == lane)
                .map(|n| put(2, n, 4)),
        );
        let h = HEAD + rows.iter().max().copied().unwrap_or(0) as f32 * (H + GAP_Y);
        l.bands.push(FlowBand {
            title: if lane.is_empty() { unknown } else { lane }.into(),
            y: top,
            h,
            collapsed: false,
        });
        top += h + LANE_GAP;
    }
    l.height = top + PAD;
    let mid = |(x, y): (f32, f32), right: bool| (if right { x + W } else { x }, y + H / 2.0);
    let arrow = |a, b| FlowArrow {
        path: diagram::curve(a, b).0.into(),
        unresolved: false,
    };
    l.arrows
        .extend(from.iter().map(|&p| arrow(mid(p, true), mid(at, false))));
    l.arrows
        .extend(to.iter().map(|&h| arrow(mid(at, true), mid(h, false))));
    l
}

/// The publish sites and the handlers of integration event `i`.
pub fn integration(d: &DomainModel, i: usize, texts: &Texts) -> Lanes {
    let e = &d.integration_events[i];
    let publishers: Vec<Node> = (e.published_by.iter())
        .map(|p| Node {
            title: class(&p.by).into(),
            service: p.service.clone(),
            what: texts.t("publishes").into(),
            file: p.file.clone(),
            line: p.line,
        })
        .collect();
    let handlers: Vec<Node> = (d.message_handlers.iter())
        .filter(|h| h.message == e.type_name)
        .map(|h| Node {
            title: h.id.clone(),
            service: h.service.clone(),
            what: if h.sends.is_empty() {
                texts.t("handles").into()
            } else {
                texts.f("handles · sends {what}", &[("what", h.sends.join(", "))])
            },
            file: h.file.clone(),
            line: h.line,
        })
        .collect();
    let event = Node {
        title: short(&e.type_name).into(),
        service: publishers
            .first()
            .or(handlers.first())
            .map_or(String::new(), |n| n.service.clone()),
        what: texts.t("integration event").into(),
        file: String::new(),
        line: 0,
    };
    layout(&event, &publishers, &handlers, texts.t("service not read"))
}

/// The raising methods and the handler classes of domain event `i`.
pub fn domain(d: &DomainModel, i: usize, services: &Services, texts: &Texts) -> Lanes {
    let e = &d.events[i];
    let transitions = || d.state_machines.iter().flat_map(|m| &m.transitions);
    let publishers: Vec<Node> = (e.raised_by.iter())
        .map(|m| {
            let (file, line) = (transitions().find(|t| t.method == *m))
                .map(|t| (t.file.clone(), t.line))
                .or_else(|| {
                    (d.entities.iter())
                        .find(|x| x.name == class(m))
                        .map(|x| (x.file.clone(), x.line))
                })
                .unwrap_or_default();
            Node {
                title: m.clone(),
                service: services.of(&file).into(),
                what: texts.t("raises").into(),
                file,
                line,
            }
        })
        .collect();
    let handlers: Vec<Node> = (e.handled_by.iter())
        .map(|h| handler(d, &e.type_name, h, services, texts))
        .collect();
    let event = Node {
        title: short(&e.type_name).into(),
        service: (publishers.first()).map_or(String::new(), |n| n.service.clone()),
        what: texts.t("domain event").into(),
        file: String::new(),
        line: 0,
    };
    layout(&event, &publishers, &handlers, texts.t("service not read"))
}

/// Handler class `h` of domain event `event`: placed by its declaration,
/// else by its integration-event publish site or its use case, else in the
/// lane "service not read".
fn handler(d: &DomainModel, event: &str, h: &str, services: &Services, texts: &Texts) -> Node {
    let publish = (d.integration_events.iter())
        .flat_map(|ie| ie.published_by.iter().map(move |p| (ie, p)))
        .find(|(_, p)| class(&p.by) == h);
    let what = match publish {
        Some((ie, _)) => {
            let ie = short(&ie.type_name).to_string();
            texts.f("handles · publishes {what}", &[("what", ie)])
        }
        None => texts.t("handles").into(),
    };
    let site = d.event_handler_sites(event).find(|s| s.handler == h);
    let use_case = d.use_cases.iter().find(|u| u.name == h);
    let (file, line, service) = match (site, publish, use_case) {
        (Some(s), _, _) => (s.file.clone(), s.line, services.of(&s.file).into()),
        (None, Some((_, p)), _) => (p.file.clone(), p.line, p.service.clone()),
        (None, None, Some(u)) => (u.file.clone(), u.line, services.of(&u.file).into()),
        (None, None, None) => (String::new(), 0, String::new()),
    };
    Node {
        title: h.into(),
        service,
        what,
        file,
        line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_ui::tree::tests::shop;

    fn n(title: &str, service: &str) -> Node {
        Node {
            title: title.into(),
            service: service.into(),
            what: "w".into(),
            file: format!("{title}.cs"),
            line: 1,
        }
    }

    #[test]
    fn each_service_gets_a_lane_and_the_event_sits_with_its_publisher() {
        let l = layout(
            &n("Paid", "Ordering"),
            &[n("P", "Ordering")],
            &[
                n("A", "Catalog"),
                n("B", "WebApp"),
                n("C", "Catalog"),
                n("D", ""),
            ],
            "service not read",
        );
        let bands: Vec<&str> = l.bands.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(bands, ["Ordering", "Catalog", "WebApp", "service not read"]);
        let at = |t: &str| {
            let c = l.cards.iter().find(|c| c.title == t).unwrap();
            let lane = l
                .bands
                .iter()
                .position(|b| c.y >= b.y && c.y < b.y + b.h)
                .unwrap();
            (lane, ((c.x - PAD - 16.0) / (W + GAP_X)).round() as usize)
        };
        assert_eq!(
            [at("P"), at("Paid"), at("A"), at("C"), at("B"), at("D")],
            [(0, 0), (0, 1), (1, 2), (1, 2), (2, 2), (3, 2)]
        );
        assert_eq!(l.arrows.len(), 5, "one in, four out");
        assert!(l.bands[1].h > l.bands[2].h, "two handlers in Catalog");
        let paid = l.cards.iter().find(|c| c.title == "Paid").unwrap();
        assert_eq!(paid.state, 0, "the event is the accent card");
    }

    #[test]
    fn integration_and_domain_events_find_their_cards() {
        let d = shop();
        let l = integration(&d, 0, &Texts::default());
        let titles: Vec<&str> = l.cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["P", "OrderPaid", "C"], "class names");
        assert_eq!(l.cards[2].file, "Catalog/C.cs:2");
        let services = Services::of_pairs(&[("Ordering/Order.cs", "Ordering")]);
        let l = domain(&d, 0, &services, &Texts::default());
        let bands: Vec<&str> = l.bands.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(
            bands,
            ["Ordering", "service not read"],
            "handler H has no file"
        );
        let mut d = d;
        d.event_handler_sites = vec![archguard_engine::core::domain_model::EventHandlerSite {
            event: "OrderShippedDomainEvent".into(),
            handler: "H".into(),
            file: "Ordering/H.cs".into(),
            line: 9,
        }];
        let services = Services::of_pairs(&[
            ("Ordering/Order.cs", "Ordering"),
            ("Ordering/H.cs", "Ordering"),
        ]);
        let l = domain(&d, 0, &services, &Texts::default());
        assert_eq!(l.bands.len(), 1, "the handler's declaration places it");
        assert_eq!(l.cards[2].file, "Ordering/H.cs:9");
    }

    #[test]
    fn long_names_wrap_at_a_capital_and_lose_their_middle_only_when_too_long() {
        assert_eq!(two_lines("PayHandler"), "PayHandler");
        assert_eq!(
            two_lines("OrderStatusChangedToPaidIntegrationEventHandler"),
            "OrderStatusChangedToPaid\nIntegrationEventHandler"
        );
        let long = "OrderStatusChangedToAwaitingValidationIntegrationEventHandlerForWebhooks";
        let t = two_lines(long);
        assert!(t.contains('…') && t.ends_with("ForWebhooks"), "{t}");
        assert!(t.lines().all(|l| l.chars().count() <= LINE), "{t}");
    }
}
