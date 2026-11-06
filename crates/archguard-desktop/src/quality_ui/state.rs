//! All state of the Quality tab and the page it renders:
//! `engineeringUi`, `classHeatmapUi` and `renderEngineeringQuality`.

use std::collections::BTreeSet;

use super::card::{Card, Kind, Line};
use super::cohesion::Selection;
use super::heatmap::{self, Cohorts, Filter, METRICS, Sort, State};
use super::heatmap_text;
use super::inputs::Inputs;
use super::lang::{number, percent};
use super::notes::Section;
use super::rows::{self, Scope};
use super::{cards, cohesion, evidence};
use crate::tab_support::Texts;

/// Records per page and per "Show next 20".
pub const PAGE: usize = 20;
/// Targets listed at most in the scope picker.
const TARGETS: usize = 2000;

/// What the user chose (`engineeringUi`, `classHeatmapUi`).
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub section: Section,
    pub target: String,
    pub target_query: String,
    pub query: String,
    pub limit: usize,
    pub filter: Filter,
    pub sort: Sort,
    pub selected: Option<String>,
    pub cohesion: Selection,
}

impl Default for View {
    fn default() -> Self {
        View {
            section: Section::Heatmap,
            target: String::new(),
            target_query: String::new(),
            query: String::new(),
            limit: PAGE,
            filter: Filter::All,
            sort: Sort::Priority,
            selected: None,
            cohesion: Selection::default(),
        }
    }
}

pub struct QualityState {
    pub lang: Texts,
    /// `None` while the analysis runs.
    pub inputs: Option<Inputs>,
    pub view: View,
    names: BTreeSet<String>,
    cohorts: Cohorts,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeatCell {
    pub text: String,
    pub state: State,
    pub tip: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeatRow {
    pub class_id: String,
    pub label: String,
    pub file: String,
    pub selected: bool,
    pub cells: Vec<HeatCell>,
}

/// Everything the window shows for the tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub loading: bool,
    pub section: usize,
    pub scope_summary: String,
    pub targets: Vec<String>,
    pub target_index: usize,
    pub target_note: String,
    pub status: String,
    pub notes: Card,
    pub heatmap: bool,
    pub filter: usize,
    pub sort: usize,
    pub heat: Vec<HeatRow>,
    pub detail: Option<Card>,
    pub cards: Vec<Card>,
    pub more: bool,
    pub empty: String,
}

impl QualityState {
    pub fn loading(lang: Texts) -> Self {
        QualityState {
            lang,
            inputs: None,
            view: View::default(),
            names: BTreeSet::new(),
            cohorts: Cohorts::default(),
        }
    }

    pub fn new(lang: Texts, inputs: Inputs) -> Self {
        let names = inputs.source.nodes.iter().map(|n| n.name.clone()).collect();
        let cohorts = Cohorts::new(&inputs.code.classes);
        let section = if inputs.code.classes.is_empty() {
            Section::Cohesion
        } else {
            Section::Heatmap
        };
        QualityState {
            lang,
            view: View {
                section,
                ..View::default()
            },
            inputs: Some(inputs),
            names,
            cohorts,
        }
    }

    /// Render the page; may fix the selection (as the web does on render).
    pub fn page(&mut self) -> Page {
        let Some(inputs) = self.inputs.take() else {
            return Page {
                loading: true,
                ..Page::default()
            };
        };
        let page = self.render(&inputs);
        self.inputs = Some(inputs);
        page
    }

    fn render(&mut self, inputs: &Inputs) -> Page {
        let scope = rows::scope(&inputs.source, &self.view.target);
        let (total, cards, heat) = self.records(inputs, &scope);
        let mut page = Page {
            section: self.view.section.index(),
            status: self.status(inputs, total),
            notes: self.notes(inputs),
            heatmap: self.view.section == Section::Heatmap,
            filter: filter_index(self.view.filter),
            sort: sort_index(self.view.sort),
            cards,
            heat,
            more: self.view.limit < total,
            ..Page::default()
        };
        if total == 0 {
            page.empty = self
                .lang
                .t("No matching measurements. Missing data is not evidence of good quality.")
                .into();
        }
        self.targets(inputs, &mut page);
        page.detail = self.detail(inputs);
        page
    }

    fn status(&self, inputs: &Inputs, total: usize) -> String {
        let status = self
            .view
            .section
            .status(&self.lang, source_status(inputs, self.view.section));
        let counts = self.lang.f(
            "{shown} / {total} matching records",
            &[
                ("shown", total.min(self.view.limit).to_string()),
                ("total", total.to_string()),
            ],
        );
        format!("{status} · {counts}")
    }
}

/// `status` of the section's source report.
fn source_status(inputs: &Inputs, section: Section) -> Option<&str> {
    Some(match section {
        Section::Heatmap | Section::Classes | Section::Methods => &inputs.code.status,
        Section::Knowledge => &inputs.knowledge.as_ref()?.status,
        Section::Measurements => &inputs.measurements.status,
        Section::Cohesion => &inputs.cohesion.status,
        Section::History => &inputs.history.status,
        Section::Tests => &inputs.tests.status,
        Section::Mutations => &inputs.mutations.status,
        Section::Duplicates => &inputs.duplicates.status,
    })
}

fn source_diagnostics(inputs: &Inputs, section: Section) -> (&[String], &[String]) {
    let none: &[String] = &[];
    match section {
        Section::Heatmap | Section::Classes | Section::Methods => {
            (&inputs.code.diagnostics, &inputs.code.coverage_actions)
        }
        Section::Knowledge => (
            inputs.knowledge.as_ref().map_or(none, |k| &k.diagnostics),
            none,
        ),
        Section::Measurements => (&inputs.measurements.diagnostics, none),
        Section::Cohesion => (&inputs.cohesion.diagnostics, none),
        Section::History => (&inputs.history.diagnostics, none),
        Section::Tests => (&inputs.tests.diagnostics, none),
        Section::Mutations => (&inputs.mutations.diagnostics, none),
        Section::Duplicates => (&inputs.duplicates.diagnostics, none),
    }
}

fn filter_index(f: Filter) -> usize {
    match f {
        Filter::All => 0,
        Filter::High => 1,
        Filter::Low => 2,
        Filter::Unknown => 3,
    }
}

pub fn filter_of(i: usize) -> Filter {
    [Filter::All, Filter::High, Filter::Low, Filter::Unknown][i.min(3)]
}

fn sort_index(s: Sort) -> usize {
    match s {
        Sort::Priority => 0,
        Sort::Name => 1,
        Sort::Metric(i) => 2 + i,
    }
}

pub fn sort_of(i: usize) -> Sort {
    match i {
        0 => Sort::Priority,
        1 => Sort::Name,
        i => Sort::Metric((i - 2).min(METRICS.len() - 1)),
    }
}

impl QualityState {
    /// The section's records: total count, cards and heatmap rows (one page).
    fn records(&mut self, inputs: &Inputs, scope: &Scope) -> (usize, Vec<Card>, Vec<HeatRow>) {
        let (lang, names, v) = (&self.lang, &self.names, &self.view);
        let (q, n) = (v.query.as_str(), v.limit);
        // Count all rows, build cards for the first page only.
        macro_rules! list {
            ($rows:expr, $card:expr) => {{
                let rows = $rows;
                (
                    rows.len(),
                    rows.iter().take(n).map($card).collect(),
                    Vec::new(),
                )
            }};
        }
        match v.section {
            Section::Heatmap => {
                let rows = self.heat_rows(inputs, scope);
                let total = rows.len();
                (total, Vec::new(), rows.into_iter().take(n).collect())
            }
            Section::Classes => list!(rows::classes(&inputs.code.classes, scope, q), |r| {
                cards::class(lang, names, r)
            }),
            Section::Methods => list!(rows::methods(&inputs.code.methods, scope, q), |r| {
                cards::method(lang, names, r)
            }),
            Section::Knowledge => {
                let files = inputs
                    .knowledge
                    .as_ref()
                    .map_or(&[][..], |k| k.files.as_slice());
                list!(rows::knowledge(files, scope, q), |r| cards::knowledge(
                    lang, names, r
                ))
            }
            Section::Measurements => {
                list!(rows::measurements(&inputs.measurements.groups, q), |r| {
                    cards::measurement(lang, names, r)
                })
            }
            Section::Cohesion => self.cohesion_records(inputs, scope),
            Section::History => list!(
                rows::history(&inputs.history, &inputs.source, &v.target, scope, q),
                |r| evidence::history(lang, names, &inputs.history, r)
            ),
            Section::Tests => list!(
                rows::tests(&inputs.tests.tests, &inputs.source, scope, q),
                |r| evidence::test(lang, names, r)
            ),
            Section::Mutations => list!(
                rows::mutations(&inputs.mutations.files, &inputs.source, scope, q),
                |r| evidence::mutation(lang, names, r)
            ),
            Section::Duplicates => {
                list!(rows::duplicates(&inputs.duplicates.groups, scope, q), |r| {
                    evidence::duplicate(lang, names, r)
                })
            }
        }
    }

    fn cohesion_records(
        &mut self,
        inputs: &Inputs,
        scope: &Scope,
    ) -> (usize, Vec<Card>, Vec<HeatRow>) {
        let rows = rows::cohesion(&inputs.cohesion.packages, scope, &self.view.query);
        let sel = &mut self.view.cohesion;
        if !rows.iter().any(|p| Some(&p.node) == sel.package.as_ref()) {
            *sel = Selection {
                package: rows.first().map(|p| p.node.clone()),
                ..Selection::default()
            };
        }
        let cards = rows
            .iter()
            .take(self.view.limit)
            .map(|p| cohesion::card(&self.lang, &self.names, p, &self.view.cohesion))
            .collect();
        (rows.len(), cards, Vec::new())
    }

    fn heat_rows(&self, inputs: &Inputs, scope: &Scope) -> Vec<HeatRow> {
        let found = rows::classes(&inputs.code.classes, scope, &self.view.query);
        heatmap::rows(&self.cohorts, found, self.view.filter, self.view.sort)
            .into_iter()
            .map(|row| HeatRow {
                class_id: row.class_id.clone(),
                label: row
                    .class_id
                    .rsplit('.')
                    .next()
                    .unwrap_or(&row.class_id)
                    .to_string(),
                file: row.file.clone(),
                selected: self.view.selected.as_deref() == Some(row.class_id.as_str()),
                cells: (0..METRICS.len()).map(|i| self.heat_cell(row, i)).collect(),
            })
            .collect()
    }

    fn heat_cell(&self, row: &archguard_engine::ClassMetrics, i: usize) -> HeatCell {
        let a = self.cohorts.assess(row, METRICS[i]);
        HeatCell {
            text: number(a.value),
            state: a.state,
            tip: format!(
                "{} · {} {}",
                row.class_id,
                heatmap_text::help(&self.lang, i),
                heatmap_text::interpretation(&self.lang, &self.cohorts, row, i)
            ),
        }
    }

    /// The selected heatmap class as a class card, while it is listed.
    fn detail(&mut self, inputs: &Inputs) -> Option<Card> {
        if self.view.section != Section::Heatmap {
            return None;
        }
        let scope = rows::scope(&inputs.source, &self.view.target);
        let id = self.view.selected.clone()?;
        let listed = self
            .heat_rows(inputs, &scope)
            .iter()
            .any(|r| r.class_id == id);
        let row = inputs
            .code
            .classes
            .iter()
            .find(|r| r.class_id == id)
            .filter(|_| listed);
        if row.is_none() {
            self.view.selected = None;
        }
        row.map(|r| cards::class(&self.lang, &self.names, r))
    }

    fn targets(&self, inputs: &Inputs, page: &mut Page) {
        let lang = &self.lang;
        let query = self.view.target_query.trim().to_lowercase();
        let matching: Vec<&str> = inputs
            .source
            .nodes
            .iter()
            .map(|n| n.name.as_str())
            .filter(|n| query.is_empty() || n.to_lowercase().contains(&query))
            .collect();
        let mut shown: Vec<&str> = matching.iter().copied().take(TARGETS).collect();
        let listed = shown.len();
        let target = self.view.target.as_str();
        let retained =
            !target.is_empty() && self.names.contains(target) && !shown.contains(&target);
        if retained {
            shown.push(target);
        }
        page.targets = std::iter::once(lang.t("Entire source scope").to_string())
            .chain(shown.iter().map(|s| s.to_string()))
            .collect();
        page.target_index = shown.iter().position(|&s| s == target).map_or(0, |i| i + 1);
        page.scope_summary = if target.is_empty() {
            lang.t("All packages").into()
        } else {
            lang.f("Scope: {path}", &[("path", target.into())])
        };
        if matching.len() > TARGETS || !query.is_empty() || retained {
            page.target_note = target_note(lang, listed, matching.len(), retained);
        }
    }
}

fn target_note(lang: &Texts, listed: usize, matching: usize, retained: bool) -> String {
    let mut note = lang.f(
        "{shown} / {total} matching targets.",
        &[
            ("shown", listed.to_string()),
            ("total", matching.to_string()),
        ],
    );
    if matching > TARGETS {
        note.push(' ');
        note.push_str(lang.t("Only the first 2000 are listed. Refine the package or file search to find other targets.",
        ));
    }
    if retained {
        note.push(' ');
        note.push_str(lang.t("The current selection is also retained."));
    }
    note
}

impl QualityState {
    /// `#engineering-note`: shown behind the status line's "?".
    fn notes(&self, inputs: &Inputs) -> Card {
        let (lang, section) = (&self.lang, self.view.section);
        let mut b = super::card::Builder::new(lang, &self.names, "");
        b.push(Kind::Muted, lang.t("Original analysis and imported measurements. Graph simulations do not rewrite these results.",
        ))
        .push(Kind::Text, section.description(lang));
        let (diagnostics, actions) = source_diagnostics(inputs, section);
        if !diagnostics.is_empty() || !actions.is_empty() {
            let lines = diagnostics
                .iter()
                .take(20)
                .map(|d| Line::new(Kind::Muted, d))
                .chain(actions.iter().map(|a| Line::new(Kind::Text, a)))
                .collect();
            b.details(lang.t("Measurement coverage and definitions"), lines);
        }
        let status = source_status(inputs, section);
        if matches!(status, None | Some("not_imported" | "not_recorded"))
            || section == Section::History
        {
            b.push(Kind::Pre, section.command());
        }
        self.section_notes(&mut b, inputs);
        b.done()
    }

    fn section_notes(&self, b: &mut super::card::Builder, inputs: &Inputs) {
        let lang = &self.lang;
        match self.view.section {
            Section::History => history_notes(b, inputs),
            Section::Mutations => {
                let value = percent(inputs.mutations.summary.killed_percent);
                b.push(
                    Kind::Text,
                    lang.f("Detected mutation ratio: {value}", &[("value", value)]),
                );
            }
            Section::Measurements => {
                let value = percent(inputs.measurements.flaky_test_percent);
                b.push(
                    Kind::Text,
                    lang.f("Mixed test groups: {percent}", &[("percent", value)]),
                );
            }
            Section::Duplicates => {
                let d = &inputs.duplicates;
                let version = d
                    .producer_version
                    .clone()
                    .unwrap_or_else(|| lang.t("version unknown").into());
                let errors = lang.f(
                    "{count} reported scan errors",
                    &[("count", d.producer_errors.to_string())],
                );
                b.push(Kind::Muted, format!("CPD · {version} · {errors}"));
            }
            _ => {}
        }
    }
}

fn history_notes(b: &mut super::card::Builder, inputs: &Inputs) {
    let lang = b.lang;
    let h = &inputs.history;
    b.push(Kind::Muted, lang.f("{count} recent snapshots shown; full measurements are stored in .archguard/quality-history.",
        &[("count", h.records.len().to_string())],
    ));
    let mut lines = Vec::new();
    if let Some(l) = &h.latest {
        lines.push(Line::new(
            Kind::Path,
            format!(
                "Source SHA256: {} · Config SHA256: {}",
                l.source_sha256, l.config_sha256
            ),
        ));
        for a in &l.analyzers {
            let version = a
                .version
                .clone()
                .unwrap_or_else(|| lang.t("version unknown").into());
            let project = a.project.clone().unwrap_or_default();
            lines.push(Line::new(
                Kind::Path,
                format!("{} · {} · {version} · {project}", a.language, a.backend),
            ));
        }
    }
    for r in &h.records {
        let commit = r.git_commit.as_deref().unwrap_or("no Git commit");
        lines.push(Line::new(
            Kind::Path,
            format!(
                "{} · {commit} · {} · {}",
                r.tag.as_deref().unwrap_or(""),
                r.path,
                r.source_sha256
            ),
        ));
    }
    b.details(lang.t("Snapshot provenance"), lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality_ui::heatmap::tests::class;
    use crate::quality_ui::inputs::Inputs;

    fn inputs(classes: Vec<archguard_engine::ClassMetrics>) -> Inputs {
        let root = std::path::Path::new("/nonexistent-archguard-root");
        let graph = archguard_engine::core::GraphBuilder::new().build(root.display().to_string());
        let config = archguard_engine::core::Config::default();
        let sources = archguard_engine::core::ReviewSources::default();
        let calls = archguard_engine::analyze_calls(root, &config);
        let mut i = Inputs::compute(root, &graph, &config, &sources, &calls);
        i.code.status = "partial".into();
        i.code.classes = classes;
        i
    }

    fn classes(n: usize) -> Vec<archguard_engine::ClassMetrics> {
        (0..n)
            .map(|i| {
                class(
                    &format!("rust:a/f{i:02}.rs::C"),
                    &format!("a/f{i:02}.rs"),
                    None,
                    None,
                )
            })
            .collect()
    }

    #[test]
    fn heatmap_pages_twenty_and_counts_all() {
        let mut s = QualityState::new(Texts::default(), inputs(classes(25)));
        let page = s.page();
        assert!(page.heatmap);
        assert_eq!(page.status, "partial analysis · 20 / 25 matching records");
        assert_eq!(page.heat.len(), 20);
        assert_eq!(page.heat[0].label, "rs::C");
        assert_eq!(page.heat[0].cells[0].text, "—");
        assert!(page.more);
        s.view.limit += PAGE;
        let page = s.page();
        assert_eq!((page.heat.len(), page.more), (25, false));
        assert_eq!(page.scope_summary, "All packages");
        assert_eq!(page.targets[0], "Entire source scope");
    }

    #[test]
    fn without_classes_the_tab_opens_on_cohesion_and_explains_the_import() {
        let mut s = QualityState::new(Texts::default(), inputs(Vec::new()));
        assert_eq!(s.view.section, Section::Cohesion);
        s.view.section = Section::Tests;
        let page = s.page();
        assert_eq!(page.status, "not imported · 0 / 0 matching records");
        assert!(page.empty.starts_with("No matching measurements."));
        assert!(
            page.notes
                .items
                .iter()
                .any(|i| i.kind == Kind::Pre && i.text.starts_with("archguard test-evidence"))
        );
    }

    #[test]
    fn a_selected_class_shows_its_card_until_it_leaves_the_list() {
        let mut s = QualityState::new(Texts::default(), inputs(classes(3)));
        s.view.selected = Some("rust:a/f01.rs::C".into());
        let page = s.page();
        assert_eq!(
            page.detail.as_ref().map(|c| c.title.as_str()),
            Some("rust:a/f01.rs::C")
        );
        assert!(page.heat[1].selected);
        s.view.query = "f02".into();
        assert!(s.page().detail.is_none());
        assert_eq!(s.view.selected, None);
    }
}
