//! A [`Layout`] in a project's table of `desktop.toml`:
//!
//! ```toml
//! [projects."/work/shop"]
//! # view tabs per pane: a view id, or a view instance with its node
//! panes = [["graph", { view = "graph", node = "crates/core", display = 1 }],
//!          [{ view = "file", node = "crates/core/src/lib.rs" }, "dsm"],
//!          # the Start tab's World view (`start_ui::memo`)
//!          [{ view = "start", world = { mode = "architecture", lens = "1", area = "crates/core" } }]]
//! pane_active = [1, 0]                          # selected tab per pane
//! pane_focus = 1
//! pane_ratio = 0.5                              # left pane's share
//! pane_follow = false                           # right pane follows the left
//! ```
//!
//! Reading repairs what does not fit (`panes::repaired`); a
//! table without `panes` has no layout. Nodes are checked when the
//! analysis is there (`instances::sync`).

use toml_edit::{Array, InlineTable, Item, TableLike, Value, value};

use crate::panes::{self, Layout, Pane, Tab};

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
            Some(Pane {
                tabs: p.as_array()?.iter().filter_map(read_tab).collect(),
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
    let follow = t
        .get("pane_follow")
        .and_then(Item::as_bool)
        .unwrap_or(false);
    Some(panes::repaired(Layout {
        panes,
        focus,
        ratio,
        next: 1,
        follow_left: follow,
    }))
}

/// `"graph"` or `{ view = "graph", node = "…", display = 1 }`.
fn read_tab(v: &Value) -> Option<Tab> {
    let tab = |kind: &str| Tab {
        kind: kind.to_string(),
        id: 0,
        node: None,
        display: 0,
        world: Vec::new(),
    };
    if let Some(kind) = v.as_str() {
        return Some(tab(kind));
    }
    let t = v.as_inline_table()?;
    let mut out = tab(t.get("view")?.as_str()?);
    out.node = t.get("node").and_then(Value::as_str).map(str::to_string);
    out.display = t
        .get("display")
        .and_then(Value::as_integer)
        .and_then(|d| i32::try_from(d).ok())
        .unwrap_or(0);
    if let Some(w) = t.get("world").and_then(Value::as_inline_table) {
        out.world = w
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string())))
            .collect();
    }
    Some(out)
}

/// A tab as [`read_tab`] reads it: a plain view id when it has nothing of
/// its own.
fn tab_value(tab: &Tab) -> Value {
    if tab.node.is_none() && tab.display == 0 && tab.world.is_empty() {
        return tab.kind.as_str().into();
    }
    let mut t = InlineTable::new();
    t.insert("view", tab.kind.as_str().into());
    if let Some(node) = &tab.node {
        t.insert("node", node.as_str().into());
    }
    if tab.display != 0 {
        t.insert("display", i64::from(tab.display).into());
    }
    if !tab.world.is_empty() {
        let world: InlineTable = tab
            .world
            .iter()
            .map(|(k, v)| (k.as_str(), Value::from(v.as_str())))
            .collect();
        t.insert("world", Value::InlineTable(world));
    }
    Value::InlineTable(t)
}

/// The values to store for `layout`, as `(key, item)`.
pub fn items(layout: &Layout) -> [(&'static str, Item); 5] {
    let panes: Array = layout
        .panes
        .iter()
        .map(|p| p.tabs.iter().map(tab_value).collect::<Array>())
        .collect();
    let active: Array = layout.panes.iter().map(|p| p.active as i64).collect();
    // Two decimals: the divider moves in whole pixels anyway.
    let ratio = (f64::from(layout.ratio) * 100.0).round() / 100.0;
    [
        ("panes", value(panes)),
        ("pane_active", value(active)),
        ("pane_focus", value(layout.focus as i64)),
        ("pane_ratio", value(ratio)),
        ("pane_follow", value(layout.follow_left)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Start tab's kept World view is written as its `world` table and
    /// read back; a tab without one stays a plain view id.
    #[test]
    fn the_world_view_round_trips_with_the_layout() {
        let mut l = Layout::single("start");
        l.open_tab("graph", None);
        let (p, t) = l.find("start").expect("start tab");
        l.panes[p].tabs[t].world = vec![
            ("mode".into(), "architecture".into()),
            ("area".into(), "crates/core".into()),
        ];
        let mut table = toml_edit::Table::new();
        for (k, item) in items(&l) {
            table.insert(k, item);
        }
        let text = table.to_string();
        let start = r#"{ view = "start", world = { mode = "architecture", area = "crates/core" } }, "graph""#;
        assert!(text.contains(start), "{text}");
        assert_eq!(read(&table), Some(l));
    }
}
