//! Tests for the class view model (state to Slint data), ticket 12.

use std::path::Path;
use std::sync::Arc;

use archguard_engine::analyze_services;
use serde_json::json;
use slint::Model;

use super::types::{Group, Key};
use super::view::{build, fit};
use crate::es_ui::state::{EsState, build_from};
use crate::ui::{EsClassBox, EsClassModel, EsClassRow};

fn state() -> EsState {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/12");
    let mut s = build_from(Arc::new(Ok(analyze_services(&root).expect("fixture"))));
    s.root = root;
    s
}

fn rows(m: &EsClassModel) -> Vec<EsClassRow> {
    m.groups
        .iter()
        .flat_map(|g| g.rows.iter().collect::<Vec<_>>())
        .collect()
}

fn boxes(m: &EsClassModel) -> Vec<EsClassBox> {
    m.boxes.iter().collect()
}

fn selected(m: &EsClassModel) -> Vec<String> {
    boxes(m)
        .into_iter()
        .filter(|b| b.selected)
        .map(|b| b.name.to_string())
        .collect()
}

/// The flat position of the row labelled `label`.
fn row_of(m: &EsClassModel, label: &str) -> usize {
    rows(m)
        .iter()
        .position(|r| r.label == label)
        .unwrap_or_else(|| panic!("no row {label}"))
}

#[test]
fn the_model_has_a_list_a_diagram_and_the_details_of_the_first_index() {
    let mut s = state();
    let m = build(&mut s);
    assert_eq!(m.state, 1);
    assert_eq!(m.count as usize, rows(&m).len());
    assert_eq!(s.row_keys.len(), rows(&m).len());
    assert!(!boxes(&m).is_empty() && boxes(&m).len() == s.box_keys.len());
    assert_eq!(selected(&m), ["produkte_aktuell"]);
    assert_eq!(m.details.head.title, "produkte_aktuell");
    assert_eq!(m.details.mode, 0);
}

#[test]
fn a_selected_list_row_redraws_the_diagram_around_that_box() {
    let mut s = state();
    let m = build(&mut s);
    let i = row_of(&m, "produkte-2026-01");
    s.pick_row(i);
    let m = build(&mut s);
    assert_eq!(selected(&m), ["produkte-2026-01"]);
    assert_eq!(m.details.head.title, "produkte-2026-01");
    assert!(
        rows(&m)
            .iter()
            .any(|r| r.selected && r.label == "produkte-2026-01")
    );
    let names: Vec<String> = boxes(&m).iter().map(|b| b.name.to_string()).collect();
    assert!(
        !names.contains(&"produkte_aktuell".to_string()),
        "{names:?}"
    );
}

#[test]
fn a_clicked_box_selects_it_and_a_part_shows_its_block_details() {
    let mut s = state();
    build(&mut s);
    let i = s
        .box_keys
        .iter()
        .position(|k| matches!(k, Key::Part { name, .. } if name == "german_custom"))
        .expect("the analyzer is in the neighbourhood");
    s.pick_box(i);
    let m = build(&mut s);
    assert_eq!(selected(&m), ["german_custom"]);
    assert_eq!(m.details.mode, 1);
    assert!(m.details.chain.row_count() > 0);
    let b = boxes(&m).into_iter().find(|b| b.selected).expect("box");
    assert_eq!(b.kind, 4, "an analysis block");
}

#[test]
fn the_cluster_link_shows_every_key_of_the_list() {
    let mut s = state();
    let near = build(&mut s).boxes.row_count();
    s.toggle_cluster();
    let m = build(&mut s);
    assert!(m.cluster && m.boxes.row_count() > near);
    assert_eq!(m.crumb_count as usize, m.boxes.row_count());
    for k in &s.row_keys {
        assert!(s.box_keys.contains(k), "{k:?} has no box");
    }
}

#[test]
fn row_tops_follow_the_group_header_and_row_pitch() {
    let mut s = state();
    let m = build(&mut s);
    let mut y = 0.0;
    let mut index = 0;
    for g in m.groups.iter() {
        y += 30.0;
        for r in g.rows.iter() {
            assert_eq!((r.index, r.y), (index, y), "{}", r.label);
            index += 1;
            y += 26.0;
        }
    }
    assert!(index > 8);
}

#[test]
fn search_and_kind_filter_narrow_the_list() {
    let mut s = state();
    s.set_query("2026");
    let m = build(&mut s);
    assert_eq!(rows(&m).len(), 1);
    s.set_query("");
    s.set_group(Group::Analysis);
    let m = build(&mut s);
    assert_eq!(m.kind, 5);
    assert!(
        rows(&m).iter().all(|r| !r.side.is_empty()),
        "analysis rows carry their kind"
    );
}

#[test]
fn a_usage_that_cannot_open_goes_into_problem_and_select_clears_it() {
    let mut s = state();
    build(&mut s);
    s.open_usage("../outside.py", 3);
    assert!(build(&mut s).problem.contains("not a file of this project"));
    s.pick_row(0);
    assert_eq!(build(&mut s).problem, "");
}

#[test]
fn fit_scales_the_canvas_into_the_viewport() {
    let mut s = state();
    let m = build(&mut s);
    fit(&mut s, 600.0, 400.0);
    let z = ((600.0f32 - 24.0) / m.canvas_w).min((400.0 - 24.0) / m.canvas_h);
    assert!((s.zoom - z).abs() < 1e-4 && s.zoom < 1.0, "{}", s.zoom);
}

#[test]
fn a_ghost_has_a_dashed_border_and_the_ghost_index_a_root() {
    let mut s = state();
    let m = build(&mut s);
    let ghost = boxes(&m)
        .into_iter()
        .find(|b| b.ghost)
        .expect("the ILM ghost");
    assert!(ghost.border.starts_with('M'), "{}", ghost.border);
    assert!(
        boxes(&m)
            .iter()
            .filter(|b| !b.ghost)
            .all(|b| b.border.is_empty())
    );
}

#[test]
fn data_streams_are_processing_in_list_and_diagram() {
    let es = json!({"artifacts": [{"kind": "data_stream", "name": "logs-app", "address": "r.logs",
        "scope": "terraform:infra", "patterns": [], "references": [], "evidence": []}], "usages": []});
    let t = archguard_engine::core::ServiceTopology {
        elasticsearch: serde_json::from_value(es).unwrap(),
        ..Default::default()
    };
    let mut s = build_from(Arc::new(Ok(t)));
    let m = build(&mut s);
    let g = m.groups.row_data(0).expect("a group");
    assert_eq!(g.title, 5, "Processing");
    assert_eq!(boxes(&m)[0].kind, 5);
    assert_eq!(boxes(&m)[0].stereotype, "data_stream");
}
