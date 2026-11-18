//! Ruby import resolution (ADR 0019) over walked (project-root-relative)
//! paths, so derived source roots such as `app` or `lib` do not hide them:
//! - `require_relative` specs resolve against the requiring file.
//! - a literal `require 'a/b'` resolves to `<lib>/a/b.rb` for every `lib/`
//!   directory of the project (a gem's default load path). Several matches:
//!   the `lib/` of the requiring file's own tree wins, else none.
//! - a constant reference resolves by Zeitwerk naming (`Billing::Invoice` →
//!   `billing/invoice.rb`) below the autoload roots: in a Rails app (a
//!   `config/application.rb`) every `app/*` directory and `app/*/concerns`;
//!   in a gem, the directory of a file that references `Zeitwerk::Loader`.
//!   Lexical scopes are tried innermost first. Without autoload roots,
//!   constant references stay unresolved.
use std::collections::{BTreeMap, BTreeSet};

use archguard_extract::{RawImport, TargetSpec, resolve_relative};

use super::scanned_file::ScannedFile;

pub(super) struct RubyImports<'a> {
    /// Walked path without `.rb` → stripped path (the node key).
    stems: BTreeMap<String, &'a str>,
    /// Every `lib` directory.
    libs: Vec<String>,
    /// Zeitwerk autoload roots.
    roots: BTreeSet<String>,
}

impl<'a> RubyImports<'a> {
    pub(super) fn new(scanned: &'a [ScannedFile]) -> Self {
        let mut imports = RubyImports {
            stems: BTreeMap::new(),
            libs: Vec::new(),
            roots: BTreeSet::new(),
        };
        let ruby: Vec<_> = scanned.iter().filter(|file| file.lang == "rb").collect();
        let mut libs = BTreeSet::new();
        for file in &ruby {
            imports.add(file, &mut libs);
        }
        let rails: Vec<_> = ruby
            .iter()
            .filter_map(|file| file.rel_path.strip_suffix("config/application.rb"))
            .collect();
        for file in &ruby {
            for prefix in &rails {
                imports.rails_roots(&file.rel_path, prefix);
            }
        }
        imports.libs = libs.into_iter().collect();
        imports
    }

    /// Index one file: its stem, the `lib` directories above it and a
    /// Zeitwerk loader in it.
    fn add(&mut self, file: &'a ScannedFile, libs: &mut BTreeSet<String>) {
        let path = file.rel_path.as_str();
        let stem = path.strip_suffix(".rb").unwrap_or(path);
        self.stems.insert(stem.to_string(), &file.stripped);
        let parts: Vec<&str> = path.split('/').collect();
        for (at, part) in parts.iter().enumerate().take(parts.len() - 1) {
            if *part == "lib" {
                libs.insert(parts[..=at].join("/"));
            }
        }
        let loader = ["Zeitwerk".to_string(), "Loader".to_string()];
        let zeitwerk = file.extraction.imports.iter().any(
            |import| matches!(&import.target, TargetSpec::Qualified(s) if s.get(1..) == Some(&loader[..])),
        );
        if zeitwerk {
            self.roots.insert(parent(path).to_string());
        }
    }

    /// The autoload roots of the Rails app at `prefix` that `path` lies in:
    /// `app/<x>` and `app/<x>/concerns`.
    fn rails_roots(&mut self, path: &str, prefix: &str) {
        let Some(rest) = path.strip_prefix(&format!("{prefix}app/")) else {
            return;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() < 2 {
            return;
        }
        self.roots.insert(format!("{prefix}app/{}", parts[0]));
        if parts.len() > 2 && parts[1] == "concerns" {
            self.roots
                .insert(format!("{prefix}app/{}/concerns", parts[0]));
        }
    }

    /// The stripped path of the file `raw` names, `None` when it is external
    /// or unknown.
    pub(super) fn resolve(&self, raw: &RawImport, file: &ScannedFile) -> Option<&'a str> {
        let found = match &raw.target {
            TargetSpec::Path(spec) if spec.starts_with('.') => self
                .stems
                .get(&resolve_relative(&file.rel_path, spec)?)
                .copied(),
            TargetSpec::Path(spec) => self.required(spec, &file.rel_path),
            TargetSpec::Qualified(segments) => self.constant(segments),
            TargetSpec::Symbol { .. } => None,
        };
        found.filter(|path| *path != file.stripped)
    }

    /// `require 'spec'` against the `lib/` directories.
    fn required(&self, spec: &str, source: &str) -> Option<&'a str> {
        let spec = spec.strip_suffix(".rb").unwrap_or(spec);
        let found: Vec<(&str, &'a str)> = self
            .libs
            .iter()
            .filter_map(|lib| {
                let file = self.stems.get(&format!("{lib}/{spec}"))?;
                Some((lib.as_str(), *file))
            })
            .collect();
        if let [(_, file)] = found.as_slice() {
            return Some(file);
        }
        // Several gems provide the path: the one the requiring file lives in.
        let own: Vec<_> = found
            .iter()
            .filter(|(lib, _)| source.starts_with(parent(lib)))
            .max_by_key(|(lib, _)| lib.len())
            .into_iter()
            .collect();
        own.first().map(|(_, file)| *file)
    }

    /// A constant reference by Zeitwerk naming: `[scope, segments…]`.
    fn constant(&self, segments: &[String]) -> Option<&'a str> {
        if self.roots.is_empty() || segments.len() < 2 {
            return None;
        }
        let path = segments[1..].join("::");
        let mut candidates: Vec<String> = segments[0]
            .split(' ')
            .filter(|scope| !scope.is_empty())
            .rev()
            .map(|scope| format!("{scope}::{path}"))
            .collect();
        candidates.push(path);
        for candidate in candidates {
            let relative = file_name(&candidate);
            let found: Vec<&'a str> = self
                .roots
                .iter()
                .filter_map(|root| {
                    let stem = if root.is_empty() {
                        relative.clone()
                    } else {
                        format!("{root}/{relative}")
                    };
                    self.stems.get(&stem).copied()
                })
                .collect();
            match found.as_slice() {
                [] => continue,
                [file] => return Some(file),
                // Two roots define the constant: Zeitwerk would refuse.
                _ => return None,
            }
        }
        None
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Zeitwerk's file name of a constant path, without `.rb`:
/// `Billing::HTMLInvoice2` → `billing/html_invoice2` (ActiveSupport's
/// `underscore`).
fn file_name(constant: &str) -> String {
    constant
        .split("::")
        .map(|segment| {
            let chars: Vec<char> = segment.chars().collect();
            let mut out = String::new();
            for (at, &c) in chars.iter().enumerate() {
                let previous = at.checked_sub(1).map(|k| chars[k]);
                let next = chars.get(at + 1);
                let boundary = c.is_uppercase()
                    && previous.is_some_and(|p| {
                        p.is_lowercase()
                            || p.is_ascii_digit()
                            || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))
                    });
                if boundary {
                    out.push('_');
                }
                out.extend(c.to_lowercase());
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_follow_zeitwerk() {
        assert_eq!(file_name("Billing::Invoice"), "billing/invoice");
        assert_eq!(file_name("HTMLParser"), "html_parser");
        assert_eq!(file_name("Api::V2::UserSession"), "api/v2/user_session");
        assert_eq!(file_name("OAuth2Client"), "o_auth2_client");
    }
}
