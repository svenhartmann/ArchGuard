//! Help → Metrics, commands & MCP… (ui/help_dialog.slint): the KPI
//! definitions the tooltips show ([`crate::kpi`], the class metrics of
//! the Quality heatmap) and the MCP entry for the active project and the
//! focused view. The CLI, language and prompt texts live in the dialog.

use std::path::Path;

use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::documents::with_store;
use crate::kpi::{self, Kpi};
use crate::panes_ui;
use crate::tab_support::Texts;
use crate::ui::{AppWindow, HelpBlock, HelpData};

/// The figures of the Graph tab, then those of Matrix and Start.
const GRAPH: [&Kpi; 13] = [
    &kpi::SHOWN_MODULES,
    &kpi::SHOWN_DEPENDENCIES,
    &kpi::SHOWN_CYCLE_GROUPS,
    &kpi::SHOWN_ACD,
    &kpi::SHOWN_CCD,
    &kpi::SHOWN_NCCD,
    &kpi::LARGEST_CYCLE_GROUP,
    &kpi::MODULES_IN_CYCLES,
    &kpi::HIGH_PRIORITY,
    &kpi::MODULE_ACD,
    &kpi::FILE_CYCLE_GROUPS,
    &kpi::DEPENDENCY_EVIDENCE,
    &kpi::AREA_CYCLE_GROUPS,
];

pub fn wire(ui: &AppWindow) {
    let w = ui.as_weak();
    ui.global::<HelpData>().on_show(move || {
        if let Some(ui) = w.upgrade() {
            show(&ui);
        }
    });
}

/// `ARCHGUARD_DESKTOP_HELP=<tab>`: open the dialog on that tab (0 Graph
/// KPIs … 4 MCP) for a snapshot.
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    if let Some(tab) = std::env::var("ARCHGUARD_DESKTOP_HELP")
        .ok()
        .and_then(|t| t.parse().ok())
    {
        show(ui);
        ui.global::<HelpData>().set_tab(tab);
    }
}

fn block(title: &str, text: &str) -> HelpBlock {
    HelpBlock {
        title: title.into(),
        text: text.into(),
        code: Default::default(),
    }
}

fn model(blocks: Vec<HelpBlock>) -> ModelRc<HelpBlock> {
    ModelRc::new(VecModel::from(blocks))
}

fn show(ui: &AppWindow) {
    let texts = Texts::of(ui);
    let graph = GRAPH
        .iter()
        .map(|k| block(texts.t(k.label), texts.t(k.note)));
    let classes = (0..crate::quality_ui::heatmap_text::HELP.len()).map(|i| {
        let text = crate::quality_ui::heatmap_text::help(&texts, i);
        block(text.split(" — ").next().unwrap_or_default(), text)
    });
    let layout = panes_ui::layout();
    let view = layout
        .view(layout.focus)
        .map(|t| t.kind.clone())
        .unwrap_or_default();
    let root = with_store(|s| s.active().map(|d| d.root.clone()));
    let d = ui.global::<HelpData>();
    d.set_graph(model(graph.collect()));
    d.set_classes(model(classes.collect()));
    d.set_mcp_call(mcp_call(&view).into());
    d.set_mcp_server(mcp_server(root.as_deref()).into());
    d.set_open(true);
}

/// The `development_guide` call (`tools/call` params) for the view
/// `kind`; the view only when the MCP tool has routes for it.
fn mcp_call(kind: &str) -> String {
    let routes: serde_json::Value =
        serde_json::from_str(archguard_engine::core::DEVELOPMENT_PATHS).unwrap_or_default();
    let known = routes
        .as_array()
        .is_some_and(|r| r.iter().any(|r| r["view"] == kind));
    let mut arguments = json!({"limit": 5});
    if known {
        arguments["view"] = json!(kind);
    }
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "development_guide", "arguments": arguments}});
    serde_json::to_string_pretty(&call).unwrap_or_default()
}

/// The `mcpServers` entry that serves `root` (a placeholder without one).
fn mcp_server(root: Option<&Path>) -> String {
    let root = root.map_or("/path/to/project".into(), |r| r.display().to_string());
    let entry = json!({"mcpServers": {"archguard": {"command": "archguard", "args": ["mcp", "--root", root]}}});
    serde_json::to_string_pretty(&entry).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn read(rel: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).expect(rel)
    }

    /// `archguard <name>` titles of the dialog's CLI tab.
    fn dialog_commands() -> BTreeSet<String> {
        let slint = read("../archguard-desktop-ui/ui/help_dialog.slint");
        slint
            .match_indices("{ title: \"archguard ")
            .map(|(at, m)| {
                let rest = &slint[at + m.len()..];
                rest[..rest.find('"').unwrap()].to_string()
            })
            .collect()
    }

    /// The variants of the CLI's `Command`, as clap names them.
    fn cli_commands() -> BTreeSet<String> {
        let args = read("../archguard-cli/src/args.rs");
        let body = &args[args.find("pub(crate) enum Command {").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        body.lines()
            .filter_map(|l| l.strip_prefix("    "))
            .filter(|l| l.starts_with(|c: char| c.is_ascii_uppercase()))
            .map(|l| {
                let name: String = l.chars().take_while(|c| c.is_alphanumeric()).collect();
                let mut kebab = String::new();
                for (i, c) in name.chars().enumerate() {
                    if c.is_uppercase() && i > 0 {
                        kebab.push('-');
                    }
                    kebab.push(c.to_ascii_lowercase());
                }
                kebab
            })
            .collect()
    }

    /// The CLI tab lists every command of the current CLI and no other
    /// (the web viewer's list still had `serve`).
    #[test]
    fn the_cli_tab_lists_the_current_commands() {
        let cli = cli_commands();
        assert!(cli.contains("suggest") && cli.contains("diagram") && cli.contains("report"));
        assert_eq!(dialog_commands(), cli);
    }

    /// Every MCP prompt names a tool the server has.
    #[test]
    fn the_mcp_tab_names_existing_tools() {
        let slint = read("../archguard-desktop-ui/ui/help_dialog.slint");
        // The tools are spread over the server's modules.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../archguard-mcp/src");
        let server: String = std::fs::read_dir(dir)
            .expect("mcp sources")
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .collect();
        let mcp = &slint[slint.find("property <[HelpBlock]> mcp:").unwrap()..];
        let tools: Vec<&str> = mcp
            .match_indices("{ title: \"")
            .map(|(at, m)| {
                let rest = &mcp[at + m.len()..];
                &rest[..rest.find('"').unwrap()]
            })
            .filter(|t| t.chars().all(|c| c.is_ascii_lowercase() || c == '_') && !t.is_empty())
            .collect();
        assert!(tools.len() > 40, "{tools:?}");
        for t in tools {
            assert!(server.contains(&format!("async fn {t}(")), "{t}");
        }
    }

    /// The MCP entry asks `development_guide` for the shown view when the
    /// tool knows it, and serves the project's root.
    #[test]
    fn the_mcp_entry_names_the_view_and_the_project() {
        let call: serde_json::Value = serde_json::from_str(&mcp_call("dsm")).unwrap();
        assert_eq!(call["params"]["name"], "development_guide");
        assert_eq!(call["params"]["arguments"]["view"], "dsm");
        let start: serde_json::Value = serde_json::from_str(&mcp_call("start")).unwrap();
        assert!(start["params"]["arguments"].get("view").is_none());
        let server: serde_json::Value =
            serde_json::from_str(&mcp_server(Some(Path::new("/p/shop")))).unwrap();
        assert_eq!(
            server["mcpServers"]["archguard"]["args"],
            json!(["mcp", "--root", "/p/shop"])
        );
    }

    /// The help texts are the tooltips' own, translated.
    #[test]
    fn graph_definitions_are_the_tooltips() {
        let de = Texts::german();
        for k in GRAPH {
            assert_ne!(de.t(k.note), k.note, "{}", k.id);
        }
    }
}
