//! `domain_model`: entities, relations, state machines, use cases and events
//! read from code (ADR 0022), read-only and paginated within 4 KiB.
use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use archguard_engine::core::UiFlow;
use archguard_engine::core::domain_model::{
    DomainModel, StateMachine, StateTransition, event_chain,
};
use archguard_engine::domain_chain::{chains, triggers_of_state};
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
    /// Revision from a previous page. Rejects a changed model; restart at offset 0.
    pub expected_revision: Option<String>,
}

pub fn validate(p: &DomainModelParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    let bad = |v: &str| v.is_empty() || v.len() > 256 || v.contains('\0');
    let strings = [&p.entity, &p.state, &p.expected_revision];
    if strings.iter().any(|v| v.as_deref().is_some_and(bad)) {
        return Err(crate::tools::invalid_params(
            "entity, state and expected_revision must be 1..=256 bytes without NUL",
        ));
    }
    if p.state.is_some() && p.entity.is_none() {
        return Err(crate::tools::invalid_params("state requires entity"));
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

/// Chain rows: every chain, those changing `entity`, or those causing `state`.
fn chain_rows(
    flow: &UiFlow,
    model: &DomainModel,
    entity: Option<&str>,
    state: Option<&str>,
) -> Vec<Value> {
    let found = match (entity, state) {
        (Some(e), Some(s)) => triggers_of_state(flow, model, e, s),
        (Some(e), None) => (chains(flow, model).into_iter())
            .filter(|c| c.transitions.iter().any(|t| t.entity == e))
            .collect(),
        (None, _) => chains(flow, model),
    };
    (found.iter())
        .map(|c| {
            let mut row = json!({"kind": "chain"});
            row.as_object_mut().unwrap().extend(
                serde_json::to_value(c)
                    .expect("chain serializes")
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
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
    let rev = crate::ui_flow_tools::revision(&(model, entity, flow, p.chain, &p.state));
    crate::ui_flow_tools::check_revision(&rev, p.expected_revision.as_deref())?;
    let is = |id: &str| entity.is_none_or(|e| e == id);
    // `Order.Submit` belongs to `Order`.
    let owns =
        |call: &str| entity.is_none_or(|e| call.split_once('.').is_some_and(|(o, _)| o == e));
    if let Some(flow) = flow.filter(|_| p.chain || p.state.is_some()) {
        let rows = chain_rows(flow, model, entity, p.state.as_deref());
        return page(model, rev, entity, &rows, &p.page);
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
    rows.extend(
        model
            .diagnostics
            .iter()
            .map(|d| json!({"kind":"diagnostic","diagnostic":d})),
    );
    page(model, rev, entity, &rows, &p.page)
}

fn page(
    model: &DomainModel,
    rev: String,
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
        json!({"definition":model.definition,"revision":rev,"summary":archguard_engine::domain_model::summary(model),"entity":entity,
            "limitations":"Static evidence from rules over the code (ADR 0022), not a runtime trace. A chain links an endpoint to use cases only through request types its handler sends literally (mediator/gateway Send of new X or a typed variable) or use-case fields it calls; state changes from integration-event handlers or jobs have no endpoint and are not in a chain. A guard is source text for when the transition is allowed (allowed when), never evaluated; an exit guard (throw or return) is the literal negation `!(source condition)`. A use case that calls an entity method through an interface or a name several entities share is not linked. Diagnostics stay complete under the entity filter."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        params,
        Clone::clone,
    )
}

#[tool_router(router = domain_model_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Read the domain model from code (ADR 0022): entities (aggregate root, entity, value object), relations, state machines with their transitions (from, to, method, guard as source text of when it is allowed, raised events), use cases with the entity methods they call, and domain events with raisers and handlers, plus summary and gap diagnostics. Built-in rules for C# plus .archguard/domain-model.json. entity narrows to one entity and adds to each transition its event chain (event -> handlers) and the use cases calling its method. chain=true returns instead one row per endpoint (and calling UI action): its handler, the request types it sends, the use cases handling them, their state transitions, events and handlers; entity+state returns the chains that cause that state (which endpoints and screens can cause Order -> Paid). Static evidence, not a runtime trace; guards are never evaluated. Read-only; flattened 4 KiB pages, follow next_offset with metadata.revision as expected_revision."
    )]
    pub(crate) async fn domain_model(
        &self,
        Parameters(params): Parameters<DomainModelParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let model =
            archguard_engine::domain_model::analyze(&root).map_err(crate::server::insight_error)?;
        let flow = if params.chain || params.state.is_some() {
            Some(archguard_engine::ui_flow::analyze(&root).map_err(crate::server::insight_error)?)
        } else {
            None
        };
        domain_model_value(&model, flow.as_ref(), &params).map(|v| v.to_string())
    }
}
