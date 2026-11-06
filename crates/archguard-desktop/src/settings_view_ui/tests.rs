//! Tests of the Settings view's state and data. Each test writes its
//! own configuration into a temp dir; no fixture is edited.

use std::fs;
use std::path::Path;

use archguard_engine::core::{Config, Finding, FindingKind, LayerConfig, LayerOrigin, Severity};
use slint::Model;

use super::form::{ARRAY, ENTRY, FIELD, HEADING, NOTE};
use super::preview_panel::{self, LISTED, PreviewStatus};
use super::session::SettingsState;
use crate::config_editor::FIELDS;
use crate::config_editor::{CheckSummary, Group, Preview};
use crate::documents::{self, DocId, with_store};
use crate::tab_support::Texts;
use crate::ui::SettingsItem;

/// A project with `text` as `.archguard/archguard.toml`, open as a
/// document, and its Settings state.
fn project(text: &str) -> (tempfile::TempDir, DocId, SettingsState) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".archguard")).unwrap();
    fs::write(config_file(dir.path()), text).unwrap();
    let (doc, _) = with_store(|s| s.open(dir.path()));
    let state = SettingsState::open(doc, dir.path().to_path_buf(), None);
    (dir, doc, state)
}

fn config_file(root: &Path) -> std::path::PathBuf {
    root.join(".archguard/archguard.toml")
}

fn in_group(s: &mut SettingsState, group: Group) -> Vec<SettingsItem> {
    s.group = group;
    s.items().0
}

fn item<'a>(items: &'a [SettingsItem], path: &str) -> &'a SettingsItem {
    items
        .iter()
        .find(|i| i.kind == FIELD && i.path == path)
        .unwrap_or_else(|| panic!("no row {path}"))
}

fn kinds(items: &[SettingsItem]) -> Vec<i32> {
    items.iter().map(|i| i.kind).collect()
}

/// A group's rows come from `fields()`: label, widget by kind,
/// provenance and Reset only for values written in the file.
#[test]
fn a_group_lists_its_fields_with_provenance() {
    let (_dir, _, mut s) = project("[rules]\nmax_acd = 3.5\n");
    let items = in_group(&mut s, Group::Rules);
    assert!(
        items.iter().all(|i| i.kind == FIELD),
        "no heading for [rules]"
    );
    let paths: Vec<_> = items.iter().map(|i| i.path.to_string()).collect();
    assert_eq!(
        paths,
        [
            "rules.deny_cycles",
            "rules.max_scc_size",
            "rules.baseline",
            "rules.max_acd",
            "rules.max_instability_core"
        ]
    );
    let acd = item(&items, "rules.max_acd");
    assert_eq!((acd.label.as_str(), acd.text.as_str()), ("max_acd", "3.5"));
    assert_eq!((acd.widget, acd.provenance, acd.can_reset), (2, 1, true));
    let cycles = item(&items, "rules.deny_cycles");
    assert_eq!(
        (cycles.widget, cycles.provenance, cycles.can_reset),
        (3, 2, false)
    );
    let options: Vec<_> = cycles.options.iter().map(|o| o.to_string()).collect();
    assert_eq!(options, ["module", "file", "off"]);
    assert_eq!(cycles.option, 0);
    assert!(!cycles.description.is_empty());
    let semantic = in_group(&mut s, Group::Semantic);
    let headings: Vec<_> = semantic
        .iter()
        .filter(|i| i.kind == HEADING)
        .map(|i| i.label.to_string())
        .collect();
    assert_eq!(
        headings,
        ["semantic.java", "semantic.csharp", "semantic.typescript"]
    );
}

/// Derived fields are read-only; without an analysis they have no value.
#[test]
fn derived_fields_are_read_only() {
    let (_dir, _, mut s) = project("");
    let items = in_group(&mut s, Group::Project);
    let roots = item(&items, "project.source_roots");
    assert_eq!((roots.provenance, roots.read_only), (3, true));
    assert!(!item(&items, "project.exclude").read_only);
}

/// An edit marks the document dirty with its group; editing back to the
/// file's value makes it clean again.
#[test]
fn an_edit_marks_the_document_and_its_group() {
    let (dir, doc, mut s) = project("[rules]\nmax_acd = 3.5\n");
    assert!(!documents::is_dirty(doc));
    s.edit("rules.max_acd", "2");
    assert!(documents::is_dirty(doc));
    assert_eq!(
        with_store(|st| st.get(doc).map(|d| d.unsaved())),
        Some("Rules and thresholds".into())
    );
    assert_eq!(s.edited(), [Group::Rules]);
    let groups = s.groups();
    let rules = &groups[5];
    assert!(rules.edited && !rules.failed && !groups[0].edited);
    assert_eq!(rules.matches, -1);
    s.edit("rules.max_acd", "3.5");
    assert!(!documents::is_dirty(doc));
    assert!(s.edited().is_empty());
    assert_eq!(
        fs::read_to_string(config_file(dir.path())).unwrap(),
        "[rules]\nmax_acd = 3.5\n"
    );
}

/// Input that is no value of the field stays as typed with its error;
/// the file keeps its value and Save is blocked.
#[test]
fn invalid_input_stays_on_its_row() {
    let (_dir, doc, mut s) = project("[rules]\nmax_acd = 3.5\n");
    s.edit("rules.max_acd", "abc");
    let items = in_group(&mut s, Group::Rules);
    let acd = item(&items, "rules.max_acd");
    assert_eq!(
        (acd.text.as_str(), acd.error.as_str()),
        ("abc", "expected a number")
    );
    assert_eq!(s.errors(), 1);
    assert!(!documents::is_dirty(doc), "the file's value stays");
    assert!(s.groups()[5].failed);
    s.save();
    assert!(
        s.save_error.is_empty() && !s.changed_on_disk,
        "nothing tried"
    );
    s.reset("rules.max_acd");
    assert_eq!(s.errors(), 0);
}

/// The engine's validation problem shows on the row it names, else as
/// the view's general error.
#[test]
fn a_validation_error_lands_on_its_row() {
    let (_dir, _, mut s) = project("");
    s.edit("performance.min_barrel_ratio", "2");
    let (items, placed) = {
        s.group = Group::Performance;
        s.items()
    };
    assert!(placed);
    let ratio = item(&items, "performance.min_barrel_ratio");
    assert!(!ratio.error.is_empty(), "{ratio:?}");
    assert_eq!(s.errors(), 1);
    assert!(s.groups()[7].failed);
    let (_, placed) = {
        s.group = Group::Rules;
        s.items()
    };
    assert!(!placed, "another group: the general error");
}

/// Table arrays: add, move and remove entries as cards; an empty array
/// shows a note.
#[test]
fn table_array_entries_are_cards() {
    let (_dir, doc, mut s) = project("");
    let items = in_group(&mut s, Group::DependencyRules);
    assert_eq!(kinds(&items), [ARRAY, NOTE]);
    s.add_entry("dependency_rules");
    s.add_entry("dependency_rules");
    s.edit("dependency_rules[0].name", "first");
    s.edit("dependency_rules[1].name", "second");
    assert!(documents::is_dirty(doc));
    let items = in_group(&mut s, Group::DependencyRules);
    let entries: Vec<_> = items.iter().filter(|i| i.kind == ENTRY).collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        (
            entries[0].text.as_str(),
            entries[0].can_up,
            entries[0].can_down
        ),
        ("first", false, true)
    );
    assert_eq!(
        (entries[1].index, entries[1].can_up, entries[1].can_down),
        (1, true, false)
    );
    assert!(item(&items, "dependency_rules[1].name").nested);
    s.move_entry("dependency_rules", 1, 0);
    let items = in_group(&mut s, Group::DependencyRules);
    assert_eq!(item(&items, "dependency_rules[0].name").text, "second");
    s.remove_entry("dependency_rules", 0);
    let items = in_group(&mut s, Group::DependencyRules);
    assert_eq!(item(&items, "dependency_rules[0].name").text, "first");
    assert_eq!(items.iter().filter(|i| i.kind == ENTRY).count(), 1);
}

/// Derived layers follow the declared ones, read-only.
#[test]
fn derived_layers_follow_the_file_entries() {
    let (_dir, _, mut s) = project("[[layers]]\nname = \"api\"\ncontains = [\"api/**\"]\n");
    let mut config = Config::default();
    config.layers.push(LayerConfig {
        name: "domain".into(),
        contains: vec!["domain/**".into()],
        may_depend_on: Vec::new(),
        origin: LayerOrigin::Derived,
    });
    s.effective = Some(config);
    let items = in_group(&mut s, Group::Layers);
    let entries: Vec<_> = items.iter().filter(|i| i.kind == ENTRY).collect();
    assert_eq!(entries.len(), 2, "{items:?}");
    assert_eq!(
        (entries[0].text.as_str(), entries[0].read_only),
        ("api", false)
    );
    assert_eq!(
        (
            entries[1].text.as_str(),
            entries[1].index,
            entries[1].read_only
        ),
        ("domain", -1, true)
    );
    assert!(
        !entries[0].can_down,
        "a derived layer is no place to move to"
    );
    let derived_rows = items
        .iter()
        .filter(|i| i.kind == FIELD && i.read_only)
        .count();
    assert_eq!(derived_rows, 3, "name, contains, may_depend_on");
}

/// A search lists matching fields of every group under its heading.
#[test]
fn a_search_spans_the_groups() {
    let (_dir, _, mut s) = project("");
    s.query = "SINCE".into();
    let items = s.items().0;
    assert_eq!(kinds(&items), [HEADING, FIELD]);
    assert_eq!(
        (items[0].label.as_str(), items[1].path.as_str()),
        ("Review", "review.git_since")
    );
    let groups = s.groups();
    assert_eq!(groups[6].matches, 1);
    assert_eq!(groups[0].matches, 0);
    s.query = "may_depend".into();
    let items = s.items().0;
    assert_eq!(kinds(&items), [HEADING, ARRAY, NOTE], "the whole array");
}

/// Descriptions moved into tooltips stay searchable and reach the row.
#[test]
fn a_search_finds_a_field_by_its_description() {
    let (_dir, _, mut s) = project("");
    s.query = "history window".into();
    let items = s.items().0;
    let row = item(&items, "review.git_since");
    assert!(
        row.description
            .starts_with("History window for change counts")
    );
}

/// Every group name and field description of the catalogue has a
/// German template, so a new field cannot stay English.
#[test]
fn every_catalogue_text_has_german() {
    let de = Texts::german();
    let texts = Group::ALL.iter().map(|g| g.label());
    let texts = texts.chain(FIELDS.iter().map(|f| f.description));
    let missing: Vec<_> = texts.filter(|t| de.t(t) == *t).collect();
    assert!(missing.is_empty(), "no German for {missing:?}");
}

/// In German, groups, headings and descriptions read German, and the
/// search finds a field by its German or English description.
#[test]
fn german_texts_are_searchable_in_both_languages() {
    let (_dir, _, mut s) = project("");
    s.texts = Texts::german();
    assert_eq!(s.groups()[2].label, "Schichten");
    for query in ["zeitfenster", "history window"] {
        s.query = query.into();
        let items = s.items().0;
        assert_eq!(items[0].label, "Prüfung", "heading for {query}");
        let row = item(&items, "review.git_since");
        assert!(row.description.starts_with("Zeitfenster"), "{query}");
    }
}

/// Save writes the file and makes the document clean; a file changed
/// on disk is reported, and Overwrite writes over it.
#[test]
fn save_and_a_file_changed_on_disk() {
    let (dir, doc, mut s) = project("[rules]\nmax_acd = 3.5\n");
    s.edit("rules.max_acd", "2.5");
    s.save();
    assert!(!documents::is_dirty(doc));
    assert!(s.edited().is_empty());
    let file = config_file(dir.path());
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "[rules]\nmax_acd = 2.5\n"
    );
    s.edit("rules.max_acd", "4");
    fs::write(&file, "# changed elsewhere\n[rules]\nmax_acd = 9.0\n").unwrap();
    s.save();
    assert!(s.changed_on_disk, "{}", s.save_error);
    assert!(documents::is_dirty(doc));
    s.overwrite();
    assert!(!s.changed_on_disk && s.save_error.is_empty());
    assert!(!documents::is_dirty(doc));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "[rules]\nmax_acd = 4.0\n"
    );
}

/// Revert reads the file again and drops drafts and edits.
#[test]
fn revert_drops_every_edit() {
    let (_dir, doc, mut s) = project("[rules]\nmax_acd = 3.5\n");
    s.edit("rules.max_acd", "2");
    s.edit("rules.max_scc_size", "x");
    s.reload();
    assert!(!documents::is_dirty(doc));
    assert_eq!(s.errors(), 0);
    let items = in_group(&mut s, Group::Rules);
    assert_eq!(item(&items, "rules.max_acd").text, "3.5");
}

/// The text pane keeps text that does not parse with its error and line;
/// valid text replaces the configuration.
#[test]
fn the_text_pane_reports_the_parse_error() {
    let (_dir, doc, mut s) = project("[rules]\nmax_acd = 3.5\n");
    s.edit_text("[rules]\nmax_acd = \n");
    let draft = s.text_draft.clone().expect("kept");
    assert!(
        draft.error.as_deref().is_some_and(|e| e.contains("line 2")),
        "{draft:?}"
    );
    assert_eq!(s.errors(), 1);
    assert!(!documents::is_dirty(doc));
    s.edit_text("[rules]\nmax_acd = 1.5\n");
    assert!(s.text_draft.is_none());
    assert!(documents::is_dirty(doc));
    let items = in_group(&mut s, Group::Rules);
    assert_eq!(item(&items, "rules.max_acd").text, "1.5");
}

/// A new layer without a name is invalid; the error shows in its card.
#[test]
fn an_entry_error_lands_in_its_card() {
    let (_dir, _, mut s) = project("derive_layers = false\n");
    s.add_entry("layers");
    let invalid = s.invalid.clone().expect("a layer needs a name");
    let (items, placed) = {
        s.group = Group::Layers;
        s.items()
    };
    assert!(placed, "{invalid:?}");
    let shown: Vec<_> = items.iter().filter(|i| !i.error.is_empty()).collect();
    assert_eq!(shown.len(), 1);
    assert!(
        shown[0].kind == ENTRY || shown[0].path.starts_with("layers[0]"),
        "{shown:?}"
    );
    assert!(s.groups()[2].failed);
}

/// The engine names some paths only at the start of its message
/// (`dependency_rules[0].to must contain …`); the error still lands on
/// that row.
#[test]
fn a_path_at_the_start_of_the_message_finds_its_row() {
    let (_dir, _, mut s) = project("");
    s.add_entry("dependency_rules");
    s.edit("dependency_rules[0].name", "r");
    s.edit("dependency_rules[0].from", "web/**");
    let invalid = s.invalid.clone().expect("`to` is empty");
    let (items, placed) = {
        s.group = Group::DependencyRules;
        s.items()
    };
    assert!(placed, "{invalid:?}");
    let to = item(&items, "dependency_rules[0].to");
    assert!(to.error.contains("at least one pattern"), "{to:?}");
}

/// A file that is no TOML does not load; the view says why.
#[test]
fn an_unreadable_file_is_a_load_error() {
    let (_dir, _, s) = project("[rules\n");
    assert!(s.editor.as_ref().is_err());
    assert!(s.items().0.is_empty());
    assert_eq!(s.file(), "");
}

fn finding(id: &str, severity: Severity) -> Finding {
    Finding {
        kind: FindingKind::MetricThreshold,
        id: id.into(),
        severity,
        message: Some(format!("{id} message")),
        evidence: Vec::new(),
        nodes: vec!["a".into()],
        edge_count: 0,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: Vec::new(),
    }
}

fn summary(pass: bool, findings: Vec<Finding>) -> CheckSummary {
    let count = |s: Severity| findings.iter().filter(|f| f.severity == s).count();
    CheckSummary {
        pass,
        errors: count(Severity::Error),
        warnings: count(Severity::Warning),
        infos: count(Severity::Info),
        findings,
    }
}

/// The preview's result as the panel shows it: before and after, counts,
/// added and removed findings with the rest counted.
#[test]
fn a_preview_result_maps_to_the_panel() {
    let added: Vec<_> = (0..LISTED + 2)
        .map(|i| finding(&format!("m-{i}"), Severity::Error))
        .collect();
    let removed = vec![finding("c-1", Severity::Warning)];
    let preview = Preview {
        before: Ok(summary(true, removed.clone())),
        after: summary(false, added.clone()),
        added,
        removed,
    };
    let v = preview_panel::view(&PreviewStatus::Done(Box::new(preview.clone())));
    assert_eq!(v.state, 2);
    assert!(v.before_pass && !v.after_pass && v.changed);
    assert_eq!((v.before_errors, v.before_warnings), (0, 1));
    assert_eq!(v.after_errors, LISTED as i32 + 2);
    assert_eq!((v.added.row_count(), v.added_more), (LISTED, 2));
    let first = v.added.row_data(0).unwrap();
    assert_eq!(
        (
            first.severity.as_str(),
            first.id.as_str(),
            first.text.as_str()
        ),
        ("error", "m-0", "m-0 message")
    );
    assert_eq!(v.removed.row_data(0).unwrap().severity, "warning");
    let failed_before = Preview {
        before: Err("bad saved config".into()),
        ..preview
    };
    let v = preview_panel::view(&PreviewStatus::Done(Box::new(failed_before)));
    assert_eq!(v.before_error, "bad saved config");
    assert_eq!(preview_panel::view(&PreviewStatus::Running(1)).state, 1);
    let v = preview_panel::view(&PreviewStatus::Failed("rules.x: bad".into()));
    assert_eq!((v.state, v.error.as_str()), (3, "rules.x: bad"));
    assert_eq!(preview_panel::view(&PreviewStatus::Idle).state, 0);
}
