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
        }
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

/// Union of two models (extractor result + import): sorted, identical entries
/// once, complete only when both are.
pub fn merge_domain_models(a: DomainModel, b: DomainModel) -> DomainModel {
    fn union<T: Ord>(mut a: Vec<T>, b: Vec<T>) -> Vec<T> {
        a.extend(b);
        a.sort();
        a.dedup();
        a
    }
    DomainModel {
        definition: DOMAIN_MODEL_DEFINITION.into(),
        entities: union(a.entities, b.entities),
        relations: union(a.relations, b.relations),
        state_machines: union(a.state_machines, b.state_machines),
        use_cases: union(a.use_cases, b.use_cases),
        events: union(a.events, b.events),
        diagnostics: union(a.diagnostics, b.diagnostics),
        complete: a.complete && b.complete,
    }
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
            ..order_model(Some("Submitted"))
        };
        let json = serde_json::to_string(&model).unwrap();
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
}
