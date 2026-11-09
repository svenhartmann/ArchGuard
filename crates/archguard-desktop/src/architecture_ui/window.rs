//! The tab state as one `ArchitectureModel` value, built without the
//! window so that a push can keep the models the view shows
//! ([`super::sync`]).

use archguard_engine::core::Arc42Config;
use archguard_engine::report::Arc42Document;
use slint::SharedString;

use super::state::{ArchitectureState, current, derived, labels_in};
use super::visual::Kind;
use super::{catalogue, figure, view};
use crate::format::Locale;
use crate::kpi;
use crate::tab_support::Texts;
use crate::ui::ArchitectureModel;

/// The window model of the state.
pub(super) fn model(s: &ArchitectureState, texts: &Texts) -> ArchitectureModel {
    let mut m = ArchitectureModel {
        has_model: s.has_model,
        saved: s.saved.as_str().into(),
        ..Default::default()
    };
    m.map.kind = -1;
    match &s.doc {
        Err(e) => {
            m.state = 2;
            m.error = e.as_str().into();
        }
        Ok(doc) if doc.chapters.is_empty() => m.state = 2,
        Ok(doc) => {
            m.state = 1;
            let drawn: Vec<_> = doc
                .diagrams
                .iter()
                .filter(|d| d.chapter == s.chapter)
                .cloned()
                .collect();
            let text = catalogue::strip(&s.editor.chapter(doc, s.chapter), &drawn, s.format);
            view::fill_document(
                &mut m,
                doc,
                view::Sources {
                    chapter: &text,
                    whole: &s.editor.preamble(doc),
                },
                s.chapter,
                texts.t("Diagram source · this diagram syntax is not supported in the viewer."),
                derived(s, s.chapter).is_some(),
            );
            (m.language_note, m.chapter_note) = notes(texts, m.chapter);
            let mut view = None;
            if let Some((kind, map)) = current(s, &labels_in(texts)) {
                view = Some((kind, catalogue::view_id(kind, map.scope, &s.graph)));
                m.map = view::map_model(
                    texts,
                    &s.graph,
                    &s.calls_status,
                    s.maps[kind as usize].relations,
                    kind,
                    &map,
                );
            }
            fill_figures(&mut m, s, doc, texts, view);
        }
    }
    m.figure_error = s.figure_error.as_str().into();
    s.editor.fill(&mut m);
    m
}

/// The catalogue diagrams: the open chapter's (its package map gets only
/// a tick), or all of them in the diagram view.
fn fill_figures(
    m: &mut ArchitectureModel,
    s: &ArchitectureState,
    doc: &Arc42Document,
    texts: &Texts,
    view: Option<(Kind, String)>,
) {
    // The map draws its overview and the open level itself.
    let map = view.as_ref().map(|(kind, _)| catalogue::map_id(*kind));
    let open = view.as_ref().map(|(_, id)| id.as_str());
    if s.catalogue {
        m.catalogue = true;
        m.chapter_title = texts.t("Diagrams").into();
        m.map.kind = -1;
        m.blocks = Default::default();
        m.source = Default::default();
    } else if let Some(id) = open {
        let entry = doc.diagrams.iter().find(|d| d.id == id);
        m.map_id = id.into();
        m.map_optional = Arc42Config::level_of(id).is_some();
        m.map_title = entry
            .map_or_else(|| id.replace('@', " · "), |d| d.title.clone())
            .into();
        m.map_shown = entry.map_or(s.shown.iter().any(|x| x == id), |d| d.shown);
    }
    let figures = doc
        .diagrams
        .iter()
        .filter(|d| {
            let drawn = Some(d.id.as_str()) == map || Some(d.id.as_str()) == open;
            s.catalogue || (d.chapter == s.chapter && !drawn)
        })
        .map(|d| {
            let chapter = if s.catalogue {
                format!("{} {}", texts.t("Chapter"), d.chapter)
            } else {
                String::new()
            };
            figure::figure(d, s.format, chapter, texts)
        });
    m.figures = slint::ModelRc::new(slint::VecModel::from(figures.collect::<Vec<_>>()));
}

/// What the desktop says around the engine's German arc42 text: its
/// language (in an English window) and, in chapter 10, the level its
/// figures are counted on.
fn notes(texts: &Texts, chapter: i32) -> (SharedString, SharedString) {
    let language = if texts.locale() == Locale::En {
        texts.t("Document language: German. The analysis engine writes the arc42 text in German.")
    } else {
        ""
    };
    let level = if chapter == 10 {
        texts.f(
            "{acd} and the cycle figures of this chapter are counted on modules (depth 2). {shown} is in the Graph tab; package cycles are in the chapter 5 diagram.",
            &[
                ("acd", texts.t(kpi::MODULE_ACD.label).to_string()),
                ("shown", texts.t(kpi::SHOWN_ACD.label).to_string()),
            ],
        )
    } else {
        String::new()
    };
    (language.into(), level.into())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use slint::{Model, ModelRc};

    use super::super::state::build_from;
    use super::super::sync::keep;
    use super::*;
    use crate::ui::ArcBlock;

    /// The engine's arc42 is German: an English window says so, and
    /// chapter 10 names the level of its ACD.
    #[test]
    fn arc42_chrome_names_language_and_level() {
        let en = Texts::default();
        let (language, level) = notes(&en, 10);
        assert!(language.starts_with("Document language: German"));
        assert!(
            level.starts_with("ACD (module level) and the cycle figures"),
            "{level}"
        );
        assert!(level.contains("ACD (shown level) is in the Graph tab"));
        assert_eq!(notes(&en, 5).1, "");
        let de = Texts::german();
        let (language, level) = notes(&de, 10);
        assert_eq!(language, "", "a German window needs no note");
        assert!(level.starts_with("ACD (Modulebene)"), "{level}");
    }

    /// The tab state of a small layered fixture of this repository.
    fn state() -> ArchitectureState {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/py-layers")
            .canonicalize()
            .expect("fixture");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let calls = archguard_engine::analyze_calls(&root, &run.config);
        build_from(&root, &run.config, run.graph.clone(), &run.report, &calls)
    }

    fn built(s: &ArchitectureState) -> ArchitectureModel {
        model(s, &Texts::default())
    }

    fn rows<T: Clone + 'static>(m: &ModelRc<T>) -> Vec<T> {
        m.iter().collect()
    }

    /// Blocks by value: their table cells as rows, not as model instances.
    fn blocks(m: &ModelRc<ArcBlock>) -> Vec<String> {
        m.iter()
            .map(|b| {
                let cells = (
                    rows(&b.cells),
                    rows(&b.heads),
                    rows(&b.align),
                    rows(&b.widths),
                );
                format!(
                    "{:?} {:?} {} {} {:?} {cells:?}",
                    b.kind, b.level, b.marker, b.plain, b.text
                )
            })
            .collect()
    }

    /// Opening another chapter keeps the shown models (blocks, chapters,
    /// map lists) and leaves them equal to a fresh build of the new state.
    #[test]
    fn a_push_keeps_the_shown_models() {
        let mut s = state();
        let old = built(&s);
        assert!(old.map.cards.row_count() > 0, "chapter 5 draws its map");
        let chapter5 = blocks(&old.blocks);
        s.chapter = 11;
        let mut new = built(&s);
        let fresh = built(&s);
        keep(&old, &mut new);
        assert_eq!(new.blocks, old.blocks, "the same blocks model");
        assert_eq!(new.chapters, old.chapters);
        assert_eq!(new.about, old.about);
        assert_eq!(new.map.cards, old.map.cards);
        assert_eq!(new.map.lines, old.map.lines);
        assert_eq!(new.map.scopes, old.map.scopes);
        assert_eq!(blocks(&new.blocks), blocks(&fresh.blocks));
        assert_eq!(rows(&new.chapters), rows(&fresh.chapters));
        assert_eq!(rows(&new.map.cards), rows(&fresh.map.cards));
        assert_eq!(rows(&new.map.lines), rows(&fresh.map.lines));
        assert_ne!(blocks(&new.blocks), chapter5, "chapter 11 shown");
    }

    /// The diagram view keeps its figures and their drawings across pushes.
    #[test]
    fn figures_keep_their_drawings() {
        let mut s = state();
        s.catalogue = true;
        let old = built(&s);
        assert!(old.figures.row_count() > 1, "the catalogue lists diagrams");
        let mut new = built(&s);
        keep(&old, &mut new);
        assert_eq!(new.figures, old.figures, "the same figures model");
        let (a, b) = (old.figures.row_data(0), new.figures.row_data(0));
        let (a, b) = (a.expect("figure"), b.expect("figure"));
        assert_eq!(b.cards, a.cards, "the same cards model");
        assert_eq!(
            rows(&b.cards),
            rows(&built(&s).figures.row_data(0).expect("figure").cards)
        );
    }
}
