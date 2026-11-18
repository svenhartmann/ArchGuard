//! Swift Package Manager targets are Swift's modules (ADR 0018): `import X`
//! links to the directory of the project target `X`, the fixture's calls
//! resolve only where syntax names a unique project type.

use std::collections::BTreeSet;
use std::path::Path;

use archguard_core::Config;
use archguard_engine::{Level, analyze, analyze_calls};

/// A copy of `fixtures/swift-spm`, so the analysis writes nothing into the repo.
fn fixture() -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/swift-spm");
    copy(&source, dir.path());
    dir
}

#[test]
fn swift_imports_link_files_to_their_target_directory() {
    let dir = fixture();
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).unwrap();
    let name = |id| {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap()
            .name
            .clone()
    };
    let edges: BTreeSet<_> = graph
        .edges
        .iter()
        .map(|edge| (name(edge.from), name(edge.to)))
        .collect();
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    // The derived source roots `Sources` and `Tests` are stripped. `import`
    // links to the target directory; `Foundation`, `XCTest` and
    // `PackageDescription` are platform modules, and `Logging` is no target
    // of this package and stays external. Resolved project calls add the
    // file edges to `Price.swift`, the extension's among them.
    assert_eq!(
        edges,
        BTreeSet::from([
            pair("App/main.swift", "Core/*"),
            pair("App/main.swift", "Core/Model/Price.swift"),
            pair("Core/Price+Format.swift", "Core/Model/Price.swift"),
            pair("CoreTests/PriceTests.swift", "Core/*"),
            pair("CoreTests/PriceTests.swift", "Core/Model/Price.swift"),
        ])
    );
}

#[test]
fn swift_calls_resolve_unique_project_types_only() {
    let dir = fixture();
    let report = analyze_calls(dir.path(), &Config::default());
    let analyzer = report
        .analyzers
        .iter()
        .find(|a| a.language == "swift")
        .unwrap();
    assert_eq!((analyzer.status.as_str(), analyzer.files), ("partial", 5));
    let project: BTreeSet<_> = report
        .calls
        .iter()
        .filter(|c| c.resolution == "project")
        .map(|c| {
            (
                c.file.as_str(),
                c.target_label.as_str(),
                c.dispatch.as_str(),
            )
        })
        .collect();
    assert_eq!(
        project,
        BTreeSet::from([
            ("Sources/App/main.swift", "Price", "constructor"),
            ("Sources/App/main.swift", "Price.zero", "static"),
            ("Sources/Core/Model/Price.swift", "Price", "constructor"),
            ("Sources/Core/Price+Format.swift", "Price", "constructor"),
            ("Tests/CoreTests/PriceTests.swift", "Price", "constructor"),
            ("Tests/CoreTests/PriceTests.swift", "Price.zero", "static"),
        ])
    );
    // `rounded` comes from an extension in another file and belongs to `Price`.
    let rounded = report.methods.iter().find(|m| m.name == "rounded").unwrap();
    assert_eq!(
        rounded.class_id,
        "swift:Sources/Core/Model/Price.swift::Price"
    );
    let price = report
        .classes
        .iter()
        .find(|c| c.id == rounded.class_id)
        .unwrap();
    assert_eq!(price.package, "Core/Model");
    assert_eq!(
        price.supertypes,
        ["swift:CustomStringConvertible", "swift:Equatable"]
    );
}
