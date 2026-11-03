//! JavaScript/TypeScript import scanner using lexical shielding and multiline tokens.
//! Literal imports and re-exports are captured; computed imports and template
//! expressions remain unresolved. No project code or package installation is executed.

use crate::extract::{Extraction, Extractor, RawImport, TargetSpec};
use crate::walk::Lang;

/// TypeScript/JavaScript scanner (T09).
///
/// Every detected spec — relative (`./util`, `../api/x`), absolute-looking
/// or bare (`react`, `@scope/pkg`) — is emitted verbatim as
/// [`TargetSpec::Path`]; unknown bare specs end up external during
/// resolution (T07). [`Extraction::declarations`] is always empty: the
/// language has no module declarations archguard derives a hierarchy from.
///
/// The same scanner code serves both languages of the family; one instance
/// per [`Lang`] variant is registered (see [`crate::extract::registry`]) so
/// `.ts`/`.tsx` and `.js`/`.jsx`/`.mjs` files each find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TsExtractor {
    lang: Lang,
}

impl TsExtractor {
    /// The scanner registered for [`Lang::Ts`] (`.ts`/`.tsx` files).
    pub const TS: Self = Self { lang: Lang::Ts };

    /// The scanner registered for [`Lang::Js`] (`.js`/`.jsx`/`.mjs` files).
    pub const JS: Self = Self { lang: Lang::Js };
}

impl Extractor for TsExtractor {
    fn lang(&self) -> Lang {
        self.lang
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let tokens = crate::lex::tokens(content, crate::lex::Flavor::Js);
        let tokens: Vec<_> = tokens.iter().filter(|t| t.text != "\n").collect();
        let lines: Vec<_> = content.lines().collect();
        let mut imports = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if token.literal
                || !matches!(token.text.as_str(), "import" | "export" | "require")
                || index > 0 && tokens[index - 1].text == "."
            {
                continue;
            }
            let next = tokens.get(index + 1);
            let target = if next.is_some_and(|t| t.text == "(") && token.text != "export" {
                tokens
                    .get(index + 2)
                    .filter(|t| t.literal)
                    .filter(|_| {
                        tokens.get(index + 3).is_some_and(|t| {
                            t.text == ")" || (token.text == "import" && t.text == ",")
                        })
                    })
                    .copied()
            } else if token.text == "import" && next.is_some_and(|t| t.literal) {
                next.copied()
            } else if token.text != "require"
                && next.is_some_and(|t| {
                    token.text == "import" || matches!(t.text.as_str(), "{" | "*" | "type")
                })
            {
                let mut found = None;
                for j in index + 1..tokens.len() {
                    if !tokens[j].literal
                        && matches!(tokens[j].text.as_str(), ";" | "import" | "export" | "=")
                    {
                        break;
                    }
                    if !tokens[j].literal
                        && tokens[j].text == "from"
                        && tokens.get(j + 1).is_some_and(|t| t.literal)
                    {
                        found = Some(tokens[j + 1]);
                        break;
                    }
                }
                found
            } else {
                None
            };
            if let Some(target) = target
                && let Some(spec) = literal_spec(&target.text)
                && !spec.is_empty()
            {
                imports.push(RawImport {
                    source: rel_path.into(),
                    target: TargetSpec::Path(spec),
                    line: token.line,
                    raw: lines
                        [(token.line - 1) as usize..(target.end_line as usize).min(lines.len())]
                        .join("\n"),
                });
            }
        }
        Extraction {
            imports,
            declarations: Vec::new(),
        }
    }
}
fn literal_spec(text: &str) -> Option<String> {
    let quote = text.chars().next()?;
    if !matches!(quote, '\'' | '"') || !text.ends_with(quote) {
        return None;
    }
    let mut chars = text[1..text.len() - 1].chars();
    let mut result = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next()? {
            '\n' => {}
            '\r' => {
                if chars.clone().next() == Some('\n') {
                    chars.next();
                }
            }
            as_code @ ('x' | 'u') => {
                let count = if as_code == 'x' { 2 } else { 4 };
                let digits: String = chars.by_ref().take(count).collect();
                if digits.len() != count {
                    return None;
                }
                result.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
            }
            'n' => result.push('\n'),
            'r' => result.push('\r'),
            't' => result.push('\t'),
            other => result.push(other),
        }
    }
    Some(result)
}

/// `true` for characters that continue an identifier (JS/TS also allows
/// `$` and Unicode letters/digits).
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Exported/internal symbol split of one file (WP-02).
///
/// Rule (R12): `export` → exported; `private`/`#` → internal; a
/// default without `export` is module-internal.
pub fn scan_visibility(content: &str) -> archguard_core::Visibility {
    let clean = crate::lex::code_only(content, crate::lex::Flavor::Js);
    let content = clean.as_str();
    let mut visibility = archguard_core::Visibility::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export")
            && !is_ident_continuation(rest)
        {
            collect_exported(rest, &mut visibility);
            continue;
        }
        // Private class members and `#`-fields are internal.
        let is_member = if let Some(rest) = strip_keyword(line, "private") {
            rest
        } else if line.starts_with('#') {
            line
        } else {
            for keyword in ["function", "class", "const", "let", "var"] {
                if let Some(rest) = strip_keyword(line, keyword) {
                    if let Some(name) = first_identifier(rest) {
                        visibility.internal.insert(name);
                    }
                    break;
                }
            }
            continue;
        };
        if let Some(name) = first_identifier(is_member) {
            visibility.internal.insert(name);
        }
    }
    visibility
}

/// Names an `export …` statement puts on the module surface.
fn collect_exported(rest: &str, visibility: &mut archguard_core::Visibility) {
    let rest = rest.trim();
    if let Some(inner) = rest.strip_prefix('{') {
        let inner = inner.split('}').next().unwrap_or(inner);
        for part in inner.split(',') {
            let part = part.trim();
            let name = part.split(" as ").next().unwrap_or(part).trim();
            if !name.is_empty()
                && !name.starts_with("type ")
                && let Some(name) = first_identifier(name)
            {
                visibility.exported.insert(name);
            }
        }
        return;
    }
    if rest.starts_with('*') {
        return; // `export * from …` names no symbol here.
    }
    let rest = strip_keyword(rest, "default").unwrap_or(rest).trim();
    for keyword in [
        "const",
        "let",
        "var",
        "async function",
        "function",
        "abstract class",
        "class",
        "type",
        "interface",
        "enum",
        "declare",
        "namespace",
    ] {
        if let Some(inner) = strip_keyword(rest, keyword) {
            if let Some(name) = first_identifier(inner) {
                visibility.exported.insert(name);
            }
            return;
        }
    }
    // `export Foo;` (re-export of a binding).
    if let Some(name) = first_identifier(rest) {
        visibility.exported.insert(name);
    }
}

/// The rest of the line after `keyword`, only when it is a whole word.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    (!is_ident_continuation(rest)).then_some(rest)
}

/// True when the next character continues the keyword identifier.
fn is_ident_continuation(rest: &str) -> bool {
    rest.chars().next().is_some_and(is_ident_char)
}

/// First identifier token of `text`, `None` when there is none.
fn first_identifier(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() && !is_ident_char(chars[index]) && chars[index] != '$' {
        index += 1;
    }
    let start = index;
    while index < chars.len() && (is_ident_char(chars[index]) || chars[index] == '$') {
        index += 1;
    }
    (index > start).then(|| chars[start..index].iter().collect())
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn exported_forms_are_recognized() {
        let source = "export const NAME = 1;\nexport function helper() {}\nexport class App extends Base {}\nexport default class Main {}\nexport { alpha, beta as b } from '../util/helper';\nexport type Id = string;\nexport * from './re-export';\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("NAME"));
        assert!(visibility.exported.contains("helper"));
        assert!(visibility.exported.contains("App"));
        assert!(visibility.exported.contains("Main"));
        assert!(visibility.exported.contains("alpha"));
        assert!(
            visibility.exported.contains("beta"),
            "re-export keeps the origin name"
        );
        assert!(visibility.exported.contains("Id"));
        assert_eq!(visibility.exported.len(), 7, "{:?}", visibility.exported);
    }

    #[test]
    fn declarations_without_export_are_module_internal() {
        let source = "function hidden() {}\nconst secret = 1;\nclass Impl {}\n";
        let visibility = scan_visibility(source);
        assert!(visibility.internal.contains("hidden"));
        assert!(visibility.internal.contains("secret"));
        assert!(visibility.internal.contains("Impl"));
        assert!(visibility.exported.is_empty());
    }

    #[test]
    fn private_and_hash_members_are_internal() {
        let source = "class A {\n  private compute() {}\n  #state = 1;\n  render() {}\n}\n";
        let visibility = scan_visibility(source);
        assert!(visibility.internal.contains("compute"));
        assert!(
            visibility.internal.contains("state"),
            "`#` prefix is stripped"
        );
    }

    #[test]
    fn comments_are_not_scanned() {
        let visibility = scan_visibility("// export ghost\n/* class Ghost */\n* export fake\n");
        assert!(visibility.exported.is_empty());
        assert!(visibility.internal.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    /// 25-line fixture covering every T09 form: the three statement forms
    /// (both quote styles), `require`, plain and `await`/arrow dynamic
    /// imports, a trailing comment, line and block comments (incl. a
    /// multi-line block), a multi-line import, keyword-lookalike text and a non-import statement.
    const FIXTURE: &str = "\
import { render } from \"react\";
import './styles/global.css';
import Default, { named } from \"./mixed\";
import type { Config } from '../types/index';
import * as path from \"node:path\"; // trailing comment
export * from \"./re-export\";
export { helper } from '../util/helper';
const fs = require(\"node:fs\");
const worker = await import(\"./worker.js\");
const lazy = () => import('./lazy');
require(\"./conditional\");
// import commented from \"./gone\";
/* import blocked from \"./gone\"; */
/*
import inside from \"./gone\";
still inside the block
*/
import afterBlock from \"./after-block\";
import {
  broken,
} from \"./multiline\";
console.log(\"import\", { from: \"x\" });
export default class App extends React.Component {}
await import(\"../deep/chain\").then((m) => m.go());
import \"side-effect-pkg\";
";

    /// The expected import for fixture line `line_no` with spec `spec`
    /// (raw is taken verbatim from the fixture itself).
    fn expected(line_no: u32, spec: &str) -> RawImport {
        RawImport {
            source: "src/app/main.ts".to_string(),
            target: TargetSpec::Path(spec.to_string()),
            line: line_no,
            raw: FIXTURE
                .lines()
                .nth((line_no - 1) as usize)
                .expect("fixture line exists")
                .to_string(),
        }
    }

    fn extract(content: &str) -> Extraction {
        TsExtractor::TS.extract("src/app/main.ts", content)
    }

    fn specs(extraction: &Extraction) -> Vec<String> {
        extraction
            .imports
            .iter()
            .map(|import| match &import.target {
                TargetSpec::Path(spec) => spec.clone(),
                TargetSpec::Qualified(segments) => format!("q:{}", segments.join(".")),
                TargetSpec::Symbol { path, .. } => path.clone(),
            })
            .collect()
    }

    #[test]
    fn fixture_yields_exactly_the_expected_imports() {
        let extraction = extract(FIXTURE);
        assert_eq!(
            extraction.imports,
            vec![
                expected(1, "react"),
                expected(2, "./styles/global.css"),
                expected(3, "./mixed"),
                expected(4, "../types/index"),
                expected(5, "node:path"),
                expected(6, "./re-export"),
                expected(7, "../util/helper"),
                expected(8, "node:fs"),
                expected(9, "./worker.js"),
                expected(10, "./lazy"),
                expected(11, "./conditional"),
                expected(18, "./after-block"),
                RawImport {
                    raw: FIXTURE
                        .lines()
                        .skip(18)
                        .take(3)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    ..expected(19, "./multiline")
                },
                expected(24, "../deep/chain"),
                expected(25, "side-effect-pkg"),
            ],
            "imports: {:#?}",
            extraction.imports
        );
    }

    #[test]
    fn ts_never_reports_declarations() {
        assert!(extract(FIXTURE).declarations.is_empty());
        assert_eq!(extract(""), Extraction::default());
    }

    #[test]
    fn both_quote_styles_delimit_specs() {
        assert_eq!(
            specs(&extract(
                "import a from \"double\";\nimport b from 'single';\n"
            )),
            ["double", "single"]
        );
    }

    #[test]
    fn relative_and_bare_specs_are_emitted_verbatim_as_paths() {
        let extraction = extract(
            "import a from \"./rel\";\nimport b from \"../up\";\nimport c from \"pkg/sub\";\n",
        );
        assert_eq!(specs(&extraction), ["./rel", "../up", "pkg/sub"]);
        for import in &extraction.imports {
            assert!(matches!(&import.target, TargetSpec::Path(_)));
        }
    }

    #[test]
    fn import_without_from_is_a_side_effect_import() {
        assert_eq!(specs(&extract("import \"./polyfill\";\n")), ["./polyfill"]);
    }

    #[test]
    fn require_calls_are_detected() {
        let content = "const fs = require(\"a\");\nrequire(\"b\");\nimport c = require(\"d\");\n";
        assert_eq!(specs(&extract(content)), ["a", "b", "d"]);
    }

    #[test]
    fn dynamic_import_calls_are_detected() {
        let content =
            "await import(\"a\");\nconst m = () => import('b');\nimport(\"c\").then((x) => x);\n";
        assert_eq!(specs(&extract(content)), ["a", "b", "c"]);
    }

    #[test]
    fn export_from_forms_are_detected() {
        let content = "export * from \"a\";\nexport { x } from \"b\";\nexport * as ns from 'c';\n\
                       export const y = 1;\n";
        assert_eq!(specs(&extract(content)), ["a", "b", "c"]);
    }

    #[test]
    fn comment_forms_are_skipped() {
        let content = "\
import a from \"./before\";
// import b from \"./line\";
/* import c from \"./same_line\"; */
/*
import d from \"./inside\";
still inside
*/
import e from \"./after\";
/* unterminated block starts here
import f from \"./swallowed\";
";
        assert_eq!(specs(&extract(content)), ["./before", "./after"]);
    }

    #[test]
    fn multiline_import_is_assembled() {
        // The original source location spans the complete import.
        let content = "import {\n  a,\n  b,\n} from \"./multi\";\nimport { c } from \"./ok\";\n";
        assert_eq!(specs(&extract(content)), ["./multi", "./ok"]);
    }

    #[test]
    fn only_plain_quoted_literals_count_as_specs() {
        let content = "import x from `template`;\nimport(variable);\nrequire noParens(\"x\");\n";
        assert!(extract(content).imports.is_empty());
    }

    #[test]
    fn unterminated_string_literal_is_ignored() {
        assert!(extract("import x from \"oops\n").imports.is_empty());
    }

    #[test]
    fn comment_markers_inside_module_literals_are_preserved() {
        assert_eq!(
            specs(&extract("import x from \"https://example.com/m.js\";\n")),
            ["https://example.com/m.js"]
        );
    }

    #[test]
    fn multiple_specs_on_one_line_keep_their_line_order() {
        let content = "import \"./first\"; const lazy = require(\"./second\");\n";
        assert_eq!(specs(&extract(content)), ["./first", "./second"]);
    }

    #[test]
    fn line_numbers_are_one_based_and_raw_is_verbatim() {
        let extraction = extract("let x = 1;\nimport c from \"./c\"; // trailing \"quoted\"\n");
        assert_eq!(extraction.imports.len(), 1);
        assert_eq!(extraction.imports[0].line, 2);
        assert_eq!(
            extraction.imports[0].raw,
            "import c from \"./c\"; // trailing \"quoted\""
        );
    }

    #[test]
    fn extractor_reports_both_family_languages() {
        assert_eq!(TsExtractor::TS.lang(), Lang::Ts);
        assert_eq!(TsExtractor::JS.lang(), Lang::Js);
    }

    #[test]
    fn ts_and_js_extractors_are_registered() {
        assert!(registry().iter().any(|e| e.lang() == Lang::Ts));
        assert!(registry().iter().any(|e| e.lang() == Lang::Js));
        assert!(matches!(&extractor_for(Lang::Ts), Some(e) if e.lang() == Lang::Ts));
        assert!(matches!(&extractor_for(Lang::Js), Some(e) if e.lang() == Lang::Js));
    }
}
