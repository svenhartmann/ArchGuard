//! Convention checks for the UI-flow and domain views (ADR 0021 decision 6,
//! ADR 0022 decision 6): the opt-in `[views] conventions` of `check`. Each
//! broken convention is one `target_architecture` error finding with file:line
//! and a fix hint for the agent that wrote the code. The id is
//! `view-<rule>:<file>:<subject>` (`#2`, `#3` … for repeats in one file), so
//! it survives moved lines.
//!
//! - `navigation_literal`: a transition whose target stayed `Unresolved`.
//! - `endpoint_access_declared`: an `Undeclared` endpoint.
//! - `http_url_literal`: a `Call::Unresolved` action.
//! - `frontend_backend_agree`: a `shown_but_refused` or
//!   `hidden_but_unprotected` finding of [`ui_flow_findings`].
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
    AccessState, Call, Config, Finding, FindingKind, Ref, Severity, Target, UiFlow,
    UiFlowFindingKind, ui_flow_findings,
};

use crate::project_files::read_repo_file;
use crate::services::support::{excluded, relative, test_path};
use csharp::{
    RAISE_FUNCTION, blank, constructions, event_types, line, member_assignments, passed_to_raise,
    raise_arguments,
};

#[path = "view_conventions_csharp.rs"]
mod csharp;

const MAX_BYTES: u64 = 1024 * 1024;
const UI_RULES: [&str; 4] = [
    "navigation_literal",
    "endpoint_access_declared",
    "http_url_literal",
    "frontend_backend_agree",
];
const STATE: &str = "state_changes_in_entity";
const EVENTS: &str = "events_through_raise_function";

/// The findings of every convention in `[views] conventions`; none when the
/// list is empty, and then nothing is read.
pub fn convention_findings(root: &Path, config: &Config) -> Vec<Finding> {
    let on: BTreeSet<&str> = config.views.conventions.iter().map(|c| c.name()).collect();
    let mut out = Vec::new();
    if UI_RULES.iter().any(|r| on.contains(r)) {
        match crate::ui_flow::analyze(root) {
            Ok(flow) => out.extend(ui_findings(&flow, &on)),
            Err(e) => out.push(unreadable("ui-flow", e)),
        }
    }
    if on.contains(STATE) || on.contains(EVENTS) {
        match crate::domain_model::analyze(root) {
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

fn ui_findings(flow: &UiFlow, on: &BTreeSet<&str>) -> Vec<Finding> {
    let rules: [(&str, UiRule); 4] = [
        ("navigation_literal", navigation_literal),
        ("endpoint_access_declared", endpoint_access_declared),
        ("http_url_literal", http_url_literal),
        ("frontend_backend_agree", frontend_backend_agree),
    ];
    rules
        .into_iter()
        .filter(|(rule, _)| on.contains(rule))
        .flat_map(|(_, check)| check(flow))
        .collect()
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

/// `(relative path, text with comments and string contents blanked)` of every
/// non-test `*.cs` file, sorted.
fn csharp_sources(root: &Path) -> Vec<(String, String)> {
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .git_global(false)
        .max_depth(Some(64))
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !(excluded(&name) || matches!(name.as_ref(), "bin" | "obj"))
        })
        .build();
    let mut paths: Vec<String> = walk
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(root, e.path()))
        .filter(|rel| rel.ends_with(".cs") && !test_path(rel))
        .collect();
    paths.sort();
    // shortcut: unreadable or oversized files are skipped silently; the domain
    // extractor already reports them as diagnostics.
    paths
        .into_iter()
        .filter_map(|rel| {
            let bytes = read_repo_file(root, Path::new(&rel), MAX_BYTES).ok()?;
            Some((rel.clone(), blank(&String::from_utf8_lossy(&bytes))))
        })
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

fn finding(rule: &str, file: &str, line: u32, subject: &str, message: String) -> Finding {
    Finding {
        kind: FindingKind::TargetArchitecture,
        id: format!("view-{rule}:{file}:{subject}"),
        severity: Severity::Error,
        message: Some(format!("[views] {rule}: {message}")),
        evidence: vec![Ref {
            file: file.to_string(),
            line,
            raw: subject.to_string(),
        }],
        nodes: Vec::new(),
        edge_count: 0,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: Vec::new(),
    }
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

    const RULES: [&str; 6] = [
        "navigation_literal",
        "endpoint_access_declared",
        "http_url_literal",
        "frontend_backend_agree",
        "state_changes_in_entity",
        "events_through_raise_function",
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
 {"id":"home","name":"Home","kind":"page","route":"/","application":"web","file":"Web/Home.razor","line":1,"source":"x"},
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
        assert_eq!(findings[0].kind, FindingKind::TargetArchitecture);
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
