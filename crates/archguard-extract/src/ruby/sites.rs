//! Call sites and constant references in one method body. Locals are the
//! method's parameters, block parameters and every name assigned anywhere in
//! the body (Ruby decides per position; a name assigned later in the body
//! is taken as a local throughout). A bare name that is no local is a call
//! of a method of `self`.
use std::collections::BTreeSet;

use super::code::{Code, KEYWORDS, Kind, nested_scope_end};
use super::symbols::{RubyCallSite, RubyConst, RubyTarget};

/// Reflective calls: their target is only known at runtime.
const DYNAMIC: [&str; 14] = [
    "send",
    "public_send",
    "__send__",
    "method_missing",
    "define_method",
    "define_singleton_method",
    "instance_eval",
    "instance_exec",
    "class_eval",
    "class_exec",
    "module_eval",
    "eval",
    "method",
    "instance_variable_get",
];

fn is_constant(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_uppercase())
        && !text.ends_with([':', '?', '!'])
        && !KEYWORDS.contains(&text)
}

/// A method name after a dot, or a bare name that may be a call.
fn is_name(text: &str) -> bool {
    text.starts_with(|c: char| c.is_alphabetic() || c == '_') && !text.ends_with(':')
}

fn is_lower_name(text: &str) -> bool {
    text.starts_with(|c: char| c.is_lowercase() || c == '_') && !text.ends_with(':')
}

/// The constant path (`A::B`, `::C`) starting at `i` and the index after
/// it, read in the lexical scope `nesting`.
pub(super) fn constant_at(code: &Code, i: usize, nesting: &[String]) -> Option<(RubyConst, usize)> {
    if code.after_dot(i) {
        return None;
    }
    let mut at = i;
    let absolute = code.text(at) == "::";
    if absolute {
        // `value::C` reads a constant of a value; only a leading `::` is absolute.
        let previous = code.text(i.wrapping_sub(1));
        if (is_name(previous) && !KEYWORDS.contains(&previous)) || matches!(previous, ")" | "]") {
            return None;
        }
        at += 1;
    }
    let mut segments = Vec::new();
    while is_constant(code.text(at)) {
        segments.push(code.text(at));
        if code.text(at + 1) == "::" && is_constant(code.text(at + 2)) {
            at += 2;
        } else {
            at += 1;
            break;
        }
    }
    if segments.is_empty() {
        return None;
    }
    let constant = RubyConst {
        path: segments.join("::"),
        absolute,
        nesting: nesting.to_vec(),
    };
    Some((constant, at))
}

pub(super) struct Sites<'a> {
    code: &'a Code<'a>,
    nesting: &'a [String],
    from: (&'a str, &'a str),
    singleton: bool,
    locals: BTreeSet<String>,
    sites: Vec<RubyCallSite>,
    references: Vec<(u32, RubyConst)>,
}

impl<'a> Sites<'a> {
    pub(super) fn new(
        code: &'a Code<'a>,
        nesting: &'a [String],
        from: (&'a str, &'a str),
        singleton: bool,
    ) -> Self {
        Sites {
            code,
            nesting,
            from,
            singleton,
            locals: BTreeSet::new(),
            sites: Vec::new(),
            references: Vec::new(),
        }
    }

    /// Parameter names: names right after `(`, `,`, `*`, `**`, `&` or at the
    /// start; keyword parameters lose their colon. Defaults are no names.
    pub(super) fn params(&mut self, (from, to): (usize, usize)) {
        for k in from..to {
            let text = self.code.text(k);
            let starts =
                k == from || matches!(self.code.text(k - 1), "(" | "," | "*" | "**" | "&" | "\n");
            if starts && is_lower_name(text.trim_end_matches(':')) {
                self.locals.insert(text.trim_end_matches(':').to_string());
            }
        }
    }

    /// Locals assigned or bound anywhere in `from..to`.
    fn locals(&mut self, (from, to): (usize, usize)) {
        let code = self.code;
        for k in from..to {
            let text = code.text(k);
            let next = code.text(k + 1);
            let assigns = next == "="
                || (next.len() > 1
                    && next.ends_with('=')
                    && !matches!(next, "==" | "!=" | ">=" | "<=" | "===" | "=~" | "=>"));
            if is_lower_name(text) && !code.after_dot(k) && assigns {
                self.locals.insert(text.to_string());
            }
            if code.kinds.get(&k) == Some(&Kind::Block) && next == "|" {
                self.block_params(k + 2, to);
            }
            if (text == "for" || self.rescue_binding(from, k)) && is_lower_name(next) {
                self.locals.insert(next.to_string());
            }
        }
    }

    /// The names between `|` … `|` from `at`.
    fn block_params(&mut self, mut at: usize, to: usize) {
        while at < to && !matches!(self.code.text(at), "|" | "") {
            let name = self.code.text(at).trim_end_matches(':');
            if is_lower_name(name) {
                self.locals.insert(name.to_string());
            }
            at += 1;
        }
    }

    /// `=>` at `k` binds the exception of a `rescue` on the same line.
    fn rescue_binding(&self, from: usize, k: usize) -> bool {
        self.code.text(k) == "=>"
            && (from..k)
                .rev()
                .take_while(|&j| self.code.text(j) != "\n")
                .any(|j| self.code.text(j) == "rescue")
    }

    /// Record the call sites and constant references of the body `from..to`
    /// of the method `name`.
    pub(super) fn scan(&mut self, (from, to): (usize, usize), name: &str) {
        self.locals((from, to));
        let mut i = from;
        while i < to {
            if let Some(end) = nested_scope_end(self.code, i) {
                i = end;
                continue;
            }
            i = self.site(i, name).unwrap_or(i + 1);
        }
    }

    pub(super) fn finish(self) -> (Vec<RubyCallSite>, Vec<(u32, RubyConst)>) {
        (self.sites, self.references)
    }

    /// The site starting at `i`, if any; the index to continue at.
    fn site(&mut self, i: usize, method: &str) -> Option<usize> {
        if let Some(next) = self.constant_site(i) {
            return Some(next);
        }
        if self.code.after_dot(i) {
            return None;
        }
        self.dot_site(i)
            .or_else(|| self.self_site(i))
            .or_else(|| self.word_site(i, method))
    }

    /// A constant reference at `i`, and `Const.m(…)` after it.
    fn constant_site(&mut self, i: usize) -> Option<usize> {
        let code = self.code;
        let (constant, at) = constant_at(code, i, self.nesting)?;
        self.references.push((code.toks[i].line, constant.clone()));
        if !(matches!(code.text(at), "." | "&." | "::") && is_name(code.text(at + 1))) {
            return Some(at);
        }
        let called = code.text(at + 1).to_string();
        let label = format!("{}.{called}", constant.path);
        let target = RubyTarget::Const {
            constant,
            method: called,
        };
        self.push(i, at + 1, target, label);
        Some(at + 2)
    }

    /// `receiver.m(…)` on any receiver but a constant or `self`.
    fn dot_site(&mut self, i: usize) -> Option<usize> {
        let code = self.code;
        let called = code.text(i + 1);
        if !matches!(code.text(i), "." | "&.") || !is_name(called) {
            return None;
        }
        let receiver = code.text(i - 1);
        let (start, label) = if is_name(receiver) {
            (i - 1, format!("{receiver}.{called}"))
        } else {
            (i, format!(".{called}"))
        };
        let target = if DYNAMIC.contains(&called) {
            RubyTarget::Dynamic
        } else {
            RubyTarget::Unresolved
        };
        self.push(start, i + 1, target, label);
        Some(i + 2)
    }

    /// `self.m(…)` or the setter call `self.m = …`.
    fn self_site(&mut self, i: usize) -> Option<usize> {
        let code = self.code;
        let named = matches!(code.text(i + 1), "." | "&.") && is_name(code.text(i + 2));
        if code.text(i) != "self" || !named {
            return None;
        }
        let mut called = code.text(i + 2).to_string();
        if code.text(i + 3) == "=" {
            called.push('=');
        }
        let label = format!("self.{called}");
        self.push(i, i + 2, RubyTarget::SelfCall(called), label);
        Some(i + 3)
    }

    /// `super`, or a bare name that is no local.
    fn word_site(&mut self, i: usize, method: &str) -> Option<usize> {
        let word = self.code.text(i);
        if word == "super" {
            self.push(i, i, RubyTarget::Super(method.to_string()), "super".into());
            return Some(i + 1);
        }
        let bare = is_lower_name(word)
            && !KEYWORDS.contains(&word)
            && !matches!(word, "self" | "nil" | "true" | "false")
            && !self.locals.contains(word);
        if !bare {
            return None;
        }
        let target = if DYNAMIC.contains(&word) {
            RubyTarget::Dynamic
        } else {
            RubyTarget::SelfCall(word.to_string())
        };
        self.push(i, i, target, word.to_string());
        Some(i + 1)
    }

    /// A site from token `start` to the name at `name`, with the argument
    /// list when it touches the name.
    fn push(&mut self, start: usize, name: usize, target: RubyTarget, label: String) {
        let code = self.code;
        let open = name + 1;
        let end = if code.text(open) == "(" && code.toks[open].start == code.toks[name].end {
            code.after_group(open) - 1
        } else {
            name
        };
        let end = end.min(code.toks.len() - 1);
        let text = code.source[code.toks[start].start..code.toks[end].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.sites.push(RubyCallSite {
            from_class: self.from.0.to_string(),
            from_method: Some(self.from.1.to_string()),
            singleton: self.singleton,
            target,
            label,
            line: code.toks[start].line,
            column: code.column(start),
            code: text,
        });
    }
}
