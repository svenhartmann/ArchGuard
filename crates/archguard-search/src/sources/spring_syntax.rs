//! The Java/Kotlin syntax `spring.rs` reads: annotations, their arguments, imports and classes,
//! over text whose comments and literal contents are blanked (`lang_java::mask`, same offsets).
use archguard_flow_common::lang_code::split_args;
use archguard_flow_common::lang_java::mask;
use archguard_flow_common::lang_text::{ident_end, idents, matching, offset};

const SPRING: &str = "org.springframework.data.elasticsearch.annotations";
const OPENSEARCH: &str = "org.opensearch.data";

/// `@Name(args)`: offsets of its `@`, of the text between the parentheses, and of its end.
pub(super) struct Ann {
    pub(super) name: String,
    pub(super) start: usize,
    pub(super) args: Option<(usize, usize)>,
    pub(super) end: usize,
}

/// `key = value` as written; a lone value has the key `value`. `at` is the value's offset.
pub(super) struct Arg<'a> {
    pub(super) key: &'a str,
    pub(super) val: &'a str,
    pub(super) at: usize,
}

/// A class with a `{}` body.
pub(super) struct Class {
    pub(super) name: String,
    pub(super) keyword: usize,
    pub(super) name_end: usize,
    pub(super) close: usize,
}

/// One source file: original and masked text, its annotations, imports and classes.
pub(super) struct Scan<'t> {
    pub(super) text: &'t str,
    pub(super) m: String,
    pub(super) anns: Vec<Ann>,
    imports: Vec<String>,
    pub(super) classes: Vec<Class>,
    pub(super) kotlin: bool,
}

impl<'t> Scan<'t> {
    pub(super) fn new(text: &'t str, kotlin: bool) -> Self {
        let m = mask(text);
        let imports = m
            .lines()
            .filter_map(|l| l.trim().strip_prefix("import "))
            .map(|i| {
                i.trim()
                    .trim_end_matches(';')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        Self {
            text,
            anns: annotations(&m),
            classes: classes(&m),
            m,
            imports,
            kotlin,
        }
    }

    /// The flavor (`""` or `opensearch`) of the import that provides annotation `name`.
    pub(super) fn provider(&self, name: &str) -> Option<&'static str> {
        self.imports.iter().find_map(|i| {
            let (pkg, last) = i.rsplit_once('.')?;
            let flavor = if pkg.starts_with(OPENSEARCH) {
                "opensearch"
            } else if pkg == SPRING {
                ""
            } else {
                return None;
            };
            (last == name || last == "*").then_some(flavor)
        })
    }

    pub(super) fn args_of(&self, a: &Ann) -> Vec<Arg<'_>> {
        a.args
            .map(|r| args(&self.m, self.text, r))
            .unwrap_or_default()
    }

    /// The arguments of the annotation call that starts at or after `from`.
    pub(super) fn call_args(&self, from: usize) -> Vec<Arg<'_>> {
        let Some(open) = self.m[from..].find('(').map(|p| p + from) else {
            return vec![];
        };
        matching(self.m.as_bytes(), open)
            .map_or(vec![], |c| args(&self.m, self.text, (open + 1, c)))
    }

    /// The annotations that directly precede the class keyword (modifiers between are fine).
    pub(super) fn head(&self, class: &Class) -> Vec<&Ann> {
        let mut out: Vec<&Ann> = vec![];
        let mut next = class.keyword;
        for a in self.anns.iter().rev().filter(|a| a.end <= class.keyword) {
            if self.m[a.end..next].contains([';', '{', '}']) {
                break;
            }
            out.push(a);
            next = a.start;
        }
        out
    }

    /// The property that the annotations from index `i` on (a chain) are put on.
    pub(super) fn property(&self, i: usize) -> Option<String> {
        let mut last = &self.anns[i];
        for a in &self.anns[i + 1..] {
            if !self.m[last.end..a.start].trim().is_empty() {
                break;
            }
            last = a;
        }
        let stop: &[char] = if self.kotlin {
            &[';', '=', ',', ')', '{', '@']
        } else {
            &[';', '=', '(', '{', '@']
        };
        let head = self.m[last.end..].split(stop).next()?;
        let toks: Vec<&str> = head.split_whitespace().collect();
        let name = if self.kotlin {
            let at = toks.iter().position(|t| matches!(*t, "val" | "var"))?;
            toks.get(at + 1)?.split(':').next()?
        } else {
            toks.last()?
        };
        let ident = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        ident.then(|| name.to_owned())
    }
}

pub(super) fn find<'x, 'a>(args: &'x [Arg<'a>], key: &str) -> Option<&'x Arg<'a>> {
    args.iter().find(|a| a.key == key)
}

fn args<'a>(m: &'a str, t: &'a str, (a, z): (usize, usize)) -> Vec<Arg<'a>> {
    split_args(&m[a..z])
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let at = offset(m, p);
            let (key, from) = p
                .find('=')
                .map_or(("value", at), |e| (p[..e].trim(), at + e + 1));
            let val = t[from..at + p.len()].trim();
            Arg {
                key,
                val,
                at: offset(t, val),
            }
        })
        .collect()
}

/// Annotations at top level; nested ones are read from their parent's arguments.
fn annotations(m: &str) -> Vec<Ann> {
    let (mut found, mut k) = (vec![], 0);
    while let Some(p) = m[k..].find('@') {
        let start = k + p;
        let mut at = start + 1;
        // `@field:Field` and `@get:Field`: the use-site target is no name.
        if m[at..].starts_with("field:") || m[at..].starts_with("get:") {
            at += m[at..].find(':').unwrap_or(0) + 1;
        }
        let end_name = dotted_end(m, at);
        let name = m[at..end_name].rsplit('.').next().unwrap_or("");
        if name.is_empty() {
            k = start + 1;
            continue;
        }
        let args = (m.as_bytes().get(end_name) == Some(&b'('))
            .then(|| matching(m.as_bytes(), end_name).map(|c| (end_name + 1, c)))
            .flatten();
        let end = args.map_or(end_name, |a| a.1 + 1);
        found.push(Ann {
            name: name.to_owned(),
            start,
            args,
            end,
        });
        k = end;
    }
    found
}

/// End of the dotted identifier starting at `from`.
fn dotted_end(m: &str, from: usize) -> usize {
    let b = m.as_bytes();
    let mut k = ident_end(b, from);
    while b.get(k) == Some(&b'.') && ident_end(b, k + 1) > k + 1 {
        k = ident_end(b, k + 1);
    }
    k
}

fn classes(m: &str) -> Vec<Class> {
    idents(m, 0, m.len())
        .windows(2)
        .filter(|w| matches!(w[0].1, "class" | "record") && !m[..w[0].0].ends_with('.'))
        .filter_map(|w| {
            let name_end = w[1].0 + w[1].1.len();
            let open = m[name_end..].find('{')? + name_end;
            Some(Class {
                name: w[1].1.to_owned(),
                keyword: w[0].0,
                name_end,
                close: matching(m.as_bytes(), open)?,
            })
        })
        .collect()
}
