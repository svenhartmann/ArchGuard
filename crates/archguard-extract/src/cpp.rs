//! C and C++ include scanner (ADR 0017). Both languages share one
//! [`Lang::Cpp`], so a `.h` header needs no guess which of the two it is.
//! `#include "x.h"` becomes a [`TargetSpec::Path`], `#include <x.h>` a
//! [`TargetSpec::Symbol`] with the spec as its path and no stem; the engine
//! resolves both against the file tree and the project's include
//! directories. Includes inside comments and strings never count; an include
//! named by a macro (`#include CONFIG_H`) is not reported. Files declare no
//! module: the package is the directory.

mod structure;

pub use structure::{CppStructure, scan_cpp_structure};

use crate::extract::{Extraction, Extractor, RawImport, TargetSpec};
use crate::lex::Flavor;
use crate::walk::Lang;

/// C/C++ scanner. See the [module documentation](self).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CppExtractor;

impl Extractor for CppExtractor {
    fn lang(&self) -> Lang {
        Lang::Cpp
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let lines: Vec<&str> = content.lines().collect();
        let tokens = crate::lex::tokens(content, Flavor::Cpp);
        let mut extraction = Extraction::default();
        let mut line_start = true;
        for (i, token) in tokens.iter().enumerate() {
            let first = std::mem::replace(&mut line_start, token.text == "\n");
            if !first || token.literal || token.text != "#" {
                continue;
            }
            if tokens.get(i + 1).is_none_or(|t| t.text != "include") {
                continue;
            }
            let raw = lines.get(token.line as usize - 1).copied().unwrap_or("");
            let Some(target) = include_target(raw) else {
                continue;
            };
            extraction.imports.push(RawImport {
                source: rel_path.to_string(),
                target,
                line: token.line,
                raw: raw.to_string(),
            });
        }
        extraction
    }
}

/// The target of an `#include` line: quoted as a path, angled as a symbol.
fn include_target(line: &str) -> Option<TargetSpec> {
    let rest = line.trim_start().strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("include")?.trim_start();
    let (close, quoted) = match rest.chars().next()? {
        '"' => ('"', true),
        '<' => ('>', false),
        _ => return None,
    };
    let spec = &rest[1..1 + rest[1..].find(close)?];
    if spec.is_empty() {
        return None;
    }
    Some(if quoted {
        TargetSpec::Path(spec.to_string())
    } else {
        TargetSpec::Symbol {
            path: spec.to_string(),
            stem: None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    fn imports(content: &str) -> Vec<(u32, TargetSpec)> {
        let extraction = CppExtractor.extract("src/app/main.cpp", content);
        assert!(extraction.declarations.is_empty());
        extraction
            .imports
            .into_iter()
            .map(|import| (import.line, import.target))
            .collect()
    }

    fn system(spec: &str) -> TargetSpec {
        TargetSpec::Symbol {
            path: spec.into(),
            stem: None,
        }
    }

    #[test]
    fn quoted_and_angled_includes_are_told_apart() {
        let source = "#include \"order.h\"\n\
            #  include <vector>\n\
            #include \"../util/log.hpp\" // logging\n\
            #include <shop/pay.h>\n\
            #include CONFIG_H\n\
            #define X 1\n";
        assert_eq!(
            imports(source),
            vec![
                (1, TargetSpec::Path("order.h".into())),
                (2, system("vector")),
                (3, TargetSpec::Path("../util/log.hpp".into())),
                (4, system("shop/pay.h")),
            ]
        );
    }

    #[test]
    fn comments_strings_and_macro_bodies_never_create_includes() {
        let source = "/* #include \"hidden.h\" */\n\
            // #include \"line.h\"\n\
            const char* s = \"#include \\\"fake.h\\\"\";\n\
            int x; #include \"not_at_line_start.h\"\n\
            #include \"real.h\"\n";
        assert_eq!(
            imports(source),
            vec![(5, TargetSpec::Path("real.h".into()))]
        );
    }

    #[test]
    fn an_apostrophe_in_a_directive_does_not_hide_the_next_include() {
        let source = "#warning Don't use this file\n#include \"b.h\"\n";
        assert_eq!(imports(source), vec![(2, TargetSpec::Path("b.h".into()))]);
    }

    #[test]
    fn cpp_is_registered_for_c_and_cpp_extensions() {
        for ext in ["c", "h", "cc", "cpp", "cxx", "hpp", "hh", "hxx"] {
            assert_eq!(Lang::from_extension(ext), Some(Lang::Cpp), "ext {ext}");
        }
        assert!(registry().iter().any(|e| e.lang() == Lang::Cpp));
        assert!(matches!(&extractor_for(Lang::Cpp), Some(e) if e.lang() == Lang::Cpp));
    }
}
