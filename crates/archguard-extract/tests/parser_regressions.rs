use archguard_extract::{Extractor, TargetSpec};
fn paths(data: archguard_extract::Extraction) -> Vec<String> {
    data.imports
        .into_iter()
        .map(|i| match i.target {
            TargetSpec::Path(path) => path,
            TargetSpec::Symbol { path, stem } => {
                stem.map_or(path.clone(), |s| format!("{path}:{s}"))
            }
            TargetSpec::Qualified(segments) => format!("q:{}", segments.join(".")),
        })
        .collect()
}
#[test]
fn javascript_literals_comments_regexes_and_member_calls_do_not_invent_edges() {
    let source = r#"const doc = `
import forged from "./fake";
require('./also-fake');
`;
const regex = /require("fake")/;
const fnRegex = () => /require('fake')/;
if (flag) /require('fake')/.test(text);
const ratio = denominator() / require('./real-divisor') / 2;
obj.require("./member"); obj.import("./member");
require("./prefix" + variable); import("./prefix" + variable);
import {
  actual
} from
"./actual";
const lazy = import(
 './lazy'
);
import './after';
"#;
    assert_eq!(
        paths(archguard_extract::TsExtractor::TS.extract("a.ts", source)),
        ["./real-divisor", "./actual", "./lazy", "./after"]
    );
}
#[test]
fn javascript_specifier_escapes_and_comment_markers_are_not_lost() {
    assert_eq!(
        paths(archguard_extract::TsExtractor::JS.extract(
            "a.js",
            r#"import './\u0061'; require('./\x62'); import 'https://example.test/a.js';"#
        )),
        ["./a", "./b", "https://example.test/a.js"]
    );
}
#[test]
fn java_text_blocks_are_not_declarations_and_multiline_imports_are_kept() {
    let source = "package p; import\n a.b.Outer.Inner;\nclass Text { String value = \"\"\"\nimport falsehood.Fake;\npackage forged;\n\"\"\"; }\n";
    let data = archguard_extract::JavaExtractor.extract("Odd.java", source);
    assert_eq!(data.declarations.len(), 1);
    assert_eq!(data.declarations[0].module_path, "p");
    assert_eq!(data.imports[0].line, 1);
    assert_eq!(paths(data), ["a/b/Outer:Inner"]);
}
#[test]
fn csharp_namespace_nesting_aliases_and_raw_strings_are_distinct() {
    let source = "global using global::A.B;\nnamespace Outer { namespace Inner {\nusing Alias = global::Types.Container;\nclass Text {string value = \"\"\"\nusing Forged.Namespace;\nnamespace Fake;\n\"\"\";}\n} }\nnamespace Other {using Good;}";
    let data = archguard_extract::CsExtractor.extract("a.cs", source);
    assert_eq!(
        data.declarations
            .iter()
            .map(|d| d.module_path.as_str())
            .collect::<Vec<_>>(),
        ["Outer", "Outer/Inner", "Other"]
    );
    assert_eq!(paths(data), ["A/B", "Types:Container", "Good"]);
}
#[test]
fn python_multiline_relative_imports_and_ordinary_quotes_are_correct() {
    let source = "text = \"''' # not a docstring\"\nfrom ...pkg import (\n Thing,\n other as renamed,\n)\nimport first; import second\nfrom .module import *\n";
    assert_eq!(
        paths(archguard_extract::PyExtractor.extract("a/b/c.py", source)),
        [
            "../../pkg/Thing",
            "../../pkg/other",
            "first",
            "second",
            "./module"
        ]
    );
}
#[test]
fn php_heredocs_html_and_group_imports_are_distinct() {
    let source = "use Fake\\Html;\n<?php\n$text = <<<'DOC'\nuse Fake\\Doc;\nDOC;\nuse App\\{\n One, Two as Alias\n};\nuse Third\\Type, Fourth\\Type;\n?>\nuse Fake\\Tail;";
    assert_eq!(
        paths(archguard_extract::PhpExtractor.extract("a.php", source)),
        ["App:One", "App:Two", "Third:Type", "Fourth:Type"]
    );
}
#[test]
fn go_module_identity_does_not_accept_a_truncated_invalid_directive() {
    assert_eq!(
        archguard_extract::go::module_path("module \"example.org/\\x61\" // comment\n"),
        Some("example.org/a".into())
    );
    for source in [
        "module example.org/a extra",
        "module \"example.org/a extra\"",
        "module invalid path\nmodule example.org/a",
    ] {
        assert_eq!(archguard_extract::go::module_path(source), None, "{source}");
    }
}
#[test]
fn literal_examples_do_not_become_exported_symbols() {
    assert!(
        !archguard_extract::ts::scan_visibility(
            "const text = `\nexport class Forged {}\n`;\nexport class Real {}\n"
        )
        .exported
        .contains("Forged")
    );
    assert!(
        !archguard_extract::java::scan_visibility(
            "class Text {String x=\"\"\"\npublic class Forged {}\n\"\"\";}\npublic class Real {}\n"
        )
        .exported
        .contains("Forged")
    );
    assert!(
        !archguard_extract::cs::scan_visibility(
            "class Text {string x=\"\"\"\npublic class Forged {}\n\"\"\";}\npublic class Real {}\n"
        )
        .exported
        .contains("Forged")
    );
    let python = archguard_extract::py::scan_visibility(
        "\"\"\"\ndef forged():\n pass\n__all__ = ['forged']\n\"\"\"\ndef real():\n pass\n",
    );
    assert!(!python.exported.contains("forged"));
    assert!(python.exported.contains("real"));
}
#[test]
fn modern_javascript_and_typescript_extensions_are_discovered() {
    for (ext, lang) in [
        ("cjs", archguard_extract::Lang::Js),
        ("mts", archguard_extract::Lang::Ts),
        ("cts", archguard_extract::Lang::Ts),
    ] {
        assert_eq!(archguard_extract::Lang::from_extension(ext), Some(lang));
    }
}

#[test]
fn rust_raw_strings_nested_comments_and_lifetimes_do_not_forge_api_symbols() {
    let source = "const DOC:&str=r###\"\nuse crate::fake;\npub struct Forged;\n\"###;\n/* outer /* nested */ pub struct AlsoFake; */\npub async fn real<'a>(x: &'a str) {}\npub const fn size() -> usize {1}\n";
    let imports = archguard_extract::RustExtractor.extract("lib.rs", source);
    assert!(imports.imports.is_empty());
    let visibility = archguard_extract::rust::scan_visibility(source);
    assert!(!visibility.exported.contains("Forged"));
    assert!(!visibility.exported.contains("AlsoFake"));
    assert!(visibility.exported.contains("real"));
    assert!(visibility.exported.contains("size"));
}

#[test]
fn php_function_groups_and_dir_relative_includes_resolve_statically() {
    assert_eq!(paths(archguard_extract::PhpExtractor.extract("src/a.php", "<?php\nuse function App\\Tools\\{one, two as alias};\nrequire __DIR__ . '/child.php';\nrequire (__DIR__ . '/../other.php');\nrequire __DIR__ . $dynamic;\n")),["App/Tools","App/Tools","./child.php","./../other.php"]);
}
