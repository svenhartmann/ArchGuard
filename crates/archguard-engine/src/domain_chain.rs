//! Chains from a start to the domain (ADR 0022 decision 4): action → endpoint
//! → use cases → state transitions → events → handlers, or message handler →
//! use cases → …, and the reverse query "what can cause this state".
//!
//! A start reaches the use cases whose `request` (generics and namespace
//! stripped) is one of the types it sends, or whose name is a use-case class
//! it calls directly; several matches all count. Starts are HTTP endpoints
//! (`EndpointAccess.sends`) and message handlers (`DomainModel.message_handlers`,
//! integration events and broker messages that run inside the system). A use
//! case reaches the transitions whose method it calls, and those reach their
//! events and handlers (`event_chain`). Actions come from
//! `join_actions_to_endpoints`; an endpoint without one still forms a chain.
//!
//! [`rights`] answers "can this user cause it" per chain: the endpoint access
//! and, with an action, the reach of its screen and the action's condition,
//! in three-valued logic. A message-handler chain is no user path.
use std::collections::{BTreeMap, BTreeSet};

use archguard_core::domain_model::{
    DomainModel, MessageHandler, StateTransition, UseCase, event_chain,
};
use archguard_core::{
    AccessState, Action, Assignment, EndpointAccess, Origin, Outcome, ScreenReach, UiFlow,
    evaluate_access, evaluate_condition, join_actions_to_endpoints, screen_reaches,
};
use serde::Serialize;

use crate::ui_flow_backend::simple_type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chain<'a> {
    #[serde(flatten)]
    pub start: ChainStart<'a>,
    pub use_cases: Vec<&'a UseCase>,
    pub transitions: Vec<ChainTransition<'a>>,
    pub events: Vec<ChainEvent<'a>>,
}

/// Where a chain starts. JSON: the variant's fields beside the chain's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ChainStart<'a> {
    /// An HTTP endpoint, with the UI action calling it when one does.
    Endpoint {
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<&'a Action>,
        endpoint: &'a EndpointAccess,
    },
    /// A message handler: runs inside the system, no user starts it.
    Message { message_handler: &'a MessageHandler },
}

impl<'a> Chain<'a> {
    pub fn endpoint(&self) -> Option<&'a EndpointAccess> {
        match self.start {
            ChainStart::Endpoint { endpoint, .. } => Some(endpoint),
            ChainStart::Message { .. } => None,
        }
    }
    pub fn action(&self) -> Option<&'a Action> {
        match self.start {
            ChainStart::Endpoint { action, .. } => action,
            ChainStart::Message { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainTransition<'a> {
    /// Entity id.
    pub entity: &'a str,
    pub field: &'a str,
    pub transition: &'a StateTransition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainEvent<'a> {
    pub event: &'a str,
    pub handlers: Vec<String>,
}

/// One chain per endpoint and action calling it (one without an action when
/// none does), in endpoint order, then one per message handler in model order.
pub fn chains<'a>(flow: &'a UiFlow, model: &'a DomainModel) -> Vec<Chain<'a>> {
    let joined = join_actions_to_endpoints(flow);
    let mut found = Vec::new();
    let chain = |start, sends: &[String]| {
        let use_cases = use_cases_of(model, sends);
        let transitions = transitions_of(model, &use_cases);
        let events = events_of(model, &transitions);
        Chain {
            start,
            use_cases,
            transitions,
            events,
        }
    };
    for endpoint in &flow.endpoints {
        let mut actions: Vec<Option<&Action>> = (joined.iter())
            .filter(|(_, hits)| hits.contains(&endpoint))
            .map(|(a, _)| Some(*a))
            .collect();
        if actions.is_empty() {
            actions.push(None);
        }
        found.extend(
            actions
                .into_iter()
                .map(|action| chain(ChainStart::Endpoint { action, endpoint }, &endpoint.sends)),
        );
    }
    for message_handler in &model.message_handlers {
        let start = ChainStart::Message { message_handler };
        found.push(chain(start, &message_handler.sends));
    }
    found
}

/// The chains with a transition of `entity` (id or name) into `state`.
pub fn triggers_of_state<'a>(
    flow: &'a UiFlow,
    model: &'a DomainModel,
    entity: &str,
    state: &str,
) -> Vec<Chain<'a>> {
    let id = (model.entities.iter())
        .find(|e| e.id == entity || e.name == entity)
        .map_or(entity, |e| e.id.as_str());
    chains(flow, model)
        .into_iter()
        .filter(|c| (c.transitions.iter()).any(|t| t.entity == id && t.transition.to == state))
        .collect()
}

/// Whether a simulated user can run a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allowed,
    Denied,
    Unknown,
    /// A message-handler chain: it runs inside the system.
    NotAUserPath,
}

/// The verdict and one reason per part that decided it, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainRight {
    pub verdict: Verdict,
    pub reasons: Vec<String>,
}

/// The right of `assignment` for each chain, in order: endpoint access and,
/// with an action, the reach of its screen and the action's condition, joined
/// in Kleene logic (one denied decides, else one unknown keeps it open).
/// `undeclared` access is unknown, never allowed (ADR 0021 decision 2).
pub fn rights(flow: &UiFlow, chains: &[Chain], assignment: &Assignment) -> Vec<ChainRight> {
    let reaches = screen_reaches(flow, assignment);
    (chains.iter())
        .map(|c| match c.start {
            ChainStart::Message { message_handler } => ChainRight {
                verdict: Verdict::NotAUserPath,
                reasons: vec![format!(
                    "not a user path: {} handles {} inside the system",
                    message_handler.id, message_handler.message
                )],
            },
            ChainStart::Endpoint { action, endpoint } => {
                let mut parts = vec![endpoint_part(endpoint, assignment)];
                if let Some(action) = action {
                    parts.push(screen_part(flow, action, &reaches));
                    if let Some(condition) = &action.condition {
                        let outcome = evaluate_condition(condition, assignment);
                        let text = format!("action needs {}", condition.text());
                        parts.push((outcome, text));
                    }
                }
                let verdict = if parts.iter().any(|p| p.0 == Outcome::Denied) {
                    Verdict::Denied
                } else if parts.iter().any(|p| p.0 == Outcome::Unknown) {
                    Verdict::Unknown
                } else {
                    Verdict::Allowed
                };
                let reasons = (parts.into_iter())
                    .map(|(o, text)| format!("{text}: {}", outcome_text(o)))
                    .collect();
                ChainRight { verdict, reasons }
            }
        })
        .collect()
}

fn outcome_text(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Allowed => "allowed",
        Outcome::Denied => "denied",
        Outcome::Unknown => "unknown",
    }
}

fn endpoint_part(e: &EndpointAccess, assignment: &Assignment) -> (Outcome, String) {
    let access = match &e.access {
        AccessState::Protected(c) => format!("needs {}", c.text()),
        AccessState::Open => "open".into(),
        AccessState::Undeclared => "no access declared".into(),
    };
    let text = format!("endpoint {} {} {access}", e.method, e.path);
    (evaluate_access(&e.access, assignment), text)
}

/// The action's screen reach; for an `any_screen` action the best screen of
/// its application.
fn screen_part(
    flow: &UiFlow,
    action: &Action,
    reaches: &BTreeMap<String, ScreenReach>,
) -> (Outcome, String) {
    let part = |id: &str| {
        let (outcome, how) = match reaches.get(id) {
            Some(ScreenReach::Sure) => (Outcome::Allowed, "reached surely"),
            Some(ScreenReach::Possible) => (Outcome::Unknown, "reached possibly"),
            Some(ScreenReach::Denied) => (Outcome::Denied, "denied"),
            Some(ScreenReach::NoKnownPath { .. }) => (Outcome::Unknown, "no known path"),
            Some(ScreenReach::NoStartScreen) => (Outcome::Unknown, "no start screen"),
            None => (Outcome::Unknown, "not in the flow"),
        };
        let route = (flow.screens.iter().find(|s| s.id == id)).and_then(|s| s.route.as_deref());
        (outcome, format!("screen {} {how}", route.unwrap_or(id)))
    };
    match &action.screen {
        Origin::Screen(id) => part(id),
        Origin::AnyScreen { application } => {
            let rank = |o: &Outcome| match o {
                Outcome::Allowed => 0,
                Outcome::Unknown => 1,
                Outcome::Denied => 2,
            };
            (flow.screens.iter())
                .filter(|s| s.application.as_deref().unwrap_or("") == application)
                .map(|s| part(&s.id))
                .min_by_key(|(o, _)| rank(o))
                .unwrap_or((
                    Outcome::Unknown,
                    format!("no screen of application {application:?}"),
                ))
        }
    }
}

fn use_cases_of<'a>(model: &'a DomainModel, sends: &[String]) -> Vec<&'a UseCase> {
    let sent: BTreeSet<&str> = sends.iter().map(|s| simple_type(s)).collect();
    (model.use_cases.iter())
        .filter(|u| {
            let request = u.request.as_deref().map(simple_type);
            request.is_some_and(|r| sent.contains(r)) || sent.contains(u.name.as_str())
        })
        .collect()
}

fn transitions_of<'a>(model: &'a DomainModel, use_cases: &[&UseCase]) -> Vec<ChainTransition<'a>> {
    let mut found = Vec::new();
    for m in &model.state_machines {
        for t in &m.transitions {
            // The C# rules qualify `method` (`Order.Submit`); an import may not.
            let qualified = format!("{}.{}", m.entity, t.method);
            let called = |c: &String| *c == t.method || *c == qualified;
            if use_cases.iter().any(|u| u.calls.iter().any(called)) {
                found.push(ChainTransition {
                    entity: &m.entity,
                    field: &m.field,
                    transition: t,
                });
            }
        }
    }
    found
}

fn events_of<'a>(model: &'a DomainModel, transitions: &[ChainTransition]) -> Vec<ChainEvent<'a>> {
    let mut found: Vec<ChainEvent> = Vec::new();
    for t in transitions {
        for (event, handlers) in event_chain(model, t.transition) {
            if !found.iter().any(|f| f.event == event.type_name) {
                found.push(ChainEvent {
                    event: &event.type_name,
                    handlers,
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const FILES: &[(&str, &str)] = &[
        ("Web/Web.csproj", "<Project />"),
        (
            "Web/Pages/Checkout.razor",
            "@page \"/checkout\"\n@code {\n    async Task Buy() => await Http.PostAsJsonAsync(\"api/orders\", basket);\n}\n",
        ),
        ("Ordering.API/Ordering.API.csproj", "<Project />"),
        (
            "Ordering.API/Apis/OrdersApi.cs",
            r#"public static class OrdersApi
{
    public static RouteGroupBuilder MapOrdersApiV1(this IEndpointRouteBuilder app)
    {
        var api = app.MapGroup("api/orders");
        api.MapPost("/", CreateOrderAsync);
        api.MapPut("/pay", PayAsync);
        api.MapPut("/cancel", CancelAsync);
        api.MapGet("/", ListAsync);
        return api;
    }
    public static async Task<bool> CreateOrderAsync(CreateOrderRequest request, [AsParameters] OrderServices services)
    {
        var createOrderCommand = new CreateOrderCommand(request.Items);
        var wrapped = new IdentifiedCommand<CreateOrderCommand, bool>(createOrderCommand, request.Id);
        return await services.Mediator.Send(wrapped);
    }
    public static Task<bool> PayAsync(SetPaidOrderStatusCommand command, IMediator mediator) => mediator.Send(command);
    public static Task<bool> CancelAsync(CancelOrderCommand command, IMediator mediator) => mediator.Send(command);
    public static Task<int[]> ListAsync() => Task.FromResult(new int[0]);
}
"#,
        ),
        (
            "Ordering.Domain/Order.cs",
            r#"public interface IAggregateRoot { }
public enum OrderStatus { Draft, Submitted, Paid, Cancelled }
public class Order : IAggregateRoot
{
    public OrderStatus Status { get; private set; }
    public Order() { Status = OrderStatus.Draft; }
    public void Submit()
    {
        if (Status == OrderStatus.Draft)
        {
            AddDomainEvent(new OrderStartedDomainEvent(Id));
            Status = OrderStatus.Submitted;
        }
    }
    public void SetPaidStatus()
    {
        if (Status == OrderStatus.Submitted)
        {
            AddDomainEvent(new OrderStatusChangedToPaidDomainEvent(Id));
            Status = OrderStatus.Paid;
        }
    }
    public void SetCancelledStatus() { Status = OrderStatus.Cancelled; }
}
"#,
        ),
        (
            "Ordering.API/Application/Commands.cs",
            r#"public class IdentifiedCommandHandler<T, R> : IRequestHandler<IdentifiedCommand<T, R>, R> { }
public class CreateOrderCommandHandler : IRequestHandler<CreateOrderCommand, bool>
{
    public async Task<bool> Handle(CreateOrderCommand message, CancellationToken token)
    {
        var order = new Order();
        order.Submit();
        return true;
    }
}
public class SetPaidOrderStatusCommandHandler : IRequestHandler<SetPaidOrderStatusCommand, bool>
{
    public async Task<bool> Handle(SetPaidOrderStatusCommand command, CancellationToken token)
    {
        var order = await _repo.GetAsync(command.OrderNumber);
        order.SetPaidStatus();
        return true;
    }
}
public class CancelOrderCommandHandler : IRequestHandler<Shop.Orders.CancelOrderCommand, bool>
{
    public Task<bool> Handle(Shop.Orders.CancelOrderCommand c, CancellationToken t) { _order.SetCancelledStatus(); return Task.FromResult(true); }
}
public class AdminCancelOrderHandler : IRequestHandler<Shop.Admin.CancelOrderCommand, bool> { }
public class OrderStartedDomainEventHandler : INotificationHandler<OrderStartedDomainEvent> { }
public class OrderPaidDomainEventHandler : INotificationHandler<OrderStatusChangedToPaidDomainEvent> { }
"#,
        ),
    ];

    /// An integration-event handler, an admin page linked from `/` and an
    /// admin-only controller, on top of [`FILES`].
    const MORE: &[(&str, &str)] = &[
        (
            "Ordering.API/Application/PaymentSucceededHandler.cs",
            r#"public class OrderPaymentSucceededIntegrationEventHandler(IMediator mediator) :
    IIntegrationEventHandler<OrderPaymentSucceededIntegrationEvent>
{
    public async Task Handle(OrderPaymentSucceededIntegrationEvent @event)
    {
        var command = new SetPaidOrderStatusCommand(@event.OrderId);
        await mediator.Send(command);
    }
}
"#,
        ),
        (
            "Web/Pages/Home.razor",
            "@page \"/\"\n<a href=\"admin/cancel\">Cancel</a>\n",
        ),
        (
            "Web/Pages/AdminCancel.razor",
            "@page \"/admin/cancel\"\n@attribute [Authorize(Roles = \"Admin\")]\n@code {\n    async Task Go() => await Http.PutAsJsonAsync(\"api/admin/cancel\", order);\n}\n",
        ),
        (
            "Ordering.API/Apis/AdminController.cs",
            r#"[ApiController]
[Route("api/admin")]
[Authorize(Roles = "Admin")]
public class AdminController : ControllerBase
{
    private readonly IMediator _mediator;
    [HttpPut("cancel")]
    public Task<bool> Cancel(CancelOrderCommand command) => _mediator.Send(command);
}
"#,
        ),
    ];

    fn analyzed() -> (UiFlow, DomainModel) {
        analyzed_with(&[])
    }

    fn analyzed_with(more: &[(&str, &str)]) -> (UiFlow, DomainModel) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in FILES.iter().chain(more) {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let root: &Path = dir.path();
        (
            crate::ui_flow::analyze(root).unwrap(),
            crate::domain_model::analyze(root).unwrap(),
        )
    }

    fn chain<'a>(all: &'a [Chain<'a>], method: &str, path: &str) -> &'a Chain<'a> {
        let hits: Vec<_> = (all.iter())
            .filter(|c| {
                c.endpoint()
                    .is_some_and(|e| e.method == method && e.path == path)
            })
            .collect();
        assert_eq!(hits.len(), 1, "{method} {path} in {all:#?}");
        hits[0]
    }

    fn use_case_ids(c: &Chain) -> Vec<String> {
        let mut ids: Vec<String> = c.use_cases.iter().map(|u| u.id.clone()).collect();
        ids.sort();
        ids
    }

    #[test]
    fn chain_runs_from_the_screen_to_the_event_handlers() {
        let (flow, model) = analyzed();
        let all = chains(&flow, &model);
        let create = chain(&all, "POST", "/api/orders");
        let action = create.action().expect("checkout action");
        assert_eq!(action.file, "Web/Pages/Checkout.razor");
        assert_eq!(
            create.endpoint().unwrap().handler.as_deref(),
            Some("OrdersApi.CreateOrderAsync")
        );
        assert_eq!(
            use_case_ids(create),
            ["CreateOrderCommandHandler", "IdentifiedCommandHandler"]
        );
        let steps: Vec<_> = (create.transitions.iter())
            .map(|t| {
                (
                    t.entity,
                    t.transition.to.as_str(),
                    t.transition.method.as_str(),
                )
            })
            .collect();
        assert_eq!(steps, [("Order", "Submitted", "Order.Submit")]);
        assert_eq!(
            create.events,
            [ChainEvent {
                event: "OrderStartedDomainEvent",
                handlers: vec!["OrderStartedDomainEventHandler".into()],
            }]
        );
    }

    #[test]
    fn endpoint_without_action_or_use_case_still_forms_a_chain() {
        let (flow, model) = analyzed();
        let all = chains(&flow, &model);
        let list = chain(&all, "GET", "/api/orders");
        assert!(list.action().is_none());
        assert!(list.use_cases.is_empty() && list.transitions.is_empty());
        assert!(list.events.is_empty());
        assert_eq!(all.len(), flow.endpoints.len());
    }

    #[test]
    fn an_ambiguous_request_name_links_every_use_case() {
        let (flow, model) = analyzed();
        let all = chains(&flow, &model);
        let cancel = chain(&all, "PUT", "/api/orders/cancel");
        assert_eq!(
            use_case_ids(cancel),
            ["AdminCancelOrderHandler", "CancelOrderCommandHandler"]
        );
        assert_eq!(cancel.transitions[0].transition.to, "Cancelled");
    }

    #[test]
    fn triggers_of_a_state_are_the_endpoints_that_reach_it() {
        let (flow, model) = analyzed();
        let paid = triggers_of_state(&flow, &model, "Order", "Paid");
        let paths: Vec<_> = paid
            .iter()
            .map(|c| c.endpoint().unwrap().path.as_str())
            .collect();
        assert_eq!(paths, ["/api/orders/pay"]);
        assert_eq!(paid[0].events[0].handlers, ["OrderPaidDomainEventHandler"]);
        assert!(triggers_of_state(&flow, &model, "Order", "Draft").is_empty());
        assert!(triggers_of_state(&flow, &model, "Nope", "Paid").is_empty());
    }

    #[test]
    fn integration_event_handlers_start_chains_too() {
        let (flow, model) = analyzed_with(MORE);
        let paid = triggers_of_state(&flow, &model, "Order", "Paid");
        let starts: Vec<String> = (paid.iter())
            .map(|c| match c.start {
                ChainStart::Endpoint { endpoint, .. } => endpoint.path.clone(),
                ChainStart::Message { message_handler } => {
                    format!("{} {}", message_handler.message, message_handler.id)
                }
            })
            .collect();
        assert_eq!(
            starts,
            [
                "/api/orders/pay",
                "OrderPaymentSucceededIntegrationEvent OrderPaymentSucceededIntegrationEventHandler",
            ]
        );
        let message = &paid[1];
        assert!(message.endpoint().is_none() && message.action().is_none());
        assert_eq!(use_case_ids(message), ["SetPaidOrderStatusCommandHandler"]);
        assert_eq!(
            message.transitions[0].transition.method,
            "Order.SetPaidStatus"
        );
        assert_eq!(message.events[0].handlers, ["OrderPaidDomainEventHandler"]);
        let json = serde_json::to_value(message).unwrap();
        assert_eq!(
            json["message_handler"]["message"],
            "OrderPaymentSucceededIntegrationEvent"
        );
        assert!(json.get("endpoint").is_none() && json.get("action").is_none());
        let json = serde_json::to_value(&paid[0]).unwrap();
        assert_eq!(json["endpoint"]["path"], "/api/orders/pay");
        assert!(json.get("message_handler").is_none());
    }

    #[test]
    fn rights_per_role_along_the_chain() {
        let (flow, model) = analyzed_with(MORE);
        let cancelled = triggers_of_state(&flow, &model, "Order", "Cancelled");
        let admin_path = |c: &&Chain| c.endpoint().is_some_and(|e| e.path == "/api/admin/cancel");
        let at = cancelled
            .iter()
            .position(|c| admin_path(&c))
            .expect("admin chain");
        let open = (cancelled.iter())
            .position(|c| c.endpoint().is_some_and(|e| e.path == "/api/orders/cancel"))
            .expect("undeclared chain");
        let user = |roles: &[&str]| Assignment {
            authenticated: !roles.is_empty(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            ..Assignment::default()
        };
        let admin = rights(&flow, &cancelled, &user(&["Admin"]));
        assert_eq!(admin.len(), cancelled.len());
        assert_eq!(admin[at].verdict, Verdict::Allowed, "{:?}", admin[at]);
        assert_eq!(
            admin[at].reasons,
            [
                "endpoint PUT /api/admin/cancel needs role Admin: allowed",
                "screen /admin/cancel reached surely: allowed",
            ]
        );
        let anonymous = rights(&flow, &cancelled, &user(&[]));
        assert_eq!(anonymous[at].verdict, Verdict::Denied);
        assert_eq!(
            anonymous[at].reasons,
            [
                "endpoint PUT /api/admin/cancel needs role Admin: denied",
                "screen /admin/cancel denied: denied",
            ]
        );
        assert_eq!(
            rights(&flow, &cancelled, &user(&["Clerk"]))[at].verdict,
            Verdict::Denied
        );
        // Undeclared is never allowed, not even for the admin.
        assert_eq!(admin[open].verdict, Verdict::Unknown);
        assert_eq!(
            admin[open].reasons,
            ["endpoint PUT /api/orders/cancel no access declared: unknown"]
        );
        // A message handler runs inside the system.
        let paid = triggers_of_state(&flow, &model, "Order", "Paid");
        let right = &rights(&flow, &paid, &user(&["Admin"]))[1];
        assert_eq!(right.verdict, Verdict::NotAUserPath);
        assert_eq!(
            serde_json::to_value(right).unwrap(),
            serde_json::json!({"verdict": "not_a_user_path", "reasons": [
                "not a user path: OrderPaymentSucceededIntegrationEventHandler handles OrderPaymentSucceededIntegrationEvent inside the system"
            ]})
        );
    }
}
