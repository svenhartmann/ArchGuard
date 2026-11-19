//! `archguard domain`: entities, state machines, use cases and domain events
//! (ADR 0022), optionally limited to one entity.

use std::collections::BTreeSet;
use std::path::Path;

use archguard_engine::core::domain_model::{DomainModel, Entity, StateMachine};
use archguard_engine::domain_model::summary;
use serde_json::json;

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use crate::ui_flow_cmd::{label, print_gaps, yes_no};

pub(crate) fn run(cli: &Cli, root: &Path, entity: Option<&str>) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("domain supports --format text|json");
    }
    let mut model = match archguard_engine::domain_model::analyze(root) {
        Ok(model) => model,
        Err(error) => return analysis_error(error),
    };
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
    outln!("  from → to · method · guard · events");
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
