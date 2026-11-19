//! `domain_model`: entities, relations, state machines, use cases and events
//! read from code (ADR 0022), read-only and paginated within 4 KiB.
use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use archguard_engine::core::domain_model::{
    DomainModel, StateMachine, StateTransition, event_chain,
};
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
    /// Revision from a previous page. Rejects a changed model; restart at offset 0.
    pub expected_revision: Option<String>,
}

pub fn validate(p: &DomainModelParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    let bad = |v: &str| v.is_empty() || v.len() > 256 || v.contains('\0');
    if p.entity.as_deref().is_some_and(bad) || p.expected_revision.as_deref().is_some_and(bad) {
        return Err(crate::tools::invalid_params(
            "entity and expected_revision must be 1..=256 bytes without NUL",
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

pub fn domain_model_value(model: &DomainModel, p: &DomainModelParams) -> Result<Value, ErrorData> {
    let entity = match p.entity.as_deref() {
        None => None,
        Some(e) => Some(
            (model.entities.iter().find(|x| x.id == e || x.name == e))
                .map(|x| x.id.as_str())
                .ok_or_else(|| crate::tools::invalid_params("Unknown entity"))?,
        ),
    };
    let rev = crate::ui_flow_tools::revision(&(model, entity));
    crate::ui_flow_tools::check_revision(&rev, p.expected_revision.as_deref())?;
    let is = |id: &str| entity.is_none_or(|e| e == id);
    // `Order.Submit` belongs to `Order`.
    let owns =
        |call: &str| entity.is_none_or(|e| call.split_once('.').is_some_and(|(o, _)| o == e));
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
    let q = p.page.query.as_deref().unwrap_or("").trim().to_lowercase();
    crate::engineering_tools::page(
        if model.complete {
            "complete"
        } else {
            "incomplete"
        },
        json!({"definition":model.definition,"revision":rev,"summary":archguard_engine::domain_model::summary(model),"entity":entity,
            "limitations":"Static evidence from rules over the code (ADR 0022), not a runtime trace. A guard is source text for when the transition is allowed (allowed when), never evaluated; an exit guard (throw or return) is the literal negation `!(source condition)`. A use case that calls an entity method through an interface or a name several entities share is not linked. Diagnostics stay complete under the entity filter."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        &p.page,
        Clone::clone,
    )
}

#[tool_router(router = domain_model_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Read the domain model from code (ADR 0022): entities (aggregate root, entity, value object), relations, state machines with their transitions (from, to, method, guard as source text of when it is allowed, raised events), use cases with the entity methods they call, and domain events with raisers and handlers, plus summary and gap diagnostics. Built-in rules for C# plus .archguard/domain-model.json. entity narrows to one entity and adds to each transition its event chain (event -> handlers) and the use cases calling its method. Static evidence, not a runtime trace; guards are never evaluated. Read-only; flattened 4 KiB pages, follow next_offset with metadata.revision as expected_revision."
    )]
    pub(crate) async fn domain_model(
        &self,
        Parameters(params): Parameters<DomainModelParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let model =
            archguard_engine::domain_model::analyze(&root).map_err(crate::server::insight_error)?;
        domain_model_value(&model, &params).map(|v| v.to_string())
    }
}
