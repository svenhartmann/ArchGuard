//! Rust workspace crates reach each other only through the dependencies their
//! manifests declare. A compiler-resolved call or type use names the crate
//! that defines its target, also when the caller reaches it through a
//! re-export (`pub use app_core as core` in a facade crate) whose import edge
//! already points at the facade. Projected onto the defining crate it would
//! add a dependency the manifest rules out, e.g. a layer violation past the
//! facade. Cargo forbids crate cycles, so leaving these out hides no cycle.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::{CallAnalysis, Config, SemanticDetails};
use archguard_extract::{Lang, SourceFile, strip_root};

use crate::pipeline::{crate_directory, read_manifest};

/// Crate directory (graph-relative) → its package and the local packages
/// its `[dependencies]` declare, its own included.
pub(crate) struct CrateDependencies(BTreeMap<String, (String, BTreeSet<String>)>);

impl CrateDependencies {
    pub(crate) fn read(root: &Path, files: &[SourceFile], config: &Config) -> Self {
        let mut crates = BTreeMap::new();
        for file in files.iter().filter(|f| f.lang == Lang::Rust) {
            let stripped = strip_root(&file.rel_path, &config.project.source_roots);
            let Some(key) = crate_directory(&stripped) else {
                continue;
            };
            let key = key.to_string_lossy().into_owned();
            if crates.contains_key(&key) {
                continue;
            }
            let manifest = crate_directory(&file.rel_path).and_then(|d| read_manifest(root, d));
            if let Some(manifest) = manifest {
                let declared = manifest.dependencies.into_values().collect();
                crates.insert(key, (manifest.package, declared));
            }
        }
        CrateDependencies(crates)
    }

    /// Whether a source at `from` may depend on one at `to` (graph-relative
    /// paths): always inside a crate or outside known crates.
    pub(crate) fn allows(&self, from: &str, to: &str) -> bool {
        let known = |file: &str| {
            let directory = crate_directory(file)?.to_string_lossy().into_owned();
            self.0.get(&directory)
        };
        match (known(from), known(to)) {
            (Some((_, declared)), Some((package, _))) => declared.contains(package),
            _ => true,
        }
    }
}

/// `calls` without the Rust dependencies between workspace crates that are
/// not declared, and how many were left out; `None` when none is.
pub(crate) fn declared_only(
    calls: &CallAnalysis,
    crates: &CrateDependencies,
) -> Option<(CallAnalysis, usize)> {
    let files: BTreeMap<&str, &str> = calls
        .classes
        .iter()
        .map(|c| (c.id.as_str(), c.file.as_str()))
        .collect();
    let reaches = |from: &str, class: Option<&str>| {
        class
            .and_then(|id| files.get(id))
            .is_none_or(|to| crates.allows(from, to))
    };
    let mut kept = CallAnalysis {
        classes: calls.classes.clone(),
        methods: calls.methods.clone(),
        ..Default::default()
    };
    let mut left_out = 0;
    for class in &mut kept.classes {
        let before = class.supertypes.len();
        class
            .supertypes
            .retain(|parent| reaches(&class.file, Some(parent)));
        left_out += before - class.supertypes.len();
    }
    for call in calls.calls.iter().filter(|c| c.resolution == "project") {
        if reaches(&call.file, call.to_class.as_deref()) {
            kept.calls.push(call.clone());
        } else {
            left_out += 1;
        }
    }
    let uses = calls.details.iter().flat_map(|d| &d.type_uses);
    let mut type_uses = Vec::new();
    for used in uses.filter(|u| u.resolution == "project") {
        if reaches(&used.file, used.to_class.as_deref()) {
            type_uses.push(used.clone());
        } else {
            left_out += 1;
        }
    }
    kept.details = Some(SemanticDetails {
        type_uses,
        ..Default::default()
    });
    (left_out > 0).then_some((kept, left_out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{CallClass, MethodCall, SemanticTypeUse};

    fn workspace() -> (tempfile::TempDir, Vec<SourceFile>, Config) {
        let root = tempfile::tempdir().unwrap();
        let put = |path: &str, text: &str| {
            let path = root.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        put("crates/core/Cargo.toml", "[package]\nname = \"app-core\"\n");
        put("crates/core/src/lib.rs", "pub fn f() {}");
        put(
            "crates/engine/Cargo.toml",
            "[package]\nname = \"app-engine\"\n[dependencies]\napp-core = { path = \"../core\" }\n",
        );
        put("crates/engine/src/lib.rs", "pub use app_core as core;");
        put(
            "crates/cli/Cargo.toml",
            "[package]\nname = \"app-cli\"\n[dependencies]\napp-engine.workspace = true\n",
        );
        put(
            "crates/cli/src/main.rs",
            "fn main() { app_engine::core::f(); }",
        );
        put("crates/cli/src/args.rs", "");
        put("loose/x.rs", "");
        let config = Config {
            project: archguard_core::ProjectConfig {
                source_roots: vec!["crates".into()],
                ..Config::default().project
            },
            ..Config::default()
        };
        let files = crate::pipeline::walk_files(root.path(), &config).unwrap();
        (root, files, config)
    }

    #[test]
    fn only_declared_crate_dependencies_are_allowed() {
        let (root, files, config) = workspace();
        let crates = CrateDependencies::read(root.path(), &files, &config);
        assert!(
            !crates.allows("cli/src/main.rs", "core/src/lib.rs"),
            "past the facade"
        );
        assert!(crates.allows("cli/src/main.rs", "engine/src/lib.rs"));
        assert!(crates.allows("engine/src/lib.rs", "core/src/lib.rs"));
        assert!(
            crates.allows("cli/src/main.rs", "cli/src/args.rs"),
            "same crate"
        );
        assert!(
            crates.allows("../loose/x.rs", "core/src/lib.rs"),
            "no crate"
        );
    }

    fn class(id: &str, file: &str) -> CallClass {
        CallClass {
            language: Some("rust".into()),
            id: id.into(),
            name: id.into(),
            package: String::new(),
            file: file.into(),
            graph_node: file.into(),
            line: 1,
            kind: "struct".into(),
            is_abstract: Some(false),
            visibility: Some("pub".into()),
            supertypes: vec![],
            supertype_sites: None,
            declaration_files: None,
        }
    }

    fn call(from: &str, file: &str, to: &str) -> MethodCall {
        MethodCall {
            from_class: from.into(),
            from_method: None,
            to_class: Some(to.into()),
            to_method: None,
            target_label: to.into(),
            dispatch: "static".into(),
            resolution: "project".into(),
            file: file.into(),
            line: 1,
            column: 1,
            code: String::new(),
        }
    }

    #[test]
    fn calls_type_uses_and_supertypes_past_a_facade_are_left_out() {
        let (root, files, config) = workspace();
        let crates = CrateDependencies::read(root.path(), &files, &config);
        let mut cli = class("Cli", "cli/src/main.rs");
        cli.supertypes = vec!["Core".into(), "Engine".into()];
        let analysis = CallAnalysis {
            classes: vec![
                cli,
                class("Core", "core/src/lib.rs"),
                class("Engine", "engine/src/lib.rs"),
            ],
            calls: vec![
                call("Cli", "cli/src/main.rs", "Core"),
                call("Cli", "cli/src/main.rs", "Engine"),
                call("Engine", "engine/src/lib.rs", "Core"),
            ],
            details: Some(SemanticDetails {
                type_uses: vec![SemanticTypeUse {
                    from_class: "Cli".into(),
                    from_method: None,
                    to_class: Some("Core".into()),
                    target_label: "Core".into(),
                    role: "field".into(),
                    resolution: "project".into(),
                    file: "cli/src/main.rs".into(),
                    line: 1,
                    code: String::new(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let (kept, left_out) = declared_only(&analysis, &crates).expect("some left out");
        assert_eq!(left_out, 3, "call, type use and supertype into core");
        let targets: Vec<_> = kept.calls.iter().map(|c| c.to_class.as_deref()).collect();
        assert_eq!(targets, [Some("Engine"), Some("Core")]);
        assert!(kept.details.unwrap().type_uses.is_empty());
        assert_eq!(kept.classes[0].supertypes, ["Engine"]);
        let inside = CallAnalysis {
            calls: vec![call("Engine", "engine/src/lib.rs", "Core")],
            classes: vec![
                class("Core", "core/src/lib.rs"),
                class("Engine", "engine/src/lib.rs"),
            ],
            ..Default::default()
        };
        assert!(
            declared_only(&inside, &crates).is_none(),
            "nothing to leave out"
        );
    }
}
