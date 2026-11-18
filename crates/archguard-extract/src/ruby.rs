//! Ruby import scanner with lexical shielding (ADR 0019). The package of a
//! file is its directory; Ruby files declare no module of their own.
//!
//! - `require_relative 'x'` becomes the path spec `./x`, resolved against
//!   the requiring file.
//! - `require 'x'` with a literal argument becomes the bare path spec `x`;
//!   the engine resolves it against the project's `lib/` directories (the
//!   default load path of a gem) and reports anything else, gems and the
//!   standard library alike, as external.
//! - Every constant reference in code (`Billing::Invoice`, `::Invoice`)
//!   becomes a [`TargetSpec::Qualified`] whose first segment is the lexical
//!   nesting, space-separated (`"Shop Shop::Cart"`, empty for none or for an
//!   absolute `::X`), followed by the constant's segments. The engine maps it
//!   to a file only in Rails/Zeitwerk projects; elsewhere it stays
//!   unresolved without a warning.
//!
//! `load`, `autoload`, interpolated or computed paths and `$LOAD_PATH`
//! changes are not followed.

mod code;
mod sites;
mod structure;
mod symbols;

pub use structure::{RubyStructure, scan_ruby_structure};
pub use symbols::{
    RubyCallSite, RubyClassFacts, RubyConst, RubyMethodFacts, RubySymbols, RubyTarget, Superclass,
};

use crate::extract::{Extraction, Extractor, RawImport, TargetSpec};
use crate::lex::Flavor;
use crate::walk::Lang;

/// Ruby scanner. See the [module documentation](self).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RubyExtractor;

impl Extractor for RubyExtractor {
    fn lang(&self) -> Lang {
        Lang::Ruby
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let lines: Vec<&str> = content.lines().collect();
        let raw = |line: u32| {
            lines
                .get(line as usize - 1)
                .copied()
                .unwrap_or("")
                .to_string()
        };
        let import = |line: u32, target: TargetSpec| RawImport {
            source: rel_path.to_string(),
            target,
            line,
            raw: raw(line),
        };
        let mut imports: Vec<RawImport> = requires(content)
            .into_iter()
            .map(|(line, target)| import(line, target))
            .collect();
        let found = scan_ruby_structure(rel_path, rel_path, content);
        imports.extend(
            found
                .symbols
                .references
                .into_iter()
                .map(|(line, constant)| {
                    let scope = if constant.absolute {
                        String::new()
                    } else {
                        constant.nesting.join(" ")
                    };
                    let mut segments = vec![scope];
                    segments.extend(constant.path.split("::").map(str::to_string));
                    import(line, TargetSpec::Qualified(segments))
                }),
        );
        Extraction {
            imports,
            declarations: Vec::new(),
        }
    }
}

/// `require` and `require_relative` with one literal, uninterpolated string.
fn requires(content: &str) -> Vec<(u32, TargetSpec)> {
    let tokens = crate::lex::tokens(content, Flavor::Ruby);
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.literal || !matches!(token.text.as_str(), "require" | "require_relative") {
            continue;
        }
        let dotted = i > 0 && matches!(tokens[i - 1].text.as_str(), "." | "&." | "::");
        let Some(path) = literal_argument(&tokens, i + 1).filter(|_| !dotted) else {
            continue;
        };
        let spec = if token.text == "require_relative" && !path.starts_with('.') {
            format!("./{path}")
        } else {
            path.to_string()
        };
        out.push((token.line, TargetSpec::Path(spec)));
    }
    out
}

/// The unquoted text of a sole plain string argument at `at` (with or
/// without parentheses).
fn literal_argument(tokens: &[crate::lex::Token], mut at: usize) -> Option<&str> {
    if tokens.get(at).is_some_and(|t| t.text == "(") {
        at += 1;
    }
    let text = tokens.get(at).filter(|t| t.literal)?.text.as_str();
    let closes = tokens
        .get(at + 1)
        .is_none_or(|t| matches!(t.text.as_str(), "\n" | ")" | ";"));
    let quoted = (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"') && !text.contains("#{"));
    (closes && quoted && text.len() >= 3).then(|| &text[1..text.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    fn imports(content: &str) -> Vec<(u32, TargetSpec)> {
        RubyExtractor
            .extract("lib/shop/cart.rb", content)
            .imports
            .into_iter()
            .map(|import| (import.line, import.target))
            .collect()
    }

    fn qualified(segments: &[&str]) -> TargetSpec {
        TargetSpec::Qualified(segments.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn literal_requires_become_path_specs() {
        let source = "require 'json'\n\
            require_relative \"item\"\n\
            require_relative '../billing/invoice.rb'\n\
            require('shop/tax')\n\
            require \"plugins/#{name}\"\n\
            require File.join(dir, 'x')\n\
            # require 'commented'\n\
            text = \"require 'quoted'\"\n";
        assert_eq!(
            imports(source),
            [
                (1, TargetSpec::Path("json".into())),
                (2, TargetSpec::Path("./item".into())),
                (3, TargetSpec::Path("../billing/invoice.rb".into())),
                (4, TargetSpec::Path("shop/tax".into())),
                (6, qualified(&["", "File"])),
            ]
        );
    }

    #[test]
    fn constant_references_carry_their_lexical_scope() {
        let source = "module Shop\n  class Cart < Base\n    def total\n      Billing::Invoice.new\n      ::Tax.rate\n    end\n  end\nend\n";
        assert_eq!(
            imports(source),
            [
                (2, qualified(&["Shop", "Base"])),
                (4, qualified(&["Shop Shop::Cart", "Billing", "Invoice"])),
                (5, qualified(&["", "Tax"])),
            ]
        );
    }

    #[test]
    fn heredocs_and_percent_literals_hide_requires() {
        let source =
            "x = <<~RUBY\n  require 'hidden'\nRUBY\nwords = %w[require x]\nrequire 'shown'\n";
        assert_eq!(imports(source), [(5, TargetSpec::Path("shown".into()))]);
    }

    #[test]
    fn ruby_is_registered_for_rb() {
        assert_eq!(Lang::from_extension("rb"), Some(Lang::Ruby));
        assert!(registry().iter().any(|e| e.lang() == Lang::Ruby));
        assert!(matches!(&extractor_for(Lang::Ruby), Some(e) if e.lang() == Lang::Ruby));
        assert!(
            RubyExtractor
                .extract("a.rb", "class A; end\n")
                .declarations
                .is_empty()
        );
    }
}
