//! Stable ids of the Domain view's items that take a note for the coding
//! agent, in the shape of the convention ids (`view-<rule>:<file>:<subject>`,
//! ADR 0021; see `screens_ui::notes`):
//!
//! - a rule: `view-domain_rule:<file>:<method>`, e.g.
//!   `view-domain_rule:Ordering.Domain/…/Order.cs:Order.SetShippedStatus`;
//!   several rules of one method in one file are numbered `#2`, `#3` in
//!   model order;
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
    numbered((d.rules.iter()).map(|r| id("domain_rule", &r.file, &r.method)))
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

    #[test]
    fn ids_follow_the_convention_shape_and_number_repeats() {
        let rule = |method: &str| DomainRule {
            entity: "Order".into(),
            method: method.into(),
            condition: None,
            message: None,
            exception: "X".into(),
            file: "Order.cs".into(),
            line: 1,
            source: "t".into(),
        };
        let d = DomainModel {
            rules: vec![rule("Order.Ship"), rule("Order.Pay"), rule("Order.Ship")],
            integration_events: vec![IntegrationEvent {
                type_name: "Paid".into(),
                published_by: vec![Publisher {
                    by: "H.Handle".into(),
                    service: "Ordering.API".into(),
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
                "view-domain_rule:Order.cs:Order.Ship",
                "view-domain_rule:Order.cs:Order.Pay",
                "view-domain_rule:Order.cs:Order.Ship#2",
            ]
        );
        assert_eq!(integration_event(&d, 0), "view-integration_event:H.cs:Paid");
    }
}
