//! Swift import scanner and Swift Package Manager target reader (ADR 0018).
//! A Swift module is a package target: `import Core` names the target `Core`,
//! never a file, so every import is a module-level [`TargetSpec::Symbol`]
//! without a stem; the engine binds it to the target's directory. Swift files
//! declare no package and are named by their path. Comments (nested block
//! comments included) and string bodies, interpolations and raw strings
//! included, cannot create imports. Xcode projects are not read.

mod structure;
#[cfg(test)]
mod structure_tests;
mod tokens;

pub use structure::{
    SwiftCallSite, SwiftStructure, SwiftTarget, attach_extensions, scan_swift_structure,
};

use std::collections::BTreeMap;

use crate::extract::{Extraction, Extractor, RawImport, TargetSpec};
use crate::lex::Flavor;
use crate::walk::Lang;

/// Toolchain and platform modules: never a project target, so not reported
/// as unresolved imports.
const PLATFORM: [&str; 20] = [
    "Swift",
    "Foundation",
    "FoundationEssentials",
    "Dispatch",
    "Darwin",
    "Glibc",
    "Musl",
    "WinSDK",
    "ObjectiveC",
    "CoreFoundation",
    "UIKit",
    "AppKit",
    "SwiftUI",
    "Combine",
    "Observation",
    "XCTest",
    "Testing",
    "os",
    "PackageDescription",
    "_Concurrency",
];

/// Swift scanner. See the [module documentation](self).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SwiftExtractor;

impl Extractor for SwiftExtractor {
    fn lang(&self) -> Lang {
        Lang::Swift
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let lines: Vec<&str> = content.lines().collect();
        let mut extraction = Extraction::default();
        for (line, words) in code_lines(content) {
            let Some(module) = imported_module(&words) else {
                continue;
            };
            if PLATFORM.contains(&module) {
                continue;
            }
            extraction.imports.push(RawImport {
                source: rel_path.to_string(),
                target: TargetSpec::Symbol {
                    path: module.to_string(),
                    stem: None,
                },
                line,
                raw: lines.get(line as usize - 1).unwrap_or(&"").to_string(),
            });
        }
        extraction
    }
}

/// The module of an import statement's words: `import A`, `import A.B`,
/// `@testable import A`, `public import A`, `import struct A.B`.
fn imported_module<'a>(words: &[&'a str]) -> Option<&'a str> {
    let mut rest = words;
    loop {
        match rest {
            ["@", _, tail @ ..] => rest = tail,
            [
                "public" | "internal" | "package" | "private" | "fileprivate",
                tail @ ..,
            ] => rest = tail,
            ["import", tail @ ..] => {
                let tail = match tail {
                    [
                        "typealias" | "struct" | "class" | "enum" | "protocol" | "let" | "var"
                        | "func",
                        tail @ ..,
                    ] => tail,
                    tail => tail,
                };
                let module = *tail.first()?;
                let named = module.starts_with(|c: char| c.is_alphabetic() || c == '_');
                return named.then_some(module);
            }
            _ => return None,
        }
    }
}

/// The non-literal tokens of each source line that has any.
fn code_lines(content: &str) -> Vec<(u32, Vec<&str>)> {
    let tokens = crate::lex::tokens(content, Flavor::Swift);
    let mut lines: Vec<(u32, Vec<&str>)> = Vec::new();
    let mut fresh = true;
    for token in &tokens {
        if token.text == "\n" {
            fresh = true;
            continue;
        }
        if fresh {
            lines.push((token.line, Vec::new()));
            fresh = false;
        }
        if !token.literal {
            let text = &content[token.start..token.end];
            lines.last_mut().expect("pushed above").1.push(text);
        }
    }
    lines
}

/// A target that a `Package.swift` declares with a literal name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTarget {
    pub name: String,
    /// The target directory relative to the `Package.swift` directory: the
    /// literal `path:`, else `Sources/<name>` (`Tests/<name>` for test
    /// targets).
    pub path: String,
}

/// The targets of a `Package.swift`: every `.target(…)`, `.executableTarget(…)`,
/// `.testTarget(…)` and `.macro(…)` whose `name:` (and `path:`,
/// when given) is a plain string literal. Anything computed is left out;
/// the manifest is never run.
pub fn parse_package_swift(content: &str) -> Vec<PackageTarget> {
    let tokens: Vec<_> = crate::lex::tokens(content, Flavor::Swift)
        .into_iter()
        .filter(|t| t.text != "\n")
        .collect();
    let text = |i: usize| tokens.get(i).map_or("", |t| t.text.as_str());
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        let default = match text(i + 1) {
            "target" | "executableTarget" | "macro" => "Sources",
            "testTarget" => "Tests",
            _ => continue,
        };
        if text(i) != "." || text(i + 2) != "(" {
            continue;
        }
        let args = arguments(&tokens, i + 2);
        let Some(Some(name)) = args.get("name").cloned() else {
            continue;
        };
        let path = match args.get("path") {
            None => format!("{default}/{name}"),
            Some(Some(path)) => path.trim_end_matches('/').to_string(),
            Some(None) => continue,
        };
        out.push(PackageTarget { name, path });
    }
    out
}

/// The labeled arguments of the call whose `(` is at `open`: each label
/// with its value when that is a plain string literal.
fn arguments(tokens: &[crate::lex::Token], open: usize) -> BTreeMap<&str, Option<String>> {
    let text = |i: usize| tokens.get(i).map_or("", |t| t.text.as_str());
    let literal = |i: usize| {
        let token = tokens.get(i).filter(|t| t.literal)?;
        let inner = token.text.strip_prefix('"')?.strip_suffix('"')?;
        (!inner.contains(['\\', '"']) && !inner.is_empty()).then(|| inner.to_string())
    };
    let mut out = BTreeMap::new();
    let mut depth = 0usize;
    for j in open..tokens.len() {
        match text(j) {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" if depth == 1 => break,
            ")" | "]" | "}" => depth -= 1,
            label if depth == 1 && text(j + 1) == ":" => {
                out.insert(label, literal(j + 2));
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    fn imports(content: &str) -> Vec<(u32, String)> {
        SwiftExtractor
            .extract("Sources/App/main.swift", content)
            .imports
            .into_iter()
            .map(|import| match import.target {
                TargetSpec::Symbol { path, stem: None } => (import.line, path),
                other => panic!("unexpected target {other:?}"),
            })
            .collect()
    }

    #[test]
    fn import_forms_name_the_module() {
        let source = "import Core\n\
            @testable import Billing\n\
            import struct Shop.Order\n\
            @_exported import Shared.Sub\n\
            public import Api\n\
            import Foundation\n\
            import SwiftUI\n\
            let x = 1\n";
        assert_eq!(
            imports(source),
            [
                (1, "Core".to_string()),
                (2, "Billing".to_string()),
                (3, "Shop".to_string()),
                (4, "Shared".to_string()),
                (5, "Api".to_string()),
            ]
        );
        let found = SwiftExtractor.extract("a.swift", source);
        assert!(found.declarations.is_empty());
        assert_eq!(found.imports[1].raw, "@testable import Billing");
    }

    #[test]
    fn comments_strings_and_interpolations_never_create_imports() {
        let source = "/* outer /* nested */\nimport Fake1\n*/\n\
            // import Fake2\n\
            let a = \"\"\"\nimport Fake3\n\"\"\"\n\
            let b = \"\\(f(\")\")) import Fake4\"\n\
            let c = #\"\\\" import Fake5\"#\n\
            import Core\n";
        assert_eq!(imports(source), [(10, "Core".to_string())]);
    }

    #[test]
    fn package_targets_with_literal_names_and_default_paths() {
        let manifest = r#"// swift-tools-version:5.9
import PackageDescription

let name = "Computed"
let package = Package(
    name: "Shop",
    products: [.library(name: "Shop", targets: ["Core"])],
    dependencies: [.package(url: "https://example.org/x.git", from: "1.0.0")],
    targets: [
        .target(name: "Core", dependencies: [.product(name: "X", package: "x")]),
        .executableTarget(name: "App", dependencies: ["Core"], path: "Sources/Main/"),
        .testTarget(name: "CoreTests", dependencies: ["Core"]),
        .target(name: name),
        .target(name: "Dynamic", path: base + "/x"),
    ]
)
"#;
        let found = parse_package_swift(manifest);
        let pairs: Vec<_> = found
            .iter()
            .map(|t| (t.name.as_str(), t.path.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("Core", "Sources/Core"),
                ("App", "Sources/Main"),
                ("CoreTests", "Tests/CoreTests"),
            ]
        );
    }

    #[test]
    fn swift_is_registered_for_swift() {
        assert_eq!(Lang::from_extension("swift"), Some(Lang::Swift));
        assert!(registry().iter().any(|e| e.lang() == Lang::Swift));
        assert!(matches!(&extractor_for(Lang::Swift), Some(e) if e.lang() == Lang::Swift));
    }
}
