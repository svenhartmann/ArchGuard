//! Tests for the class view state, ticket 03.

use std::sync::Arc;

use archguard_engine::core::ServiceTopology;

use super::types::{Group, Key, Switch, Switches};
use crate::es_ui::state::{EsState, build_from};

fn state() -> EsState {
    build_from(Arc::new(Ok(ServiceTopology::default())))
}

#[test]
fn defaults_and_changes() {
    let mut s = state();
    assert_eq!(s.selected, None);
    assert!(!s.cluster);
    assert_eq!(s.switches, Switches::default());
    assert!(s.switches.services && s.switches.analysis && s.switches.multi_fields);
    assert!(!s.switches.collapse_fields);
    assert_eq!(s.query, "");
    assert_eq!(s.group, Group::All);

    s.problem = "error".into();
    s.select(Some(Key::Node(2)));
    assert_eq!(s.selected, Some(Key::Node(2)));
    assert!(s.problem.is_empty());

    s.toggle_cluster();
    assert!(s.cluster);
    s.toggle_cluster();
    assert!(!s.cluster);

    s.set_switch(Switch::Services, false);
    s.set_switch(Switch::CollapseFields, true);
    assert!(!s.switches.services && s.switches.collapse_fields);
    s.set_switch(Switch::Analysis, false);
    s.set_switch(Switch::MultiFields, false);
    assert!(!s.switches.analysis && !s.switches.multi_fields);

    s.set_query("abc");
    assert_eq!(s.query, "abc");
    s.set_group(Group::Aliases);
    assert_eq!(s.group, Group::Aliases);
}

#[test]
fn a_field_selection_is_an_index_key_and_a_path_that_another_selection_drops() {
    let mut s = state();
    s.set_field(Key::Node(2), "title");
    assert_eq!(s.field, Some((Key::Node(2), "title".to_string())));
    assert_eq!(
        s.selected,
        Some(Key::Node(2)),
        "the index is selected with it"
    );
    let tick = s.fit_tick;
    s.set_field(Key::Node(2), "body");
    assert_eq!(s.field, Some((Key::Node(2), "body".to_string())));
    assert_eq!(s.fit_tick, tick, "the same index keeps the view");

    s.select(Some(Key::Node(3)));
    assert_eq!(s.field, None);
    s.set_field(Key::Node(2), "title");
    s.row_keys = vec![Key::Node(5)];
    s.pick_row(0);
    assert_eq!(s.field, None, "a list row selects the index, not the field");
    s.set_field(Key::Node(2), "title");
    s.clear_field();
    assert_eq!(s.field, None);
    assert_eq!(s.selected, Some(Key::Node(2)));
}
