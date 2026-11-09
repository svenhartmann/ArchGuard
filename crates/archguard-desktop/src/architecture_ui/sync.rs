//! The Architecture window model across pushes: a new
//! [`ArchitectureModel`] keeps the list models the view already shows,
//! with only the changed rows set (`model_sync`), so selecting a card or
//! ticking a diagram no longer rebuilds every block, card and line.

use slint::Model;

use crate::model_sync::{sync_rows, sync_rows_with};
use crate::ui::{ArcBlock, ArcFigure, ArcMap, ArchitectureModel};

/// `new` takes over the list models of `old`, synced to its contents.
pub fn keep(old: &ArchitectureModel, new: &mut ArchitectureModel) {
    new.chapters = sync_rows(&old.chapters, new.chapters.iter());
    new.blocks = sync_rows_with(&old.blocks, new.blocks.iter(), adopt_block);
    new.about = sync_rows_with(&old.about, new.about.iter(), adopt_block);
    new.figures = sync_rows_with(&old.figures, new.figures.iter(), adopt_figure);
    keep_map(&old.map, &mut new.map);
}

/// The editor's preview, synced on every keystroke.
pub fn keep_blocks(
    old: &slint::ModelRc<ArcBlock>,
    rows: Vec<ArcBlock>,
) -> slint::ModelRc<ArcBlock> {
    sync_rows_with(old, rows, adopt_block)
}

fn keep_map(old: &ArcMap, new: &mut ArcMap) {
    new.levels = sync_rows(&old.levels, new.levels.iter());
    new.crumbs = sync_rows(&old.crumbs, new.crumbs.iter());
    new.scopes = sync_rows(&old.scopes, new.scopes.iter());
    new.cards = sync_rows(&old.cards, new.cards.iter());
    new.lines = sync_rows(&old.lines, new.lines.iter());
    new.relations = sync_rows(&old.relations, new.relations.iter());
}

fn adopt_block(old: &ArcBlock, new: &mut ArcBlock) {
    new.cells = sync_rows(&old.cells, new.cells.iter());
    new.heads = sync_rows(&old.heads, new.heads.iter());
    new.align = sync_rows(&old.align, new.align.iter());
    new.widths = sync_rows(&old.widths, new.widths.iter());
}

fn adopt_figure(old: &ArcFigure, new: &mut ArcFigure) {
    new.cards = sync_rows(&old.cards, new.cards.iter());
    new.lines = sync_rows(&old.lines, new.lines.iter());
    new.frames = sync_rows(&old.frames, new.frames.iter());
}
