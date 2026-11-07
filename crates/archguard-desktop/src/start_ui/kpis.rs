//! "Architecture at a glance" below the World: the port of
//! `renderStartKpis` (viewer_development.js) and `renderDashKpis`
//! (viewer_dash_kpis.js). The figures are the analysis' review KPIs
//! (`Graph::review_kpis`, as the CLI's report embeds them); a KPI the
//! analysis did not measure hides its card and is named as not measured,
//! never shown as zero. Texts are templates in `start_kpis.slint`.

use archguard_engine::Snapshot;
use archguard_engine::core::{
    AbstractionMetric, Finding, FindingKind, PropagationCost, ReviewKpis,
};
use slint::SharedString;

use super::kpis_profile::{self, Point, level, signed};
use super::kpis_zones;
use crate::format::Locale;
use crate::model_sync::sync_rows;
use crate::ui::{StartKpiCard, StartModel};

/// Card kinds shared with `start_kpis.slint`, in display order.
pub mod card {
    pub const HEALTH: i32 = 0;
    pub const PROPAGATION: i32 = 1;
    pub const SDP: i32 = 2;
    pub const TANGLED: i32 = 3;
    pub const GINI: i32 = 4;
    pub const DEBT: i32 = 5;
    pub const LEAKS: i32 = 6;
    pub const GAPS: i32 = 7;
}

/// What the section shows, copied from the analysis once.
#[derive(Debug, Clone, Default)]
pub struct Glance {
    pub kpis: Option<ReviewKpis>,
    /// Measured health scores, oldest first; unmeasured snapshots dropped.
    pub history: Vec<Point>,
    /// The first node of the top propagation-cost cycle, for its card.
    pub cycle_node: Option<String>,
    /// Measured node boundaries of the A/I scatter ([`kpis_zones`]).
    pub abstractions: Vec<AbstractionMetric>,
    /// Snapshot runs: scroll to the section, open the profile.
    pub reveal: bool,
    pub profile: bool,
}

impl Glance {
    pub fn new(kpis: Option<ReviewKpis>, findings: &[Finding], snapshots: &[Snapshot]) -> Self {
        let cycle_node = kpis.as_ref().and_then(top_cycle).and_then(|top| {
            findings
                .iter()
                .find(|f| f.kind == FindingKind::Cycle && f.id == top.scc_id)
                .and_then(|f| f.nodes.first().cloned())
        });
        let history = snapshots
            .iter()
            .filter_map(|s| {
                let score = s.review_kpis.as_ref()?.health.as_ref()?.score?;
                score.is_finite().then_some(Point {
                    timestamp: s.timestamp,
                    score,
                })
            })
            .collect();
        Glance {
            kpis,
            history,
            cycle_node,
            ..Default::default()
        }
    }
}

/// The highest-impact cycle: highest propagation cost, then lowest id.
fn top_cycle(k: &ReviewKpis) -> Option<&PropagationCost> {
    k.propagation_cost
        .iter()
        .max_by(|a, b| a.pc.total_cmp(&b.pc).then_with(|| b.scc_id.cmp(&a.scc_id)))
}

/// Fill the section of the model (the architecture World only).
pub fn fill(m: &mut StartModel, old: &StartModel, g: &Glance, project: &str, l: Locale) {
    let (cards, missing) = cards(g, l);
    let gl = &mut m.glance;
    gl.shown = true;
    gl.project = project.into();
    gl.measured = g.kpis.is_some();
    gl.reveal = g.reveal;
    gl.profile = g.profile;
    gl.cards = sync_rows(&old.glance.cards, cards);
    gl.missing = sync_rows(&old.glance.missing, missing);
    if let Some(k) = &g.kpis {
        kpis_profile::fill(gl, &old.glance, k, &g.history, l);
        kpis_zones::fill(gl, &old.glance, &g.abstractions, l);
    }
}

/// The measured cards in display order and the kinds without a measurement.
pub fn cards(g: &Glance, l: Locale) -> (Vec<StartKpiCard>, Vec<i32>) {
    let Some(k) = &g.kpis else {
        return (Vec::new(), Vec::new());
    };
    let all = [
        (card::HEALTH, health(k, l)),
        (card::PROPAGATION, Some(propagation(k, g, l))),
        (card::SDP, Some(sdp(k, l))),
        (card::TANGLED, tangled(k, l)),
        (card::GINI, gini(k, l)),
        (card::DEBT, debt(k, l)),
        (card::LEAKS, Some(leaks(k, l))),
        (card::GAPS, gaps(k, l)),
    ];
    let missing = all
        .iter()
        .filter(|(_, c)| c.is_none())
        .map(|(kind, _)| *kind)
        .collect();
    (all.into_iter().filter_map(|(_, c)| c).collect(), missing)
}

/// A card with its value and the arguments of its texts.
fn new(kind: i32, value: String, args: &[String]) -> StartKpiCard {
    let arg = |i: usize| SharedString::from(args.get(i).map_or("", String::as_str));
    StartKpiCard {
        kind,
        value: value.into(),
        a: arg(0),
        b: arg(1),
        c: arg(2),
        d: arg(3),
        ..Default::default()
    }
}

/// A fraction as a percentage with one decimal.
fn percent(l: Locale, v: f64) -> String {
    l.pct(v * 100.0, 1)
}

/// The architecture score out of 100 and its trend (`a`, "" none).
fn health(k: &ReviewKpis, l: Locale) -> Option<StartKpiCard> {
    let h = k.health.as_ref()?;
    let score = h.score.filter(|s| s.is_finite())?;
    let trend = h.trend.map_or(String::new(), |t| signed(l, t, 1));
    Some(StartKpiCard {
        level: level(h.level),
        ..new(card::HEALTH, l.dec(score, 1), &[trend])
    })
}

/// `a` the cycle, `b` of `c` nodes affected, `d` in the cycle.
fn propagation(k: &ReviewKpis, g: &Glance, l: Locale) -> StartKpiCard {
    let Some(top) = top_cycle(k) else {
        return StartKpiCard {
            empty: true,
            ..new(card::PROPAGATION, String::new(), &[])
        };
    };
    let args = [
        top.scc_id.clone(),
        l.count(top.affected),
        l.count(top.total),
        l.count(top.size),
    ];
    StartKpiCard {
        node: g.cycle_node.clone().unwrap_or_default().into(),
        ..new(card::PROPAGATION, percent(l, top.pc), &args)
    }
}

/// The count; `a` → `b` the first violation.
fn sdp(k: &ReviewKpis, l: Locale) -> StartKpiCard {
    let value = l.count(k.sdp_violations.len());
    match k.sdp_violations.first() {
        Some(v) => StartKpiCard {
            node: v.from.as_str().into(),
            ..new(card::SDP, value, &[v.from.clone(), v.to.clone()])
        },
        None => new(card::SDP, value, &[]),
    }
}

/// `a` of `b` nodes in a cyclic group.
fn tangled(k: &ReviewKpis, l: Locale) -> Option<StartKpiCard> {
    let t = k.tangledness.as_ref()?;
    let args = [l.count(t.tangled_nodes), l.count(t.total)];
    Some(new(card::TANGLED, percent(l, t.quote), &args))
}

/// `a` the share of the top-5 hubs.
fn gini(k: &ReviewKpis, l: Locale) -> Option<StartKpiCard> {
    let g = k.hub_gini.as_ref()?;
    let args = [percent(l, g.top_share)];
    Some(new(card::GINI, l.dec(g.gini, 2), &args))
}

/// `a` markers in `b` lines, `c` per kloc ("" not measured).
fn debt(k: &ReviewKpis, l: Locale) -> Option<StartKpiCard> {
    let d = k.debt.as_ref()?;
    let per_kloc = d.per_kloc.map_or(String::new(), |v| l.dec(v, 1));
    let args = [l.count(d.markers), l.count(d.loc), per_kloc];
    Some(new(card::DEBT, l.count(d.markers), &args))
}

/// The count; `a` reaches `b`, which `c` does not export.
fn leaks(k: &ReviewKpis, l: Locale) -> StartKpiCard {
    let value = l.count(k.leaks.len());
    match k.leaks.first() {
        Some(x) => StartKpiCard {
            node: x.from_file.as_str().into(),
            ..new(
                card::LEAKS,
                value,
                &[x.from_file.clone(), x.symbol.clone(), x.in_file.clone()],
            )
        },
        None => new(card::LEAKS, value, &[]),
    }
}

/// The count; `a` the hotspot, `b` uncovered ("" unknown), `c` the path.
fn gaps(k: &ReviewKpis, l: Locale) -> Option<StartKpiCard> {
    let value = l.count(k.risk_gap_count()?);
    Some(match k.risk_gaps.first() {
        Some(gap) => {
            let uncovered = gap.coverage.map_or(String::new(), |c| percent(l, 1.0 - c));
            let args = [l.dec(gap.hotspot, 1), uncovered, gap.path.clone()];
            StartKpiCard {
                node: gap.path.as_str().into(),
                ..new(card::GAPS, value, &args)
            }
        }
        None => new(card::GAPS, value, &[]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{
        DebtDensity, HealthIndex, HealthLevel, HubGini, RiskCoverageGap, SdpViolation, Tangledness,
    };

    fn kpis() -> ReviewKpis {
        ReviewKpis {
            propagation_cost: vec![
                PropagationCost {
                    scc_id: "scc-2".into(),
                    size: 2,
                    affected: 3,
                    total: 10,
                    pc: 0.3,
                },
                PropagationCost {
                    scc_id: "scc-1".into(),
                    size: 4,
                    affected: 3,
                    total: 10,
                    pc: 0.3,
                },
            ],
            sdp_violations: vec![SdpViolation {
                from: "core".into(),
                to: "ui".into(),
                ..Default::default()
            }],
            tangledness: Some(Tangledness {
                tangled_nodes: 6,
                total: 10,
                quote: 0.6,
            }),
            hub_gini: Some(HubGini {
                gini: 0.4567,
                top_share: 0.25,
            }),
            debt: Some(DebtDensity {
                markers: 1234,
                loc: 20_000,
                per_kloc: None,
                ..Default::default()
            }),
            risk_gaps: vec![RiskCoverageGap {
                path: "src/a.rs".into(),
                hotspot: 2.25,
                coverage: Some(0.75),
                gap: Some(0.5),
            }],
            health: Some(HealthIndex {
                score: Some(72.46),
                level: Some(HealthLevel::Yellow),
                trend: Some(-1.26),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn glance(k: Option<ReviewKpis>) -> Glance {
        Glance::new(k, &[], &[])
    }

    /// Without review KPIs the section has no card to show.
    #[test]
    fn no_kpis_means_no_cards() {
        assert_eq!(cards(&glance(None), Locale::En), (Vec::new(), Vec::new()));
    }

    /// Health leads; every measured card carries its formatted figure.
    #[test]
    fn measured_kpis_become_cards_in_display_order() {
        let (cards, missing) = cards(&glance(Some(kpis())), Locale::En);
        let kinds: Vec<i32> = cards.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(missing.is_empty());
        let value = |k: i32| cards[k as usize].value.to_string();
        assert_eq!(value(card::HEALTH), "72.5");
        assert_eq!(cards[0].a, "-1.3");
        assert_eq!(cards[0].level, 2);
        // Equal cost: the lower cycle id wins, as `compareNames`.
        assert_eq!(value(card::PROPAGATION), "30.0%");
        assert_eq!(cards[1].a, "scc-1");
        assert_eq!((cards[1].b.as_str(), cards[1].d.as_str()), ("3", "4"));
        assert_eq!(value(card::SDP), "1");
        assert_eq!(cards[2].node, "core");
        assert_eq!(value(card::TANGLED), "60.0%");
        assert_eq!(value(card::GINI), "0.46");
        assert_eq!(cards[4].a, "25.0%");
        assert_eq!(value(card::DEBT), "1,234");
        assert_eq!(cards[5].c, "", "per kloc not measured");
        assert_eq!(value(card::LEAKS), "0");
        assert!(!cards[6].empty, "an empty list is a recorded zero");
        assert_eq!(
            (cards[7].b.as_str(), cards[7].c.as_str()),
            ("25.0%", "src/a.rs")
        );
    }

    /// An unmeasured KPI hides its card and is listed as missing; an empty
    /// propagation list is the recorded state, not a missing measurement.
    #[test]
    fn unmeasured_kpis_hide_their_cards() {
        let k = ReviewKpis {
            health: Some(HealthIndex::default()),
            ..Default::default()
        };
        let (cards, missing) = cards(&glance(Some(k)), Locale::De);
        assert_eq!(
            missing,
            [
                card::HEALTH,
                card::TANGLED,
                card::GINI,
                card::DEBT,
                card::GAPS
            ]
        );
        let kinds: Vec<i32> = cards.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [card::PROPAGATION, card::SDP, card::LEAKS]);
        assert!(cards[0].empty);
        assert_eq!(cards[0].value, "");
    }

    fn gap(path: &str, coverage: Option<f64>) -> RiskCoverageGap {
        RiskCoverageGap {
            path: path.into(),
            hotspot: 1.0,
            coverage,
            gap: coverage.map(|c| 1.0 - c),
        }
    }

    /// Risk gaps count only files with a measured gap; without any
    /// coverage the card is not measured, never zero.
    #[test]
    fn risk_gaps_count_measured_gaps_only() {
        let only_unmeasured = ReviewKpis {
            risk_gaps: vec![gap("a.rs", None), gap("b.rs", None)],
            ..Default::default()
        };
        let (cards_none, missing) = cards(&glance(Some(only_unmeasured)), Locale::En);
        assert!(missing.contains(&card::GAPS));
        assert!(cards_none.iter().all(|c| c.kind != card::GAPS));

        let mixed = ReviewKpis {
            risk_gaps: vec![
                gap("a.rs", Some(0.0)),
                gap("b.rs", Some(1.0)),
                gap("c.rs", None),
            ],
            ..Default::default()
        };
        let (cards_mixed, missing) = cards(&glance(Some(mixed)), Locale::En);
        assert!(!missing.contains(&card::GAPS));
        let card = cards_mixed
            .iter()
            .find(|c| c.kind == card::GAPS)
            .expect("gaps card");
        assert_eq!(card.value, "1");
    }

    /// The top cycle's card opens the cycle's first node; the history keeps
    /// only measured scores, oldest first.
    #[test]
    fn glance_finds_the_cycle_node_and_the_measured_history() {
        let finding = Finding {
            kind: FindingKind::Cycle,
            id: "scc-1".into(),
            severity: archguard_engine::core::Severity::Warning,
            message: None,
            evidence: Vec::new(),
            nodes: vec!["a".into(), "b".into()],
            edge_count: 0,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: Vec::new(),
        };
        let snap = |timestamp: u64, score: Option<f64>| Snapshot {
            analysis_scope: None,
            schema_version: 1,
            timestamp,
            git_sha: "notag".into(),
            tag: None,
            nodes: 0,
            edges: 0,
            sccs: 0,
            largest_scc: 0,
            acd: 0.0,
            nccd: 0.0,
            cycle_hashes: Vec::new(),
            review_kpis: score.map(|score| ReviewKpis {
                health: Some(HealthIndex {
                    score: Some(score),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        let snaps = [snap(1, Some(50.0)), snap(2, None), snap(3, Some(60.0))];
        let g = Glance::new(Some(kpis()), &[finding], &snaps);
        assert_eq!(g.cycle_node.as_deref(), Some("a"));
        let scores: Vec<f64> = g.history.iter().map(|p| p.score).collect();
        assert_eq!(scores, [50.0, 60.0]);
        let (cards, _) = cards(&g, Locale::En);
        assert_eq!(cards[1].node, "a");
    }
}
