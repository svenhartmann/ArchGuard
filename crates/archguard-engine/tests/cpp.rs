//! C/C++ includes resolve against the including file's directory and the
//! include directories of CMakeLists.txt and compile_commands.json (ADR 0017).

use std::collections::BTreeSet;
use std::fs;

use archguard_engine::{Level, analyze_run_quiet, load_config};

const FIXTURE: [(&str, &str); 8] = [
    (
        "CMakeLists.txt",
        "cmake_minimum_required(VERSION 3.20)\nproject(shop CXX)\nadd_subdirectory(app)\n",
    ),
    (
        "app/CMakeLists.txt",
        "add_executable(app main.cpp)\ntarget_include_directories(app PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/../include)\n",
    ),
    (
        "compile_commands.json",
        r#"[{"directory": "build", "command": "cc -I../vendor -c ../app/main.cpp", "file": "../app/main.cpp"}]"#,
    ),
    (
        "app/main.cpp",
        "#include \"local.h\"\n#include \"shop/order.h\"\n#include <lib/log.h>\n#include <vector>\n#include \"missing.h\"\nint main() { return 0; }\n",
    ),
    ("app/local.h", "#pragma once\n"),
    (
        "include/shop/order.h",
        "#pragma once\n#include \"../shop/pay.h\"\nclass Order {};\n",
    ),
    ("include/shop/pay.h", "#pragma once\n"),
    ("vendor/lib/log.h", "#pragma once\n"),
];

#[test]
fn includes_resolve_against_directory_and_include_dirs() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in &FIXTURE {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    fs::create_dir_all(dir.path().join("build")).expect("build dir");
    let loaded = load_config(None, dir.path()).expect("config");
    let run = analyze_run_quiet(dir.path(), &loaded, Level::File).expect("analyze");
    let (graph, warnings) = (&run.graph, &run.warnings);
    let name = |id| {
        let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
        node.name.clone()
    };
    let edges: BTreeSet<(String, String)> = graph
        .edges
        .iter()
        .map(|edge| (name(edge.from), name(edge.to)))
        .collect();
    let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
    // `app/` and `include/` are detected source roots, so node names drop them.
    assert_eq!(
        edges,
        BTreeSet::from([
            pair("main.cpp", "local.h"),
            pair("main.cpp", "shop/order.h"),
            pair("main.cpp", "vendor/lib/log.h"),
            pair("shop/order.h", "shop/pay.h"),
        ])
    );
    // The unresolved quoted include is a gap; the system header is not.
    let gaps: Vec<_> = warnings.iter().filter(|w| w.contains(".h")).collect();
    assert_eq!(gaps.len(), 1, "{warnings:?}");
    assert!(gaps[0].contains("missing.h"), "{gaps:?}");
    assert!(
        !warnings.iter().any(|w| w.contains("vector")),
        "{warnings:?}"
    );
}
