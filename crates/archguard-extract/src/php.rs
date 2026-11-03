//! PHP import scanner with lexical shielding and multiline statement assembly.
//! Comments and string bodies cannot create imports. This is a bounded static
//! scanner; dynamic loading, preprocessing and runtime resolution remain unsupported.

use crate::extract::{Declaration, Extraction, Extractor, RawImport, TargetSpec};
use crate::walk::Lang;
use serde_json::Value;

/// PHP scanner (T43). See the [module documentation](self) for the emitted
/// declaration and target shapes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhpExtractor;

impl Extractor for PhpExtractor {
    fn lang(&self) -> Lang {
        Lang::Php
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut extraction = Extraction::default();
        for statement in crate::lex::statements(content, crate::lex::Flavor::Php) {
            let code = statement.code.trim();
            if let Some(declaration) = namespace_declaration(rel_path, code) {
                extraction.declarations.push(declaration);
                continue;
            }
            if let Some(targets) = use_targets(code) {
                extraction
                    .imports
                    .extend(targets.into_iter().map(|target| RawImport {
                        source: rel_path.into(),
                        target,
                        line: statement.line,
                        raw: statement.raw.clone(),
                    }));
            } else {
                extraction.imports.extend(include_import(
                    rel_path,
                    &statement.raw,
                    statement.line,
                    code,
                ));
            }
        }
        extraction
    }
}

/// The [`Declaration`] of a comment-stripped `namespace A\B;` or
/// `namespace A\B {` line, `None` for every other line — the namespace
/// name becomes the module path with backslashes → slashes.
fn namespace_declaration(rel_path: &str, code: &str) -> Option<Declaration> {
    let body = strip_word(code, "namespace")?;
    let body = body
        .strip_suffix(';')
        .or_else(|| body.strip_suffix('{'))?
        .trim();
    let segments = ns_segments(body)?;
    Some(Declaration {
        module_path: segments.join("/"),
        file: rel_path.to_string(),
    })
}

/// The [`TargetSpec`]s of a comment-stripped `use …;` statement: one for
/// the single form, one per element for the group form, `None` for every
/// other line and for malformed or empty-path specs.
fn use_targets(code: &str) -> Option<Vec<TargetSpec>> {
    let body = strip_word(code, "use")?;
    let body = body.trim_end().strip_suffix(';')?.trim();
    if body.is_empty() {
        return None;
    }
    if body.ends_with('}') {
        let (prefix, elements) = body.strip_suffix('}')?.split_once('{')?;
        let (kind, prefix) = if let Some(rest) = strip_word(prefix, "function") {
            ("function ", rest)
        } else if let Some(rest) = strip_word(prefix, "const") {
            ("const ", rest)
        } else {
            ("", prefix)
        };
        let prefix = group_prefix_segments(prefix)?;
        elements
            .split(',')
            .map(|element| group_element_target(&prefix, &format!("{kind}{}", element.trim())))
            .collect()
    } else {
        let prefix = if body.starts_with("function ") {
            "function "
        } else if body.starts_with("const ") {
            "const "
        } else {
            ""
        };
        body.split(',')
            .enumerate()
            .map(|(index, item)| {
                let item = item.trim();
                if index == 0 || prefix.is_empty() {
                    single_use_target(item)
                } else {
                    single_use_target(&format!("{prefix}{item}"))
                }
            })
            .collect()
    }
}

/// The segments of a group prefix such as `A\B\` — the prefix must either
/// be empty (root-namespace group) or end with a single `\`; the trailing
/// backslash is not part of the path.
fn group_prefix_segments(prefix: &str) -> Option<Vec<&str>> {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return Some(Vec::new());
    }
    let prefix = prefix.strip_suffix('\\')?;
    if prefix.is_empty() {
        return Some(Vec::new());
    }
    use_segments(prefix)
}

/// The target of one group element: `C`, `C as X`, `Sub\C` or with a
/// `function`/`const` modifier. The element's last segment stems (plain
/// form) or is dropped (function/const form); the group prefix is prepended
/// to the remaining segments. A `function`/`const` element names only a
/// module (`stem: None`), a plain element names a class.
fn group_element_target(prefix: &[&str], element: &str) -> Option<TargetSpec> {
    if element.is_empty() {
        return None;
    }
    let (module_only, element) =
        match strip_word(element, "function").or_else(|| strip_word(element, "const")) {
            Some(spec) => (true, spec),
            None => (false, element),
        };
    let spec = strip_alias(element)?;
    let segments = use_segments(spec)?;
    let (stem, init) = segments.split_last()?;
    let mut path: Vec<&str> = prefix.to_vec();
    path.extend_from_slice(init);
    symbol(&path, (!module_only).then_some(stem))
}

/// The target of a single (non-group) `use …;` spec:
/// `use function`/`use const` drop the trailing name and keep the prefix
/// as module path without stem; the plain form stems the last segment.
fn single_use_target(body: &str) -> Option<TargetSpec> {
    if let Some(spec) = strip_word(body, "function").or_else(|| strip_word(body, "const")) {
        let spec = strip_alias(spec)?;
        let segments = use_segments(spec)?;
        let (_, path) = segments.split_last()?;
        return symbol(path, None);
    }
    let spec = strip_alias(body)?;
    let segments = use_segments(spec)?;
    let (stem, path) = segments.split_last()?;
    symbol(path, Some(stem))
}

/// The [`RawImport`] of a comment-stripped `require`/`require_once`/
/// `include`/`include_once` line with a quoted relative path literal,
/// `None` for every other line and for non-static or absolute targets.
fn include_import(rel_path: &str, raw: &str, line: u32, code: &str) -> Option<RawImport> {
    const KEYWORDS: [&str; 4] = ["require_once", "require", "include_once", "include"];
    let literal = KEYWORDS
        .into_iter()
        .find_map(|keyword| strip_word(code, keyword))?;
    let expression = literal.trim().trim_end_matches(';').trim();
    let expression = expression
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(expression)
        .trim();
    let spec = if let Some(rest) = strip_word(expression, "__DIR__").or_else(|| {
        expression
            .strip_prefix("__DIR__.")
            .map(|_| &expression[7..])
    }) {
        let suffix = quoted_relative_path(rest.trim().strip_prefix('.')?.trim(), true)?;
        if !suffix.starts_with('/') {
            return None;
        }
        format!(".{suffix}")
    } else {
        quoted_relative_path(literal, false)?
    };
    Some(RawImport {
        source: rel_path.to_string(),
        target: TargetSpec::Path(spec),
        line,
        raw: raw.to_string(),
    })
}

/// The path of an optional-parenthesized, single- or double-quoted string
/// literal that opens `rest`, `None` unless the literal closes, the
/// remainder is only an optional `)` and optional `;`, and the path is
/// relative: non-empty, not starting with `/` (after backslash → slash)
/// and free of `$` (variable interpolation). Escape sequences are not
/// unescaped beyond keeping the quote scan correct.
fn quoted_relative_path(rest: &str, allow_absolute: bool) -> Option<String> {
    let rest = rest.trim().strip_prefix('(').unwrap_or(rest).trim_start();
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let chars: Vec<char> = rest.chars().collect();
    let mut index = 1;
    let end = loop {
        match chars.get(index).copied()? {
            '\\' => index += 2,
            c if c == quote => break index,
            _ => index += 1,
        }
    };
    let literal: String = chars[1..end].iter().copied().collect();
    let tail: String = chars[end + 1..].iter().copied().collect();
    let tail = tail.trim();
    let tail = tail.strip_prefix(')').unwrap_or(tail).trim();
    if !(tail.is_empty() || tail == ";") {
        return None;
    }
    let path = literal.replace('\\', "/");
    if path.is_empty() || (!allow_absolute && path.starts_with('/')) || path.contains('$') {
        return None;
    }
    Some(path)
}

/// A symbolic target from module `segments` and optional stem, `None`
/// when the path would be empty.
fn symbol(segments: &[&str], stem: Option<&str>) -> Option<TargetSpec> {
    if segments.is_empty() {
        return None;
    }
    Some(TargetSpec::Symbol {
        path: segments.join("/"),
        stem: stem.map(str::to_string),
    })
}

/// The remainder of `code` after the whole word `word` (followed by
/// whitespace) and its trailing whitespace, `None` when `code` does not
/// open with that word plus a non-empty remainder.
fn strip_word<'a>(code: &'a str, word: &str) -> Option<&'a str> {
    let rest = code.strip_prefix(word)?;
    let trimmed = rest.trim_start();
    (!trimmed.is_empty() && trimmed.len() != rest.len()).then_some(trimmed)
}

/// The name part of a spec with an optional ` as Alias` suffix, `None` for
/// a malformed alias. The alias itself carries no module information.
fn strip_alias(spec: &str) -> Option<&str> {
    match spec.split_once(" as ") {
        Some((name, alias)) if is_identifier(alias.trim()) => Some(name.trim()),
        Some(_) => None,
        None => Some(spec),
    }
}

/// The segments of a `use` spec path `A\B\C`, tolerating a single leading
/// `\` (fully-qualified form), `None` when the string is not a sequence of
/// non-empty identifier segments.
fn use_segments(spec: &str) -> Option<Vec<&str>> {
    if spec.is_empty() {
        return None;
    }
    let spec = spec.strip_prefix('\\').unwrap_or(spec);
    if spec.is_empty() {
        return None;
    }
    spec.split('\\')
        .map(|segment| is_identifier(segment).then_some(segment))
        .collect()
}

/// The segments of a `namespace` declaration path `A\B` — like
/// [`use_segments`] but strict: a leading `\` is not valid here.
fn ns_segments(spec: &str) -> Option<Vec<&str>> {
    if spec.is_empty() {
        return None;
    }
    spec.split('\\')
        .map(|segment| is_identifier(segment).then_some(segment))
        .collect()
}

/// `true` for a plain PHP identifier (digits up front, namespaces
/// separators and stray characters never match).
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// The PSR-4 mappings of a `composer.json` document, as pairs of
/// namespace prefix in path form (backslashes → `/`, trailing separator
/// dropped, e.g. `"App"` for `"App\\"`) and source directory (e.g.
/// `"src"`).
///
/// Reads `autoload.psr-4` first and `autoload-dev.psr-4` second; within
/// each section the namespace prefixes come in ascending order (the
/// order `serde_json`'s object map iterates in), and a prefix with an
/// array of directories contributes one entry per array element, in array
/// order. Directories are trimmed and stripped of a leading `./` and a
/// trailing `/`; an empty prefix (root namespace) is kept as `""`.
///
/// The function is pure and infallible: a malformed `composer.json`, a
/// non-object document or a malformed `psr-4` section (not an object,
/// non-string directory values, empty directories) contribute no entries
/// — missing sections are simply skipped, never an error.
pub fn parse_psr4(composer_json: &str) -> Vec<(String, String)> {
    let Ok(root) = serde_json::from_str::<Value>(composer_json) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for section in ["autoload", "autoload-dev"] {
        let Some(map) = root
            .get(section)
            .and_then(|section| section.get("psr-4"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (prefix, dirs) in map {
            let prefix = normalize_prefix(prefix);
            match dirs {
                Value::String(dir) => push_dir(&mut entries, &prefix, dir),
                Value::Array(dirs) => {
                    for dir in dirs {
                        if let Value::String(dir) = dir {
                            push_dir(&mut entries, &prefix, dir);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    entries
}

/// A PSR-4 namespace prefix in path form: trimmed, trailing backslashes
/// dropped, remaining backslashes → `/`.
fn normalize_prefix(prefix: &str) -> String {
    prefix.trim().trim_end_matches('\\').replace('\\', "/")
}

/// One normalized `(prefix, dir)` entry, unless the directory is empty
/// after trimming, backslash → slash normalization, a leading `./` and a
/// trailing `/`.
fn push_dir(entries: &mut Vec<(String, String)>, prefix: &str, dir: &str) {
    let dir = dir.trim().replace('\\', "/");
    let dir = dir.trim_start_matches("./").trim_end_matches('/');
    if !dir.is_empty() {
        entries.push((prefix.to_string(), dir.to_string()));
    }
}

/// Exported/internal symbol split of one file (WP-02).
///
/// Rule (R16): `public` classes/methods → exported, `_`-prefixed
/// members are internal.
pub fn scan_visibility(content: &str) -> archguard_core::Visibility {
    let clean = crate::lex::code_only(content, crate::lex::Flavor::Php);
    let content = clean.as_str();
    let mut visibility = archguard_core::Visibility::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with('#')
        {
            continue;
        }
        // Explicit member visibility first: `private`/`protected` are
        // internal, `public` exported (PHP members default to public).
        if let Some(rest) = strip_word(line, "private")
            .or_else(|| strip_word(line, "protected"))
            .or_else(|| strip_word(line, "var"))
        {
            let body = strip_visibility_noise(rest);
            if let Some(name) = member_or_fn_name(body) {
                let name = name.trim_start_matches('_').to_string();
                if !name.is_empty() {
                    visibility.internal.insert(name);
                }
            }
            continue;
        }
        let body = strip_word(line, "public").unwrap_or(line);
        let body = strip_visibility_noise(body);
        // Classes/interfaces/traits/enums are public unless marked otherwise.
        for keyword in ["class", "interface", "trait", "enum"] {
            if let Some(rest) = strip_word(body, keyword) {
                if let Some(name) = first_identifier(rest) {
                    visibility.exported.insert(name);
                }
                break;
            }
        }
        // `function name(…)` without a private modifier is public; a
        // leading `_` in the name marks it internal by convention.
        if let Some(rest) = strip_word(body, "function")
            && let Some(name) = first_identifier(rest)
        {
            if name.starts_with('_') {
                visibility
                    .internal
                    .insert(name.trim_start_matches('_').to_string());
            } else {
                visibility.exported.insert(name);
            }
        }
    }
    visibility
}

/// Drop remaining leading modifiers that carry no visibility meaning.
fn strip_visibility_noise(mut body: &str) -> &str {
    for modifier in ["final", "abstract", "static", "readonly"] {
        while let Some(rest) = strip_word(body, modifier) {
            body = rest;
        }
    }
    body
}

/// The name of a method (`function x`) or a property (`$x`) declaration.
fn member_or_fn_name(body: &str) -> Option<String> {
    if let Some(rest) = strip_word(body, "function") {
        return first_identifier(rest);
    }
    // Properties: `private $foo` / `private $bar = 1;`.
    let rest = body.trim_start();
    rest.strip_prefix('$').and_then(first_identifier)
}

/// First identifier token of `text` (ignoring a leading `$`), `None`
/// when there is none.
fn first_identifier(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len()
        && !(chars[index].is_alphanumeric() || chars[index] == '_' || chars[index] == '$')
    {
        index += 1;
    }
    let start = index;
    while index < chars.len()
        && (chars[index].is_alphanumeric() || chars[index] == '_' || chars[index] == '$')
    {
        index += 1;
    }
    (index > start).then(|| chars[start..index].iter().filter(|c| **c != '$').collect())
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn classes_are_exported_and_methods_follow_their_modifier() {
        let source = "<?php\nfinal class Repo {}\ninterface Cache {}\ntrait Shared {}\nclass Svc {\n  public function run() {}\n  private function secret() {}\n  protected function helper() {}\n  function implicit() {}\n}\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("Repo"));
        assert!(visibility.exported.contains("Cache"));
        assert!(visibility.exported.contains("Shared"));
        assert!(visibility.exported.contains("run"));
        assert!(
            visibility.exported.contains("implicit"),
            "members default to public"
        );
        assert!(visibility.internal.contains("secret"));
        assert!(visibility.internal.contains("helper"));
    }

    #[test]
    fn underscore_prefix_marks_internal() {
        let visibility =
            scan_visibility("class C {\n  public function _hidden() {}\n  private $_x = 1;\n}\n");
        assert!(visibility.internal.contains("hidden"));
        assert!(visibility.internal.contains("x"));
    }

    #[test]
    fn comments_are_skipped() {
        let visibility =
            scan_visibility("// class Ghost\n# class G2\n/* class G3 */\n* class G4\nclass Real\n");
        assert_eq!(
            visibility
                .exported
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["Real"]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    /// Fixture covering every T43 case: HTML and `<?php`/`declare` noise, a
    /// `#` comment containing `/*`, line and block comments (incl. a
    /// multi-line block and an unterminated one), the namespace
    /// declaration, plain/group/aliased/function/const/fully-qualified
    /// uses, trait `use`s inside a class (single-segment → dropped),
    /// require/include forms with quoted relative literals and the
    /// skipped absolute/variable/interpolated forms.
    const FIXTURE: &str = "\
<?php
# a # comment with /* that does not open a block
use App\\HashGuard;
declare(strict_types=1);
// require 'commented.slash';
namespace App\\Domain\\Models;

use App\\Shared\\Utils;
use App\\Domain\\Order;
use App\\Domain\\{Customer, Invoice as Inv};
use App\\Util\\Calc as C;
use function App\\Domain\\helper;
use const App\\Domain\\MAX;
use App\\Deep\\{Sub\\Thing};
use \\Global\\Prefixed\\Form;

require 'src/bootstrap.php';
require_once \"./vendor/autoload.php\";
include 'src/Foo.php';
include_once ('templates/header.php');
require $generated;
require \"/etc/passwd\";
require \"dir/$name.php\";

/* use App\\Inside\\Block;
   require 'inside.php'; */
use App\\After\\Block;

#[Route('/x')]
class Widget {
    use SomeTrait;
    use App\\Injected\\Service;
}
/* unterminated block swallows the tail
require 'swallowed.php';
";

    /// The expected symbolic import for fixture line `line_no` with module
    /// `path` and class `stem` (raw is taken verbatim from the fixture
    /// itself).
    fn symbol_import(line_no: u32, path: &str, stem: Option<&str>) -> RawImport {
        RawImport {
            source: "src/App/Domain/Widget.php".to_string(),
            target: TargetSpec::Symbol {
                path: path.to_string(),
                stem: stem.map(str::to_string),
            },
            line: line_no,
            raw: fixture_line(line_no),
        }
    }

    /// The expected path import for fixture line `line_no` with path
    /// literal `spec`.
    fn path_import(line_no: u32, spec: &str) -> RawImport {
        RawImport {
            source: "src/App/Domain/Widget.php".to_string(),
            target: TargetSpec::Path(spec.to_string()),
            line: line_no,
            raw: fixture_line(line_no),
        }
    }

    fn fixture_line(line_no: u32) -> String {
        FIXTURE
            .lines()
            .nth((line_no - 1) as usize)
            .expect("fixture line exists")
            .to_string()
    }

    fn extract(content: &str) -> Extraction {
        PhpExtractor.extract("src/App/Domain/Widget.php", content)
    }

    fn targets(extraction: &Extraction) -> Vec<String> {
        extraction
            .imports
            .iter()
            .map(|import| match &import.target {
                TargetSpec::Symbol { path, stem } => match stem {
                    Some(stem) => format!("{path}:{stem}"),
                    None => path.clone(),
                },
                TargetSpec::Path(spec) => spec.clone(),
            })
            .collect()
    }

    #[test]
    fn fixture_yields_exactly_the_expected_imports() {
        let extraction = extract(FIXTURE);
        assert_eq!(
            extraction.imports,
            vec![
                symbol_import(3, "App", Some("HashGuard")),
                symbol_import(8, "App/Shared", Some("Utils")),
                symbol_import(9, "App/Domain", Some("Order")),
                symbol_import(10, "App/Domain", Some("Customer")),
                symbol_import(10, "App/Domain", Some("Invoice")),
                symbol_import(11, "App/Util", Some("Calc")),
                symbol_import(12, "App/Domain", None),
                symbol_import(13, "App/Domain", None),
                symbol_import(14, "App/Deep/Sub", Some("Thing")),
                symbol_import(15, "Global/Prefixed", Some("Form")),
                path_import(17, "src/bootstrap.php"),
                path_import(18, "./vendor/autoload.php"),
                path_import(19, "src/Foo.php"),
                path_import(20, "templates/header.php"),
                symbol_import(27, "App/After", Some("Block")),
                symbol_import(32, "App/Injected", Some("Service")),
            ],
            "imports: {:#?}",
            extraction.imports
        );
    }

    #[test]
    fn fixture_yields_exactly_the_expected_declarations() {
        let extraction = extract(FIXTURE);
        assert_eq!(
            extraction.declarations,
            vec![Declaration {
                module_path: "App/Domain/Models".to_string(),
                file: "src/App/Domain/Widget.php".to_string(),
            }],
            "declarations: {:#?}",
            extraction.declarations
        );
    }

    /// One T43 `use`-table row: source line and its expected
    /// `(path, stem)` targets (or `None` when no import may be emitted).
    type UseCase = (
        &'static str,
        Option<&'static [(&'static str, Option<&'static str>)]>,
    );

    /// The T43 `use` table: each source line and its expected outcome.
    const USE_TABLE: &[UseCase] = &[
        ("use A\\B\\C;", Some(&[("A/B", Some("C"))])),
        ("use A\\B;", Some(&[("A", Some("B"))])),
        ("use A\\B\\C\\D;", Some(&[("A/B/C", Some("D"))])),
        ("use \\A\\B\\C;", Some(&[("A/B", Some("C"))])),
        ("use A\\B\\C ;", Some(&[("A/B", Some("C"))])),
        ("use A\\B\\C as D;", Some(&[("A/B", Some("C"))])),
        ("use A\\B\\C as 9D;", None),
        ("use A\\B\\C as D as E;", None),
        ("use A;", None),
        ("use A\\B\\C", None),
        ("use ;", None),
        ("use;", None),
        ("use A\\\\B;", None),
        ("use A\\9B;", None),
        ("use A\\B. C;", None),
        ("use A\\B\\C, D;", None),
        (
            "use A\\B\\{C, D};",
            Some(&[("A/B", Some("C")), ("A/B", Some("D"))]),
        ),
        ("use A\\B\\{C};", Some(&[("A/B", Some("C"))])),
        (
            "use \\A\\B\\{C, D};",
            Some(&[("A/B", Some("C")), ("A/B", Some("D"))]),
        ),
        (
            "use A\\B\\{C as X, D};",
            Some(&[("A/B", Some("C")), ("A/B", Some("D"))]),
        ),
        ("use A\\B\\{Sub\\C};", Some(&[("A/B/Sub", Some("C"))])),
        (
            "use A\\B\\{function f, const K};",
            Some(&[("A/B", None), ("A/B", None)]),
        ),
        ("use A\\B\\{Sub\\f as x};", Some(&[("A/B/Sub", Some("f"))])),
        ("use A\\B\\{};", None),
        ("use A\\B\\{, C};", None),
        ("use A\\B\\{C,, D};", None),
        ("use {C, D};", None),
        ("use A\\B{C};", None),
        ("use A\\B\\{C} D;", None),
        ("use function A\\B\\f;", Some(&[("A/B", None)])),
        ("use const A\\B\\K;", Some(&[("A/B", None)])),
        ("use function A\\B\\C\\f;", Some(&[("A/B/C", None)])),
        ("use const A\\B;", Some(&[("A", None)])),
        ("use function A\\B\\f as g;", Some(&[("A/B", None)])),
        ("use function f;", None),
        ("use function;", None),
        ("use const;", None),
        ("use functionx A\\B;", None),
        ("usea A\\B\\C;", None),
        ("usex A\\B\\C;", None),
        ("useA A\\B\\C;", None),
        ("use  A\\B\\C ;", Some(&[("A/B", Some("C"))])),
    ];

    #[test]
    fn use_table_yields_exactly_the_expected_targets() {
        for &(source, expected) in USE_TABLE {
            let extraction = extract(&format!("{source}\n"));
            let actual = targets(&extraction);
            let expected: Vec<String> = expected
                .unwrap_or_default()
                .iter()
                .map(|(path, stem)| match stem {
                    Some(stem) => format!("{path}:{stem}"),
                    None => path.to_string(),
                })
                .collect();
            assert_eq!(actual, expected, "source {source:?}");
            for import in &extraction.imports {
                assert!(matches!(&import.target, TargetSpec::Symbol { .. }));
                assert_eq!(import.source, "src/App/Domain/Widget.php");
            }
        }
    }

    /// `namespace`-declaration table: source line → expected
    /// `module_path` (`None` = no declaration).
    const NAMESPACE_TABLE: &[(&str, Option<&str>)] = &[
        ("namespace A\\B;", Some("A/B")),
        ("namespace A;", Some("A")),
        ("namespace A\\B\\C;", Some("A/B/C")),
        ("namespace A\\B {", Some("A/B")),
        ("namespace A\\B{", Some("A/B")),
        ("namespace  A\\B  ;  ", Some("A/B")),
        ("namespace global;", Some("global")),
        ("namespace A\\B", None),
        ("namespace A\\B }", None),
        ("namespace {", None),
        ("namespace;", None),
        ("namespace ;", None),
        ("namespace A\\\\B;", None),
        ("namespace A\\9B;", None),
        ("namespace \\A\\B;", None),
        ("namespace A\\B\\;", None),
        ("namespace", None),
        ("namespace_x A\\B;", None),
        ("namespacea A\\B;", None),
    ];

    #[test]
    fn namespace_table_yields_exactly_the_expected_declarations() {
        for &(source, expected) in NAMESPACE_TABLE {
            let extraction = extract(&format!("{source}\n"));
            let actual: Vec<&str> = extraction
                .declarations
                .iter()
                .map(|declaration| declaration.module_path.as_str())
                .collect();
            let expected: Vec<&str> = expected.into_iter().collect();
            assert_eq!(actual, expected, "source {source:?}");
            for declaration in &extraction.declarations {
                assert_eq!(declaration.file, "src/App/Domain/Widget.php");
            }
        }
    }

    /// `require`/`include` table: source line → expected path spec
    /// (`None` = no import).
    const INCLUDE_TABLE: &[(&str, Option<&str>)] = &[
        ("require 'src/Foo.php';", Some("src/Foo.php")),
        ("require_once 'src/Foo.php';", Some("src/Foo.php")),
        ("include 'a.php';", Some("a.php")),
        ("include_once 'a.php';", Some("a.php")),
        ("require \"src/Foo.php\";", Some("src/Foo.php")),
        ("require ('x.php');", Some("x.php")),
        ("require_once ('x.php') ;", Some("x.php")),
        ("require '../inc/x.php';", Some("../inc/x.php")),
        ("require './x.php';", Some("./x.php")),
        ("require 'src\\Foo.php';", Some("src/Foo.php")),
        ("require 'a b.php';", Some("a b.php")),
        ("require 'x.php'", Some("x.php")),
        ("require 'x.php'; // trailing", Some("x.php")),
        ("require '/abs/x.php';", None),
        ("require \"dir/$name.php\";", None),
        ("require $file;", None),
        ("require x.php;", None),
        ("require '';", None),
        ("require;", None),
        ("require", None),
        ("require_once'x.php';", Some("x.php")),
        ("requirex 'x.php';", None),
        ("requires 'x.php';", None),
        ("require_me('x.php');", None),
        ("require 'x.php' or die();", None),
        ("include \"unclosed.php;", None),
    ];

    #[test]
    fn include_table_yields_exactly_the_expected_targets() {
        for &(source, expected) in INCLUDE_TABLE {
            let extraction = extract(&format!("{source}\n"));
            let actual = targets(&extraction);
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert_eq!(actual, expected, "source {source:?}");
            for import in &extraction.imports {
                assert!(matches!(&import.target, TargetSpec::Path(_)));
                assert_eq!(import.source, "src/App/Domain/Widget.php");
            }
        }
    }

    #[test]
    fn group_use_emits_one_import_per_element() {
        let extraction = extract("use App\\Domain\\{Customer, Invoice};\n");
        assert_eq!(
            extraction.imports.len(),
            2,
            "imports: {:#?}",
            extraction.imports
        );
        assert_eq!(
            extraction.imports[0].target,
            TargetSpec::Symbol {
                path: "App/Domain".to_string(),
                stem: Some("Customer".to_string()),
            }
        );
        assert_eq!(
            extraction.imports[1].target,
            TargetSpec::Symbol {
                path: "App/Domain".to_string(),
                stem: Some("Invoice".to_string()),
            }
        );
        assert_eq!(extraction.imports[0].line, 1);
        assert_eq!(extraction.imports[1].line, 1);
    }

    #[test]
    fn function_and_const_use_drop_their_trailing_name() {
        assert_eq!(
            extract("use function A\\B\\f;\n").imports[0].target,
            TargetSpec::Symbol {
                path: "A/B".to_string(),
                stem: None,
            }
        );
        assert_eq!(
            extract("use const A\\B\\K;\n").imports[0].target,
            TargetSpec::Symbol {
                path: "A/B".to_string(),
                stem: None,
            }
        );
    }

    #[test]
    fn hash_comment_with_open_marker_does_not_open_a_block() {
        let content = "\
# a # comment containing /* stays a line comment
use A\\B;
";
        let extraction = extract(content);
        assert_eq!(targets(&extraction), ["A:B"]);
    }

    #[test]
    fn line_comments_and_block_comment_state_are_skipped() {
        let content = "\
namespace a\\b;
// use a\\LineComment;
# use a\\HashComment;
/* use a\\SameLine; */ use a\\AfterBlock;
/*
namespace swallowed\\pkg;
use a\\InsideBlock;
*/
use a\\AfterMultiLineBlock;
use a\\BeforeOpen;
/* unterminated block starts here
use a\\Swallowed;
";
        let extraction = extract(content);
        assert_eq!(
            targets(&extraction),
            ["a:AfterBlock", "a:AfterMultiLineBlock", "a:BeforeOpen"]
        );
    }

    #[test]
    fn keywords_in_code_are_not_statements() {
        let content = "\
String $s = \"namespace A\\\\B; use C\\\\D;\";
$requires = include_thing();
";
        let extraction = extract(content);
        assert!(extraction.imports.is_empty());
        assert!(extraction.declarations.is_empty());
    }

    #[test]
    fn file_without_a_namespace_gets_no_declaration() {
        let extraction = extract("use A\\B\\C;\n\nclass Widget {}\n");
        assert!(extraction.declarations.is_empty());
        assert_eq!(targets(&extraction), ["A/B:C"]);
    }

    #[test]
    fn empty_and_comment_only_files_yield_nothing() {
        assert_eq!(extract(""), Extraction::default());
        assert_eq!(
            extract("// nothing\n# neither\n/* here */\n\n"),
            Extraction::default()
        );
    }

    #[test]
    fn line_numbers_are_one_based_and_raw_is_verbatim() {
        let extraction = extract("namespace A\\B;\n\nuse A\\B\\C; // trailing\n");
        assert_eq!(extraction.imports[0].line, 3);
        assert_eq!(extraction.imports[0].raw, "use A\\B\\C; // trailing");
        assert_eq!(extraction.declarations[0].file, "src/App/Domain/Widget.php");
    }

    #[test]
    fn extractor_reports_the_php_language() {
        assert_eq!(PhpExtractor.lang(), Lang::Php);
    }

    #[test]
    fn php_extractor_is_registered() {
        assert!(registry().iter().any(|e| e.lang() == Lang::Php));
        assert!(matches!(&extractor_for(Lang::Php), Some(e) if e.lang() == Lang::Php));
    }

    /// One PSR-4 table row: composer.json content and the expected
    /// `(prefix, dir)` pairs.
    const PSR4_TABLE: &[(&str, &[(&str, &str)])] = &[
        (
            r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
            &[("App", "src")],
        ),
        (
            r#"{"autoload":{"psr-4":{"App\\":"src"}}}"#,
            &[("App", "src")],
        ),
        (
            r#"{"autoload":{"psr-4":{"App\\Sub\\":"src/Sub/"}}}"#,
            &[("App/Sub", "src/Sub")],
        ),
        (
            r#"{"autoload":{"psr-4":{"Zeta\\":"src","App\\":"lib"}}}"#,
            &[("App", "lib"), ("Zeta", "src")],
        ),
        (
            r#"{"autoload":{"psr-4":{"App\\":["src/","lib"]}}}"#,
            &[("App", "src"), ("App", "lib")],
        ),
        (
            r#"{"autoload":{"psr-4":{"App\\":["src",5]}}}"#,
            &[("App", "src")],
        ),
        (
            r#"{"autoload":{"psr-4":{"App\\":"./src"}}}"#,
            &[("App", "src")],
        ),
        (r#"{"autoload":{"psr-4":{"":"app"}}}"#, &[("", "app")]),
        (
            r#"{"autoload-dev":{"psr-4":{"Tests\\":"tests"}}}"#,
            &[("Tests", "tests")],
        ),
        (
            r#"{"autoload":{"psr-4":{"B\\":"b"}},"autoload-dev":{"psr-4":{"A\\":"a"}}}"#,
            &[("B", "b"), ("A", "a")],
        ),
        (r#"{"autoload":{"psr-4":{"App\\":123}}}"#, &[]),
        (r#"{"autoload":{"psr-4":{"App\\":""}}}"#, &[]),
        (r#"{"autoload":{"psr-4":{"App\\":{}}}}"#, &[]),
        (r#"{"autoload":{"psr-4":"nope"}}"#, &[]),
        (r#"{"autoload":"nope"}"#, &[]),
        (r#"{"autoload":{},"name":"x"}"#, &[]),
        (r#"{}"#, &[]),
        ("", &[]),
        ("not json", &[]),
        ("[]", &[]),
        ("null", &[]),
    ];

    #[test]
    fn psr4_table_yields_exactly_the_expected_mappings() {
        for &(composer_json, expected) in PSR4_TABLE {
            let actual = parse_psr4(composer_json);
            let expected: Vec<(String, String)> = expected
                .iter()
                .map(|(prefix, dir)| (prefix.to_string(), dir.to_string()))
                .collect();
            assert_eq!(actual, expected, "composer.json {composer_json:?}");
        }
    }

    #[test]
    fn psr4_lists_autoload_before_autoload_dev() {
        let entries = parse_psr4(
            r#"{"autoload-dev":{"psr-4":{"Dev\\":"tests"}},"autoload":{"psr-4":{"App\\":"src"}}}"#,
        );
        assert_eq!(
            entries,
            vec![
                ("App".to_string(), "src".to_string()),
                ("Dev".to_string(), "tests".to_string()),
            ]
        );
    }
}
