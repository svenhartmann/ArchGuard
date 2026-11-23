//! Convention checks for the UI-flow and domain views (ADR 0021 decision 6,
//! ADR 0022 decision 6): the opt-in `[views] conventions` of `check`. Each
//! broken convention is one `convention` error finding with file:line
//! and a fix hint for the agent that wrote the code. The id is
//! `view-<rule>:<file>:<subject>` (`#2`, `#3` … for repeats in one file), so
//! it survives moved lines.
//!
//! - `navigation_literal`: a transition whose target stayed `Unresolved`.
//! - `endpoint_access_declared`: an `Undeclared` endpoint.
//! - `screen_access_declared`: an `Undeclared` screen (not `Open`, not protected).
//! - `http_url_literal`: a `Call::Unresolved` action.
//! - `frontend_backend_agree`: a `shown_but_refused` or
//!   `hidden_but_unprotected` finding of [`ui_flow_findings`].
//! - `navigation_resolves`: a navigation target that is readable but matches
//!   no route of its application (the extractors' "matches no route" /
//!   "is not a route" diagnostics, so the route matching is theirs); not for an
//!   application with no route read at all, nor for non-path expressions.
//! - `endpoint_called`: an endpoint no action of [`join_actions_to_endpoints`]
//!   hits, only when the flow has a frontend (a screen or an action). No
//!   endpoint can be marked open-for-external-use yet. `ANY` (MVC page) routes
//!   are skipped; calls without a literal URL are named in the message.
//! - `state_changes_in_entity`: in a C# file other than the entity's,
//!   `x.Field = Enum.Value` for a state field of a found state machine; and
//!   every non-literal state assignment the domain extractor reports.
//! - `events_through_raise_function`: `new X(…)` of a domain-event type (a C#
//!   class/record named `*DomainEvent` or listing `INotification` as a base)
//!   that is neither an `AddDomainEvent(…)` argument nor assigned to a
//!   variable that the same file passes to `AddDomainEvent`; and every raise
//!   argument the domain extractor could not read.
//!
//! The C# scan is text-level like `domain_csharp.rs`: comments and string
//! contents are blanked first. Not covered: Java/Kotlin domain code, object
//! initializers (`new Order { OrderStatus = … }`), qualified enum values.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::domain_model::DomainModel;
use archguard_core::{
    AccessState, Call, Config, Finding, Target, UiFlow, UiFlowFindingKind, ui_flow_findings,
};

use archguard_flow_common::extract_ctx::{ExtractContext, domain_sources};
use csharp::{
    RAISE_FUNCTION, blank, constructions, event_types, line, member_assignments, passed_to_raise,
    raise_arguments,
};

#[path = "view_conventions_csharp.rs"]
mod csharp;
#[path = "view_conventions_links.rs"]
mod links;
use links::finding;

const UI_RULES: [&str; 7] = [
    "navigation_literal",
    "endpoint_access_declared",
    "screen_access_declared",
    "http_url_literal",
    "frontend_backend_agree",
    "navigation_resolves",
    "endpoint_called",
];
const STATE: &str = "state_changes_in_entity";
const EVENTS: &str = "events_through_raise_function";

/// The findings of every convention in `[views] conventions`; none when the
/// list is empty, and then nothing is read.
pub fn convention_findings(root: &Path, config: &Config) -> Vec<Finding> {
    convention_findings_in(root, config, None)
}

/// [`convention_findings`]; with `allowed_root` (the MCP server root) a
/// `[views] backends` path outside it is not read.
pub fn convention_findings_in(
    root: &Path,
    config: &Config,
    allowed_root: Option<&Path>,
) -> Vec<Finding> {
    let on: BTreeSet<&str> = config.views.conventions.iter().map(|c| c.name()).collect();
    let mut out = Vec::new();
    let mut ctx = ExtractContext::new(root, config);
    ctx.allowed_root = allowed_root;
    if UI_RULES.iter().any(|r| on.contains(r)) {
        match crate::ui_flow::analyze_with(&ctx) {
            Ok(flow) => out.extend(ui_findings(root, &flow, &on)),
            Err(e) => out.push(unreadable("ui-flow", e)),
        }
    }
    if on.contains(STATE) || on.contains(EVENTS) {
        match crate::domain_model::analyze_with(&ctx) {
            Ok(model) => {
                let sources = csharp_sources(root);
                if on.contains(STATE) {
                    out.extend(state_findings(&model, &sources));
                }
                if on.contains(EVENTS) {
                    out.extend(event_findings(&model, &sources));
                }
            }
            Err(e) => out.push(unreadable("domain-model", e)),
        }
    }
    distinct_ids(out)
}

type UiRule = fn(&UiFlow) -> Vec<Finding>;

fn ui_findings(root: &Path, flow: &UiFlow, on: &BTreeSet<&str>) -> Vec<Finding> {
    let rules: [(&str, UiRule); 5] = [
        ("navigation_literal", navigation_literal),
        ("endpoint_access_declared", endpoint_access_declared),
        ("screen_access_declared", screen_access_declared),
        ("http_url_literal", http_url_literal),
        ("frontend_backend_agree", frontend_backend_agree),
    ];
    let mut out: Vec<Finding> = rules
        .into_iter()
        .filter(|(rule, _)| on.contains(rule))
        .flat_map(|(_, check)| check(flow))
        .collect();
    if on.contains("navigation_resolves") {
        out.extend(links::navigation_resolves(root, flow));
    }
    if on.contains("endpoint_called") {
        out.extend(links::endpoint_called(root, flow));
    }
    out
}

fn navigation_literal(flow: &UiFlow) -> Vec<Finding> {
    let unresolved = flow.transitions.iter().filter_map(|t| match &t.target {
        Target::Unresolved(text) => Some((t, text)),
        Target::Screen(_) => None,
    });
    unresolved.map(|(t, text)| finding("navigation_literal", &t.file, t.line, text, format!(
        "Navigation target `{text}` is not a literal, so the UI flow cannot follow it. Fix: write the target as a literal route or a route constant."
    ))).collect()
}

fn endpoint_access_declared(flow: &UiFlow) -> Vec<Finding> {
    let undeclared = flow
        .endpoints
        .iter()
        .filter(|e| e.access == AccessState::Undeclared);
    undeclared.map(|e| {
        let subject = format!("{} {}", e.method, e.path);
        finding("endpoint_access_declared", &e.file, e.line, &subject, format!(
            "Endpoint {subject} declares no access, so nobody can tell who may call it. Fix: declare [Authorize]/[AllowAnonymous] (ASP.NET) or @PreAuthorize/@PermitAll (Spring) on the endpoint or its controller."
        ))
    }).collect()
}

fn screen_access_declared(flow: &UiFlow) -> Vec<Finding> {
    let undeclared = flow.screens.iter().filter(|s| s.access.is_undeclared());
    undeclared.map(|s| {
        let subject = s.route.clone().unwrap_or_else(|| s.name.clone());
        finding("screen_access_declared", &s.file, s.line, &subject, format!(
            "Screen {subject} declares no access, so nobody can tell who may open it. Fix: declare its guard (route guard, [Authorize]/[AllowAnonymous]) or map it under [views.access]."
        ))
    }).collect()
}

fn http_url_literal(flow: &UiFlow) -> Vec<Finding> {
    let unresolved = flow.actions.iter().filter_map(|a| match &a.call {
        Call::Unresolved(text) => Some((a, text)),
        _ => None,
    });
    unresolved.map(|(a, text)| finding("http_url_literal", &a.file, a.line, text, format!(
        "HTTP call `{text}` has no literal URL, so it joins no backend endpoint. Fix: write the URL as a literal path or a constant."
    ))).collect()
}

fn frontend_backend_agree(flow: &UiFlow) -> Vec<Finding> {
    let mut out = Vec::new();
    for f in ui_flow_findings(flow) {
        let (kind, hint) = match f.kind {
            UiFlowFindingKind::ShownButRefused => (
                "shown_but_refused",
                "hide the action from users the backend refuses, or let the backend allow them",
            ),
            UiFlowFindingKind::HiddenButUnprotected => (
                "hidden_but_unprotected",
                "declare the frontend's guard on the endpoint too",
            ),
            _ => continue,
        };
        let Some(action) = &f.action else { continue };
        let endpoint = (f.endpoints.first()).map(|e| format!(" {} {}", e.method, e.path));
        let subject = format!("{kind}{}", endpoint.unwrap_or_default());
        let message = format!("{} Fix: {hint}.", f.message);
        out.push(finding(
            "frontend_backend_agree",
            &action.file,
            action.line,
            &subject,
            message,
        ));
    }
    out
}

/// `(relative path, text with comments and string contents blanked)` of the
/// C# files the domain extractor reads, sorted.
fn csharp_sources(root: &Path) -> Vec<(String, String)> {
    // Unreadable or oversized files are skipped: the domain extractor already
    // reports them as diagnostics.
    let (sources, _) = domain_sources(root, &["bin", "obj"], |r| r.ends_with(".cs"), "C#");
    (sources.into_iter())
        .map(|(rel, text)| (rel, blank(&text)))
        .collect()
}

fn state_findings(model: &DomainModel, sources: &[(String, String)]) -> Vec<Finding> {
    let mut out = Vec::new();
    for m in &model.state_machines {
        let own = model
            .entities
            .iter()
            .find(|e| e.id == m.entity)
            .map(|e| e.file.as_str());
        for (rel, text) in sources.iter().filter(|(rel, _)| Some(rel.as_str()) != own) {
            for (pos, value) in member_assignments(text, &m.field, &m.enum_type) {
                let subject = format!("{}.{} = {}.{value}", m.entity, m.field, m.enum_type);
                out.push(finding(STATE, rel, line(text, pos), &subject, format!(
                    "`{subject}` changes the state outside the entity, so the state machine of {} does not show it. Fix: call a method of {} that makes this change, or add one.",
                    m.entity, m.entity
                )));
            }
        }
    }
    for d in model
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("the non-literal"))
    {
        out.push(finding(STATE, &d.file, d.line, &d.message, format!(
            "{} Fix: assign the state as an enum literal (`Field = Enum.Value;`) inside a method of its entity, one method per target state.",
            d.message
        )));
    }
    out
}

fn event_findings(model: &DomainModel, sources: &[(String, String)]) -> Vec<Finding> {
    let types: BTreeSet<&str> = sources
        .iter()
        .flat_map(|(_, text)| event_types(text))
        .collect();
    let mut out = Vec::new();
    for (rel, text) in sources {
        let raised = raise_arguments(text);
        for (pos, name) in constructions(text) {
            if types.contains(name) && !passed_to_raise(text, pos, &raised) {
                out.push(finding(EVENTS, rel, line(text, pos), name, format!(
                    "`new {name}(…)` creates a domain event that does not go through {RAISE_FUNCTION}, so the domain view does not see who raises it. Fix: pass it to {RAISE_FUNCTION}(…) in a method of the entity that raises it."
                )));
            }
        }
    }
    for d in model
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("argument is neither"))
    {
        out.push(finding(EVENTS, &d.file, d.line, &d.message, format!(
            "{} Fix: pass `new X(…)` or a variable assigned `new X(…)` in the same method to {RAISE_FUNCTION}.",
            d.message
        )));
    }
    out
}

fn unreadable(what: &str, error: impl std::fmt::Display) -> Finding {
    finding(
        "conventions",
        "",
        0,
        what,
        format!(
            "The {what} view could not be read, so its conventions are unchecked: {error}. Fix: repair the import file."
        ),
    )
}

/// Appends `#2`, `#3` … to repeated ids, in finding order.
fn distinct_ids(mut findings: Vec<Finding>) -> Vec<Finding> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for f in &mut findings {
        let n = seen.entry(f.id.clone()).or_default();
        *n += 1;
        if *n > 1 {
            f.id = format!("{}#{n}", f.id);
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{FindingKind, Severity};

    const RULES: [&str; 9] = [
        "navigation_literal",
        "endpoint_access_declared",
        "screen_access_declared",
        "http_url_literal",
        "frontend_backend_agree",
        "state_changes_in_entity",
        "events_through_raise_function",
        "navigation_resolves",
        "endpoint_called",
    ];

    fn write(dir: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    /// `[views]` with `rules`, parsed the way `check` loads it.
    fn config(dir: &Path, rules: &[&str]) -> Result<Config, String> {
        let list: Vec<String> = rules.iter().map(|r| format!("{r:?}")).collect();
        let toml = format!("[views]\nconventions = [{}]\n", list.join(", "));
        write(dir, &[(".archguard/archguard.toml", &toml)]);
        crate::config::load_config(None, dir)
            .map(|loaded| loaded.config)
            .map_err(|e| e.to_string())
    }

    fn run(dir: &Path, rules: &[&str]) -> Vec<Finding> {
        convention_findings(dir, &config(dir, rules).unwrap())
    }

    /// Every element breaks a UI-flow convention once.
    const BROKEN_FLOW: &str = r#"{"definition":"ui-flow/v1","complete":false,
"screens":[
 {"id":"home","name":"Home","kind":"page","route":"/","application":"web","file":"Web/Home.razor","line":1,"source":"x"},
 {"id":"admin","name":"Admin","kind":"page","route":"/admin","application":"web","access":"authenticated","file":"Web/Admin.razor","line":1,"source":"x"}],
"transitions":[
 {"from":{"screen":"home"},"target":{"screen":"admin"},"file":"Web/Home.razor","line":2,"source":"x"},
 {"from":{"screen":"home"},"target":{"unresolved":"returnUrl"},"file":"Web/Home.razor","line":3,"source":"x"}],
"actions":[
 {"screen":{"screen":"home"},"call":{"http":{"method":"GET","path":"/api/orders"}},"file":"Web/Home.razor","line":4,"source":"x"},
 {"screen":{"screen":"home"},"call":{"unresolved":"ItemUrl(Id)"},"file":"Web/Home.razor","line":5,"source":"x"},
 {"screen":{"screen":"admin"},"call":{"http":{"method":"POST","path":"/api/admin"}},"file":"Web/Admin.razor","line":6,"source":"x"}],
"endpoints":[
 {"method":"GET","path":"/api/orders","access":{"protected":{"role":"Clerk"}},"file":"Api/Orders.cs","line":7,"source":"x"},
 {"method":"POST","path":"/api/admin","access":"open","file":"Api/Admin.cs","line":8,"source":"x"},
 {"method":"DELETE","path":"/api/items/{id}","access":"undeclared","file":"Api/Items.cs","line":9,"source":"x"}]}"#;

    /// The same application with every convention kept.
    const KEPT_FLOW: &str = r#"{"definition":"ui-flow/v1","complete":true,
"screens":[
 {"id":"home","name":"Home","kind":"page","route":"/","application":"web","access":"open","file":"Web/Home.razor","line":1,"source":"x"},
 {"id":"admin","name":"Admin","kind":"page","route":"/admin","application":"web","access":"authenticated","file":"Web/Admin.razor","line":1,"source":"x"}],
"transitions":[
 {"from":{"screen":"home"},"target":{"screen":"admin"},"file":"Web/Home.razor","line":2,"source":"x"}],
"actions":[
 {"screen":{"screen":"home"},"call":{"http":{"method":"GET","path":"/api/orders"}},"file":"Web/Home.razor","line":4,"source":"x"},
 {"screen":{"screen":"admin"},"call":{"http":{"method":"POST","path":"/api/admin"}},"file":"Web/Admin.razor","line":6,"source":"x"}],
"endpoints":[
 {"method":"GET","path":"/api/orders","access":"open","file":"Api/Orders.cs","line":7,"source":"x"},
 {"method":"POST","path":"/api/admin","access":{"protected":"authenticated"},"file":"Api/Admin.cs","line":8,"source":"x"}]}"#;

    const ORDER: &str = r#"namespace Shop.Domain;
public enum OrderStatus { Submitted, Paid, Shipped }
public class Order : Entity, IAggregateRoot
{
    public OrderStatus OrderStatus { get; private set; }
    public Order() { OrderStatus = OrderStatus.Submitted; }
    public void SetPaid()
    {
        if (OrderStatus != OrderStatus.Submitted) throw new InvalidOperationException();
        OrderStatus = OrderStatus.Paid;
        AddDomainEvent(new OrderPaidDomainEvent(this));
    }
    public void SetShipped()
    {
        var shipped = new OrderShippedDomainEvent(this);
        OrderStatus = OrderStatus.Shipped;
        AddDomainEvent(shipped);
    }
}
public record OrderPaidDomainEvent(Order Order) : INotification;
public record OrderShippedDomainEvent(Order Order) : INotification;
public class StockReserved : INotification { }
"#;

    /// Changes the state and creates events past the entity.
    const BROKEN_HANDLER: &str = r#"namespace Shop.App;
public class ShipOrderCommandHandler
{
    // order.OrderStatus = OrderStatus.Paid; in a comment is no assignment
    public async Task Handle(Order order, IMediator mediator)
    {
        order.OrderStatus = OrderStatus.Shipped;
        await mediator.Publish(new OrderShippedDomainEvent(order));
        var text = "new StockReserved()";
        await mediator.Publish(new StockReserved { });
    }
}
"#;

    const KEPT_HANDLER: &str = r#"namespace Shop.App;
public class ShipOrderCommandHandler
{
    public void Handle(Order order)
    {
        if (order.OrderStatus == OrderStatus.Paid) order.SetShipped();
    }
}
"#;

    fn ids_and_places(findings: &[Finding]) -> Vec<(String, String, u32)> {
        findings
            .iter()
            .map(|f| (f.id.clone(), f.evidence[0].file.clone(), f.evidence[0].line))
            .collect()
    }

    #[test]
    fn config_accepts_every_rule_and_rejects_an_unknown_one() {
        let dir = tempfile::tempdir().unwrap();
        let parsed = config(dir.path(), &RULES).unwrap();
        let names: Vec<&str> = parsed.views.conventions.iter().map(|c| c.name()).collect();
        assert_eq!(names, RULES);
        let err = config(dir.path(), &["navigation_literals"]).unwrap_err();
        assert!(err.contains("archguard.toml"), "{err}");
        assert!(
            err.contains("unknown variant `navigation_literals`"),
            "{err}"
        );
    }

    #[test]
    fn an_absent_section_checks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                (".archguard/ui-flow.json", BROKEN_FLOW),
                ("Domain/Order.cs", ORDER),
                ("App/Handler.cs", BROKEN_HANDLER),
            ],
        );
        assert!(Config::default().views.conventions.is_empty());
        assert!(convention_findings(dir.path(), &Config::default()).is_empty());
    }

    fn broken() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                (".archguard/ui-flow.json", BROKEN_FLOW),
                ("Domain/Order.cs", ORDER),
                ("App/Handler.cs", BROKEN_HANDLER),
            ],
        );
        dir
    }

    fn kept() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                (".archguard/ui-flow.json", KEPT_FLOW),
                ("Domain/Order.cs", ORDER),
                ("App/Handler.cs", KEPT_HANDLER),
            ],
        );
        dir
    }

    #[test]
    fn navigation_literal_names_the_unresolved_target() {
        let findings = run(broken().path(), &["navigation_literal"]);
        assert_eq!(
            ids_and_places(&findings),
            [(
                "view-navigation_literal:Web/Home.razor:returnUrl".into(),
                "Web/Home.razor".into(),
                3
            )]
        );
        let message = findings[0].message.as_deref().unwrap();
        assert!(
            message.contains("write the target as a literal route or a route constant"),
            "{message}"
        );
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].kind, FindingKind::Convention);
    }

    #[test]
    fn screen_access_declared_names_the_undeclared_screen() {
        let findings = run(broken().path(), &["screen_access_declared"]);
        assert_eq!(
            ids_and_places(&findings),
            [(
                "view-screen_access_declared:Web/Home.razor:/".into(),
                "Web/Home.razor".into(),
                1
            )]
        );
    }

    #[test]
    fn endpoint_access_declared_names_the_undeclared_endpoint() {
        let findings = run(broken().path(), &["endpoint_access_declared"]);
        assert_eq!(
            ids_and_places(&findings),
            [(
                "view-endpoint_access_declared:Api/Items.cs:DELETE /api/items/{id}".into(),
                "Api/Items.cs".into(),
                9
            )]
        );
        let message = findings[0].message.as_deref().unwrap();
        assert!(
            message.contains("[Authorize]/[AllowAnonymous]"),
            "{message}"
        );
        assert!(message.contains("@PreAuthorize/@PermitAll"), "{message}");
    }

    #[test]
    fn http_url_literal_names_the_unresolved_call() {
        let findings = run(broken().path(), &["http_url_literal"]);
        assert_eq!(
            ids_and_places(&findings),
            [(
                "view-http_url_literal:Web/Home.razor:ItemUrl(Id)".into(),
                "Web/Home.razor".into(),
                5
            )]
        );
    }

    #[test]
    fn frontend_backend_agree_names_both_mismatches() {
        let findings = run(broken().path(), &["frontend_backend_agree"]);
        assert_eq!(
        ids_and_places(&findings),
        [
            (
                "view-frontend_backend_agree:Web/Home.razor:shown_but_refused GET /api/orders"
                    .into(),
                "Web/Home.razor".into(),
                4
            ),
            (
                "view-frontend_backend_agree:Web/Admin.razor:hidden_but_unprotected POST /api/admin"
                    .into(),
                "Web/Admin.razor".into(),
                6
            ),
        ]
    );
    }

    #[test]
    fn state_changes_in_entity_names_the_assignment_outside_the_entity() {
        let findings = run(broken().path(), &["state_changes_in_entity"]);
        assert_eq!(
        ids_and_places(&findings),
        [(
            "view-state_changes_in_entity:App/Handler.cs:Order.OrderStatus = OrderStatus.Shipped"
                .into(),
            "App/Handler.cs".into(),
            7
        )]
    );
        let message = findings[0].message.as_deref().unwrap();
        assert!(message.contains("method of Order"), "{message}");
    }

    #[test]
    fn state_changes_in_entity_counts_non_literal_assignments_in_the_entity() {
        let dir = tempfile::tempdir().unwrap();
        let order = ORDER.replace(
            "    public void SetShipped()",
            "    public void Set(OrderStatus s) { OrderStatus = s; }\n    public void SetShipped()",
        );
        write(dir.path(), &[("Domain/Order.cs", &order)]);
        let findings = run(dir.path(), &["state_changes_in_entity"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].evidence[0].file, "Domain/Order.cs");
        assert_eq!(findings[0].evidence[0].line, 13);
        assert!(findings[0].id.contains("non-literal"), "{}", findings[0].id);
    }

    #[test]
    fn events_through_raise_function_names_events_created_past_it() {
        let findings = run(broken().path(), &["events_through_raise_function"]);
        assert_eq!(
            ids_and_places(&findings),
            [
                (
                    "view-events_through_raise_function:App/Handler.cs:OrderShippedDomainEvent"
                        .into(),
                    "App/Handler.cs".into(),
                    8
                ),
                (
                    "view-events_through_raise_function:App/Handler.cs:StockReserved".into(),
                    "App/Handler.cs".into(),
                    10
                ),
            ]
        );
        let message = findings[0].message.as_deref().unwrap();
        assert!(message.contains("AddDomainEvent"), "{message}");
    }

    /// One lost link, one endpoint nobody calls.
    const LINK_FLOW: &str = r#"{"definition":"ui-flow/v1","complete":true,
"screens":[
 {"id":"home","name":"Home","kind":"page","route":"/","application":"web","file":"Web/Home.razor","line":1,"source":"x"}],
"transitions":[
 {"from":{"screen":"home"},"target":{"unresolved":"/nowhere"},"file":"Web/Home.razor","line":3,"source":"x"}],
"actions":[
 {"screen":{"screen":"home"},"call":{"http":{"method":"GET","path":"/api/orders"}},"file":"Web/Home.razor","line":4,"source":"x"}],
"endpoints":[
 {"method":"GET","path":"/api/orders","access":"open","file":"Api/Orders.cs","line":7,"source":"x"},
 {"method":"GET","path":"/api/unused","access":"open","file":"Api/Unused.cs","line":9,"source":"x"}],
"diagnostics":[
 {"file":"Web/Home.razor","line":3,"message":"Navigation target /nowhere matches no route of application web."},
 {"file":"Web/Home.razor","line":5,"message":"Navigation target is not a literal: x"},
 {"file":"Web/Home.razor","line":6,"message":"Navigation target returnUrl.ToString() is not a route of this application."},
 {"file":"Web/Home.razor","line":7,"message":"Navigation target /x matches no route of application other."}]}"#;

    fn linked(flow: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &[(".archguard/ui-flow.json", flow)]);
        dir
    }

    #[test]
    fn navigation_resolves_names_the_target_without_a_route() {
        let findings = run(linked(LINK_FLOW).path(), &["navigation_resolves"]);
        // The app `other` has no route read: a note, not a finding.
        assert_eq!(
            ids_and_places(&findings),
            [
                (
                    "view-conventions::navigation_resolves other".into(),
                    "".into(),
                    0
                ),
                (
                    "view-navigation_resolves:Web/Home.razor:/nowhere".into(),
                    "Web/Home.razor".into(),
                    3
                )
            ]
        );
        assert!(run(kept().path(), &["navigation_resolves"]).is_empty());
    }

    #[test]
    fn endpoint_called_names_the_endpoint_no_action_calls() {
        let findings = run(linked(LINK_FLOW).path(), &["endpoint_called"]);
        assert_eq!(
            ids_and_places(&findings),
            [(
                "view-endpoint_called:Api/Unused.cs:GET /api/unused".into(),
                "Api/Unused.cs".into(),
                9
            )]
        );
        assert!(run(kept().path(), &["endpoint_called"]).is_empty());
    }

    #[test]
    fn endpoint_called_is_silent_without_a_frontend() {
        let api = r#"{"definition":"ui-flow/v1","complete":true,
"endpoints":[{"method":"GET","path":"/api/x","access":"open","file":"Api/X.cs","line":1,"source":"x"}]}"#;
        assert!(run(linked(api).path(), &["endpoint_called"]).is_empty());
    }

    fn diag(file: &str, line: u32, message: &str) -> String {
        let message = message.replace('"', "\\\"");
        format!(r#"{{"file":"{file}","line":{line},"message":"{message}"}}"#)
    }

    /// A one-screen app `web` with `diagnostics`; the endpoint list is `endpoints`.
    fn flow_with(diagnostics: &[String], extra_actions: &str, endpoints: &str) -> String {
        format!(
            r#"{{"definition":"ui-flow/v1","complete":true,
"screens":[{{"id":"home","name":"Home","kind":"page","route":"/","application":"web","file":"Web/Home.razor","line":1,"source":"x"}}],
"actions":[{{"screen":{{"screen":"home"}},"call":{{"http":{{"method":"GET","path":"/api/orders"}}}},"file":"Web/Home.razor","line":4,"source":"x"}}{extra_actions}],
"endpoints":[{{"method":"GET","path":"/api/orders","access":"open","file":"Api/Orders.cs","line":7,"source":"x"}}{endpoints}],
"diagnostics":[{}]}}"#,
            diagnostics.join(",")
        )
    }

    fn severities(findings: &[Finding]) -> Vec<Severity> {
        findings.iter().map(|f| f.severity).collect()
    }

    #[test]
    fn navigation_resolves_is_not_evaluated_for_a_partly_read_route_table() {
        let flow = flow_with(
            &[
                diag(
                    "Web/Home.razor",
                    3,
                    "Navigation target /cart matches no route of application web.",
                ),
                diag(
                    "Web/App.jsx",
                    19,
                    "Route element is not a JSX component; the screen is unknown: element",
                ),
            ],
            "",
            "",
        );
        let findings = run(linked(&flow).path(), &["navigation_resolves"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].id, "view-conventions::navigation_resolves web");
        assert_eq!(severities(&findings), [Severity::Info]);
        let message = findings[0].message.as_deref().unwrap();
        assert!(message.contains("not evaluated for web"), "{message}");
        assert!(message.contains("1 literal target"), "{message}");
    }

    #[test]
    fn navigation_resolves_is_not_evaluated_for_an_app_without_routes() {
        let flow = flow_with(
            &[
                diag(
                    "React/NavMenu.jsx",
                    12,
                    "Navigation target /login matches no route of application react.",
                ),
                diag(
                    "React/NavMenu.jsx",
                    20,
                    "Navigation target /todo matches no route of application react.",
                ),
            ],
            "",
            "",
        );
        let findings = run(linked(&flow).path(), &["navigation_resolves"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0]
                .message
                .as_deref()
                .unwrap()
                .contains("not evaluated for react")
        );
        assert!(
            findings[0]
                .message
                .as_deref()
                .unwrap()
                .contains("2 literal targets")
        );
        assert_eq!(findings[0].severity, Severity::Info);
    }

    #[test]
    fn navigation_resolves_reads_every_blazor_literal_form() {
        let dir = linked(&flow_with(
            &[
                diag(
                    "Web/Home.razor",
                    2,
                    "Navigation target nowhere2 is not a route of this application.",
                ),
                diag(
                    "Web/Home.razor",
                    3,
                    "Navigation target \"/nowhere4\" is not a route of this application.",
                ),
                diag(
                    "Web/Home.razor",
                    4,
                    "Navigation target \"user/orders\" is not a route of this application.",
                ),
                diag(
                    "Web/Home.razor",
                    5,
                    "Navigation target returnUrl is not a route of this application.",
                ),
                diag(
                    "Web/Home.razor",
                    6,
                    "Navigation target returnUrl.ToString() is not a route of this application.",
                ),
            ],
            "",
            "",
        ));
        write(
            dir.path(),
            &[(
                "Web/Home.razor",
                "@page \"/\"\n<a href=\"nowhere2\">x</a>\n@code { void A() { Nav.NavigateTo(\"/nowhere4\"); } }\n// Nav.NavigateTo(\"user/orders\")\n// NavigateTo(returnUrl)\n// returnUrl.ToString()\n",
            )],
        );
        let findings = run(dir.path(), &["navigation_resolves"]);
        let subjects: Vec<&str> = findings
            .iter()
            .map(|f| f.evidence[0].raw.as_str())
            .collect();
        assert_eq!(
            subjects,
            ["/nowhere2", "/nowhere4", "/user/orders"],
            "{findings:#?}"
        );
        // A base href other than `/` moves a base-relative literal.
        write(
            dir.path(),
            &[("Web/wwwroot/index.html", "<base href=\"/app/\" />")],
        );
        let findings = run(dir.path(), &["navigation_resolves"]);
        let subjects: Vec<&str> = findings
            .iter()
            .map(|f| f.evidence[0].raw.as_str())
            .collect();
        assert_eq!(subjects, ["/app/nowhere2", "/nowhere4", "/app/user/orders"]);
    }

    #[test]
    fn endpoint_called_does_not_skip_any_method_endpoints() {
        let any = r#",{"method":"ANY","path":"/api/legacy","access":"open","file":"Api/Legacy.cs","line":3,"source":"x"}"#;
        let findings = run(
            linked(&flow_with(&[], "", any)).path(),
            &["endpoint_called"],
        );
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0].id.ends_with("ANY /api/legacy"),
            "{}",
            findings[0].id
        );
        let message = findings[0].message.as_deref().unwrap();
        assert!(message.contains("no reader found a caller"), "{message}");
    }

    #[test]
    fn endpoint_called_counts_a_literal_link_as_reaching_the_endpoint() {
        let any = r#",{"method":"ANY","path":"/Account/Login","access":"open","file":"Api/Account.cs","line":3,"source":"x"}"#;
        let flow = flow_with(&[], "", any);
        let flow = flow.replacen(
            r#""actions":"#,
            r#""transitions":[{"from":{"screen":"home"},"target":{"unresolved":"\"/Account/Login\""},"file":"Web/Home.razor","line":2,"source":"x"}],"actions":"#,
            1,
        );
        let findings = run(linked(&flow).path(), &["endpoint_called"]);
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn endpoint_called_is_not_evaluated_while_a_frontend_has_unresolved_calls() {
        let blind = r#",{"screen":{"screen":"home"},"call":{"unresolved":"Url(x)"},"file":"Web/Home.razor","line":9,"source":"x"}"#;
        let any = r#",{"method":"GET","path":"/api/unused","access":"open","file":"Api/U.cs","line":3,"source":"x"}"#;
        let findings = run(
            linked(&flow_with(&[], blind, any)).path(),
            &["endpoint_called"],
        );
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].id, "view-conventions::endpoint_called");
        assert_eq!(findings[0].severity, Severity::Info);
        let message = findings[0].message.as_deref().unwrap();
        assert!(
            message.contains("not evaluated: web has 1 unresolved"),
            "{message}"
        );
    }

    #[test]
    fn endpoint_called_is_not_evaluated_while_a_frontend_has_an_unread_client() {
        let gap = diag(
            "Web/Api.razor",
            2,
            "HTTP client import not read; its calls are unknown.",
        );
        let flow = flow_with(
            &[gap],
            "",
            r#",{"method":"GET","path":"/api/x","access":"open","file":"Api/X.cs","line":1,"source":"x"}"#,
        );
        let findings = run(linked(&flow).path(), &["endpoint_called"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0]
                .message
                .as_deref()
                .unwrap()
                .contains("web has 1 unresolved/unread")
        );
    }

    #[test]
    fn navigation_resolves_counts_a_link_to_a_get_or_any_endpoint_as_resolved() {
        let get = r#",{"method":"GET","path":"/account/login","access":"open","file":"Api/A.cs","line":3,"source":"x"}"#;
        let any = r#",{"method":"ANY","path":"/api/linked","access":"open","file":"Api/B.cs","line":4,"source":"x"}"#;
        let post = r#",{"method":"POST","path":"/api/post","access":"open","file":"Api/C.cs","line":5,"source":"x"}"#;
        let flow = flow_with(
            &[
                diag(
                    "Web/Home.razor",
                    3,
                    "Navigation target /account/login matches no route of application web.",
                ),
                diag(
                    "Web/Home.razor",
                    4,
                    "Navigation target /api/linked matches no route of application web.",
                ),
                diag(
                    "Web/Home.razor",
                    5,
                    "Navigation target /api/post matches no route of application web.",
                ),
            ],
            "",
            &format!("{get}{any}{post}"),
        );
        let findings = run(linked(&flow).path(), &["navigation_resolves"]);
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            ["view-navigation_resolves:Web/Home.razor:/api/post"],
            "{ids:?}"
        );
    }

    #[test]
    fn endpoint_called_is_not_evaluated_while_an_app_has_no_resolved_call() {
        let docs = r#"{"id":"docs","name":"Docs","kind":"page","route":"/docs","application":"docs","file":"Docs/Docs.razor","line":1,"source":"x"}"#;
        let unused = r#",{"method":"GET","path":"/api/unused","access":"open","file":"Api/U.cs","line":3,"source":"x"}"#;
        let flow = flow_with(&[], "", unused).replacen(
            r#""screens":["#,
            &format!(r#""screens":[{docs},"#),
            1,
        );
        let dir = linked(&flow);
        write(dir.path(), &[("Docs/Docs.razor", "<h1>Docs</h1>\n")]);
        let findings = run(dir.path(), &["endpoint_called"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, Severity::Info);
        let message = findings[0].message.as_deref().unwrap();
        assert!(message.contains("docs has 0 resolved calls"), "{message}");
    }

    /// CleanArchitecture's shape: screens, no readable call, one endpoint.
    const NO_CALLS: &str = r#"{"definition":"ui-flow/v1","complete":true,
"screens":[{"id":"home","name":"Home","kind":"page","route":"/","application":"web","file":"Web/app/home.ts","line":1,"source":"x"}],
"endpoints":[{"method":"GET","path":"/api/x","access":"open","file":"Api/X.cs","line":1,"source":"x"}]}"#;

    #[test]
    fn endpoint_called_is_not_evaluated_with_an_unread_generated_client_config() {
        let dir = linked(NO_CALLS);
        write(
            dir.path(),
            &[
                ("Web/app/home.ts", "export class Home {}\n"),
                ("Web/nswag.json", "{}"),
            ],
        );
        let findings = run(dir.path(), &["endpoint_called"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, Severity::Info);
        let message = findings[0].message.as_deref().unwrap();
        assert!(message.contains("nswag.json"), "{message}");
    }

    #[test]
    fn repeated_subjects_in_one_file_get_distinct_ids() {
        let dir = broken();
        let handler = BROKEN_HANDLER.replace(
            "var text",
            "await mediator.Publish(new OrderShippedDomainEvent(order));\n        var text",
        );
        write(dir.path(), &[("App/Handler.cs", &handler)]);
        let findings = run(dir.path(), &["events_through_raise_function"]);
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "view-events_through_raise_function:App/Handler.cs:OrderShippedDomainEvent",
                "view-events_through_raise_function:App/Handler.cs:OrderShippedDomainEvent#2",
                "view-events_through_raise_function:App/Handler.cs:StockReserved",
            ]
        );
    }

    #[test]
    fn kept_conventions_yield_no_finding() {
        let findings = run(kept().path(), &RULES);
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn an_invalid_import_file_is_a_finding_not_silence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &[(".archguard/ui-flow.json", "{not json")]);
        let findings = run(dir.path(), &["navigation_literal"]);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0]
                .message
                .as_deref()
                .unwrap()
                .contains("ui-flow.json"),
            "{findings:#?}"
        );
    }

    #[test]
    fn blank_survives_unterminated_literals_and_keeps_offsets() {
        for src in [
            "var a = \"open\nnext();",
            "var a = \"\nnext();",
            "x = '",
            "x = 'a",
            "s = \"\";",
            "/* open",
        ] {
            let out = blank(src);
            assert_eq!(out.len(), src.len(), "{src:?}");
            assert_eq!(out.matches('\n').count(), src.matches('\n').count());
        }
        assert_eq!(
            blank("a(\"new X()\") // new Y()"),
            "a(\"       \")           "
        );
    }
}
