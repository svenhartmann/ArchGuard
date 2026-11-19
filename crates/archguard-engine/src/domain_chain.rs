//! Endpoint to use-case chain (ADR 0022 decision 4): action → endpoint → use
//! cases → state transitions → events → handlers, and the reverse query
//! "which endpoints and screens can cause this state".
//!
//! An endpoint reaches the use cases whose `request` (generics and namespace
//! stripped) is one of the types its handler sends, or whose name is a
//! use-case class the handler calls directly; several matches all count. A
//! use case reaches the transitions whose method it calls, and those reach
//! their events and handlers (`event_chain`). Actions come from
//! `join_actions_to_endpoints`; an endpoint without one still forms a chain.
use std::collections::BTreeSet;

use archguard_core::domain_model::{DomainModel, StateTransition, UseCase, event_chain};
use archguard_core::{Action, EndpointAccess, UiFlow, join_actions_to_endpoints};
use serde::Serialize;

use crate::ui_flow_backend::simple_type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chain<'a> {
    /// The UI action calling the endpoint; none for an endpoint no action hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'a Action>,
    pub endpoint: &'a EndpointAccess,
    pub use_cases: Vec<&'a UseCase>,
    pub transitions: Vec<ChainTransition<'a>>,
    pub events: Vec<ChainEvent<'a>>,
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
/// none does), in endpoint order.
pub fn chains<'a>(flow: &'a UiFlow, model: &'a DomainModel) -> Vec<Chain<'a>> {
    let joined = join_actions_to_endpoints(flow);
    let mut found = Vec::new();
    for endpoint in &flow.endpoints {
        let mut actions: Vec<Option<&Action>> = (joined.iter())
            .filter(|(_, hits)| hits.contains(&endpoint))
            .map(|(a, _)| Some(*a))
            .collect();
        if actions.is_empty() {
            actions.push(None);
        }
        let use_cases = use_cases_of(model, endpoint);
        let transitions = transitions_of(model, &use_cases);
        let events = events_of(model, &transitions);
        found.extend(actions.into_iter().map(|action| Chain {
            action,
            endpoint,
            use_cases: use_cases.clone(),
            transitions: transitions.clone(),
            events: events.clone(),
        }));
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

fn use_cases_of<'a>(model: &'a DomainModel, endpoint: &EndpointAccess) -> Vec<&'a UseCase> {
    let sent: BTreeSet<&str> = endpoint.sends.iter().map(|s| simple_type(s)).collect();
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

    fn analyzed() -> (UiFlow, DomainModel) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in FILES {
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
            .filter(|c| c.endpoint.method == method && c.endpoint.path == path)
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
        let action = create.action.expect("checkout action");
        assert_eq!(action.file, "Web/Pages/Checkout.razor");
        assert_eq!(
            create.endpoint.handler.as_deref(),
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
        assert!(list.action.is_none());
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
        let paths: Vec<_> = paid.iter().map(|c| c.endpoint.path.as_str()).collect();
        assert_eq!(paths, ["/api/orders/pay"]);
        assert_eq!(paid[0].events[0].handlers, ["OrderPaidDomainEventHandler"]);
        assert!(triggers_of_state(&flow, &model, "Order", "Draft").is_empty());
        assert!(triggers_of_state(&flow, &model, "Nope", "Paid").is_empty());
    }
}
