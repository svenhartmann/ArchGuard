//! Notes for the coding agent on gaps, mismatches and undeclared
//! endpoints. A note is a decision of the review register
//! (`.archguard/reviews.json`, written as the MCP `review_decisions` tool
//! records one) whose finding is the item's stable id, the id `archguard
//! check` gives the same item as a convention finding (ADR 0021,
//! `view-<rule>:<file>:<subject>`, repeated ids numbered `#2`, `#3` in
//! finding order). A gap that is no unreadable navigation or call gets
//! `view-gap:<file>:<message>`; no convention reports it.
//!
//! Fix is "fix", intentional is accept-risk, later is defer. The suggested
//! change goes into the note after [`CHANGE`], since the register has no
//! field of its own for it.

use std::collections::BTreeMap;
use std::path::Path;

use archguard_engine::core::{
    AccessState, Call, DecisionKind, ReviewDecision, Target, UiFlow, UiFlowDiagnostic,
    UiFlowFinding, UiFlowFindingKind,
};

/// The note's kinds in the form's order: fix, intentional, later.
pub const KINDS: [DecisionKind; 3] = [
    DecisionKind::Fix,
    DecisionKind::AcceptRisk,
    DecisionKind::Defer,
];

/// Separates the free text from the suggested change in the stored note.
pub const CHANGE: &str = "\n\nSuggested change:\n";

/// What the form holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Form {
    /// Index into [`KINDS`].
    pub kind: usize,
    pub text: String,
    pub change: String,
    pub adr: String,
}

pub fn id(rule: &str, file: &str, subject: &str) -> String {
    format!("view-{rule}:{file}:{subject}")
}

/// `#2`, `#3` … on repeated ids, in order, as `check` numbers them.
pub fn numbered(ids: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    ids.map(|id| {
        let n = seen.entry(id.clone()).or_default();
        *n += 1;
        if *n > 1 { format!("{id}#{n}") } else { id }
    })
    .collect()
}

/// The ids of the flow's items, as `check` names them.
pub struct Ids {
    /// Per transition: its `navigation_literal` id when unresolved.
    transitions: Vec<Option<String>>,
    /// Per action: its `http_url_literal` id when unresolved.
    actions: Vec<Option<String>>,
    /// Per endpoint: its `endpoint_access_declared` id when undeclared.
    endpoints: Vec<Option<String>>,
    /// Per finding of `ui_flow_findings`: its `frontend_backend_agree` id.
    findings: Vec<Option<String>>,
}

/// Number the `Some` entries among themselves, keep the `None`s.
fn number_some(items: Vec<Option<String>>) -> Vec<Option<String>> {
    let mut ids = numbered(items.iter().flatten().cloned()).into_iter();
    (items.into_iter())
        .map(|x| x.and_then(|_| ids.next()))
        .collect()
}

impl Ids {
    pub fn of(flow: &UiFlow, findings: &[UiFlowFinding]) -> Ids {
        let transitions = (flow.transitions.iter())
            .map(|t| match &t.target {
                Target::Unresolved(text) => Some(id("navigation_literal", &t.file, text)),
                Target::Screen(_) => None,
            })
            .collect();
        let actions = (flow.actions.iter())
            .map(|a| match &a.call {
                Call::Unresolved(text) => Some(id("http_url_literal", &a.file, text)),
                _ => None,
            })
            .collect();
        let endpoints = (flow.endpoints.iter())
            .map(|e| {
                let subject = format!("{} {}", e.method, e.path);
                (e.access == AccessState::Undeclared)
                    .then(|| id("endpoint_access_declared", &e.file, &subject))
            })
            .collect();
        let findings = findings.iter().map(finding_id).collect();
        Ids {
            transitions: number_some(transitions),
            actions: number_some(actions),
            endpoints: number_some(endpoints),
            findings: number_some(findings),
        }
    }

    pub fn endpoint(&self, i: usize) -> Option<&str> {
        self.endpoints.get(i)?.as_deref()
    }

    pub fn finding(&self, i: usize) -> Option<&str> {
        self.findings.get(i)?.as_deref()
    }

    /// A gap's id: the unreadable navigation or call at its place, else
    /// `view-gap:<file>:<message>`.
    pub fn gap(&self, flow: &UiFlow, d: &UiFlowDiagnostic) -> String {
        let at = |file: &str, line: u32| file == d.file && line == d.line;
        let transition = (flow.transitions.iter().zip(&self.transitions))
            .find(|(t, id)| id.is_some() && at(&t.file, t.line))
            .and_then(|(_, id)| id.clone());
        let action = || {
            (flow.actions.iter().zip(&self.actions))
                .find(|(a, id)| id.is_some() && at(&a.file, a.line))
                .and_then(|(_, id)| id.clone())
        };
        transition
            .or_else(action)
            .unwrap_or_else(|| id("gap", &d.file, &d.message))
    }
}

/// The `frontend_backend_agree` id of a mismatch finding.
fn finding_id(f: &UiFlowFinding) -> Option<String> {
    let kind = match f.kind {
        UiFlowFindingKind::ShownButRefused => "shown_but_refused",
        UiFlowFindingKind::HiddenButUnprotected => "hidden_but_unprotected",
        _ => return None,
    };
    let action = f.action.as_ref()?;
    let endpoint = (f.endpoints.first()).map(|e| format!(" {} {}", e.method, e.path));
    let subject = format!("{kind}{}", endpoint.unwrap_or_default());
    Some(id("frontend_backend_agree", &action.file, &subject))
}

/// The register's decisions by finding; empty when it cannot be read.
pub fn load(root: &Path) -> BTreeMap<String, ReviewDecision> {
    let register = archguard_engine::load_register(root).unwrap_or_default();
    (register.decisions.into_iter())
        .map(|d| (d.finding_ref.clone(), d))
        .collect()
}

/// The badge of a note: "noted: fix", "accepted (intentional)" (as `check`
/// accepts the finding), "noted: later".
pub fn badge(d: &ReviewDecision) -> &'static str {
    match d.decision {
        DecisionKind::Fix => "noted: fix",
        DecisionKind::AcceptRisk => "accepted (intentional)",
        DecisionKind::Defer => "noted: later",
    }
}

/// The form for a stored note, else an empty one.
pub fn form(d: Option<&ReviewDecision>) -> Form {
    let Some(d) = d else {
        return Form::default();
    };
    let (text, change) = d.note.split_once(CHANGE).unwrap_or((&d.note, ""));
    Form {
        kind: KINDS.iter().position(|k| *k == d.decision).unwrap_or(0),
        text: text.to_string(),
        change: change.to_string(),
        adr: d.adr_link.clone().unwrap_or_default(),
    }
}

/// [`save`] by the git author (else "desktop") at the current time, with
/// the project's ADR gate; the notes as read again.
pub fn save_here(
    root: &Path,
    config: Option<&archguard_engine::core::Config>,
    finding: &str,
    form: &Form,
) -> Result<BTreeMap<String, ReviewDecision>, String> {
    let author = crate::review_decisions::git_author(root);
    let author = if author.is_empty() {
        "desktop".into()
    } else {
        author
    };
    let gate = config.is_some_and(|c| c.review.adr_required_on_accept_risk);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    save(root, finding, form, &author, gate, now)?;
    Ok(load(root))
}

/// Record the note on `finding` in the register (upsert by id). With
/// `adr_required` (`[review] adr_required_on_accept_risk`), intentional
/// needs an ADR link, as for the MCP tool.
pub fn save(
    root: &Path,
    finding: &str,
    form: &Form,
    author: &str,
    adr_required: bool,
    now: u64,
) -> Result<(), String> {
    let decision = KINDS.get(form.kind).copied().unwrap_or_default();
    let adr = form.adr.trim();
    if decision == DecisionKind::AcceptRisk && adr.is_empty() && adr_required {
        return Err("This project requires an ADR link for an intentional decision ([review] adr_required_on_accept_risk).".into());
    }
    let mut note = form.text.trim().to_string();
    if !form.change.trim().is_empty() {
        note = format!("{note}{CHANGE}{}", form.change.trim());
    }
    let d = ReviewDecision {
        id: finding.to_string(),
        finding_ref: finding.to_string(),
        decision,
        adr_link: (!adr.is_empty()).then(|| adr.to_string()),
        note,
        author: author.to_string(),
        ts: archguard_engine::iso_from_unix(now),
        ..Default::default()
    };
    archguard_engine::upsert_decision(root, d)
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{
        Action, DiagnosticSeverity, EndpointAccess, Origin, Severity, Transition, UiFlowActionRef,
        UiFlowEndpointRef,
    };

    fn flow() -> UiFlow {
        let go = |line, target| Transition {
            weak: false,
            from: Origin::Screen("home".into()),
            target,
            trigger: None,
            file: "Home.razor".into(),
            line,
            condition: archguard_engine::core::AccessState::Undeclared,
            source: "t".into(),
        };
        let x = || Target::Unresolved("Url(x)".into());
        let gap = |line, message: &str| UiFlowDiagnostic {
            file: "Home.razor".into(),
            line,
            message: message.into(),
            severity: DiagnosticSeverity::Gap,
        };
        UiFlow {
            transitions: vec![go(3, x()), go(5, Target::Screen("a".into())), go(9, x())],
            actions: vec![Action {
                weak: false,
                screen: Origin::Screen("home".into()),
                trigger: None,
                call: Call::Unresolved("base + id".into()),
                file: "Home.razor".into(),
                line: 12,
                condition: archguard_engine::core::AccessState::Undeclared,
                source: "t".into(),
            }],
            endpoints: vec![EndpointAccess {
                method: "POST".into(),
                path: "/api/x".into(),
                access: AccessState::Undeclared,
                application: None,
                file: "Api.cs".into(),
                line: 4,
                source: "t".into(),
                handler: None,
                sends: vec![],
            }],
            diagnostics: vec![
                gap(9, "target not literal"),
                gap(12, "url"),
                gap(20, "role?"),
            ],
            ..UiFlow::default()
        }
    }

    #[test]
    fn ids_are_the_ones_check_gives_the_same_items() {
        let f = flow();
        let mismatch = UiFlowFinding {
            severity: Severity::Warning,
            kind: UiFlowFindingKind::HiddenButUnprotected,
            application: None,
            action: Some(UiFlowActionRef {
                file: "Cart.razor".into(),
                line: 7,
                trigger: None,
            }),
            role: None,
            endpoints: vec![UiFlowEndpointRef {
                method: "POST".into(),
                path: "/api/x".into(),
                file: "Api.cs".into(),
                line: 4,
                application: None,
            }],
            message: "m".into(),
        };
        let ids = Ids::of(&f, std::slice::from_ref(&mismatch));
        let gaps: Vec<String> = f.diagnostics.iter().map(|d| ids.gap(&f, d)).collect();
        assert_eq!(
            gaps,
            [
                "view-navigation_literal:Home.razor:Url(x)#2",
                "view-http_url_literal:Home.razor:base + id",
                "view-gap:Home.razor:role?",
            ],
            "the second unreadable Url(x) of the file is #2"
        );
        assert_eq!(
            ids.endpoint(0),
            Some("view-endpoint_access_declared:Api.cs:POST /api/x")
        );
        assert_eq!(
            ids.finding(0),
            Some("view-frontend_backend_agree:Cart.razor:hidden_but_unprotected POST /api/x")
        );
    }

    #[test]
    fn a_note_round_trips_through_the_register() {
        let dir = tempfile::tempdir().expect("tempdir");
        let form = Form {
            kind: 1,
            text: " Checked by hand in the handler ".into(),
            change: "Add [Authorize] to OrdersApi".into(),
            adr: String::new(),
        };
        let gate = save(dir.path(), "view-x:a:b", &form, "Sven", true, 0);
        assert!(gate.expect_err("needs an ADR").contains("ADR"));
        save(dir.path(), "view-x:a:b", &form, "Sven", false, 0).expect("saved");
        let notes = load(dir.path());
        let d = &notes["view-x:a:b"];
        assert_eq!(d.decision, DecisionKind::AcceptRisk);
        assert_eq!(badge(d), "accepted (intentional)");
        assert_eq!(
            d.note,
            "Checked by hand in the handler\n\nSuggested change:\nAdd [Authorize] to OrdersApi"
        );
        assert_eq!(
            form_of(&notes),
            Form {
                text: "Checked by hand in the handler".into(),
                ..form
            }
        );
        let later = Form {
            kind: 2,
            text: "after the release".into(),
            ..Form::default()
        };
        save(dir.path(), "view-x:a:b", &later, "Sven", false, 0).expect("replaced");
        assert_eq!(load(dir.path()).len(), 1, "one note per item");
        assert_eq!(badge(&load(dir.path())["view-x:a:b"]), "noted: later");
    }

    fn form_of(notes: &BTreeMap<String, ReviewDecision>) -> Form {
        form(notes.get("view-x:a:b"))
    }
}
