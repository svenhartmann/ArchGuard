//! What View → Review decisions… shows of the review itself: the summary
//! and report page ([`review_findings::Overview`]) and the selected finding.

use std::rc::Rc;

use archguard_engine::ReviewFinding;
use slint::ComponentHandle;

use crate::review_findings::{self, Overview};
use crate::tab_support::model;
use crate::tree::Tree;
use crate::ui::{AppWindow, GapItem, NodeLink, ReviewDecisionsData, ReviewOverview};

pub fn overview(o: Overview) -> ReviewOverview {
    let gaps = o.gaps.into_iter().map(|(title, reason)| GapItem {
        title: title.into(),
        reason: reason.into(),
    });
    let strings = |v: Vec<String>| model(v.into_iter().map(Into::into));
    ReviewOverview {
        open: o.cards[0],
        risks: o.cards[1],
        gaps_count: o.cards[2],
        components: o.components.join(", ").into(),
        files: strings(o.files),
        tests: strings(o.tests),
        test_status: o.test_status.into(),
        gaps: model(gaps),
        origin: strings(o.origin),
        questions: strings(o.questions),
        decisions: strings(o.decisions),
        unmatched: o.unmatched,
        limits: strings(o.limits),
    }
}

/// The node named `name` in the active document's package tree.
fn node(name: &str) -> Option<usize> {
    crate::documents::with_active_state(|t: &mut Rc<Tree>| crate::instances::node_named(t, name))
        .flatten()
}

/// The selected finding's title, status, nodes (those the Graph has can be
/// opened) and evidence.
pub fn show_finding(ui: &AppWindow, f: &ReviewFinding) {
    let d = ui.global::<ReviewDecisionsData>();
    let meta = format!(
        "{} · {} · {} · {}",
        f.category, f.severity, f.change, f.disposition
    );
    d.set_sel_title(f.title.as_str().into());
    d.set_sel_meta(meta.into());
    d.set_sel_source(f.source_status.as_str().into());
    d.set_sel_evidence(review_findings::evidence(f).into());
    let decision = f.decision.as_ref().map(|x| {
        let expires = x.expires_on.as_deref().map(|e| format!(" · {e} UTC"));
        format!(
            "{:?} · {} · {}{}",
            x.decision,
            x.author,
            x.note,
            expires.unwrap_or_default()
        )
    });
    d.set_sel_decision(decision.unwrap_or_default().into());
    d.set_sel_nodes(model(f.nodes.iter().map(|n| NodeLink {
        name: n.as_str().into(),
        found: node(n).is_some(),
    })));
}

/// "Open in graph": the dialog steps aside (its session stays) and the
/// Graph shows `name`.
pub fn reveal(ui: &AppWindow, name: &str) {
    let Some(n) = node(name) else { return };
    ui.global::<ReviewDecisionsData>().set_open(false);
    crate::panes_ui::reveal(ui, n);
}
