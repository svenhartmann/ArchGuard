//! A [`Layout`] in a project's table of `desktop.toml`:
//!
//! ```toml
//! [projects."/work/shop"]
//! panes = [["graph", "dsm"], ["architecture"]]  # view tabs per pane
//! pane_active = [1, 0]                          # selected tab per pane
//! pane_focus = 1
//! pane_ratio = 0.5                              # left pane's share
//! ```
//!
//! Reading repairs what does not fit ([`Layout::repaired`]); a table
//! without `panes` has no layout.

use toml_edit::{Array, Item, TableLike, value};

use crate::panes::{Layout, Pane};

/// The layout stored in a project table, if any.
pub fn read(t: &dyn TableLike) -> Option<Layout> {
    let panes = t.get("panes")?.as_array()?;
    let active = |i: usize| {
        t.get("pane_active")
            .and_then(Item::as_array)
            .and_then(|a| a.get(i)?.as_integer())
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
    };
    let panes = panes
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let tabs = p.as_array()?.iter().filter_map(|v| v.as_str());
            Some(Pane {
                tabs: tabs.map(str::to_string).collect(),
                active: active(i),
            })
        })
        .collect();
    let focus = t
        .get("pane_focus")
        .and_then(Item::as_integer)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    let ratio = t.get("pane_ratio").and_then(Item::as_float).unwrap_or(0.5) as f32;
    Some(
        Layout {
            panes,
            focus,
            ratio,
        }
        .repaired(),
    )
}

/// The values to store for `layout`, as `(key, item)`.
pub fn items(layout: &Layout) -> [(&'static str, Item); 4] {
    let panes: Array = layout
        .panes
        .iter()
        .map(|p| p.tabs.iter().map(String::as_str).collect::<Array>())
        .collect();
    let active: Array = layout.panes.iter().map(|p| p.active as i64).collect();
    // Two decimals: the divider moves in whole pixels anyway.
    let ratio = (f64::from(layout.ratio) * 100.0).round() / 100.0;
    [
        ("panes", value(panes)),
        ("pane_active", value(active)),
        ("pane_focus", value(layout.focus as i64)),
        ("pane_ratio", value(ratio)),
    ]
}
