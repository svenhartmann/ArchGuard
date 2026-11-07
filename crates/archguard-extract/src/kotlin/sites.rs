//! Which call, if any, starts at a token of a function body: `f(…)`,
//! `a.b.C.m(…)`, `this.m(…)`, `super.m(…)`, `super<I>.m(…)` and trailing
//! lambdas `f { … }`; as [`KotlinTarget::Unresolved`] calls on call results,
//! safe calls, labeled `this@L`, local variables and parameters, and every
//! call where an implicit receiver of unknown type may be in scope: inside
//! lambdas, anonymous and local functions, and extension functions.
//! Object expressions and local classes are skipped; calls inside string
//! templates are part of the string literal and not seen.

use std::collections::{BTreeMap, BTreeSet};

use super::symbols::{KotlinCallSite, KotlinTarget};
use super::tokens::{Code, KEYWORDS, Start};

const CODE_LIMIT: usize = 200;
/// Words before a name that declare it.
const DECLARING: [&str; 7] = [
    "fun",
    "val",
    "var",
    "class",
    "object",
    "interface",
    "typealias",
];
/// Standard scope functions: their lambdas rebind `this` or `it`.
const SCOPE_FUNCTIONS: [&str; 7] = [
    "let",
    "run",
    "with",
    "apply",
    "also",
    "takeIf",
    "takeUnless",
];
/// Keywords whose `( … )` may be followed by a braced control body.
const CONDITIONED: [&str; 5] = ["if", "while", "for", "catch", "when"];

/// The function a body belongs to.
pub(super) struct Owner<'a> {
    pub(super) class_id: &'a str,
    pub(super) method_id: &'a str,
    /// An extension function: `this` has a type the scanner does not know.
    pub(super) extension: bool,
}

/// (target, first token, token after the callee, last token) of a call.
type Found = (KotlinTarget, usize, usize, usize);

/// One function body with what a pre-pass found: local names, regions with
/// an unknown implicit receiver, nested scopes to skip and braces that are
/// control bodies rather than lambdas.
pub(super) struct Body<'c, 'a> {
    code: &'c Code<'a>,
    source: &'c str,
    from: usize,
    to: usize,
    openers: BTreeMap<usize, usize>,
    locals: BTreeSet<String>,
    regions: Vec<(usize, usize)>,
    skip: BTreeMap<usize, usize>,
    control: BTreeSet<usize>,
}

impl<'c, 'a> Body<'c, 'a> {
    pub(super) fn new(
        code: &'c Code<'a>,
        source: &'c str,
        (from, to): (usize, usize),
        params: (usize, usize),
    ) -> Self {
        let openers = code
            .pairs
            .range(from.saturating_sub(1)..to)
            .map(|(&open, &close)| (close, open))
            .collect();
        let mut body = Body {
            code,
            source,
            from,
            to,
            openers,
            locals: BTreeSet::new(),
            regions: Vec::new(),
            skip: BTreeMap::new(),
            control: BTreeSet::new(),
        };
        body.parameters(params);
        let mut i = from;
        while i < to {
            body.mark(i);
            i = body.skip.get(&i).copied().unwrap_or(i + 1);
        }
        body
    }

    /// Parameter names: names before `:` directly in the list.
    fn parameters(&mut self, (open, close): (usize, usize)) {
        let mut j = open + 1;
        while j < close {
            if matches!(self.code.text(j), "(" | "[" | "{") {
                j = self.code.pairs[&j];
            } else if self.code.is_name(j) && self.code.text(j + 1) == ":" {
                self.locals.insert(name(self.code, j));
            }
            j += 1;
        }
    }

    /// Record what the token at `i` declares, opens or hides.
    fn mark(&mut self, i: usize) {
        let code = self.code;
        if code.toks[i].literal || code.after_member(i) {
            return;
        }
        match code.text(i) {
            "object" => self.nested_class(i, i + 1),
            "class" | "interface" if code.is_name(i + 1) => self.nested_class(i, i + 2),
            "fun" => self.local_function(i),
            "when" => self.when(i),
            "{" if !self.is_control(i) => self.regions.push((i, code.pairs[&i])),
            _ => self.local_names(i),
        }
    }

    /// Variables `val`/`var` (also destructured), `for` and `catch` declare.
    fn local_names(&mut self, i: usize) {
        let code = self.code;
        match code.text(i) {
            "val" | "var" if code.text(i + 1) == "(" => self.names_in(i + 1, None),
            "val" | "var" if code.is_name(i + 1) => {
                self.locals.insert(name(code, i + 1));
            }
            "for" if code.text(i + 1) == "(" => self.names_in(i + 1, Some("in")),
            "catch" if code.is_name(i + 2) => {
                self.locals.insert(name(code, i + 2));
            }
            _ => {}
        }
    }

    /// Names inside the brackets at `open`, up to the word `until`.
    fn names_in(&mut self, open: usize, until: Option<&str>) {
        let close = self.code.pairs[&open];
        for k in open + 1..close {
            if Some(self.code.text(k)) == until {
                break;
            }
            if self.code.is_name(k) && matches!(self.code.text(k + 1), "," | ")" | ":" | "in") {
                self.locals.insert(name(self.code, k));
            }
        }
    }

    /// Skip a local class, interface or object (expression), whose header
    /// continues at `at`; a named one is a local name.
    fn nested_class(&mut self, i: usize, at: usize) {
        if self.code.is_name(i + 1) {
            self.locals.insert(name(self.code, i + 1));
        }
        let end = match self.code.body_start(at) {
            Start::Block(open) => self.code.pairs[&open] + 1,
            Start::Expr(end) | Start::None(end) => end,
        };
        self.skip.insert(i, end.min(self.to));
    }

    /// A local or anonymous function: its name is local, its header is
    /// skipped and its body may have a receiver of unknown type.
    fn local_function(&mut self, i: usize) {
        let code = self.code;
        let Some(open) = (code.text(i + 1) == "(")
            .then_some(i + 1)
            .or_else(|| code.params(i))
        else {
            return;
        };
        if open > i + 1 {
            self.locals.insert(name(self.code, open - 1));
        }
        let next = match code.body_start(code.pairs[&open] + 1) {
            Start::Block(brace) => {
                self.regions.push((brace, code.pairs[&brace]));
                brace + 1
            }
            Start::Expr(eq) => {
                self.regions.push((eq, self.line_end(eq)));
                eq + 1
            }
            Start::None(end) => end,
        };
        self.skip.insert(i, next.min(self.to));
    }

    /// The index after the expression that starts on the line of `at`.
    fn line_end(&self, at: usize) -> usize {
        let mut j = at + 1;
        while j < self.to && (j == at + 1 || !self.code.new_line(j)) {
            if matches!(self.code.text(j), "(" | "[" | "{") {
                j = self.code.pairs[&j];
            }
            j += 1;
        }
        j
    }

    /// Mark the body of the `when` at `i` and its braced branch bodies.
    fn when(&mut self, i: usize) {
        let code = self.code;
        let open = if code.text(i + 1) == "(" {
            code.pairs[&(i + 1)] + 1
        } else {
            i + 1
        };
        if code.text(open) != "{" {
            return;
        }
        self.control.insert(open);
        let mut k = open + 1;
        while k < code.pairs[&open] {
            match code.text(k) {
                "->" if code.text(k + 1) == "{" => {
                    self.control.insert(k + 1);
                }
                "(" | "[" | "{" => k = code.pairs[&k],
                _ => {}
            }
            k += 1;
        }
    }

    /// The brace at `k` opens a control body, not a lambda.
    fn is_control(&self, k: usize) -> bool {
        let before = self.code.text(k - 1);
        self.control.contains(&k)
            || matches!(before, "else" | "try" | "finally" | "do" | "when")
            || (before == ")"
                && self
                    .openers
                    .get(&(k - 1))
                    .is_some_and(|&o| CONDITIONED.contains(&self.code.text(o.wrapping_sub(1)))))
    }

    /// The call sites of the body, in source order.
    pub(super) fn sites(&self, owner: Owner) -> Vec<KotlinCallSite> {
        let mut out = Vec::new();
        let mut i = self.from;
        while i < self.to {
            if let Some(&end) = self.skip.get(&i) {
                i = end;
                continue;
            }
            match self.at(i, owner.extension) {
                Some(found) => {
                    i = found.2.max(i + 1);
                    out.push(self.site(found, &owner));
                }
                None => i += 1,
            }
        }
        out
    }

    fn site(&self, (target, start, callee_end, end): Found, owner: &Owner) -> KotlinCallSite {
        let toks = self.code.toks;
        let first = &toks[start];
        let line_start = self.source[..first.start].rfind('\n').map_or(0, |n| n + 1);
        let code = collapse(&self.source[first.start..toks[end].end])
            .chars()
            .take(CODE_LIMIT)
            .collect();
        KotlinCallSite {
            from_class: owner.class_id.into(),
            from_method: Some(owner.method_id.into()),
            target,
            label: collapse(&self.source[first.start..toks[callee_end - 1].end]),
            line: first.line,
            column: (first.start - line_start + 1) as u32,
            code,
        }
    }

    /// The call that starts at `i`.
    fn at(&self, i: usize, extension: bool) -> Option<Found> {
        if self.code.toks[i].literal {
            return None;
        }
        let unknown = extension || self.regions.iter().any(|&(a, b)| a < i && i < b);
        match self.code.text(i) {
            "." | "?." => self.member(i),
            "super" if self.code.text(i + 1) == "<" => self.qualified_super(i, unknown),
            _ => self.head(i, unknown),
        }
    }

    /// `.m(…)` or `?.m(…)` at `i` on a receiver that is no plain name chain.
    fn member(&self, i: usize) -> Option<Found> {
        if !self.code.is_name(i + 1) {
            return None;
        }
        let (callee_end, end) = self.call_open(i + 1)?;
        if self.code.text(i) == "." && self.plain_receiver(i) {
            return None;
        }
        Some((
            KotlinTarget::Unresolved,
            self.chain_start(i),
            callee_end,
            end,
        ))
    }

    /// `super<I>.m(…)` at `i`.
    fn qualified_super(&self, i: usize, unknown: bool) -> Option<Found> {
        let code = self.code;
        let shaped = code.is_name(i + 2)
            && code.text(i + 3) == ">"
            && code.text(i + 4) == "."
            && code.is_name(i + 5);
        if !shaped {
            return None;
        }
        let (callee_end, end) = self.call_open(i + 5)?;
        let target = if unknown {
            KotlinTarget::Unresolved
        } else {
            KotlinTarget::Super {
                qualifier: Some(name(self.code, i + 2)),
                name: name(self.code, i + 5),
            }
        };
        Some((target, i, callee_end, end))
    }

    /// A call whose callee is the dotted name chain that starts at `i`.
    fn head(&self, i: usize, unknown: bool) -> Option<Found> {
        let code = self.code;
        let word = code.text(i);
        let before = code.text(i.checked_sub(1)?);
        if !code.is_name(i)
            || matches!(before, "." | "?." | "::" | "@")
            || DECLARING.contains(&before)
            || (KEYWORDS.contains(&word) && !matches!(word, "this" | "super"))
        {
            return None;
        }
        let mut segments = vec![name(self.code, i)];
        let mut last = i;
        while code.text(last + 1) == "." && code.is_name(last + 2) {
            last += 2;
            segments.push(name(self.code, last));
        }
        let (callee_end, end) = self.call_open(last)?;
        let target = if unknown {
            KotlinTarget::Unresolved
        } else {
            self.classify(segments)
        };
        Some((target, i, callee_end, end))
    }

    fn classify(&self, segments: Vec<String>) -> KotlinTarget {
        match segments.as_slice() {
            [this, name] if this == "this" => KotlinTarget::This(name.clone()),
            [sup, name] if sup == "super" => KotlinTarget::Super {
                qualifier: None,
                name: name.clone(),
            },
            [first, ..] if first == "this" || first == "super" || self.locals.contains(first) => {
                KotlinTarget::Unresolved
            }
            [only] if SCOPE_FUNCTIONS.contains(&only.as_str()) => KotlinTarget::Unresolved,
            [only] => KotlinTarget::Name(only.clone()),
            _ => KotlinTarget::Path(segments),
        }
    }

    /// The callee name at `n` is called: (token after the callee, last token
    /// of the call), over type arguments, arguments and a trailing lambda.
    fn call_open(&self, n: usize) -> Option<(usize, usize)> {
        let code = self.code;
        let mut j = n + 1;
        if code.text(j) == "<" {
            j = type_arguments_end(code, j, self.to)? + 1;
        }
        match code.text(j) {
            "(" => {
                let close = code.pairs[&j];
                let lambda = code.text(close + 1) == "{" && !code.new_line(close + 1);
                Some((
                    n + 1,
                    if lambda {
                        code.pairs[&(close + 1)]
                    } else {
                        close
                    },
                ))
            }
            "{" if !code.new_line(j) => Some((n + 1, code.pairs[&j])),
            _ => None,
        }
    }

    /// The `.` at `i` continues a plain name chain whose head starts the
    /// call (or an annotation), so no separate site starts here.
    fn plain_receiver(&self, i: usize) -> bool {
        let code = self.code;
        let mut k = i - 1;
        if !code.is_name(k) {
            return false;
        }
        while code.text(k - 1) == "." && code.is_name(k - 2) {
            k -= 2;
        }
        match code.text(k - 1) {
            "." | "?." | "::" => false,
            // `this@Label.m()` is labeled; `@a.B(…)` is an annotation.
            "@" => !(code.is_name(k - 2) && adjacent(code, k - 2, k - 1)),
            _ => true,
        }
    }

    /// The first token of the receiver chain that ends before `i`.
    fn chain_start(&self, i: usize) -> usize {
        let code = self.code;
        let mut s = i;
        while s > self.from {
            let p = s - 1;
            match code.text(p) {
                ")" | "]" | "}" => match self.openers.get(&p) {
                    Some(&open) => s = open,
                    None => break,
                },
                "." | "?." | "!!" => s = p,
                "@" if p > 0 && code.is_name(p - 1) && adjacent(code, p - 1, p) => s = p,
                _ if code.is_name(p) || code.toks[p].literal => {
                    s = p;
                    let before = code.text(p.wrapping_sub(1));
                    if !matches!(before, "." | "?." | "@") {
                        break;
                    }
                }
                _ => break,
            }
        }
        s
    }
}

fn name(code: &Code, i: usize) -> String {
    code.toks[i].text.trim_matches('`').to_string()
}

/// The `>` closing type arguments that open at `open` on one line.
fn type_arguments_end(code: &Code, open: usize, to: usize) -> Option<usize> {
    let mut depth = 0usize;
    for k in open..to {
        if k > open && code.new_line(k) {
            return None;
        }
        match code.text(k) {
            "<" => depth += 1,
            ">" => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            "." | "," | "?" | "*" => {}
            _ if code.is_name(k) => {}
            _ => return None,
        }
    }
    None
}

fn adjacent(code: &Code, a: usize, b: usize) -> bool {
    code.toks[a].end == code.toks[b].start
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
