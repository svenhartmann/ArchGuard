//! Stable ids of the Domain view's items that take a note for the coding
//! agent, in the shape of the convention ids (`view-<rule>:<file>:<subject>`,
//! ADR 0021; see `screens_ui::notes`):
//!
//! - a rule: `view-domain_rule:<file>:<Class.Method>:<condition>`, the
//!   condition as source text with whitespace collapsed, or `always` for an
//!   unconditional throw, e.g.
//!   `view-domain_rule:Ordering.Domain/…/Order.cs:Order.SetShippedStatus:OrderStatus != OrderStatus.Paid`.
//!   Only exact duplicates (same method and condition in one file) get
//!   `#2`, `#3`, numbered by line. A changed condition leaves the old note
//!   without its rule (stale) instead of moving it to another rule. Limit:
//!   the model names a method without its parameters, so overloads in one
//!   file with the same condition are told apart by line order only;
//! - an integration event: `view-integration_event:<file>:<type>`, the file
//!   of its first publish site, else of its first handler;
//! - a domain event: `view-domain_event:<file>:<type>`, the file of the
//!   first transition raising it, else empty.
//!
//! The note is a decision of `.archguard/reviews.json` under that id, read
//! by the MCP tool `review_decisions` like every other note.

use archguard_engine::core::domain_model::DomainModel;

use crate::screens_ui::notes::{id, numbered};

/// One id per rule, in model order.
pub fn rules(d: &DomainModel) -> Vec<String> {
    let base = |k: usize| {
        let r = &d.rules[k];
        let condition = (r.condition.as_deref()).map_or("always".to_string(), |c| {
            c.split_whitespace().collect::<Vec<_>>().join(" ")
        });
        id("domain_rule", &r.file, &format!("{}:{condition}", r.method))
    };
    // Duplicates are numbered by place in the file, not by model order.
    let mut by_line: Vec<usize> = (0..d.rules.len()).collect();
    by_line.sort_by_key(|&k| (&d.rules[k].file, d.rules[k].line));
    let numbered = numbered(by_line.iter().map(|&k| base(k)));
    let mut out = vec![String::new(); d.rules.len()];
    for (k, id) in by_line.into_iter().zip(numbered) {
        out[k] = id;
    }
    out
}

pub fn integration_event(d: &DomainModel, i: usize) -> String {
    let e = &d.integration_events[i];
    let file = (e.published_by.first().map(|p| p.file.as_str()))
        .or_else(|| {
            (d.message_handlers.iter())
                .find(|h| h.message == e.type_name)
                .map(|h| h.file.as_str())
        })
        .unwrap_or("");
    id("integration_event", file, &e.type_name)
}

pub fn domain_event(d: &DomainModel, i: usize) -> String {
    let e = &d.events[i];
    let file = (d.state_machines.iter().flat_map(|m| &m.transitions))
        .find(|t| t.events.contains(&e.type_name))
        .map_or("", |t| t.file.as_str());
    id("domain_event", file, &e.type_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{DomainRule, IntegrationEvent, Publisher};

    fn rule(method: &str, condition: Option<&str>, line: u32) -> DomainRule {
        DomainRule {
            entity: "Order".into(),
            method: method.into(),
            condition: condition.map(Into::into),
            message: None,
            exception: "X".into(),
            file: "Order.cs".into(),
            line,
            source: "t".into(),
        }
    }

    #[test]
    fn ids_carry_method_and_condition_and_number_only_exact_repeats() {
        let d = DomainModel {
            // Model order is the engine's sort, not the line order.
            rules: vec![
                rule("Order.Ship", None, 20),
                rule("Order.Ship", Some("a  <\n 0"), 12),
                rule("Order.Ship", None, 10),
            ],
            integration_events: vec![IntegrationEvent {
                type_name: "Paid".into(),
                published_by: vec![Publisher {
                    by: "H.Handle".into(),
                    service: "Ordering".into(),
                    file: "H.cs".into(),
                    line: 3,
                }],
                handled_by: vec![],
                source: "t".into(),
            }],
            ..DomainModel::default()
        };
        assert_eq!(
            rules(&d),
            [
                "view-domain_rule:Order.cs:Order.Ship:always#2",
                "view-domain_rule:Order.cs:Order.Ship:a < 0",
                "view-domain_rule:Order.cs:Order.Ship:always",
            ],
            "the earlier line keeps the bare id"
        );
        assert_eq!(integration_event(&d, 0), "view-integration_event:H.cs:Paid");
    }

    #[test]
    fn renaming_one_condition_leaves_the_other_rule_its_id() {
        let before = DomainModel {
            rules: vec![
                rule("Order.Pay", Some("units <= 0"), 10),
                rule("Order.Pay", Some("discount < 0"), 14),
            ],
            ..DomainModel::default()
        };
        let mut after = before.clone();
        after.rules[0].condition = Some("quantity <= 0".into());
        after.rules.reverse();
        let (old, new) = (rules(&before), rules(&after));
        assert!(new.contains(&old[1]), "discount rule keeps {}", old[1]);
        assert!(!new.contains(&old[0]), "the renamed rule's note goes stale");
    }
}
