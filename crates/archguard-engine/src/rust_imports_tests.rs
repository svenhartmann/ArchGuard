//! Kept apart so the resolver itself never depends on the pipeline that calls it.
use crate::pipeline::build_graph;
use archguard_core::Config;

fn graph(files: &[(&str, &str)]) -> (archguard_core::Graph, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let sources = archguard_extract::walk(root.path(), &[]).unwrap();
    build_graph(root.path(), &sources, &Config::default()).unwrap()
}
fn linked(graph: &archguard_core::Graph, from: &str, to: &str) -> bool {
    graph.edges.iter().any(|edge| {
        graph.node_name(edge.from) == Some(from) && graph.node_name(edge.to) == Some(to)
    })
}
#[test]
fn bare_paths_resolve_in_the_current_module_since_edition_2018() {
    // Default source_roots strip `src/`, so nodes are named `lib.rs`, `kinds.rs`, …
    let (g, warnings) = graph(&[
        (
            "src/lib.rs",
            "mod inner; mod kinds; mod nested; pub use kinds::Kind;",
        ),
        ("src/inner.rs", "pub struct Deep;"),
        (
            "src/kinds.rs",
            "pub enum Kind { A, B }\nuse Kind::*;\nfn f(_: Kind) {}",
        ),
        (
            "src/nested.rs",
            "mod inner;\nuse inner::Deep;\nuse std::fmt;\nuse serde::Serialize;\npub struct Outer;",
        ),
        ("src/nested/inner.rs", "pub struct Deep;"),
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    // `use Kind::*` names the enum in its own file, not the crate root's re-export.
    assert!(!linked(&g, "kinds.rs", "lib.rs"));
    // `use inner::Deep` inside `nested` names nested's child, not the root's `inner`.
    assert!(linked(&g, "nested.rs", "nested/inner.rs"));
    assert!(!linked(&g, "nested.rs", "inner.rs"));
    // External crates never fall back to a local file.
    assert!(!linked(&g, "nested.rs", "lib.rs"));
}
#[test]
fn declared_workspace_crates_link_to_the_defining_file() {
    let (g, warnings) = graph(&[
        ("core/Cargo.toml", "[package]\nname = \"app-core\"\n"),
        (
            "core/src/lib.rs",
            "pub mod model; pub use model::Thing; pub use model::Thing as Renamed;",
        ),
        ("core/src/model.rs", "pub struct Thing; pub struct Other;"),
        (
            "cli/Cargo.toml",
            "[package]\nname = \"app-cli\"\n[dependencies]\napp-core = { path = \"../core\" }\n",
        ),
        ("cli/src/lib.rs", "mod a; mod b; mod c;"),
        ("cli/src/a.rs", "use app_core::model::Other;"),
        ("cli/src/b.rs", "fn f() -> app_core::Thing { todo!() }"),
        ("cli/src/c.rs", "fn f() { let _ = app_core::model::Other; }"),
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(linked(&g, "cli/src/a.rs", "core/src/model.rs"));
    assert!(linked(&g, "cli/src/b.rs", "core/src/lib.rs"));
    assert!(linked(&g, "cli/src/c.rs", "core/src/model.rs"));
}
#[test]
fn renamed_and_workspace_dependencies_are_followed_and_bins_reach_their_own_library() {
    let (g, _) = graph(&[
        ("core/Cargo.toml", "[package]\nname = \"app-core\"\n"),
        ("core/src/lib.rs", "pub struct Thing;"),
        (
            "cli/Cargo.toml",
            "[package]\nname = \"app-cli\"\n[dependencies]\nshared = { path = \"../core\", package = \"app-core\" }\n",
        ),
        ("cli/src/lib.rs", "use shared::Thing; pub struct Api;"),
        ("cli/src/main.rs", "use app_cli::Api;"),
        (
            "tool/Cargo.toml",
            "[package]\nname = \"app-tool\"\n[dependencies]\napp-core.workspace = true\n",
        ),
        ("tool/src/lib.rs", "use app_core::Thing;"),
    ]);
    assert!(linked(&g, "cli/src/lib.rs", "core/src/lib.rs"));
    assert!(linked(&g, "cli/src/main.rs", "cli/src/lib.rs"));
    assert!(linked(&g, "tool/src/lib.rs", "core/src/lib.rs"));
}
#[test]
fn undeclared_registry_dev_and_ambiguous_crates_create_no_edge() {
    let core = ("core/Cargo.toml", "[package]\nname = \"app-core\"\n");
    let core_lib = ("core/src/lib.rs", "pub struct Thing;");
    for manifest in [
        "[package]\nname = \"app-cli\"\n",
        "[package]\nname = \"app-cli\"\n[dependencies]\napp-core = \"1\"\n",
        "[package]\nname = \"app-cli\"\n[dev-dependencies]\napp-core = { path = \"../core\" }\n",
    ] {
        let (g, _) = graph(&[
            core,
            core_lib,
            ("cli/Cargo.toml", manifest),
            ("cli/src/lib.rs", "use app_core::Thing;"),
        ]);
        assert!(
            !linked(&g, "cli/src/lib.rs", "core/src/lib.rs"),
            "{manifest}"
        );
    }
    let (g, _) = graph(&[
        core,
        core_lib,
        ("copy/Cargo.toml", "[package]\nname = \"app-core\"\n"),
        ("copy/src/lib.rs", "pub struct Thing;"),
        (
            "cli/Cargo.toml",
            "[package]\nname = \"app-cli\"\n[dependencies]\napp-core = { path = \"../core\" }\n",
        ),
        ("cli/src/lib.rs", "use app_core::Thing;"),
    ]);
    assert!(!linked(&g, "cli/src/lib.rs", "core/src/lib.rs"));
    assert!(!linked(&g, "cli/src/lib.rs", "copy/src/lib.rs"));
}
#[test]
fn crate_items_reexports_multiline_groups_and_inline_super_resolve_to_files() {
    let (g, warnings) = graph(&[
        (
            "src/main.rs",
            "mod commands; mod model; pub use model::Thing; struct Cli; const EXIT: u8=0;",
        ),
        ("src/model.rs", "pub struct Thing;"),
        (
            "src/commands.rs",
            "use crate::{\n Cli, EXIT, Thing,\n};\nuse crate::model::Thing as Other;\n#[cfg(test)] mod tests {use super::*;}",
        ),
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(linked(&g, "commands.rs", "main.rs"));
    assert!(linked(&g, "commands.rs", "model.rs"));
    assert!(g.edges.iter().all(|e| e.from != e.to));
}
#[test]
fn nested_super_self_and_path_attributes_follow_declared_modules() {
    let (g, warnings) = graph(&[
        (
            "src/lib.rs",
            "mod parent; #[path=\"elsewhere.rs\"] mod alias;",
        ),
        (
            "src/parent.rs",
            "pub struct Parent; mod child; pub mod inline { pub struct Item; }",
        ),
        (
            "src/parent/child.rs",
            "use super::Parent; use super::super::alias::Other; use self::local::Here; mod local {pub struct Here;}",
        ),
        ("src/elsewhere.rs", "pub struct Other;"),
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(linked(&g, "parent/child.rs", "parent.rs"));
    assert!(linked(&g, "parent/child.rs", "elsewhere.rs"));
}
#[test]
fn identical_crate_names_do_not_cross_workspace_boundaries_and_unknowns_stay_visible() {
    let (g, warnings) = graph(&[
        ("a/src/lib.rs", "mod client; pub struct Shared;"),
        ("a/src/client.rs", "use crate::Shared; use crate::Missing;"),
        (
            "b/src/lib.rs",
            "mod client; pub struct Shared; pub struct Missing;",
        ),
        ("b/src/client.rs", "use crate::Shared;"),
    ]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("crate::Missing"));
    assert!(linked(&g, "a/src/client.rs", "a/src/lib.rs"));
    assert!(linked(&g, "b/src/client.rs", "b/src/lib.rs"));
    assert!(!linked(&g, "a/src/client.rs", "b/src/lib.rs"));
}
#[test]
fn strings_and_comments_are_not_imports_and_invalid_syntax_is_reported() {
    let (_, warnings) = graph(&[(
        "src/lib.rs",
        "const TEXT: &str = r#\"use crate::Fake;\"#; // use crate::Other;\n",
    )]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let (_, warnings) = graph(&[("src/lib.rs", "fn broken(")]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("could not parse"));
}
#[test]
fn local_glob_reexports_preserve_the_exporting_file_boundary() {
    let (g, warnings) = graph(&[
        ("src/lib.rs", "mod types; pub use types::*; mod client;"),
        ("src/types.rs", "pub struct Item;"),
        ("src/client.rs", "use crate::Item;"),
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(linked(&g, "client.rs", "lib.rs"));
    assert!(linked(&g, "lib.rs", "types.rs"));
}
