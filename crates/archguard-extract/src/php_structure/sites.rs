//! Which call, if any, starts at a token: `new C`, `C::m()`, `$this->m()`,
//! `f()`, and as [`PhpTarget::Unresolved`] calls on other receivers,
//! variable functions (`$f()`), dynamic member names (`$o->$m()`,
//! `$o->{…}()`, `C::$m()`) and calls of call results or array elements.

use super::names::Names;
use super::symbols::{PhpClassRef, PhpTarget};
use super::tokens::{View, is_keyword};

/// (target, first token, token after the callee, last token) of a call.
pub(super) type Found = (PhpTarget, usize, usize, usize);

pub(super) struct Sites<'a> {
    pub(super) view: &'a View<'a>,
    pub(super) names: &'a Names,
}

impl Sites<'_> {
    /// The call that starts at `i`; `this` when `$this`, `self`, `static`
    /// and `parent` name the enclosing class.
    pub(super) fn at(&self, i: usize, this: bool) -> Option<Found> {
        let v = self.view;
        if i > 0 && matches!(v.text(i - 1), "->" | "?->" | "::") {
            return None;
        }
        let word = v.text(i);
        if word.eq_ignore_ascii_case("new") {
            return self.new_at(i, this);
        }
        if matches!(word, "->" | "?->") {
            return self.member_call(i, this);
        }
        if word.starts_with('$') && v.text(i + 1) == "(" {
            return Some((PhpTarget::Unresolved, i, i + 1, v.pairs[&(i + 1)]));
        }
        if word == "(" && self.calls_result(i) {
            return Some((PhpTarget::Unresolved, self.chain_start(i), i, v.pairs[&i]));
        }
        self.name_call(i, this)
    }

    /// `->m(…)` at the operator `i`.
    fn member_call(&self, i: usize, this: bool) -> Option<Found> {
        let v = self.view;
        let open = self.member_open(i + 1)?;
        let close = v.pairs[&open];
        if i > 0 && v.text(i - 1) == "$this" {
            let target = match v.word(i + 1) {
                Some(name) if this => PhpTarget::This(name.text.clone()),
                _ => PhpTarget::Unresolved,
            };
            return Some((target, i - 1, open, close));
        }
        Some((PhpTarget::Unresolved, self.chain_start(i), open, close))
    }

    /// `f(…)` or `C::m(…)` at the name that starts at `i`.
    fn name_call(&self, i: usize, this: bool) -> Option<Found> {
        let v = self.view;
        if !v.starts_name(i) || self.declares(i) {
            return None;
        }
        let (name, k) = v.qname(i)?;
        if v.text(k) == "::" {
            let open = self.member_open(k + 1)?;
            let class = self.names.class(&name);
            let target = match v.word(k + 1) {
                Some(member) if this || matches!(class, PhpClassRef::Named(_)) => {
                    PhpTarget::Static(class, member.text.clone())
                }
                _ => PhpTarget::Unresolved,
            };
            return Some((target, i, open, v.pairs[&open]));
        }
        if v.text(k) != "(" || (!name.contains('\\') && is_keyword(&name)) {
            return None;
        }
        Some((self.names.function(&name), i, k, v.pairs[&k]))
    }

    /// The `(` after a member name at `m` (`m`, `$m` or `{…}`), if called.
    fn member_open(&self, m: usize) -> Option<usize> {
        let v = self.view;
        let open = match v.text(m) {
            "{" => v.pairs[&m] + 1,
            name if name.starts_with('$') || v.word(m).is_some() => m + 1,
            _ => return None,
        };
        (v.text(open) == "(").then_some(open)
    }

    /// A name at `i` that a declaration or `new` introduces.
    fn declares(&self, i: usize) -> bool {
        let Some(p) = i.checked_sub(1) else {
            return false;
        };
        let before = self.view.text(p).to_ascii_lowercase();
        matches!(before.as_str(), "function" | "fn" | "const" | "new")
            || (before == "&" && p > 0 && self.view.is(p - 1, "function"))
    }

    fn new_at(&self, i: usize, this: bool) -> Option<Found> {
        let v = self.view;
        let j = i + 1;
        if v.is(j, "class") {
            return None;
        }
        let (target, after) = if let Some((name, k)) = v.qname(j) {
            let class = self.names.class(&name);
            let known = this || matches!(class, PhpClassRef::Named(_));
            let target = if known {
                PhpTarget::New(class)
            } else {
                PhpTarget::Unresolved
            };
            (target, k)
        } else if v.text(j).starts_with('$') {
            (PhpTarget::Unresolved, j + 1)
        } else if v.text(j) == "(" {
            (PhpTarget::Unresolved, v.pairs[&j] + 1)
        } else {
            return None;
        };
        let end = if v.text(after) == "(" {
            v.pairs[&after]
        } else {
            after - 1
        };
        Some((target, i, after, end))
    }

    /// `(` at `i` calls the result of a call or an array element.
    fn calls_result(&self, i: usize) -> bool {
        let v = self.view;
        let Some(p) = i.checked_sub(1) else {
            return false;
        };
        match v.text(p) {
            "]" => !v.in_attr[p],
            ")" => v.openers[&p].checked_sub(1).is_some_and(|b| {
                let before = v.text(b);
                before.starts_with('$')
                    || matches!(before, ")" | "]")
                    || v.word(b).is_some_and(|t| !is_keyword(&t.text))
            }),
            _ => false,
        }
    }

    /// The first token of the receiver chain that ends before `i`.
    fn chain_start(&self, i: usize) -> usize {
        let v = self.view;
        let mut s = i;
        while let Some(p) = s.checked_sub(1) {
            let word = v.text(p);
            if matches!(word, ")" | "]") && !v.in_attr[p] {
                s = v.openers[&p];
            } else if matches!(word, "->" | "?->" | "::") || (word == "\\" && v.adjacent(p, s)) {
                s = p;
            } else if self.receiver(p) {
                s = p;
                match p.checked_sub(1).map(|q| v.text(q)) {
                    Some("->" | "?->" | "::") => {}
                    Some("\\") if v.adjacent(p - 1, p) => {}
                    Some(new) if new.eq_ignore_ascii_case("new") => return p - 1,
                    _ => return s,
                }
            } else {
                return s;
            }
        }
        s
    }

    /// A variable or a name (`self`, `static`, `parent`, no other keyword).
    fn receiver(&self, p: usize) -> bool {
        self.view.text(p).starts_with('$')
            || self.view.word(p).is_some_and(|t| {
                !is_keyword(&t.text) || ["self", "static", "parent"].iter().any(|k| t.is(k))
            })
    }
}
