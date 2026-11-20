//! `domain_model`: entities, relations, state machines, use cases, events,
//! rules, validations and integration events read from code (ADR 0022),
//! read-only and paginated within 4 KiB.
use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use archguard_engine::core::domain_model::{
    DomainModel, StateMachine, StateTransition, event_chain,
};
use archguard_engine::core::{Assignment, UiFlow};
use archguard_engine::domain_chain::{chains, rights, touches, triggers_of_state};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DomainModelParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Exact entity id or name: only its records, each transition with its event
    /// chain and the use cases that call its method.
    pub entity: Option<String>,
    /// Only chains endpoint -> use cases -> transitions -> events -> handlers,
    /// one row per endpoint and calling UI action; with entity only chains
    /// that change it.
    pub chain: bool,
    /// With entity: only the chains that cause entity -> state.
    pub state: Option<String>,
    /// With chain or state: simulate a signed-in user holding exactly these
    /// roles; each chain gets right = allowed/denied/unknown/not_a_user_path.
    pub roles: Option<Vec<String>>,
    /// With chain or state: true simulates an anonymous visitor; not with roles.
    pub anonymous: Option<bool>,
    /// Revision from a previous page. Rejects a changed model; restart at offset 0.
    pub expected_revision: Option<String>,
}

pub fn validate(p: &DomainModelParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    let bad = |v: &str| v.is_empty() || v.len() > 256 || v.contains('\0');
    let strings = [&p.entity, &p.state, &p.expected_revision];
    if strings.iter().any(|v| v.as_deref().is_some_and(bad))
        || (p.roles.as_ref()).is_some_and(|r| r.len() > 64 || r.iter().any(|v| bad(v)))
    {
        return Err(crate::tools::invalid_params(
            "entity, state, roles (at most 64) and expected_revision must be 1..=256 bytes without NUL",
        ));
    }
    if p.state.is_some() && p.entity.is_none() {
        return Err(crate::tools::invalid_params("state requires entity"));
    }
    if p.anonymous == Some(true) && p.roles.is_some() {
        return Err(crate::tools::invalid_params(
            "anonymous cannot be combined with roles",
        ));
    }
    if (p.roles.is_some() || p.anonymous.is_some()) && !p.chain && p.state.is_none() {
        return Err(crate::tools::invalid_params(
            "roles and anonymous need chain or state",
        ));
    }
    Ok(())
}

/// One transition; with an entity selected also its event chain and the use
/// cases that call its method.
fn transition_row(
    model: &DomainModel,
    m: &StateMachine,
    t: &StateTransition,
    chain: bool,
) -> Value {
    let mut row = json!({"kind":"transition","entity":m.entity,"field":m.field,"transition":t});
    if chain {
        // The C# rules qualify `method` (`Order.Submit`); an import may not.
        let qualified = format!("{}.{}", m.entity, t.method);
        row["event_chain"] = (event_chain(model, t).into_iter())
            .map(|(e, handlers)| json!({"event":e.type_name,"raised_by":e.raised_by,"handlers":handlers}))
            .collect();
        row["use_cases"] = (model.use_cases.iter())
            .filter(|u| u.calls.contains(&t.method) || u.calls.contains(&qualified))
            .map(|u| u.id.clone())
            .collect();
    }
    row
}

/// Chain rows: every chain, those changing `entity`, or those causing `state`;
/// with an assignment each row gets its `right`.
fn chain_rows(
    flow: &UiFlow,
    model: &DomainModel,
    entity: Option<&str>,
    state: Option<&str>,
    assignment: Option<&Assignment>,
) -> Vec<Value> {
    let found = match (entity, state) {
        (Some(e), Some(s)) => triggers_of_state(flow, model, e, s),
        (Some(e), None) => (chains(flow, model).into_iter())
            .filter(|c| touches(c, e))
            .collect(),
        (None, _) => chains(flow, model),
    };
    let rights = assignment.map(|a| rights(flow, &found, a));
    (found.iter().enumerate())
        .map(|(i, c)| {
            let mut row = json!({"kind": "chain"});
            row.as_object_mut().unwrap().extend(
                serde_json::to_value(c)
                    .expect("chain serializes")
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            if let Some(rights) = &rights {
                row["right"] = json!(rights[i]);
            }
            row
        })
        .collect()
}

/// `flow` is needed only for `chain` or `state`.
pub fn domain_model_value(
    model: &DomainModel,
    flow: Option<&UiFlow>,
    p: &DomainModelParams,
) -> Result<Value, ErrorData> {
    let entity = match p.entity.as_deref() {
        None => None,
        Some(e) => Some(
            (model.entities.iter().find(|x| x.id == e || x.name == e))
                .map(|x| x.id.as_str())
                .ok_or_else(|| crate::tools::invalid_params("Unknown entity"))?,
        ),
    };
    let simulation = (&p.roles, &p.anonymous);
    let rev = crate::ui_flow_tools::revision(&(model, entity, flow, p.chain, &p.state, simulation));
    crate::ui_flow_tools::check_revision(&rev, p.expected_revision.as_deref())?;
    let is = |id: &str| entity.is_none_or(|e| e == id);
    // `Order.Submit` belongs to `Order`.
    let owns =
        |call: &str| entity.is_none_or(|e| call.split_once('.').is_some_and(|(o, _)| o == e));
    if let Some(flow) = flow.filter(|_| p.chain || p.state.is_some()) {
        let a = crate::ui_flow_tools::assignment(p.roles.as_deref(), p.anonymous);
        let rows = chain_rows(flow, model, entity, p.state.as_deref(), a.as_ref());
        let simulation = a.map(|a| json!({"anonymous": !a.authenticated, "roles": a.roles}));
        return page(model, (rev, simulation), entity, &rows, &p.page);
    }
    let mut rows: Vec<Value> = (model.entities.iter().filter(|e| is(&e.id)))
        .map(|e| json!({"kind":"entity","entity":e}))
        .collect();
    rows.extend(
        (model.relations.iter().filter(|r| is(&r.from) || is(&r.to)))
            .map(|r| json!({"kind":"relation","relation":r})),
    );
    for m in model.state_machines.iter().filter(|m| is(&m.entity)) {
        rows.push(json!({"kind":"state_machine","entity":m.entity,"field":m.field,"enum_type":m.enum_type,"states":m.states,"initial":m.initial,"transitions_total":m.transitions.len()}));
        rows.extend(
            m.transitions
                .iter()
                .map(|t| transition_row(model, m, t, entity.is_some())),
        );
    }
    rows.extend(
        (model
            .use_cases
            .iter()
            .filter(|u| entity.is_none() || u.calls.iter().any(|c| owns(c))))
        .map(|u| json!({"kind":"use_case","use_case":u})),
    );
    rows.extend(
        (model
            .events
            .iter()
            .filter(|e| entity.is_none() || e.raised_by.iter().any(|c| owns(c))))
        .map(|e| json!({"kind":"event","event":e})),
    );
    rows.extend(rich_rows(model, entity));
    // Set aside from the entities; they belong to no single entity.
    if entity.is_none() {
        rows.extend((model.api_models.iter()).map(|a| json!({"kind":"api_model","api_model":a})));
    }
    rows.extend(
        model
            .diagnostics
            .iter()
            .map(|d| json!({"kind":"diagnostic","diagnostic":d})),
    );
    page(model, (rev, None), entity, &rows, &p.page)
}

/// Rule, validation and integration-event rows; with an entity its rules,
/// the validations of its type and the integration events its use cases or
/// the handlers of its events publish.
fn rich_rows(model: &DomainModel, entity: Option<&str>) -> Vec<Value> {
    let name = entity
        .and_then(|e| model.entities.iter().find(|x| x.id == e))
        .map(|x| x.name.as_str());
    let is = |id: &str| entity.is_none_or(|e| e == id || name == Some(id));
    let owns =
        |call: &String| entity.is_none_or(|e| call.split_once('.').is_some_and(|(o, _)| o == e));
    let classes: Vec<&str> = (model.use_cases.iter().filter(|u| u.calls.iter().any(owns)))
        .map(|u| u.name.as_str())
        .chain(
            (model.events.iter().filter(|e| e.raised_by.iter().any(owns)))
                .flat_map(|e| e.handled_by.iter().map(String::as_str)),
        )
        .collect();
    let published =
        |by: &str| entity.is_none() || classes.contains(&by.split_once('.').map_or(by, |(c, _)| c));
    let mut rows: Vec<Value> = (model.rules.iter().filter(|r| is(&r.entity)))
        .map(|r| json!({"kind":"rule","rule":r}))
        .collect();
    rows.extend(
        (model.validations.iter().filter(|v| is(&v.target_type)))
            .map(|v| json!({"kind":"validation","validation":v})),
    );
    rows.extend(
        (model.integration_events.iter())
            .filter(|e| e.published_by.iter().any(|p| published(&p.by)))
            .map(|e| json!({"kind":"integration_event","integration_event":e})),
    );
    rows
}

/// `(rev, simulation)`: the revision and, for chains with roles or anonymous,
/// the simulated user.
fn page(
    model: &DomainModel,
    (rev, simulation): (String, Option<Value>),
    entity: Option<&str>,
    rows: &[Value],
    params: &EngineeringParams,
) -> Result<Value, ErrorData> {
    let q = params.query.as_deref().unwrap_or("").trim().to_lowercase();
    crate::engineering_tools::page(
        if model.complete {
            "complete"
        } else {
            "incomplete"
        },
        json!({"definition":model.definition,"revision":rev,"summary":archguard_engine::domain_model::summary(model),"entity":entity,"simulation":simulation,
            "limitations":"Static evidence from rules over the code (ADR 0022), not a runtime trace. A chain starts at an endpoint or a message handler (integration event, broker listener) and links to use cases only through request types sent literally (mediator/gateway Send of new X or a typed variable) or use-case fields called; jobs are not starts. right: endpoint access and, with an action, screen reach, undeclared = unknown; message handlers are not_a_user_path. A guard is source text (allowed when), never evaluated; an exit guard is the literal negation `!(source condition)`. A rule's condition is source text of when the method throws (never evaluated); rules are read only in entity methods. A chain continues (continues, via) into the message handlers of integration events its start, use cases or event handlers publish, at most 3 hops, no handler twice on a path. Calls through an interface or a method name several entities share are not linked."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        params,
        Clone::clone,
    )
}

#[tool_router(router = domain_model_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Read the domain model from code (ADR 0022): entities (aggregate root, entity, value object), relations, state machines with their transitions (from, to, method, guard as source text of when it is allowed, raised events), use cases with the entity methods they call, domain events with raisers and handlers, rules (kind rule: entity method, condition under which it throws as source text, message, exception, file:line), validations (kind validation: FluentValidation RuleFor chains, Bean Validation annotations per property) and integration events (kind integration_event: publish sites Class.method with service, handling message handlers), api models (kind api_model: name, file, line, reason; controller DTOs set aside from the entities, counted in summary.api_models), plus summary and gap diagnostics. Built-in rules for C#, Java/Kotlin and TypeScript plus .archguard/domain-model.json. entity narrows to one entity and adds to each transition its event chain (event -> handlers) and the use cases calling its method. chain=true returns instead one row per endpoint (and calling UI action) and per message handler (integration event, MassTransit/NServiceBus consumer, Kafka/Rabbit/JMS listener): its handler, the request types it sends, the use cases handling them, their state transitions, events and handlers, and in continues the chains of the message handlers of integration events it publishes (across services); entity+state returns the chains that cause that state (which endpoints, screens and message handlers can cause Order -> Paid). With roles or anonymous: true each chain also gets right = allowed/denied/unknown with reasons (endpoint access, screen reach), or not_a_user_path for a message handler. Static evidence, not a runtime trace; guards are never evaluated. Read-only; flattened 4 KiB pages, follow next_offset with metadata.revision as expected_revision."
    )]
    pub(crate) async fn domain_model(
        &self,
        Parameters(params): Parameters<DomainModelParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let loaded = crate::tools::load_config_at(&root)?;
        let server_root = crate::tools::resolve_root(&self.root, None)?;
        let ctx = archguard_engine::extract_ctx::ExtractContext::new(&root, &loaded.config)
            .confined_to(&server_root);
        let model = archguard_engine::domain_model::analyze_with(&ctx)
            .map_err(crate::server::insight_error)?;
        let flow = if params.chain || params.state.is_some() {
            Some(
                archguard_engine::ui_flow::analyze_with(&ctx)
                    .map_err(crate::server::insight_error)?,
            )
        } else {
            None
        };
        domain_model_value(&model, flow.as_ref(), &params).map(|v| v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{
        ApiModel, DomainRule, Entity, EntityKind, IntegrationEvent, Publisher, UseCase,
        UseCaseKind, Validation,
    };

    fn model() -> DomainModel {
        let entity = |id: &str| Entity {
            id: id.into(),
            name: id.into(),
            kind: EntityKind::Entity,
            fields: vec![],
            file: format!("{id}.cs"),
            line: 1,
            source: "csharp".into(),
        };
        let rule = |entity: &str, method: &str| DomainRule {
            entity: entity.into(),
            method: format!("{entity}.{method}"),
            condition: Some("units <= 0".into()),
            message: Some("Invalid number of units".into()),
            exception: "OrderingDomainException".into(),
            file: format!("{entity}.cs"),
            line: 27,
            source: "csharp".into(),
        };
        let event = |t: &str, by: &str| IntegrationEvent {
            type_name: t.into(),
            published_by: vec![Publisher {
                by: by.into(),
                service: "Ordering.API".into(),
                file: "H.cs".into(),
                line: 9,
            }],
            handled_by: vec!["CatalogHandler".into()],
            source: "csharp".into(),
        };
        DomainModel {
            entities: vec![entity("OrderItem"), entity("CatalogItem")],
            use_cases: vec![UseCase {
                id: "CreateOrderCommandHandler".into(),
                name: "CreateOrderCommandHandler".into(),
                kind: UseCaseKind::Command,
                request: None,
                calls: vec!["OrderItem.AddUnits".into()],
                file: "C.cs".into(),
                line: 1,
                source: "csharp".into(),
            }],
            rules: vec![
                rule("OrderItem", "AddUnits"),
                rule("CatalogItem", "RemoveStock"),
            ],
            validations: vec![Validation {
                target_type: "CreateOrderCommand".into(),
                property: "City".into(),
                rules: vec!["NotEmpty()".into()],
                file: "V.cs".into(),
                line: 6,
                source: "csharp".into(),
            }],
            integration_events: vec![
                event(
                    "OrderStartedIntegrationEvent",
                    "CreateOrderCommandHandler.Handle",
                ),
                event(
                    "ProductPriceChangedIntegrationEvent",
                    "CatalogApi.UpdateItem",
                ),
            ],
            ..DomainModel::default()
        }
    }

    #[test]
    fn api_models_are_counted_and_listed_with_their_reason() {
        let mut model = DomainModel::default();
        let empty = domain_model_value(&model, None, &DomainModelParams::default()).unwrap();
        assert!(empty["metadata"]["summary"].get("api_models").is_none());
        assert!(rows(&model, json!({})).is_empty());
        model.api_models = vec![ApiModel {
            name: "CreateOrderRequest".into(),
            file: "OrdersController.cs".into(),
            line: 12,
            reason: "action body of OrdersController.Create".into(),
        }];
        let page = domain_model_value(&model, None, &DomainModelParams::default()).unwrap();
        assert_eq!(page["metadata"]["summary"]["api_models"], 1);
        let all = rows(&model, json!({"query": "createorder"}));
        let api = kinds(&all, "api_model");
        assert_eq!(api.len(), 1);
        assert_eq!(api[0]["api_model"]["name"], "CreateOrderRequest");
        assert_eq!(api[0]["api_model"]["line"], 12);
        assert_eq!(
            api[0]["api_model"]["reason"],
            "action body of OrdersController.Create"
        );
    }

    fn rows(model: &DomainModel, args: Value) -> Vec<Value> {
        let mut args = args;
        let mut out = vec![];
        loop {
            let p: DomainModelParams = serde_json::from_value(args.clone()).unwrap();
            let page = domain_model_value(model, None, &p).unwrap();
            out.extend(page["items"].as_array().unwrap().iter().cloned());
            args["expected_revision"] = page["metadata"]["revision"].clone();
            match page["next_offset"].as_u64() {
                Some(next) => args["offset"] = json!(next),
                None => return out,
            }
        }
    }

    fn kinds(rows: &[Value], kind: &str) -> Vec<Value> {
        rows.iter().filter(|r| r["kind"] == kind).cloned().collect()
    }

    #[test]
    fn rules_validations_and_integration_events_are_rows() {
        let model = model();
        let all = rows(&model, json!({}));
        let rules = kinds(&all, "rule");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["rule"]["condition"], "units <= 0");
        assert_eq!(rules[0]["rule"]["message"], "Invalid number of units");
        let validations = kinds(&all, "validation");
        assert_eq!(validations[0]["validation"]["rules"], json!(["NotEmpty()"]));
        let events = kinds(&all, "integration_event");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0]["integration_event"]["published_by"][0]["service"],
            "Ordering.API"
        );
        assert_eq!(
            events[0]["integration_event"]["handled_by"],
            json!(["CatalogHandler"])
        );

        // With an entity: its rules and the events its use cases publish.
        let one = rows(&model, json!({"entity": "OrderItem"}));
        let rules = kinds(&one, "rule");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["rule"]["method"], "OrderItem.AddUnits");
        assert!(kinds(&one, "validation").is_empty());
        let events = kinds(&one, "integration_event");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["integration_event"]["type"],
            "OrderStartedIntegrationEvent"
        );
        let summary = &domain_model_value(&model, None, &DomainModelParams::default()).unwrap()["metadata"]
            ["summary"];
        assert_eq!(summary["rules"], 2);
        assert_eq!(summary["integration_events"], 2);
    }
}
