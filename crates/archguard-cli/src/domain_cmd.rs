//! `archguard domain`: entities, state machines, use cases and domain events
//! (ADR 0022), optionally limited to one entity.

use std::collections::BTreeSet;
use std::path::Path;

use archguard_engine::core::domain_model::{DomainModel, Entity, StateMachine};
use archguard_engine::core::{Assignment, Origin, UiFlow};
use archguard_engine::domain_chain::{
    Chain, ChainRight, ChainStart, chains, rights, triggers_of_state,
};
use archguard_engine::domain_model::summary;
use serde_json::{Value, json};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use crate::ui_flow_cmd::{assignment, label, print_gaps, yes_no};

/// `simulated`: the roles of `--role`, empty for `--anonymous`, none without either.
pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    entity: Option<&str>,
    chain: bool,
    state: Option<&str>,
    simulated: Option<&[String]>,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("domain supports --format text|json");
    }
    let mut model = match archguard_engine::domain_model::analyze(root) {
        Ok(model) => model,
        Err(error) => return analysis_error(error),
    };
    if chain || state.is_some() {
        return match archguard_engine::ui_flow::analyze(root) {
            Ok(flow) => run_chain(cli, &flow, &model, entity, state, simulated),
            Err(error) => analysis_error(error),
        };
    }
    if let Some(name) = entity {
        match only_entity(model, name) {
            Some(part) => model = part,
            None => return usage(format!("no entity with id or name {name:?}")),
        }
    }
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        let value = json!({"summary": summary(&model), "model": model});
        emit(&serde_json::to_string_pretty(&value).expect("domain model serializes"));
    } else {
        print_text(&model, cli.verbose);
    }
    EXIT_OK
}

/// `--chain` (optionally only chains touching `entity`) or, with `state`,
/// the chains that cause `entity` → `state`; with `simulated`, each chain's
/// right for that user.
fn run_chain(
    cli: &Cli,
    flow: &UiFlow,
    model: &DomainModel,
    entity: Option<&str>,
    state: Option<&str>,
    simulated: Option<&[String]>,
) -> i32 {
    let id = match entity {
        None => None,
        Some(name) => match (model.entities.iter()).find(|e| e.id == name || e.name == name) {
            Some(e) => Some(e.id.as_str()),
            None => return usage(format!("no entity with id or name {name:?}")),
        },
    };
    let found = match (id, state) {
        (Some(id), Some(state)) => triggers_of_state(flow, model, id, state),
        (Some(id), None) => (chains(flow, model).into_iter())
            .filter(|c| c.transitions.iter().any(|t| t.entity == id))
            .collect(),
        (None, _) => chains(flow, model),
    };
    let assignment = simulated.map(assignment);
    let rights = (assignment.as_ref()).map(|a| rights(flow, &found, a));
    if cli.quiet {
        return EXIT_OK;
    }
    let simulation = assignment.as_ref().zip(rights.as_deref());
    if cli.format == Format::Json {
        let value = chains_json(&found, id.zip(state), simulation);
        emit(&serde_json::to_string_pretty(&value).expect("chains serialize"));
    } else {
        print_chains(&found, id.zip(state), simulation);
    }
    EXIT_OK
}

/// `simulation`: the assignment and the right of each chain.
type Simulation<'a> = Option<(&'a Assignment, &'a [ChainRight])>;

fn chains_json(found: &[Chain], target: Option<(&str, &str)>, simulation: Simulation) -> Value {
    let mut chains = json!(found);
    if let Some((_, rights)) = simulation {
        let rows = chains.as_array_mut().into_iter().flatten();
        for (row, right) in rows.zip(rights) {
            row["right"] = json!(right);
        }
    }
    let mut value = match target {
        Some((id, state)) => json!({"entity": id, "state": state, "chains": chains}),
        None => json!({"chains": chains}),
    };
    if let Some((a, _)) = simulation {
        value["simulated"] = json!({"anonymous": !a.authenticated, "roles": a.roles});
    }
    value
}

fn print_chains(found: &[Chain], target: Option<(&str, &str)>, simulation: Simulation) {
    match target {
        Some((id, state)) => outln!("Triggers of {id} → {state}: {}", found.len()),
        None => outln!(
            "Chains {} · with a use case {}",
            found.len(),
            found.iter().filter(|c| !c.use_cases.is_empty()).count()
        ),
    }
    if let Some((a, _)) = simulation {
        let roles: Vec<&str> = a.roles.iter().map(String::as_str).collect();
        match a.authenticated {
            true => outln!("Simulated: signed in with roles {}", roles.join(", ")),
            false => outln!("Simulated: anonymous"),
        }
    }
    for (i, c) in found.iter().enumerate() {
        print_chain(c);
        if let Some((_, rights)) = simulation {
            let r = &rights[i];
            outln!("  right: {} · {}", label(&r.verdict), r.reasons.join("; "));
        }
    }
}

/// `POST /api/orders → Handler · file:line` or `message Event → Handler ·
/// file:line`, then handler, screen, and each transition with its events and
/// their handlers.
fn print_chain(c: &Chain) {
    let use_cases: Vec<String> = c.use_cases.iter().map(|u| u.name.clone()).collect();
    let target = if use_cases.is_empty() {
        "no use case found".to_string()
    } else {
        use_cases.join(", ")
    };
    let (handler, sends) = match c.start {
        ChainStart::Endpoint { endpoint: e, .. } => {
            outln!("{} {} → {target} · {}:{}", e.method, e.path, e.file, e.line);
            (e.handler.as_deref(), &e.sends)
        }
        ChainStart::Message { message_handler: m } => {
            outln!("message {} → {target} · {}:{}", m.message, m.file, m.line);
            (Some(m.id.as_str()), &m.sends)
        }
    };
    if handler.is_some() || !sends.is_empty() {
        outln!(
            "  handler {} · sends {}",
            handler.unwrap_or("-"),
            list(sends)
        );
    }
    if let Some(a) = c.action() {
        let screen = match &a.screen {
            Origin::Screen(id) => format!("screen {id}"),
            Origin::AnyScreen { application } => format!("any screen of {application:?}"),
        };
        outln!("  from {screen} · {}:{}", a.file, a.line);
    }
    for t in &c.transitions {
        let tr = t.transition;
        let from = if tr.from.is_empty() {
            "any".to_string()
        } else {
            tr.from.join(", ")
        };
        outln!("  {}: {from} → {} · {}", t.entity, tr.to, tr.method);
        for ev in c
            .events
            .iter()
            .filter(|ev| tr.events.iter().any(|n| n == ev.event))
        {
            outln!("    → {} → {}", ev.event, list(&ev.handlers));
        }
    }
}

/// The part of `model` about one entity: its relations, state machines,
/// the use cases calling it, the events it raises and the gaps in those files.
fn only_entity(mut model: DomainModel, name: &str) -> Option<DomainModel> {
    let entity = (model.entities.iter().find(|e| e.id == name))
        .or_else(|| model.entities.iter().find(|e| e.name == name))?
        .clone();
    let id = entity.id.as_str();
    let method = |m: &String| m.split_once('.').is_some_and(|(owner, _)| owner == id);
    model.relations.retain(|r| r.from == id || r.to == id);
    model.state_machines.retain(|m| m.entity == id);
    model.use_cases.retain(|u| u.calls.iter().any(method));
    model.events.retain(|e| e.raised_by.iter().any(method));
    let files: BTreeSet<&str> = std::iter::once(entity.file.as_str())
        .chain(model.use_cases.iter().map(|u| u.file.as_str()))
        .chain(
            (model.state_machines.iter())
                .flat_map(|m| m.transitions.iter().map(|t| t.file.as_str())),
        )
        .collect();
    model
        .diagnostics
        .retain(|d| files.contains(d.file.as_str()));
    model.entities = vec![entity];
    Some(model)
}

fn print_text(model: &DomainModel, verbose: bool) {
    let s = summary(model);
    outln!(
        "Entities {} · state machines {} · transitions {} · use cases {} · events {} · complete: {}",
        s.entities,
        s.state_machines,
        s.transitions,
        s.use_cases,
        s.events,
        yes_no(s.complete)
    );
    let mut entities: Vec<&Entity> = model.entities.iter().collect();
    entities.sort_by_key(|e| (e.kind, &e.id));
    for entity in entities {
        print_entity(model, entity);
    }
    model.state_machines.iter().for_each(print_state_machine);
    outln!("Use cases: {}", model.use_cases.len());
    for u in &model.use_cases {
        outln!(
            "  {} {} · request {} · calls {} · {}:{}",
            label(&u.kind),
            u.name,
            u.request.as_deref().unwrap_or("-"),
            list(&u.calls),
            u.file,
            u.line
        );
    }
    outln!("Events: {}", model.events.len());
    for e in &model.events {
        outln!(
            "  {}: raised by {} → handled by {}",
            e.type_name,
            list(&e.raised_by),
            list(&e.handled_by)
        );
    }
    let gaps: Vec<String> = (model.diagnostics.iter())
        .map(|d| format!("{}:{}: {}", d.file, d.line, d.message))
        .collect();
    print_gaps(&gaps, verbose);
}

fn print_entity(model: &DomainModel, entity: &Entity) {
    outln!(
        "{} {} · {}:{}",
        label(&entity.kind),
        entity.id,
        entity.file,
        entity.line
    );
    for r in &model.relations {
        let kind = label(&r.kind);
        if r.from == entity.id {
            outln!("  {kind} {} via {}", r.to, r.via);
        } else if r.to == entity.id {
            outln!("  {kind} from {} via {}", r.from, r.via);
        }
    }
}

fn print_state_machine(machine: &StateMachine) {
    outln!(
        "State machine {}.{} ({})",
        machine.entity,
        machine.field,
        machine.enum_type
    );
    outln!("  states: {}", list(&machine.states));
    outln!("  initial: {}", machine.initial.as_deref().unwrap_or("-"));
    outln!("  from → to · method · guard (allowed when) · events");
    for t in &machine.transitions {
        let from = if t.from.is_empty() {
            "any".to_string()
        } else {
            t.from.join(", ")
        };
        outln!(
            "  {from} → {} · {} · {} · {} · {}:{}",
            t.to,
            t.method,
            t.guard.as_deref().unwrap_or("-"),
            list(&t.events),
            t.file,
            t.line
        );
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "-".into()
    } else {
        items.join(", ")
    }
}
