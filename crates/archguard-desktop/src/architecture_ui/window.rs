//! The tab state as one `ArchitectureModel` value, built without the
//! window so that a push can keep the models the view shows
//! ([`super::sync`]).

use archguard_engine::core::Arc42Config;
use archguard_engine::report::Arc42Document;
use slint::SharedString;

use super::state::{ArchitectureState, current, derived, labels_in};
use super::visual::Kind;
use super::{catalogue, figure, view};
use crate::kpi;
use crate::tab_support::Texts;
use crate::ui::{ArcFigure, ArchitectureModel};

/// The window model of the state.
pub(super) fn model(s: &ArchitectureState, texts: &Texts) -> ArchitectureModel {
    let _span = archguard_engine::self_trace::tracing::info_span!("model").entered();
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
            m.chapter_note = notes(texts, m.chapter);
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
    // The diagram view shows no chapter; its unsaved changes keep the save bar.
    m.editing &= !s.catalogue;
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
    // A chapter without a generated block never gets a diagram on its own:
    // it can take one over as text.
    let text = s.editor.chapter(doc, s.chapter);
    let has_block = text.contains(archguard_engine::report::arc42_file::MARKER);
    // Diagrams in the chapter's text draw there; the rule the save uses.
    let in_text = |d: &archguard_engine::core::Diagram| {
        archguard_engine::report::arc42_file::in_text(&text, d)
    };
    let open = view.as_ref().map(|(_, id)| id.as_str());
    // The switch: expected in its chapter by the rule save and `--check`
    // use, or already in its text.
    let switch = |d: &archguard_engine::core::Diagram| {
        let own;
        let chapter = if d.chapter == s.chapter {
            &text
        } else {
            own = s.editor.chapter(doc, d.chapter);
            &own
        };
        archguard_engine::report::arc42_file::expected(doc, d, chapter)
            || (d.shown && archguard_engine::report::arc42_file::in_text(chapter, d))
    };
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
        m.map_shown = entry.map_or(s.shown.iter().any(|x| x == id), switch);
        let map_in_text = entry.is_some_and(in_text);
        m.map_insertable = !has_block && entry.is_some() && !map_in_text;
        m.missing = i32::from(!map_in_text);
    }
    let figures = doc
        .diagrams
        .iter()
        .filter(|d| {
            let drawn = Some(d.id.as_str()) == map || Some(d.id.as_str()) == open;
            s.catalogue || (d.chapter == s.chapter && !drawn && !in_text(d))
        })
        .map(|d| {
            let chapter = if s.catalogue {
                format!("{} {}", texts.t("Chapter"), d.chapter)
            } else {
                String::new()
            };
            let insertable = !s.catalogue && !has_block;
            ArcFigure {
                insertable,
                shown: switch(d),
                ..figure::figure(d, s.format, chapter, texts)
            }
        });
    let figures = figures.collect::<Vec<_>>();
    if !s.catalogue {
        m.missing += figures.len() as i32;
    }
    m.figures = slint::ModelRc::new(slint::VecModel::from(figures));
}

/// What the desktop says around the engine's arc42 text: in chapter 10,
/// the level its figures are counted on.
fn notes(texts: &Texts, chapter: i32) -> SharedString {
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
    level.into()
}

#[cfg(test)]
mod tests {
    use slint::{Model, ModelRc};

    use super::super::state::fixture_state;
    use super::super::sync::keep;
    use super::*;
    use crate::ui::ArcBlock;

    /// Chapter 10 names the level of its ACD.
    #[test]
    fn arc42_chrome_names_the_level() {
        let en = Texts::default();
        let level = notes(&en, 10);
        assert!(
            level.starts_with("ACD (module level) and the cycle figures"),
            "{level}"
        );
        assert!(level.contains("ACD (shown level) is in the Graph tab"));
        assert_eq!(notes(&en, 5), "");
        let de = Texts::german();
        let level = notes(&de, 10);
        assert!(level.starts_with("ACD (Modulebene)"), "{level}");
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
        let mut s = fixture_state(None);
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

    /// Chapter 5 of a project's own arc42 text `chapter5`, open in the tab.
    fn own_text(dir: &std::path::Path, chapter5: &str) -> ArchitectureModel {
        let (a, _) = crate::documents::with_store(|s| s.open(dir));
        std::fs::write(
            dir.join("arc42.md"),
            format!("## 5. Bausteine\n\n{chapter5}"),
        )
        .unwrap();
        let mut s = fixture_state(Some(dir));
        s.editor = super::super::edit::Editor::new(a, dir).with_format(s.format);
        s.chapter = 5;
        built(&s)
    }

    /// The Diagrams tab counts the chapter's generated diagrams that are
    /// not in its text: the package map too while its block is missing.
    #[test]
    fn the_diagrams_tab_counts_what_the_text_lacks() {
        let s = fixture_state(None);
        let doc = s.doc.as_ref().expect("document");
        let five: Vec<_> = doc.diagrams.iter().filter(|d| d.chapter == 5).collect();
        let map = five
            .iter()
            .find(|d| d.id == "building-blocks")
            .expect("the package map");
        let others = five.len() - 1;
        let dir = tempfile::tempdir().expect("temp dir");
        let own = own_text(dir.path(), "Eigener Text.\n");
        assert_eq!(own.map_id, "building-blocks");
        assert!(own.map_insertable);
        assert_eq!(own.figures.row_count(), others);
        assert_eq!(own.missing, others as i32 + 1, "the map is not in the text");
        let section = archguard_engine::report::diagram_section(map, s.format);
        let block = archguard_engine::report::arc42_file::diagram_block(&map.id, &section);
        let with_map = own_text(dir.path(), &format!("Eigener Text.\n\n{block}\n"));
        assert!(!with_map.map_insertable);
        assert_eq!(with_map.missing, others as i32, "the map is in the text");
        // The generated chapter has every shown diagram in its text.
        let generated = built(&s);
        let hidden = five.iter().filter(|d| !d.shown).count();
        assert_eq!(generated.missing, hidden as i32);
    }

    /// The diagram view keeps its figures and their drawings across pushes.
    #[test]
    fn figures_keep_their_drawings() {
        let mut s = fixture_state(None);
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
