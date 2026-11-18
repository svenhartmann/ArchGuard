//! Call sites in one Scala def body, with names as written. Only calls with
//! an argument list are sites: `f(…)`, `a.b.C.m(…)`, `C[T](…)`, `new C(…)`
//! and `new C`; parameterless calls (`xs.size`), infix and operator calls
//! (`a max b`, `a + b`) and block arguments without parentheses
//! (`Future { … }`) cannot be told from field reads or are not recorded.
//! Extractor patterns (`case Some(x) =>`) and annotations are no calls.

use super::symbols::{ScalaCallSite, ScalaTarget};
use super::tokens::{Code, KEYWORDS};

const CODE_LIMIT: usize = 200;

/// The def whose body is scanned.
pub(super) struct Owner<'a> {
    pub(super) class_id: &'a str,
    pub(super) method_id: &'a str,
    /// The def is inside an `extension` block: other extension methods of
    /// the block are in scope by their bare name, so nothing resolves.
    pub(super) extension: bool,
}

/// The call sites of the body tokens `from..to`.
pub(super) fn collect(
    code: &Code,
    source: &str,
    (from, to): (usize, usize),
    owner: Owner,
) -> Vec<ScalaCallSite> {
    let mut out = Vec::new();
    let mut pattern = false;
    let mut i = from;
    while i < to {
        if code.toks[i].literal {
            i += 1;
            continue;
        }
        match code.text(i) {
            "case" => pattern = true,
            "=>" => pattern = false,
            // `@name(…)` is an annotation; `def name(…)` and `class C(…)` declare.
            "@" | "def" | "class" | "object" | "trait" | "val" | "var" | "type" => {
                i += 2;
                continue;
            }
            _ => {}
        }
        if pattern {
            i += 1;
            continue;
        }
        let found = call_at(code, i);
        match found {
            Some((target, start, callee_end, close, next)) => {
                let target = if owner.extension {
                    ScalaTarget::Unresolved
                } else {
                    target
                };
                out.push(site(
                    code,
                    source,
                    &owner,
                    target,
                    (start, callee_end, close),
                ));
                i = next;
            }
            None => i += 1,
        }
    }
    out
}

/// The call that starts at `i`:
/// (target, start, callee end, closing token, where to go on).
fn call_at(code: &Code, i: usize) -> Option<(ScalaTarget, usize, usize, usize, usize)> {
    match code.text(i) {
        "new" if code.is_name(i + 1) => {
            let (names, end) = code.dotted(i + 1);
            let close = args_end(code, end).unwrap_or(end - 1);
            Some((ScalaTarget::New(names), i, end, close, end))
        }
        // `.m(…)` on a call result or another receiver that is no name chain.
        "." if code.is_name(i + 1) => {
            args_end(code, i + 2).map(|close| (ScalaTarget::Unresolved, i + 1, i + 2, close, i + 2))
        }
        _ if code.is_name(i) && !code.after_member(i) => head(code, i),
        _ => None,
    }
}

/// A call whose callee chain starts with the name at `i`:
/// (target, start, callee end, closing token, where to go on).
fn head(code: &Code, i: usize) -> Option<(ScalaTarget, usize, usize, usize, usize)> {
    let first = code.text(i);
    if first == "super" {
        let (qualifier, dot) = if code.text(i + 1) == "[" {
            let close = code.pairs[&(i + 1)];
            (code.is_name(i + 2).then(|| code.name(i + 2)), close + 1)
        } else {
            (None, i + 1)
        };
        if code.text(dot) != "." || !code.is_name(dot + 1) {
            return None;
        }
        let close = args_end(code, dot + 2)?;
        let name = code.name(dot + 1);
        return Some((
            ScalaTarget::Super { qualifier, name },
            i,
            dot + 2,
            close,
            dot + 2,
        ));
    }
    if KEYWORDS.contains(&first) && first != "this" {
        return None;
    }
    let (names, end) = code.dotted(i);
    let close = args_end(code, end)?;
    let target = match (first, names.len()) {
        ("this", 1) => return None,
        ("this", 2) => ScalaTarget::This(names[1].clone()),
        ("this", _) => ScalaTarget::Unresolved,
        (_, 1) => ScalaTarget::Name(names[0].clone()),
        _ => ScalaTarget::Path(names),
    };
    Some((target, i, end, close, end))
}

/// The closing `)` of the argument list at `at`, after optional type
/// arguments `[…]`; `None` when no argument list follows.
fn args_end(code: &Code, at: usize) -> Option<usize> {
    let open = if code.text(at) == "[" {
        code.pairs[&at] + 1
    } else {
        at
    };
    (code.text(open) == "(").then(|| code.pairs[&open])
}

fn site(
    code: &Code,
    source: &str,
    owner: &Owner,
    target: ScalaTarget,
    (start, callee_end, close): (usize, usize, usize),
) -> ScalaCallSite {
    let toks = code.toks;
    let first = &toks[start];
    let line_start = source[..first.start].rfind('\n').map_or(0, |n| n + 1);
    ScalaCallSite {
        from_class: owner.class_id.into(),
        from_method: Some(owner.method_id.into()),
        target,
        label: collapse(&source[first.start..toks[callee_end - 1].end]),
        line: first.line,
        column: (first.start - line_start + 1) as u32,
        code: collapse(&source[first.start..toks[close].end])
            .chars()
            .take(CODE_LIMIT)
            .collect(),
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
