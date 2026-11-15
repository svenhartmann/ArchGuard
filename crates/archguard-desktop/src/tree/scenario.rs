//! Refactoring scenarios (ADR 0011): named sets of simulated changes in
//! the web viewer's format (`viewer_scenarios.js`,
//! `archguard-refactoring-scenario/v1`), kept as one JSON file each in
//! `.archguard/scenarios/` of the project, so both apps and git can read
//! them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use archguard_engine::analysis::{SimOp, apply_ops, cycle_hash};
use archguard_engine::core::{Graph, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SCHEMA: &str = "archguard-refactoring-scenario/v1";
/// The web viewer's limits.
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_OPS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema: String,
    pub name: String,
    pub description: String,
    pub project: String,
    pub source: Source,
    pub scope: Scope,
    pub ops: Vec<SimOp>,
}

/// The analysed graph a scenario was made on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub fingerprint: String,
    pub nodes: u64,
    pub edges: u64,
}

/// The nodes visible when it was saved (the web viewer restores them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub nodes: Vec<String>,
}

/// A saved scenario as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub path: PathBuf,
    pub name: String,
    pub ops: usize,
    /// Made on another analysis of the graph (the web viewer's warning).
    pub stale: bool,
}

/// Where the scenarios of the project at `root` live.
pub fn dir(root: &Path) -> PathBuf {
    root.join(".archguard").join("scenarios")
}

/// The project as the web viewer names it: the graph's root, `/`-separated,
/// without a trailing slash.
pub fn project(g: &Graph) -> String {
    let root = g.root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    if root.is_empty() { "/" } else { root }.to_string()
}

/// The web viewer's `scenarioSource`: SHA-256 over the import structure,
/// counts and evidence by name, so dense ids do not matter.
pub fn fingerprint(g: &Graph) -> Source {
    let names: HashMap<NodeId, &str> = g.nodes.iter().map(|n| (n.id, n.name.as_str())).collect();
    let name = |id: NodeId| names.get(&id).copied();
    let mut nodes: Vec<Value> = g
        .nodes
        .iter()
        .map(|n| json!([n.name, n.kind, n.lang, n.parent.and_then(name)]))
        .collect();
    nodes.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    let mut edges: Vec<(String, Value)> = g
        .edges
        .iter()
        .map(|e| {
            let mut refs: Vec<(String, Value)> = e
                .refs
                .iter()
                .map(|r| keyed(json!([r.file, r.line, r.raw])))
                .collect();
            refs.sort_by(|a, b| a.0.cmp(&b.0));
            let refs: Vec<Value> = refs.into_iter().map(|r| r.1).collect();
            keyed(json!([name(e.from), name(e.to), e.count, refs]))
        })
        .collect();
    edges.sort_by(|a, b| a.0.cmp(&b.0));
    let edges: Vec<Value> = edges.into_iter().map(|e| e.1).collect();
    // By hand: the web viewer's object has `nodes` first; `json!` sorts keys.
    let text = format!(
        "{{\"nodes\":{},\"edges\":{}}}",
        Value::from(nodes),
        Value::from(edges)
    );
    Source {
        // shortcut: `cycle_hash` of one string is its plain SHA-256; use
        // sha2 directly once the desktop Cargo.toml may take a dependency.
        fingerprint: format!("sha256:{}", cycle_hash(&[text])),
        nodes: g.nodes.len() as u64,
        edges: g.edges.len() as u64,
    }
}

/// `v` with its JSON text, the web viewer's sort key.
fn keyed(v: Value) -> (String, Value) {
    (v.to_string(), v)
}

/// The scenario `name` of `ops` on `full`. Its scope is the top-level
/// nodes after the ops: the desktop keeps no zoom level of its own.
pub fn snapshot(
    name: &str,
    description: &str,
    full: &Graph,
    ops: &[SimOp],
) -> Result<Scenario, String> {
    let after = apply_ops(full, ops).map_err(|e| e.to_string())?;
    let mut scope: Vec<String> = after
        .nodes
        .iter()
        .filter(|n| n.parent.is_none())
        .map(|n| n.name.clone())
        .collect();
    scope.sort();
    let s = Scenario {
        schema: SCHEMA.into(),
        name: name.trim().into(),
        description: description.into(),
        project: project(full),
        source: fingerprint(full),
        scope: Scope { nodes: scope },
        ops: ops.to_vec(),
    };
    validate(&s)?;
    Ok(s)
}

/// The web viewer's checks (`scenarioValidate`) that a file can fail.
pub fn validate(s: &Scenario) -> Result<(), String> {
    if s.schema != SCHEMA {
        return Err(format!("unsupported scenario version {}", s.schema));
    }
    let name = s.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err("a scenario needs a name of 1 to 120 characters".into());
    }
    if s.description.chars().count() > 4000 {
        return Err("the description has more than 4000 characters".into());
    }
    if s.ops.len() > MAX_OPS {
        return Err(format!("a scenario holds at most {MAX_OPS} changes"));
    }
    if s.ops.iter().any(|op| matches!(op, SimOp::Merge { .. })) {
        return Err("scenarios hold only cut_edge and move_node changes".into());
    }
    let fp = s.source.fingerprint.strip_prefix("sha256:").unwrap_or("");
    if fp.len() != 64
        || !fp
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid dependency-graph fingerprint".into());
    }
    Ok(())
}

/// Read and check one scenario file.
pub fn read(path: &Path) -> Result<Scenario, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err("scenario JSON is larger than 1 MiB".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let s: Scenario = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    validate(&s)?;
    Ok(s)
}

/// The saved scenarios of `root`, by name; unreadable files are skipped.
/// `current` is the fingerprint of the graph shown now.
pub fn list(root: &Path, current: &Source) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(dir(root)) else {
        return Vec::new();
    };
    let mut out: Vec<Saved> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|path| {
            let s = read(&path).ok()?;
            Some(Saved {
                stale: s.source.fingerprint != current.fingerprint,
                ops: s.ops.len(),
                name: s.name,
                path,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The file for scenario `name`: its name in lower case, other characters
/// than letters and digits as `-`.
pub fn file_for(root: &Path, name: &str) -> PathBuf {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    dir(root).join(format!(
        "{}.json",
        if slug.is_empty() { "scenario" } else { slug }
    ))
}

/// Write `s` to its file under `root`, atomically. Refused when
/// `.archguard` or its `scenarios` is a symlink: a repository must not
/// redirect the write outside it (as for a linked `archguard.toml`).
pub fn save(root: &Path, s: &Scenario) -> Result<PathBuf, String> {
    for d in [root.join(".archguard"), dir(root)] {
        if std::fs::symlink_metadata(&d).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "{}: the scenarios folder must not be a symlink",
                d.display()
            ));
        }
    }
    let path = file_for(root, &s.name);
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())? + "\n";
    crate::config_editor::write_atomic(&path, text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The Scenarios panel of a document: what is saved, the action waiting
/// for confirmation and the last outcome.
#[derive(Default)]
pub struct Scenarios {
    pub open: bool,
    pub saved: Vec<Saved>,
    pub ask: Option<Ask>,
    pub status: Option<Status>,
}

/// An action that waits for confirmation, as in the web viewer.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Replace the `current` simulated changes by the scenario's.
    Load {
        scenario: Scenario,
        current: usize,
        stale: bool,
    },
    Delete(Saved),
    /// Overwrite the saved scenario of the same name.
    Replace(Scenario),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Saved(String),
    Loaded(String),
    Deleted(String),
    Failed(String),
}

impl Scenarios {
    /// Read the saved scenarios of `root` again.
    pub fn refresh(&mut self, root: &Path, full: &Graph) {
        self.saved = list(root, &fingerprint(full));
    }

    /// Save `ops` on `full` as `name`; asks first when that name is saved,
    /// refuses when its file holds another scenario (names can share one).
    pub fn save(&mut self, root: &Path, name: &str, full: &Graph, ops: &[SimOp]) {
        self.ask = None;
        let s = match snapshot(name, "", full, ops) {
            Ok(s) => s,
            Err(e) => return self.status = Some(Status::Failed(e)),
        };
        let path = file_for(root, &s.name);
        if !path.exists() {
            return self.write(root, full, &s);
        }
        match read(&path) {
            Ok(old) if old.name.trim() == s.name => self.ask = Some(Ask::Replace(s)),
            Ok(old) => {
                self.status = Some(Status::Failed(format!(
                    "{} holds the scenario \"{}\"; choose another name",
                    path.display(),
                    old.name
                )))
            }
            Err(e) => self.status = Some(Status::Failed(format!("{}: {e}", path.display()))),
        }
    }

    fn write(&mut self, root: &Path, full: &Graph, s: &Scenario) {
        self.status = Some(match save(root, s) {
            Ok(_) => Status::Saved(s.name.clone()),
            Err(e) => Status::Failed(e),
        });
        self.refresh(root, full);
    }

    /// Ask to load the scenario at `path` instead of `current` changes;
    /// refused when its changes do not apply to `full`.
    pub fn load(&mut self, path: &Path, full: &Graph, current: usize) {
        self.ask = None;
        let checked = read(path).and_then(|s| {
            apply_ops(full, &s.ops).map_err(|e| e.to_string())?;
            Ok(s)
        });
        match checked {
            Ok(scenario) => {
                self.ask = Some(Ask::Load {
                    stale: scenario.source.fingerprint != fingerprint(full).fingerprint,
                    scenario,
                    current,
                })
            }
            Err(e) => self.status = Some(Status::Failed(e)),
        }
    }

    pub fn delete(&mut self, i: usize) {
        self.ask = self.saved.get(i).cloned().map(Ask::Delete);
    }

    /// Do the confirmed action; the changes to simulate when it loads.
    pub fn confirm(&mut self, root: &Path, full: &Graph) -> Option<Vec<SimOp>> {
        match self.ask.take()? {
            Ask::Load { scenario, .. } => {
                self.status = Some(Status::Loaded(scenario.name));
                return Some(scenario.ops);
            }
            Ask::Delete(saved) => {
                self.status = Some(match std::fs::remove_file(&saved.path) {
                    Ok(()) => Status::Deleted(saved.name),
                    Err(e) => Status::Failed(e.to_string()),
                });
                self.refresh(root, full);
            }
            Ask::Replace(s) => self.write(root, full, &s),
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::simulation::tests::{cut, graph};

    /// The value the web viewer's `scenarioSource` computes for this graph
    /// (run with node 20 on the same nodes and edges).
    #[test]
    fn fingerprint_matches_the_web_viewer() {
        let s = fingerprint(&graph());
        assert_eq!(
            s.fingerprint,
            "sha256:8847085acaf7cc686dc846cd52b66fd99125c2accf274493b52e55fffc448741"
        );
        assert_eq!((s.nodes, s.edges), (4, 2));
    }

    #[test]
    fn a_saved_scenario_reads_back_and_lists_by_name() {
        let root = std::env::temp_dir().join(format!("ag-scenario-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let g = graph();
        let s = snapshot(" Break app/core ", "why", &g, &[cut("core", "app")]).expect("valid");
        assert_eq!(s.name, "Break app/core");
        assert_eq!(s.scope.nodes, ["app", "core"]);
        let path = save(&root, &s).expect("saved");
        assert_eq!(path, dir(&root).join("break-app-core.json"));
        assert_eq!(read(&path).expect("read"), s);
        let listed = list(&root, &fingerprint(&g));
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].name.as_str(), listed[0].ops, listed[0].stale),
            ("Break app/core", 1, false)
        );
        let mut other = g.clone();
        other.edges.pop();
        assert!(
            list(&root, &fingerprint(&other))[0].stale,
            "made on another graph"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The JSON is the web viewer's: SimOps externally tagged, every field.
    #[test]
    fn the_json_is_the_web_format() {
        let s = snapshot("x", "", &graph(), &[cut("core", "app")]).expect("valid");
        let v: Value = serde_json::to_value(&s).expect("json");
        assert_eq!(v["schema"], SCHEMA);
        assert_eq!(v["ops"][0]["cut_edge"]["from"], "core");
        let keys: Vec<&str> = v
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut keys = keys;
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "description",
                "name",
                "ops",
                "project",
                "schema",
                "scope",
                "source"
            ]
        );
    }

    #[test]
    fn invalid_scenarios_are_refused() {
        let g = graph();
        assert!(snapshot("  ", "", &g, &[]).is_err(), "no name");
        assert!(
            snapshot("x", "", &g, &[cut("nowhere", "app")]).is_err(),
            "op does not apply"
        );
        let mut s = snapshot("x", "", &g, &[]).expect("valid");
        s.schema = "other/v2".into();
        assert!(validate(&s).is_err());
        let extra = r#"{"schema":"archguard-refactoring-scenario/v1","name":"x","description":"","project":".","source":{"fingerprint":"sha256:0000000000000000000000000000000000000000000000000000000000000000","nodes":0,"edges":0},"scope":{"nodes":[]},"ops":[],"more":1}"#;
        assert!(
            serde_json::from_str::<Scenario>(extra).is_err(),
            "unknown field"
        );
    }

    /// Saving under a used name asks; loading asks and then hands back the
    /// changes; deleting asks and removes the file.
    #[test]
    fn panel_actions_ask_before_they_overwrite_load_or_delete() {
        let root = std::env::temp_dir().join(format!("ag-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (g, ops) = (graph(), vec![cut("core", "app")]);
        let mut p = Scenarios::default();
        p.save(&root, "Plan", &g, &ops);
        assert_eq!(p.status, Some(Status::Saved("Plan".into())));
        assert_eq!(p.saved.len(), 1);
        p.save(&root, "Plan", &g, &[]);
        assert!(matches!(p.ask, Some(Ask::Replace(_))), "same name asks");
        p.ask = None;
        p.load(&p.saved[0].path.clone(), &g, 3);
        let Some(Ask::Load { current, stale, .. }) = &p.ask else {
            panic!("load asks: {:?}", p.ask);
        };
        assert_eq!((*current, *stale), (3, false));
        assert_eq!(p.confirm(&root, &g), Some(ops));
        p.delete(0);
        assert_eq!(p.confirm(&root, &g), None);
        assert_eq!(p.status, Some(Status::Deleted("Plan".into())));
        assert!(p.saved.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two names with one file name: the second must not offer to replace
    /// the first, whose name differs.
    #[test]
    fn a_name_that_shares_the_file_of_another_scenario_is_refused() {
        let root = std::env::temp_dir().join(format!("ag-slug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (g, ops) = (graph(), vec![cut("core", "app")]);
        let mut p = Scenarios::default();
        p.save(&root, "Break app/core", &g, &ops);
        p.save(&root, "break app core", &g, &[]);
        assert_eq!(p.ask, None, "not the same scenario");
        assert!(
            matches!(&p.status, Some(Status::Failed(e)) if e.contains("Break app/core")),
            "{:?}",
            p.status
        );
        assert_eq!(
            read(&file_for(&root, "Break app/core")).expect("kept").ops,
            ops
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A repository must not redirect saving outside it through a linked
    /// `.archguard` or `.archguard/scenarios`.
    #[cfg(unix)]
    #[test]
    fn a_linked_scenarios_folder_is_refused() {
        let base = std::env::temp_dir().join(format!("ag-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join("repo"), base.join("outside"));
        std::fs::create_dir_all(&root).expect("repo");
        std::fs::create_dir_all(&outside).expect("outside");
        let s = snapshot("Plan", "", &graph(), &[]).expect("valid");
        std::os::unix::fs::symlink(&outside, root.join(".archguard")).expect("link");
        assert!(save(&root, &s).is_err(), "linked .archguard");
        std::fs::remove_file(root.join(".archguard")).expect("unlink");
        std::fs::create_dir_all(root.join(".archguard")).expect("dir");
        std::os::unix::fs::symlink(&outside, dir(&root)).expect("link");
        assert!(save(&root, &s).is_err(), "linked scenarios");
        assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
