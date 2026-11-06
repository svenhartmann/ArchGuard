//! PHP name resolution within one file, as PHP does it at compile time:
//! the current `namespace`, `use` imports and fully qualified `\A\B` names.

use std::collections::HashMap;

use super::symbols::{PhpClassRef, PhpTarget};

/// What a `use` statement imports.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Import {
    Class,
    Function,
    Const,
}

/// Names visible at a point of a file: the namespace and its `use` imports.
#[derive(Default)]
pub(super) struct Names {
    namespace: String,
    /// Lowercase alias to fully qualified class (or namespace) name.
    classes: HashMap<String, String>,
    /// Lowercase alias to fully qualified function name.
    functions: HashMap<String, String>,
}

impl Names {
    /// A new `namespace` statement starts with no imports.
    pub(super) fn enter(&mut self, namespace: String) {
        *self = Names {
            namespace,
            ..Default::default()
        };
    }

    /// `name` declared in the current namespace.
    pub(super) fn qualify(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.into()
        } else {
            format!("{}\\{name}", self.namespace)
        }
    }

    pub(super) fn class(&self, raw: &str) -> PhpClassRef {
        match raw.to_ascii_lowercase().as_str() {
            "self" | "static" => PhpClassRef::Current,
            "parent" => PhpClassRef::Parent,
            _ => PhpClassRef::Named(self.class_name(raw)),
        }
    }

    /// A class name, or a qualified function name: fully qualified,
    /// `namespace\…`, an imported first segment, else the current namespace.
    pub(super) fn class_name(&self, raw: &str) -> String {
        if let Some(full) = raw.strip_prefix('\\') {
            return full.into();
        }
        if let Some(rest) = strip_namespace(raw) {
            return self.qualify(rest);
        }
        let (first, rest) = raw
            .split_once('\\')
            .map_or((raw, None), |(f, r)| (f, Some(r)));
        match self.classes.get(&first.to_ascii_lowercase()) {
            Some(full) => rest.map_or_else(|| full.clone(), |rest| format!("{full}\\{rest}")),
            None => self.qualify(raw),
        }
    }

    pub(super) fn function(&self, raw: &str) -> PhpTarget {
        let fixed = |name: String| PhpTarget::Function {
            name,
            fallback: None,
        };
        if raw.contains('\\') {
            return fixed(self.class_name(raw));
        }
        if let Some(full) = self.functions.get(&raw.to_ascii_lowercase()) {
            return fixed(full.clone());
        }
        if self.namespace.is_empty() {
            return fixed(raw.into());
        }
        PhpTarget::Function {
            name: self.qualify(raw),
            fallback: Some(raw.into()),
        }
    }

    pub(super) fn bind(&mut self, kind: Import, alias: &str, name: &str) {
        let name = name.trim_start_matches('\\').to_string();
        let alias = alias.to_ascii_lowercase();
        match kind {
            Import::Class => self.classes.insert(alias, name),
            Import::Function => self.functions.insert(alias, name),
            Import::Const => None,
        };
    }
}

fn strip_namespace(raw: &str) -> Option<&str> {
    const PREFIX: &str = "namespace\\";
    raw.get(..PREFIX.len())
        .filter(|head| head.eq_ignore_ascii_case(PREFIX))
        .map(|_| &raw[PREFIX.len()..])
}
