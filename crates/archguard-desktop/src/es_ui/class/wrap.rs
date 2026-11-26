//! Cluster wrap into sub-columns and text elision, ticket 17. Imports nothing of the layout, so
//! `layout` can use it without a cycle.

use super::boxes::BoxContent;

/// Wanted width : height of a scene (a canvas viewport is wider than tall).
const TARGET_ASPECT: f32 = 1.6;
const NAME_W: f32 = 7.5;
const CAPTION_W: f32 = 6.0;
const MONO_W: f32 = 7.2;
/// Identifiers the canvas translates; they are not text to cut.
const HINTS: [&str; 3] = ["not_read", "mapping_not_read", "not_declared"];

/// One column as the wrap sees it: stacked height, box count and the width of one sub-column
/// including its gap.
pub struct Lane {
    pub height: f32,
    pub boxes: usize,
    pub pitch: f32,
}

/// Sub-columns per column: one more for the tallest column while the scene is narrower than the
/// target shape and that column has a box to give.
pub fn counts(lanes: &[Lane]) -> Vec<usize> {
    let mut k = vec![1; lanes.len()];
    loop {
        let tall = |i: usize| lanes[i].height / k[i] as f32;
        let all = 0..lanes.len();
        let width: f32 = (all.clone())
            .filter(|&i| lanes[i].boxes > 0)
            .map(|i| lanes[i].pitch * k[i] as f32)
            .sum();
        let height = all.clone().map(tall).fold(0.0, f32::max);
        let split =
            (all.filter(|&i| lanes[i].boxes > k[i])).max_by(|&a, &b| tall(a).total_cmp(&tall(b)));
        match split {
            Some(i) if height > 0.0 && width / height < TARGET_ASPECT => k[i] += 1,
            _ => return k,
        }
    }
}

/// The sub-column (0..k) of each box of a column in stacking order: contiguous chunks of about
/// equal height.
pub fn assign(heights: &[f32], k: usize) -> Vec<usize> {
    let per = heights.iter().sum::<f32>() / k as f32;
    let mut done = 0.0;
    heights
        .iter()
        .map(|h| {
            let sub = ((done / per.max(1.0)) as usize).min(k - 1);
            done += h;
            sub
        })
        .collect()
}

/// `text` cut to what fits `width` px at `char_w` px per character, ending in "…" when cut
/// (shortcut: a fixed glyph width per text role, no font measuring in Rust; the caption sizes
/// differ per OS (`Theme.font-*`), so upgrade when Slint offers a text-measuring API in Rust or
/// when a cut shows too early or too late on one platform).
pub fn fit_text(text: &str, width: f32, char_w: f32) -> String {
    let max = (width / char_w).floor().max(1.0) as usize;
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max - 1).collect();
    format!("{head}…")
}

/// Cuts every text of a box of width `w` to its row: header lines and lines without a type or
/// note (those keep Slint's elision of the note).
pub fn fit_box(c: &mut BoxContent, w: f32) {
    let inner = w - 12.0;
    // The name may carry the warning mark in front.
    c.name = fit_text(&c.name, inner - 2.0 * NAME_W, NAME_W);
    c.stereotype = fit_text(&c.stereotype, inner, CAPTION_W);
    c.root = fit_text(&c.root, inner, CAPTION_W);
    for l in c.compartments.iter_mut().flat_map(|k| &mut k.lines) {
        let plain = l.kind_text.is_empty() && l.note.is_empty() && l.summary.is_none();
        if plain && !HINTS.contains(&l.text.as_str()) {
            l.text = fit_text(&l.text, w - 16.0 - 12.0 * l.depth as f32, MONO_W);
        }
    }
}
