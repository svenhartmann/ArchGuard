//! Python import scanner with lexical shielding and multiline statement assembly.
//! Comments and string bodies cannot create imports. This is a bounded static
//! scanner; dynamic loading, preprocessing and runtime resolution remain unsupported.

use crate::extract::{Extraction, Extractor, RawImport, TargetSpec};
use crate::walk::Lang;

/// Python scanner (T10). See the [module documentation](self) for the
/// emitted target shapes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PyExtractor;

impl Extractor for PyExtractor {
    fn lang(&self) -> Lang {
        Lang::Py
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut imports = Vec::new();
        for statement in crate::lex::statements(content, crate::lex::Flavor::Python) {
            let code = statement.code.trim().trim_end_matches(';');
            // Evidence must identify this import when multiple statements share a line.
            let raw = if statement.raw.contains(';') {
                &statement.exact
            } else {
                &statement.raw
            };
            imports.extend(import_imports(rel_path, raw, statement.line, code));
            imports.extend(from_imports(rel_path, raw, statement.line, code));
        }
        Extraction {
            imports,
            declarations: Vec::new(),
        }
    }
}

/// The body after a statement `keyword` at the start of `code`: `Some`
/// only when the line opens with the whole word (the keyword is followed
/// by whitespace) and non-empty content follows, `None` otherwise.
fn keyword_body<'a>(code: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = code.strip_prefix(keyword)?;
    let body = rest.trim_start();
    (!body.is_empty() && body.len() < rest.len()).then_some(body)
}

/// The imports of a comment-stripped `import a.b.c [as x], d` line — one
/// per comma item, aliases dropped.
fn import_imports(rel_path: &str, raw: &str, line: u32, code: &str) -> Vec<RawImport> {
    let Some(body) = keyword_body(code, "import") else {
        return Vec::new();
    };
    body.split(',')
        .filter_map(|item| {
            let dotted = item.split_whitespace().next()?;
            let segments = dotted_segments(dotted)?;
            let (path, stem) = if segments.len() == 1 {
                (segments[0].to_string(), None)
            } else {
                (
                    segments[..segments.len() - 1].join("/"),
                    Some(segments[segments.len() - 1].to_string()),
                )
            };
            Some(RawImport {
                source: rel_path.to_string(),
                target: TargetSpec::Symbol { path, stem },
                line,
                raw: raw.to_string(),
            })
        })
        .collect()
}

/// The imports of a comment-stripped `from MODULE import NAMES` line.
/// Absolute modules become one stem-ed symbol per imported name (the
/// `*` form one symbol without a stem); relative modules (leading dots)
/// become one `Path` spec per name with the dots as the `./`/`../` prefix.
fn from_imports(rel_path: &str, raw: &str, line: u32, code: &str) -> Vec<RawImport> {
    let Some(body) = keyword_body(code, "from") else {
        return Vec::new();
    };
    let Some((module, rest)) = split_first_token(body) else {
        return Vec::new();
    };
    let Some(names) = keyword_body(rest.trim_start(), "import") else {
        return Vec::new();
    };
    let names = imported_names(names);
    let dots = module.chars().take_while(|c| *c == '.').count();
    if dots > 0 {
        let remainder = match dotted_segments(&module[dots..]) {
            Some(segments) => Some(segments.join("/")),
            None if module.len() == dots => None,
            None => return Vec::new(),
        };
        let mut prefix = if dots == 1 {
            "./".into()
        } else {
            "../".repeat(dots - 1)
        };
        if let Some(remainder) = remainder {
            prefix.push_str(&remainder);
            prefix.push('/');
        }
        return names
            .into_iter()
            .map(|name| RawImport {
                source: rel_path.to_string(),
                target: TargetSpec::Path(if name == "*" {
                    prefix.trim_end_matches('/').into()
                } else {
                    format!("{prefix}{name}")
                }),
                line,
                raw: raw.to_string(),
            })
            .collect();
    }
    let Some(segments) = dotted_segments(module) else {
        return Vec::new();
    };
    let path = segments.join("/");
    names
        .into_iter()
        .map(|name| RawImport {
            source: rel_path.to_string(),
            target: TargetSpec::Symbol {
                path: path.clone(),
                stem: (name != "*").then(|| name.to_string()),
            },
            line,
            raw: raw.to_string(),
        })
        .collect()
}

/// The imported names of a `from … import NAMES` clause: the comma-
/// separated first tokens with one layer of surrounding parentheses
/// removed; empty or non-identifier items are dropped (`*` survives).
fn imported_names(names: &str) -> Vec<&str> {
    let names = names.trim();
    let names = names
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(names);
    names
        .split(',')
        .filter_map(|item| {
            let name = item.split_whitespace().next()?;
            (name == "*" || is_identifier(name)).then_some(name)
        })
        .collect()
}

/// The segments of a dotted identifier path `a.b.c`, `None` when the
/// string is not a sequence of non-empty identifier segments.
fn dotted_segments(dotted: &str) -> Option<Vec<&str>> {
    if dotted.is_empty() {
        return None;
    }
    dotted
        .split('.')
        .map(|segment| is_identifier(segment).then_some(segment))
        .collect()
}

/// `true` for a plain Python identifier (attributes, calls and dotted
/// specs outside their intended position never match).
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// The first whitespace-separated token of `body` and the remainder of
/// the line behind it, `None` when `body` has no content.
fn split_first_token(body: &str) -> Option<(&str, &str)> {
    let index = body.find(char::is_whitespace)?;
    Some((&body[..index], &body[index..]))
}

/// Exported/internal symbol split of one file (WP-02).
///
/// Rule (R14): `__all__` wins; a leading `_` marks internal;
/// everything else is exported.
pub fn scan_visibility(content: &str) -> archguard_core::Visibility {
    let mut visibility = archguard_core::Visibility::default();
    let clean = crate::lex::code_only(content, crate::lex::Flavor::Python);
    let lines: Vec<&str> = clean.lines().collect();

    // `__all__` wins: exported is exactly its names (possibly a
    // multi-line list), everything else a file declares is internal.
    let statements = crate::lex::statements(content, crate::lex::Flavor::Python);
    let explicit_all = statements.iter().find(|statement| {
        !statement.raw.starts_with(char::is_whitespace)
            && statement
                .code
                .strip_prefix("__all__")
                .is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    if let Some(statement) = explicit_all {
        let buffer = &statement.code;
        if let (Some(open), Some(close)) = (buffer.find('['), buffer.rfind(']')) {
            for name in buffer[open + 1..close].split(',') {
                let name = name.trim().trim_matches(|c| c == '"' || c == '\'');
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    visibility.exported.insert(name.to_string());
                }
            }
        }
        for name in top_level_names(&lines) {
            if !visibility.exported.contains(&name) {
                visibility.internal.insert(name);
            }
        }
        return visibility;
    }

    for name in top_level_names(&lines) {
        if name.starts_with('_') {
            visibility.internal.insert(name);
        } else {
            visibility.exported.insert(name);
        }
    }
    visibility
}

/// Names of top-level `def`/`class` statements (no leading indentation).
fn top_level_names(lines: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    for line in lines {
        if line.is_empty() || line.starts_with(char::is_whitespace) || line.starts_with('#') {
            continue;
        }
        for keyword in ["def ", "class ", "async def "] {
            if let Some(rest) = line.strip_prefix(keyword) {
                if let Some(name) = first_identifier(rest) {
                    names.push(name);
                }
                break;
            }
        }
    }
    names
}

/// First identifier token of `text`, `None` when there is none.
fn first_identifier(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() && !(chars[index].is_alphanumeric() || chars[index] == '_') {
        index += 1;
    }
    let start = index;
    while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
        index += 1;
    }
    (index > start).then(|| chars[start..index].iter().collect())
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn underscore_prefix_is_internal_without_all() {
        let source = "def public_api():\n    pass\n\n\ndef _private():\n    pass\n\nclass Helper:\n    pass\n\nclass _Hidden:\n    pass\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("public_api"));
        assert!(visibility.exported.contains("Helper"));
        assert!(visibility.internal.contains("_private"));
        assert!(visibility.internal.contains("_Hidden"));
    }

    #[test]
    fn explicit_all_wins_including_multi_line_lists() {
        let source = "__all__ = [\n    \"api\",\n    'api2',\n]\n\ndef api():\n    pass\n\ndef api2():\n    pass\n\ndef not_in_all():\n    pass\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("api"));
        assert!(visibility.exported.contains("api2"));
        assert!(
            visibility.internal.contains("not_in_all"),
            "__all__ decides: everything else is internal"
        );
    }

    #[test]
    fn indented_defs_are_not_top_level() {
        let visibility = scan_visibility("class Outer:\n    def inner(self):\n        pass\n");
        assert!(visibility.exported.contains("Outer"));
        assert!(!visibility.exported.contains("inner"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    /// 26-line fixture covering every T10 case: dotted `import` with
    /// alias, comma lists, single-segment imports, `from` with names and
    /// aliases, `import *`, all relative dot forms, an indented import,
    /// a trailing `#` comment, a `#`-only comment line, a closed
    /// triple-quoted block (docstring), a `#` inside a single-line
    /// triple-quoted string, keyword lookalikes, incomplete statements
    /// and an unterminated triple-quoted block swallowing the tail.
    const FIXTURE: &str = r#"import a.b.c
import x.y.z as alias, plain
import os
from a.b import x, y
from pkg import util
from . import helper
from ..pkg import service
from .local import thing as other
from a.b import *
from .. import climb
def run():
    import nested.deep
import os.path  # trailing comment
# import commented.gone
"""
import inside_docstring.gone
from inside_docstring import gone
"""
import after_docstring.kept
'''# not a comment''' import after_hash_string.kept
print("import fake.string")
from_x = 1
import
from a import
'''
import swallowed_by_open_block.gone
"#;

    /// The expected symbolic import for fixture line `line_no`.
    fn symbol(line_no: u32, path: &str, stem: Option<&str>) -> RawImport {
        RawImport {
            source: "app/main.py".to_string(),
            target: TargetSpec::Symbol {
                path: path.to_string(),
                stem: stem.map(str::to_string),
            },
            line: line_no,
            raw: fixture_line(line_no),
        }
    }

    /// The expected path-spec import for fixture line `line_no`.
    fn path_spec(line_no: u32, spec: &str) -> RawImport {
        RawImport {
            source: "app/main.py".to_string(),
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
        PyExtractor.extract("app/main.py", content)
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
                symbol(1, "a/b", Some("c")),
                symbol(2, "x/y", Some("z")),
                symbol(2, "plain", None),
                symbol(3, "os", None),
                symbol(4, "a/b", Some("x")),
                symbol(4, "a/b", Some("y")),
                symbol(5, "pkg", Some("util")),
                path_spec(6, "./helper"),
                path_spec(7, "../pkg/service"),
                path_spec(8, "./local/thing"),
                symbol(9, "a/b", None),
                path_spec(10, "../climb"),
                symbol(12, "nested", Some("deep")),
                symbol(13, "os", Some("path")),
                symbol(19, "after_docstring", Some("kept")),
            ],
            "imports: {:#?}",
            extraction.imports
        );
    }

    #[test]
    fn py_never_reports_declarations() {
        assert!(extract(FIXTURE).declarations.is_empty());
        assert_eq!(extract(""), Extraction::default());
    }

    #[test]
    fn dotted_import_stems_its_parent_path_with_the_last_segment() {
        let extraction = extract("import a.b.c as x\n");
        assert_eq!(
            extraction.imports[0].target,
            TargetSpec::Symbol {
                path: "a/b".to_string(),
                stem: Some("c".to_string()),
            }
        );
    }

    #[test]
    fn import_comma_list_yields_one_import_per_module() {
        let extraction = extract("import a, b as bb, c.d\n");
        assert_eq!(targets(&extraction), ["a", "b", "c:d"]);
    }

    #[test]
    fn single_segment_import_is_the_module_without_a_stem() {
        // `import pkg` matches a known package directory; stdlib imports
        // such as `import os` stay External unless a `os/` module exists.
        let extraction = extract("import os\n");
        assert_eq!(
            extraction.imports[0].target,
            TargetSpec::Symbol {
                path: "os".to_string(),
                stem: None,
            }
        );
    }

    #[test]
    fn from_module_stems_each_imported_name() {
        let extraction = extract("from a.b import x as z, w\n");
        assert_eq!(targets(&extraction), ["a/b:x", "a/b:w"]);
    }

    #[test]
    fn star_import_targets_the_module_itself() {
        let extraction = extract("from a.b import *\n");
        assert_eq!(
            extraction.imports[0].target,
            TargetSpec::Symbol {
                path: "a/b".to_string(),
                stem: None,
            }
        );
    }

    #[test]
    fn parenthesized_name_list_is_split() {
        let extraction = extract("from a import (x, y)\n");
        assert_eq!(targets(&extraction), ["a:x", "a:y"]);
    }

    #[test]
    fn relative_from_forms_become_dotslash_path_specs() {
        for (content, expected) in [
            ("from . import x\n", "./x"),
            ("from .. import y\n", "../y"),
            ("from ..pkg import z\n", "../pkg/z"),
            ("from .a.b import c\n", "./a/b/c"),
        ] {
            let extraction = extract(content);
            assert_eq!(targets(&extraction), [expected], "content {content:?}");
            for import in &extraction.imports {
                assert!(matches!(&import.target, TargetSpec::Path(_)));
            }
        }
    }

    #[test]
    fn relative_star_import_targets_the_package() {
        assert_eq!(targets(&extract("from . import *\n")), ["."]);
    }

    #[test]
    fn hash_comments_are_skipped() {
        let content = "import a.b  # import x.y\nx = 1  # import q.r\nfrom a import b  # import\n";
        assert_eq!(targets(&extract(content)), ["a:b", "a:b"]);
    }

    #[test]
    fn unterminated_triple_quote_swallows_the_rest_of_the_file() {
        let content = "import a.b\n'''\nimport c.d\n";
        assert_eq!(targets(&extract(content)), ["a:b"]);
    }

    #[test]
    fn keyword_lookalikes_and_mid_line_text_are_not_statements() {
        let content = "import_x a.b\nfrom_x import y\nmy_import a.b\nprint(\"from a import b\")\n";
        assert!(extract(content).imports.is_empty());
    }

    #[test]
    fn incomplete_statements_are_ignored() {
        let content = "import\nfrom\nfrom a\nfrom a import\n";
        assert!(extract(content).imports.is_empty());
    }

    #[test]
    fn from_without_a_space_after_the_dots_is_recognized() {
        assert_eq!(targets(&extract("from .import x\n")), ["./x"]);
    }

    #[test]
    fn line_numbers_are_one_based_and_raw_is_verbatim() {
        let extraction = extract("x = 1\nimport a.b  # trailing\n");
        assert_eq!(extraction.imports[0].line, 2);
        assert_eq!(extraction.imports[0].raw, "import a.b  # trailing");
    }

    #[test]
    fn extractor_reports_the_python_language() {
        assert_eq!(PyExtractor.lang(), Lang::Py);
    }

    #[test]
    fn py_extractor_is_registered() {
        assert!(registry().iter().any(|e| e.lang() == Lang::Py));
        assert!(matches!(&extractor_for(Lang::Py), Some(e) if e.lang() == Lang::Py));
    }
}
