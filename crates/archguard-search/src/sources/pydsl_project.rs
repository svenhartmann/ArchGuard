//! The Python files below the root as one project: which class derives from a DSL `Document`,
//! also through base classes of the same file or of files it imports (ADR 0023, ticket 24).
//! A base the walk cannot follow stays `Unresolved`; the caller reports it as a gap.
use super::pydsl_module::Module;
use archguard_flow_common::lang_python::resolve_import;
use std::collections::BTreeSet;

/// Bases followed from one class; stops cycles and absurdly deep hierarchies.
const BASE_DEPTH: u8 = 8;

/// A class: file and class index in it.
pub(super) type Class = (usize, usize);

pub(super) struct Project<'a> {
    pub(super) files: Vec<(String, Module<'a>)>,
    set: BTreeSet<String>,
}

/// What a class derives from.
pub(super) enum Kind {
    /// A DSL document: flavor and the classes from the class itself up to the DSL base.
    Document(&'static str, Vec<Class>),
    /// Derives from DSL classes that are no document (`InnerDoc`) or from nothing.
    Plain,
    /// The first base that leads nowhere: not DSL, not a class below the root.
    Unresolved(String),
}

impl<'a> Project<'a> {
    pub(super) fn new(files: Vec<(String, Module<'a>)>) -> Self {
        let set = files.iter().map(|(r, _)| r.clone()).collect();
        Self { files, set }
    }

    pub(super) fn kind(&self, (f, c): Class) -> Kind {
        self.class_kind((f, c), BASE_DEPTH)
    }

    fn class_kind(&self, (f, c): Class, depth: u8) -> Kind {
        let m = &self.files[f].1;
        let mut lost = None;
        for base in m.bases(&m.defs[c]) {
            match self.name_kind(f, &base, depth) {
                Kind::Document(flavor, mut chain) => {
                    chain.insert(0, (f, c));
                    return Kind::Document(flavor, chain);
                }
                Kind::Unresolved(n) => {
                    lost.get_or_insert(n);
                }
                Kind::Plain => {}
            }
        }
        lost.map_or(Kind::Plain, Kind::Unresolved)
    }

    /// What the name `name` as written in file `f` stands for.
    fn name_kind(&self, f: usize, name: &str, depth: u8) -> Kind {
        let m = &self.files[f].1;
        let Some(depth) = depth.checked_sub(1) else {
            return Kind::Unresolved(name.into());
        };
        if let Some((last, flavor)) = m.dsl(name) {
            return match last.as_str() {
                "Document" | "AsyncDocument" => Kind::Document(flavor, vec![]),
                _ => Kind::Plain,
            };
        }
        if let Some(c) = m.class_index(name) {
            return self.class_kind((f, c), depth);
        }
        match self.import_target(f, name) {
            Some((g, symbol)) => self.name_kind(g, &symbol, depth),
            None if name == "object" => Kind::Plain,
            None => Kind::Unresolved(name.into()),
        }
    }

    /// The file and the name in it that `name` of file `f` is imported as.
    fn import_target(&self, f: usize, name: &str) -> Option<(usize, String)> {
        let (rel, m) = &self.files[f];
        let (head, rest) = name.split_once('.').unzip();
        let imp = (m.imports.iter()).find(|i| i.local == head.unwrap_or(name))?;
        let (file, symbol) = resolve_import(rel, imp, &self.set)?;
        let symbol = symbol
            .into_iter()
            .chain(rest.map(String::from))
            .collect::<Vec<_>>();
        let g = self.files.iter().position(|(r, _)| *r == file)?;
        // `from m import C` yields `C`, `from p import m` and `m.C` too; deeper is no class.
        match symbol.as_slice() {
            [one] => Some((g, one.clone())),
            _ => None,
        }
    }
}
