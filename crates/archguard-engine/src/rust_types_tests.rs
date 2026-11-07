//! Rust type relations: abstractness, implemented traits and supertraits from
//! syntax, and with rust-analyzer the precise trait targets and type uses.
//! Kept apart so `rust_calls` stays below the module size limits.
use crate::rust_calls::analyze_with;
use archguard_core::{CallAnalysis, CallClass, Config};

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    dir
}
fn syntax(files: &[(&str, &str)]) -> CallAnalysis {
    let dir = project(files);
    analyze_with(dir.path(), &Config::default(), None)
}
fn class<'a>(report: &'a CallAnalysis, id: &str) -> &'a CallClass {
    report
        .classes
        .iter()
        .find(|class| class.id == id)
        .unwrap_or_else(|| panic!("no class {id}: {:?}", report.classes))
}

#[test]
fn traits_are_abstract_and_data_types_concrete() {
    let report = syntax(&[(
        "src/lib.rs",
        "pub trait Port {}\npub struct Data;\npub enum Kind { A }\npub union Bits { a: u32 }\n",
    )]);
    for (id, kind, is_abstract) in [
        ("lib.rs::Port", "trait", true),
        ("lib.rs::Data", "struct", false),
        ("lib.rs::Kind", "enum", false),
        ("lib.rs::Bits", "union", false),
    ] {
        let class = class(&report, id);
        assert_eq!(
            (class.kind.as_str(), class.is_abstract),
            (kind, Some(is_abstract))
        );
    }
}

#[test]
fn a_struct_declared_after_its_impl_keeps_its_kind_and_line() {
    let report = syntax(&[(
        "src/lib.rs",
        "impl Late { fn new() -> Self { Late } }\n\npub struct Late;\n",
    )]);
    let late = class(&report, "lib.rs::Late");
    assert_eq!((late.kind.as_str(), late.line), ("struct", 3));
    assert_eq!(late.visibility.as_deref(), Some("public"));
}

#[test]
fn impl_trait_for_type_resolves_within_the_crate() {
    let report = syntax(&[
        (
            "src/lib.rs",
            "mod port;\nmod adapter;\npub trait Named {}\npub struct Local;\nimpl Named for Local {}\nmod inner { pub trait Deep {} }\nimpl inner::Deep for Local {}\n",
        ),
        ("src/port.rs", "pub trait Port {}\n"),
        (
            "src/adapter.rs",
            "use crate::port::Port;\npub struct ViaUse;\nimpl Port for ViaUse {}\npub struct ViaPath;\nimpl crate::port::Port for ViaPath {}\nimpl super::Named for ViaPath {}\n",
        ),
    ]);
    assert_eq!(
        class(&report, "lib.rs::Local").supertypes,
        ["lib.rs::Named", "lib.rs::inner::Deep"]
    );
    let via_use = class(&report, "adapter.rs::ViaUse");
    assert_eq!(via_use.supertypes, ["port.rs::Port"]);
    let sites = via_use.supertype_sites.as_ref().unwrap();
    assert_eq!(
        (sites[0].file.as_str(), sites[0].line),
        ("adapter.rs", 3),
        "the impl block is the evidence"
    );
    assert_eq!(
        class(&report, "adapter.rs::ViaPath").supertypes,
        ["lib.rs::Named", "port.rs::Port"]
    );
}

#[test]
fn supertraits_are_supertypes() {
    let report = syntax(&[(
        "src/lib.rs",
        "pub trait Base {}\npub trait Child: Base + std::fmt::Debug {}\n",
    )]);
    assert_eq!(
        class(&report, "lib.rs::Child").supertypes,
        ["external:std::fmt::Debug", "lib.rs::Base"]
    );
}

#[test]
fn traits_outside_the_crate_are_external_or_unresolved() {
    let report = syntax(&[(
        "src/lib.rs",
        "use std::fmt::Display;\npub struct A;\nimpl std::fmt::Display for A { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }\npub struct B;\nimpl Display for B { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }\npub struct C;\nimpl Iterator for C { type Item = u8; fn next(&mut self) -> Option<u8> { None } }\npub struct D;\nimpl serde::Serialize for D {}\n",
    )]);
    assert_eq!(
        class(&report, "lib.rs::A").supertypes,
        ["external:std::fmt::Display"]
    );
    assert_eq!(
        class(&report, "lib.rs::B").supertypes,
        ["external:std::fmt::Display"]
    );
    // A bare name without a binding may come from the prelude, an external
    // glob import or a macro: syntax cannot tell which.
    assert_eq!(
        class(&report, "lib.rs::C").supertypes,
        ["unresolved:Iterator"]
    );
    // Another crate may be a workspace member; only std/core/alloc are known.
    assert_eq!(
        class(&report, "lib.rs::D").supertypes,
        ["unresolved:serde::Serialize"]
    );
}

#[test]
fn blanket_impls_give_no_type_a_supertype() {
    let report = syntax(&[(
        "src/lib.rs",
        "pub trait Named {}\nimpl<T: Clone> Named for T {}\n",
    )]);
    assert!(
        report
            .classes
            .iter()
            .all(|class| class.supertypes.is_empty()),
        "{:?}",
        report.classes
    );
}

#[test]
fn item_macros_and_unparsed_files_leave_abstractness_unknown() {
    let report = syntax(&[
        (
            "src/lib.rs",
            "mod gen;\nmod bad;\nmacro_rules! make { () => { pub struct Made; } }\npub trait Port {}\n",
        ),
        ("src/gen.rs", "crate::make!();\npub struct Seen;\n"),
        ("src/bad.rs", "pub struct Broken {"),
    ]);
    let unknown = report
        .classes
        .iter()
        .filter(|class| class.is_abstract.is_none())
        .map(|class| (class.file.as_str(), class.kind.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    // A macro definition alone declares nothing; its invocation might.
    assert_eq!(
        unknown,
        [("bad.rs", "unknown"), ("gen.rs", "unknown")].into()
    );
}

#[test]
fn rust_has_no_implementation_inheritance() {
    let report = syntax(&[(
        "src/lib.rs",
        "pub trait Port {}\npub struct Thing;\nimpl Port for Thing {}\n",
    )]);
    let metrics = crate::code_metrics::code_metrics(&report);
    for class in &metrics.classes {
        assert_eq!(class.dit.status, "not_applicable", "{}", class.class_id);
        assert_eq!(class.noc.status, "not_applicable", "{}", class.class_id);
    }
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("DIT/NOC are not applicable")),
        "{:?}",
        report.diagnostics
    );
}

/// Requires rust-analyzer on PATH, like `rust_analyzer_resolves_calls_without_running_build_scripts`.
#[test]
fn rust_analyzer_resolves_trait_targets_and_type_uses() {
    let dir = project(&[
        (
            "Cargo.toml",
            "[package]\nname = \"typecrate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        (
            "src/lib.rs",
            "mod model;\nmod port;\nuse model::{Item, Price};\nuse port::Store;\npub struct Cart { items: Vec<Item> }\nimpl Cart {\n    pub fn total(&self, base: Price) -> Price { let _p = Price(1); base }\n}\nimpl Store for Cart {}\nimpl std::fmt::Debug for Cart { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }\n",
        ),
        (
            "src/model.rs",
            "pub struct Item;\npub struct Price(pub u32);\n",
        ),
        ("src/port.rs", "pub trait Store {}\n"),
    ]);
    let report = analyze_with(
        dir.path(),
        &Config::default(),
        Some(std::ffi::OsStr::new("rust-analyzer")),
    );
    let cart = class(&report, "lib.rs::Cart");
    assert_eq!(
        cart.supertypes,
        ["external:core::fmt::Debug", "port.rs::Store"],
        "{:?}",
        report.diagnostics
    );
    let uses = &report.details.as_ref().unwrap().type_uses;
    let used = |role: &str, target: &str| {
        uses.iter().find(|u| {
            u.from_class == "lib.rs::Cart"
                && u.role == role
                && u.to_class.as_deref() == Some(target)
                && u.resolution == "project"
        })
    };
    assert!(used("field", "model.rs::Item").is_some(), "{uses:?}");
    assert!(used("parameter", "model.rs::Price").is_some(), "{uses:?}");
    assert!(used("return", "model.rs::Price").is_some(), "{uses:?}");
    let local = used("local", "model.rs::Price").expect("body reference");
    assert!(local.from_method.as_deref().unwrap().contains("#total@"));
    assert!(
        uses.iter()
            .any(|u| u.target_label == "alloc::vec::Vec" && u.resolution == "external"),
        "{uses:?}"
    );
    assert!(
        uses.iter()
            .all(|u| u.to_class.as_deref() != Some("lib.rs::Cart")),
        "a type never couples to itself"
    );
}
