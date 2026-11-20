//! `domain-model/v1` (ADR 0022): entities, relations, state machines, use
//! cases and domain events in one framework-neutral model, plus pure queries.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const DOMAIN_MODEL_DEFINITION: &str = "domain-model/v1";
/// `source` of every entry read from `.archguard/domain-model.json`.
pub const DOMAIN_MODEL_IMPORT_SOURCE: &str = "import";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainModel {
    pub definition: String,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub state_machines: Vec<StateMachine>,
    #[serde(default)]
    pub use_cases: Vec<UseCase>,
    #[serde(default)]
    pub events: Vec<DomainEvent>,
    #[serde(default)]
    pub diagnostics: Vec<DomainDiagnostic>,
    /// False as soon as one state assignment, guard or event could not be read
    /// by a declared rule (same meaning as in ADR 0021).
    pub complete: bool,
    /// Chain starts inside the system: handlers of integration events and
    /// bus messages (ADR 0022 decision 4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_handlers: Vec<MessageHandler>,
    /// Invariants of entity methods: the throws that guard them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<DomainRule>,
    /// Declared input validation (FluentValidation, Bean Validation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validations: Vec<Validation>,
    /// Integration events with their publish sites and handlers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integration_events: Vec<IntegrationEvent>,
    /// Where each `DomainEvent.handled_by` handler is declared; see
    /// [`DomainModel::event_handler_sites`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_handler_sites: Vec<EventHandlerSite>,
    /// Classes read as API models (wire contracts of controller actions), not
    /// as entities: listed so that nothing is silently left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_models: Vec<ApiModel>,
}
impl Default for DomainModel {
    /// Empty and complete, so it is the identity of [`merge_domain_models`].
    fn default() -> Self {
        Self {
            definition: DOMAIN_MODEL_DEFINITION.into(),
            entities: vec![],
            relations: vec![],
            state_machines: vec![],
            use_cases: vec![],
            events: vec![],
            diagnostics: vec![],
            complete: true,
            message_handlers: vec![],
            rules: vec![],
            validations: vec![],
            integration_events: vec![],
            event_handler_sites: vec![],
            api_models: vec![],
        }
    }
}

impl DomainModel {
    /// The declarations of the handlers of event type `event`, sorted.
    pub fn event_handler_sites<'a>(
        &'a self,
        event: &'a str,
    ) -> impl Iterator<Item = &'a EventHandlerSite> + 'a {
        (self.event_handler_sites.iter()).filter(move |s| s.event == event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    AggregateRoot,
    Entity,
    ValueObject,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    /// Element type for collections, e.g. `OrderItem` for `List<OrderItem>`.
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default)]
    pub collection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub kind: EntityKind,
    #[serde(default)]
    pub fields: Vec<Field>,
    pub file: String,
    pub line: u32,
    /// Extractor name, or `import`.
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    HasOne,
    HasMany,
    References,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Relation {
    /// Entity ids.
    pub from: String,
    pub to: String,
    pub kind: RelationKind,
    /// The field of `from` that holds the relation.
    pub via: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StateMachine {
    /// Entity id.
    pub entity: String,
    /// The state field, e.g. `OrderStatus`.
    pub field: String,
    pub enum_type: String,
    pub states: Vec<String>,
    /// Known only when a constructor assigns a literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<String>,
    #[serde(default)]
    pub transitions: Vec<StateTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StateTransition {
    /// Allowed source states; empty means any state.
    #[serde(default)]
    pub from: Vec<String>,
    pub to: String,
    /// The entity method that performs it.
    pub method: String,
    /// The guard as source text; never evaluated (ADR 0022, decision 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    /// Event types raised in the same method.
    #[serde(default)]
    pub events: Vec<String>,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UseCaseKind {
    Command,
    Query,
    Handler,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UseCase {
    pub id: String,
    pub name: String,
    pub kind: UseCaseKind,
    /// Request type it handles, e.g. `SetPaidOrderStatusCommand`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// Entity methods it calls, e.g. `Order.SetPaidStatus`.
    #[serde(default)]
    pub calls: Vec<String>,
    pub file: String,
    pub line: u32,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DomainEvent {
    #[serde(rename = "type")]
    pub type_name: String,
    /// Entity methods or use cases that raise it.
    #[serde(default)]
    pub raised_by: Vec<String>,
    /// Handler classes.
    #[serde(default)]
    pub handled_by: Vec<String>,
    pub source: String,
}

/// Where a handler of a domain event is declared: the class (C#
/// `INotificationHandler<X>`) or the listener method (Java `@EventListener`).
/// It sits on [`DomainModel`], not on [`DomainEvent`], so code that builds
/// events literally keeps compiling; `handler` is the `handled_by` entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EventHandlerSite {
    pub event: String,
    pub handler: String,
    pub file: String,
    pub line: u32,
}

/// A class read as the wire contract of a controller action instead of a
/// domain model: a candidate for the entities that was set aside, with the
/// rule that did it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiModel {
    pub name: String,
    pub file: String,
    pub line: u32,
    /// E.g. `action body of X.Y`, `returned by X.Y`, `only reachable through api model Z`.
    pub reason: String,
}

/// A class or method that handles a message from outside a request (an
/// integration event, a bus or broker message) and sends requests to use
/// cases, e.g. eShop's `OrderPaymentSucceededIntegrationEventHandler`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MessageHandler {
    /// Class name (C#) or `Class.method` (Java listener methods).
    pub id: String,
    /// The handled message type, e.g. `OrderPaymentSucceededIntegrationEvent`.
    pub message: String,
    /// Request types it sends and use-case classes it calls, by the rule of
    /// `EndpointAccess.sends`.
    #[serde(default)]
    pub sends: Vec<String>,
    pub file: String,
    pub line: u32,
    pub source: String,
    /// The project (folder with the build file) it runs in.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub service: String,
}

/// A rule an entity method enforces by throwing (or, for
/// `IValidatableObject`, by adding a `ValidationResult`). Shown as source
/// text, never evaluated (ADR 0022, decision 5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DomainRule {
    /// Entity id.
    pub entity: String,
    /// `Entity.Method`, as on transitions.
    pub method: String,
    /// The condition under which the method throws, as source text; none for
    /// an unconditional throw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// The message literal (interpolations kept) or the argument's source text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Exception type or guard helper, e.g. `OrderingDomainException`.
    pub exception: String,
    pub file: String,
    pub line: u32,
    pub source: String,
}

/// The declared checks on one property of a request or entity type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Validation {
    /// The validated type, e.g. `CreateOrderCommand`.
    pub target_type: String,
    pub property: String,
    /// Each check as source text, e.g. `Length(12, 19)` or `@Size(max = 50)`.
    #[serde(default)]
    pub rules: Vec<String>,
    pub file: String,
    pub line: u32,
    pub source: String,
}

/// An event sent between services through a bus or broker.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IntegrationEvent {
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default)]
    pub published_by: Vec<Publisher>,
    /// Ids of the message handlers of this type.
    #[serde(default)]
    pub handled_by: Vec<String>,
    pub source: String,
}

/// One publish site of an integration event.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Publisher {
    /// `Class.method`.
    pub by: String,
    /// The project (folder with the build file) the site is in.
    pub service: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DomainDiagnostic {
    pub file: String,
    pub line: u32,
    pub message: String,
}

fn transitions_of<'a>(
    model: &'a DomainModel,
    entity: &str,
    keep: impl Fn(&StateTransition) -> bool,
) -> Vec<&'a StateTransition> {
    model
        .state_machines
        .iter()
        .filter(|m| m.entity == entity)
        .flat_map(|m| &m.transitions)
        .filter(|t| keep(t))
        .collect()
}

fn leaves(transition: &StateTransition, state: &str) -> bool {
    transition.from.is_empty() || transition.from.iter().any(|s| s == state)
}

/// Transitions of `entity` that end in `state`.
pub fn transitions_into<'a>(
    model: &'a DomainModel,
    entity: &str,
    state: &str,
) -> Vec<&'a StateTransition> {
    transitions_of(model, entity, |t| t.to == state)
}

/// Transitions of `entity` that may start in `state`; an empty `from` counts
/// for every state.
pub fn transitions_from<'a>(
    model: &'a DomainModel,
    entity: &str,
    state: &str,
) -> Vec<&'a StateTransition> {
    transitions_of(model, entity, |t| leaves(t, state))
}

/// States reachable from each machine's initial state, the initial included;
/// a machine without a known initial state adds nothing.
pub fn reachable_states(model: &DomainModel, entity: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for machine in model.state_machines.iter().filter(|m| m.entity == entity) {
        let Some(initial) = &machine.initial else {
            continue;
        };
        let mut todo = vec![initial.clone()];
        while let Some(state) = todo.pop() {
            if !seen.insert(state.clone()) {
                continue;
            }
            for t in machine.transitions.iter().filter(|t| leaves(t, &state)) {
                todo.push(t.to.clone());
            }
        }
    }
    seen
}

/// For each event the transition raises and the model knows, the event and
/// the sorted handlers of all its entries.
pub fn event_chain<'a>(
    model: &'a DomainModel,
    transition: &StateTransition,
) -> Vec<(&'a DomainEvent, Vec<String>)> {
    transition
        .events
        .iter()
        .filter_map(|name| {
            let mut entries = model.events.iter().filter(|e| &e.type_name == name);
            let first = entries.next()?;
            let handlers: BTreeSet<_> = std::iter::once(first)
                .chain(entries)
                .flat_map(|e| e.handled_by.iter().cloned())
                .collect();
            Some((first, handlers.into_iter().collect()))
        })
        .collect()
}

/// Sets each integration event's `handled_by` to its own entries plus the
/// message handlers of the same type, sorted and once.
pub fn link_integration_events(model: &mut DomainModel) {
    for event in &mut model.integration_events {
        let handlers = (model.message_handlers.iter())
            .filter(|h| h.message == event.type_name)
            .map(|h| h.id.clone());
        let all: BTreeSet<String> = event.handled_by.drain(..).chain(handlers).collect();
        event.handled_by = all.into_iter().collect();
    }
    model.integration_events.sort();
}

/// Union of two models (extractor result + import): sorted, identical entries
/// once, complete only when both are; integration events linked to handlers.
pub fn merge_domain_models(a: DomainModel, b: DomainModel) -> DomainModel {
    fn union<T: Ord>(mut a: Vec<T>, b: Vec<T>) -> Vec<T> {
        a.extend(b);
        a.sort();
        a.dedup();
        a
    }
    let mut merged = DomainModel {
        definition: DOMAIN_MODEL_DEFINITION.into(),
        entities: union(a.entities, b.entities),
        relations: union(a.relations, b.relations),
        state_machines: union(a.state_machines, b.state_machines),
        use_cases: union(a.use_cases, b.use_cases),
        events: union(a.events, b.events),
        diagnostics: union(a.diagnostics, b.diagnostics),
        complete: a.complete && b.complete,
        message_handlers: union(a.message_handlers, b.message_handlers),
        rules: union(a.rules, b.rules),
        validations: union(a.validations, b.validations),
        integration_events: union(a.integration_events, b.integration_events),
        event_handler_sites: union(a.event_handler_sites, b.event_handler_sites),
        api_models: union(a.api_models, b.api_models),
    };
    link_integration_events(&mut merged);
    merged.integration_events.dedup();
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAID_EVENT: &str = "OrderStatusChangedToPaidDomainEvent";

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn go(from: &[&str], to: &str, method: &str, line: u32) -> StateTransition {
        StateTransition {
            from: s(from),
            to: to.into(),
            method: format!("Order.{method}"),
            guard: None,
            events: vec![],
            file: "Order.cs".into(),
            line,
        }
    }

    /// eShop Ordering `Order` state machine, typed by hand.
    fn order_machine(initial: Option<&str>) -> StateMachine {
        StateMachine {
            entity: "Ordering.Order".into(),
            field: "OrderStatus".into(),
            enum_type: "OrderStatus".into(),
            states: s(&[
                "Submitted",
                "AwaitingValidation",
                "StockConfirmed",
                "Paid",
                "Shipped",
                "Cancelled",
            ]),
            initial: initial.map(Into::into),
            transitions: vec![
                go(
                    &["Submitted"],
                    "AwaitingValidation",
                    "SetAwaitingValidationStatus",
                    10,
                ),
                go(
                    &["AwaitingValidation"],
                    "StockConfirmed",
                    "SetStockConfirmedStatus",
                    20,
                ),
                StateTransition {
                    guard: Some("OrderStatus == OrderStatus.StockConfirmed".into()),
                    events: s(&[PAID_EVENT]),
                    ..go(&["StockConfirmed"], "Paid", "SetPaidStatus", 30)
                },
                go(&["Paid"], "Shipped", "SetShippedStatus", 40),
                go(
                    &["Submitted", "AwaitingValidation", "StockConfirmed"],
                    "Cancelled",
                    "SetCancelledStatus",
                    50,
                ),
                go(
                    &["AwaitingValidation"],
                    "Cancelled",
                    "SetCancelledStatusWhenStockIsRejected",
                    60,
                ),
            ],
        }
    }

    fn order_model(initial: Option<&str>) -> DomainModel {
        DomainModel {
            state_machines: vec![order_machine(initial)],
            events: vec![DomainEvent {
                type_name: PAID_EVENT.into(),
                raised_by: s(&["Order.SetPaidStatus"]),
                handled_by: s(&["OrderStatusChangedToPaidDomainEventHandler"]),
                source: "csharp".into(),
            }],
            ..DomainModel::default()
        }
    }

    fn methods(ts: Vec<&StateTransition>) -> Vec<&str> {
        ts.iter().map(|t| t.method.as_str()).collect()
    }

    #[test]
    fn transitions_into_and_from_with_empty_from_as_any() {
        let mut model = order_model(Some("Submitted"));
        assert_eq!(
            methods(transitions_into(&model, "Ordering.Order", "Cancelled")),
            [
                "Order.SetCancelledStatus",
                "Order.SetCancelledStatusWhenStockIsRejected"
            ]
        );
        assert_eq!(
            methods(transitions_from(
                &model,
                "Ordering.Order",
                "AwaitingValidation"
            )),
            [
                "Order.SetStockConfirmedStatus",
                "Order.SetCancelledStatus",
                "Order.SetCancelledStatusWhenStockIsRejected"
            ]
        );
        assert!(transitions_from(&model, "Ordering.Order", "Shipped").is_empty());
        assert!(transitions_into(&model, "Other", "Cancelled").is_empty());

        model.state_machines[0]
            .transitions
            .push(go(&[], "Archived", "Archive", 70));
        for state in ["Shipped", "Paid", "Unlisted"] {
            assert!(
                methods(transitions_from(&model, "Ordering.Order", state))
                    .contains(&"Order.Archive"),
                "{state}"
            );
        }
    }

    #[test]
    fn reachable_states_from_initial_with_cycle_and_unknown_initial() {
        let model = order_model(Some("Submitted"));
        let all: BTreeSet<String> = order_machine(None).states.into_iter().collect();
        assert_eq!(reachable_states(&model, "Ordering.Order"), all);

        let from_paid = reachable_states(&order_model(Some("Paid")), "Ordering.Order");
        assert_eq!(from_paid, s(&["Paid", "Shipped"]).into_iter().collect());

        let mut cyclic = order_model(Some("Paid"));
        cyclic.state_machines[0]
            .transitions
            .push(go(&["Shipped"], "Paid", "Reopen", 80));
        assert_eq!(
            reachable_states(&cyclic, "Ordering.Order"),
            s(&["Paid", "Shipped"]).into_iter().collect()
        );

        assert!(reachable_states(&order_model(None), "Ordering.Order").is_empty());
        assert!(reachable_states(&model, "Unknown").is_empty());
    }

    #[test]
    fn event_chain_lists_handlers_of_raised_events() {
        let mut model = order_model(Some("Submitted"));
        let paid = transitions_into(&model, "Ordering.Order", "Paid")[0].clone();
        let chain = event_chain(&model, &paid);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0.type_name, PAID_EVENT);
        assert_eq!(chain[0].1, ["OrderStatusChangedToPaidDomainEventHandler"]);

        // A second entry of the same type (e.g. from the import) adds handlers.
        model.events.push(DomainEvent {
            type_name: PAID_EVENT.into(),
            raised_by: vec![],
            handled_by: s(&["NotifyBuyerHandler"]),
            source: "import".into(),
        });
        assert_eq!(
            event_chain(&model, &paid)[0].1,
            [
                "NotifyBuyerHandler",
                "OrderStatusChangedToPaidDomainEventHandler"
            ]
        );

        let shipped = transitions_into(&model, "Ordering.Order", "Shipped")[0];
        assert!(event_chain(&model, shipped).is_empty());
        let unknown = StateTransition {
            events: s(&["NotInModel"]),
            ..paid
        };
        assert!(event_chain(&model, &unknown).is_empty());
    }

    #[test]
    fn merge_is_deterministic_and_ands_complete() {
        let entity = |id: &str| Entity {
            id: id.into(),
            name: id.into(),
            kind: EntityKind::AggregateRoot,
            fields: vec![],
            file: "a.cs".into(),
            line: 1,
            source: "csharp".into(),
        };
        let a = DomainModel {
            entities: vec![entity("b"), entity("a")],
            ..order_model(Some("Submitted"))
        };
        let b = DomainModel {
            entities: vec![entity("a"), entity("c")],
            diagnostics: vec![DomainDiagnostic {
                file: "Order.cs".into(),
                line: 9,
                message: "state assigned from a variable".into(),
            }],
            complete: false,
            ..order_model(Some("Submitted"))
        };
        let ab = merge_domain_models(a.clone(), b.clone());
        assert_eq!(ab, merge_domain_models(b, a.clone()));
        let ids: Vec<_> = ab.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(ab.state_machines.len(), 1);
        assert_eq!(ab.events.len(), 1);
        assert_eq!(ab.diagnostics.len(), 1);
        assert!(!ab.complete);
        assert!(merge_domain_models(a, DomainModel::default()).complete);
        assert_eq!(
            merge_domain_models(DomainModel::default(), DomainModel::default()),
            DomainModel::default()
        );
    }

    #[test]
    fn event_handler_sites_merge_and_filter_by_event() {
        let site = |event: &str, handler: &str, line: u32| EventHandlerSite {
            event: event.into(),
            handler: handler.into(),
            file: format!("{handler}.cs"),
            line,
        };
        let a = DomainModel {
            event_handler_sites: vec![site(PAID_EVENT, "Mailer", 9), site("Other", "X", 1)],
            ..DomainModel::default()
        };
        let b = DomainModel {
            event_handler_sites: vec![site(PAID_EVENT, "Audit", 3), site(PAID_EVENT, "Mailer", 9)],
            ..DomainModel::default()
        };
        let merged = merge_domain_models(a, b);
        let paid: Vec<_> = (merged.event_handler_sites(PAID_EVENT))
            .map(|s| (s.handler.as_str(), s.file.as_str(), s.line))
            .collect();
        assert_eq!(paid, [("Audit", "Audit.cs", 3), ("Mailer", "Mailer.cs", 9)]);
        // Older files have no sites; an empty list is left out.
        let old: DomainModel =
            serde_json::from_str(r#"{"definition":"domain-model/v1","complete":true}"#)
                .expect("reads without event_handler_sites");
        assert!(old.event_handler_sites.is_empty());
        assert!(
            !serde_json::to_string(&old)
                .unwrap()
                .contains("event_handler_sites")
        );
    }

    #[test]
    fn api_models_round_trip_merge_and_stay_out_of_old_json() {
        let api = |name: &str| ApiModel {
            name: name.into(),
            file: format!("{name}.cs"),
            line: 3,
            reason: "returned by A.B".into(),
        };
        let a = DomainModel {
            api_models: vec![api("Y"), api("X")],
            ..DomainModel::default()
        };
        let b = DomainModel {
            api_models: vec![api("X")],
            ..DomainModel::default()
        };
        let merged = merge_domain_models(a, b);
        let names: Vec<&str> = merged.api_models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["X", "Y"]);
        let json = serde_json::to_string(&merged).unwrap();
        assert_eq!(serde_json::from_str::<DomainModel>(&json).unwrap(), merged);
        let old: DomainModel =
            serde_json::from_str(r#"{"definition":"domain-model/v1","complete":true}"#).unwrap();
        assert!(old.api_models.is_empty());
        assert!(!serde_json::to_string(&old).unwrap().contains("api_models"));
    }

    #[test]
    fn serde_round_trip_of_a_full_model() {
        let model = DomainModel {
            entities: vec![Entity {
                id: "Ordering.Order".into(),
                name: "Order".into(),
                kind: EntityKind::AggregateRoot,
                fields: vec![
                    Field {
                        name: "OrderItems".into(),
                        type_name: "OrderItem".into(),
                        collection: true,
                    },
                    Field {
                        name: "Address".into(),
                        type_name: "Address".into(),
                        collection: false,
                    },
                ],
                file: "Order.cs".into(),
                line: 5,
                source: "csharp".into(),
            }],
            relations: vec![Relation {
                from: "Ordering.Order".into(),
                to: "Ordering.OrderItem".into(),
                kind: RelationKind::HasMany,
                via: "OrderItems".into(),
                source: "csharp".into(),
            }],
            use_cases: vec![UseCase {
                id: "Ordering.SetPaidOrderStatusCommandHandler".into(),
                name: "SetPaidOrderStatusCommandHandler".into(),
                kind: UseCaseKind::Command,
                request: Some("SetPaidOrderStatusCommand".into()),
                calls: s(&["Order.SetPaidStatus"]),
                file: "SetPaidOrderStatusCommandHandler.cs".into(),
                line: 7,
                source: "csharp".into(),
            }],
            diagnostics: vec![DomainDiagnostic {
                file: "a".into(),
                line: 1,
                message: "m".into(),
            }],
            complete: false,
            message_handlers: vec![MessageHandler {
                id: "OrderPaymentSucceededIntegrationEventHandler".into(),
                message: "OrderPaymentSucceededIntegrationEvent".into(),
                sends: s(&["SetPaidOrderStatusCommand"]),
                file: "OrderPaymentSucceededIntegrationEventHandler.cs".into(),
                line: 3,
                source: "csharp".into(),
                service: String::new(),
            }],
            ..order_model(Some("Submitted"))
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(
            json.contains(r#""message_handlers":[{"id":"OrderPaymentSucceededIntegrationEventHandler","message":"OrderPaymentSucceededIntegrationEvent","sends":["SetPaidOrderStatusCommand"]"#),
            "{json}"
        );
        let without = serde_json::to_string(&DomainModel::default()).unwrap();
        assert!(!without.contains("message_handlers"), "{without}");
        assert!(json.contains(r#""definition":"domain-model/v1""#), "{json}");
        assert!(json.contains(r#""kind":"aggregate_root""#), "{json}");
        assert!(json.contains(r#""kind":"has_many""#), "{json}");
        assert!(
            json.contains(r#""type":"OrderItem","collection":true"#),
            "{json}"
        );
        assert!(
            json.contains(r#""type":"OrderStatusChangedToPaidDomainEvent""#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<DomainModel>(&json).unwrap(), model);
        // Optional and list fields may be left out of hand-written JSON.
        let minimal: DomainModel =
            serde_json::from_str(r#"{"definition":"domain-model/v1","complete":true}"#).unwrap();
        assert_eq!(minimal, DomainModel::default());
    }

    #[test]
    fn rules_validations_and_integration_events_round_trip_and_link() {
        let handler = |id: &str, message: &str| MessageHandler {
            id: id.into(),
            message: message.into(),
            sends: vec![],
            file: "h.cs".into(),
            line: 1,
            source: "csharp".into(),
            service: String::new(),
        };
        let paid = IntegrationEvent {
            type_name: "OrderStatusChangedToPaidIntegrationEvent".into(),
            published_by: vec![Publisher {
                by: "OrderStatusChangedToPaidDomainEventHandler.Handle".into(),
                service: "Ordering.API".into(),
                file: "P.cs".into(),
                line: 39,
            }],
            handled_by: vec![],
            source: "csharp".into(),
        };
        let a = DomainModel {
            rules: vec![DomainRule {
                entity: "OrderItem".into(),
                method: "OrderItem.SetNewDiscount".into(),
                condition: Some("discount < 0".into()),
                message: Some("Discount is not valid".into()),
                exception: "OrderingDomainException".into(),
                file: "OrderItem.cs".into(),
                line: 48,
                source: "csharp".into(),
            }],
            validations: vec![Validation {
                target_type: "CreateOrderCommand".into(),
                property: "CardNumber".into(),
                rules: s(&["NotEmpty()", "Length(12, 19)"]),
                file: "V.cs".into(),
                line: 10,
                source: "csharp".into(),
            }],
            integration_events: vec![paid.clone()],
            ..DomainModel::default()
        };
        let b = DomainModel {
            message_handlers: vec![
                handler("Catalog.PaidHandler", &paid.type_name),
                handler("Webhooks.PaidHandler", &paid.type_name),
                handler("Other", "OtherEvent"),
            ],
            ..DomainModel::default()
        };
        let merged = merge_domain_models(a.clone(), b);
        assert_eq!(
            merged.integration_events[0].handled_by,
            ["Catalog.PaidHandler", "Webhooks.PaidHandler"]
        );
        // Handled-only types are not invented.
        assert_eq!(merged.integration_events.len(), 1);
        let json = serde_json::to_string(&merged).unwrap();
        assert!(json.contains(r#""condition":"discount < 0","message":"Discount is not valid","exception":"OrderingDomainException""#), "{json}");
        assert!(
            json.contains(r#""rules":["NotEmpty()","Length(12, 19)"]"#),
            "{json}"
        );
        assert!(json.contains(r#""type":"OrderStatusChangedToPaidIntegrationEvent","published_by":[{"by":"OrderStatusChangedToPaidDomainEventHandler.Handle","service":"Ordering.API""#), "{json}");
        assert_eq!(serde_json::from_str::<DomainModel>(&json).unwrap(), merged);
        let empty = serde_json::to_string(&DomainModel::default()).unwrap();
        for key in ["rules", "validations", "integration_events"] {
            assert!(!empty.contains(key), "{empty}");
        }
    }
}
