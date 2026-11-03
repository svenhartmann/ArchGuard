//! Static memory growth points: candidates, never proof. Whether a cache
//! fills, a collection leaks or a read-all call loads gigabytes depends on
//! the data at runtime, which source code cannot show.
//!
//! One token walk serves every language; only the API names differ, and they
//! live in the per-language tables below. Comments and strings never match.
//! Test files are the caller's concern: the scan sees one file at a time.

use crate::lex::{Flavor, Token, tokens};
use crate::walk::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryKind {
    /// A cache without a size or expiry limit: it keeps every entry it saw.
    UnboundedCache,
    /// A long-lived collection the file adds to but never removes from.
    GrowOnlyCollection,
    /// A whole file, stream, body or result set read into memory at once.
    ReadAll,
    /// A buffer whose size is not a constant, so input decides the allocation.
    VariableBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Mostly correct, but the construct can be harmless or misread.
    Medium,
    /// The construct is unambiguous in this language.
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub kind: MemoryKind,
    pub line: u32,
    /// The recognized construct, e.g. `io.ReadAll` or `ConcurrentHashMap CACHE`.
    pub construct: String,
    pub confidence: Confidence,
}

use Confidence::{High, Medium};

type Rule = (&'static [&'static str], Confidence);

/// Calls that load everything at once. A pattern is a token sequence.
fn read_all_rules(lang: Lang) -> &'static [Rule] {
    match lang {
        Lang::Java => &[
            (&[".", "readAllBytes", "("], High),
            (&["Files", ".", "readAllLines", "("], High),
            (&["Files", ".", "readString", "("], High),
            (&[".", "findAll", "("], Medium),
            (&[".", "getResultList", "("], Medium),
        ],
        Lang::CSharp => &[
            (&["File", ".", "ReadAllBytes", "("], High),
            (&["File", ".", "ReadAllBytesAsync", "("], High),
            (&["File", ".", "ReadAllText", "("], High),
            (&["File", ".", "ReadAllTextAsync", "("], High),
            (&["File", ".", "ReadAllLines", "("], High),
            (&["File", ".", "ReadAllLinesAsync", "("], High),
            (&[".", "ReadToEnd", "("], High),
            (&[".", "ReadToEndAsync", "("], High),
            (&[".", "ReadAsStringAsync", "("], High),
            (&[".", "ReadAsByteArrayAsync", "("], High),
            (&[".", "ToListAsync", "("], Medium),
            (&[".", "ToArrayAsync", "("], Medium),
        ],
        Lang::Go => &[
            (&["io", ".", "ReadAll", "("], High),
            (&["ioutil", ".", "ReadAll", "("], High),
            (&["os", ".", "ReadFile", "("], High),
            (&["ioutil", ".", "ReadFile", "("], High),
        ],
        Lang::Py => &[
            (&[".", "fetchall", "("], High),
            (&[".", "read", "(", ")"], Medium),
            (&[".", "readlines", "(", ")"], Medium),
            (&[".", "read_bytes", "(", ")"], Medium),
            (&[".", "read_text", "("], Medium),
            (&["json", ".", "load", "("], Medium),
        ],
        Lang::Ts | Lang::Js => &[
            (&["readFileSync", "("], High),
            (&[".", "readFile", "("], Medium),
            (&[".", "arrayBuffer", "(", ")"], Medium),
        ],
        Lang::Rust => &[
            (&[".", "read_to_end", "("], High),
            (&[".", "read_to_string", "("], High),
            (&["fs", ":", ":", "read", "("], High),
            (&["fs", ":", ":", "read_to_string", "("], High),
            (&["to_bytes", "("], Medium),
        ],
        Lang::Php => &[
            (&["file_get_contents", "("], High),
            (&["stream_get_contents", "("], High),
            (&["fetchAll", "("], Medium),
        ],
        // No Kotlin rules yet: no candidates rather than guessed ones.
        Lang::Kotlin => &[],
    }
}

/// Collection types whose instances can grow without bound. Weak-keyed maps
/// are left out because the garbage collector trims them; queues belong to
/// the concurrency scan.
const COLLECTIONS: &[&str] = &[
    // Java
    "HashMap",
    "ConcurrentHashMap",
    "LinkedHashMap",
    "TreeMap",
    "ConcurrentSkipListMap",
    "ArrayList",
    "LinkedList",
    "HashSet",
    "LinkedHashSet",
    "TreeSet",
    "CopyOnWriteArrayList",
    // C#
    "Dictionary",
    "ConcurrentDictionary",
    "SortedDictionary",
    "List",
    "SortedSet",
    "ConcurrentBag",
    // Rust
    "BTreeMap",
    "BTreeSet",
    "Vec",
    "VecDeque",
    "DashMap",
    "IndexMap",
];

/// Methods that add to a collection, across languages.
const GROWS: &[&str] = &[
    "add",
    "addAll",
    "put",
    "putAll",
    "putIfAbsent",
    "computeIfAbsent",
    "compute",
    "merge",
    "push",
    "append",
    "extend",
    "insert",
    "set",
    "setdefault",
    "update",
    "unshift",
    "push_back",
    "push_front",
    "entry",
    "Add",
    "AddRange",
    "TryAdd",
    "GetOrAdd",
    "AddOrUpdate",
    "Insert",
    "Push",
    "Store",
    "LoadOrStore",
];

/// Methods that remove from or reset a collection, across languages.
const SHRINKS: &[&str] = &[
    "remove",
    "removeIf",
    "removeAll",
    "retainAll",
    "clear",
    "pop",
    "popitem",
    "discard",
    "delete",
    "shift",
    "splice",
    "truncate",
    "drain",
    "retain",
    "pop_front",
    "pop_back",
    "swap_remove",
    "invalidate",
    "invalidateAll",
    "Remove",
    "TryRemove",
    "RemoveAt",
    "RemoveAll",
    "RemoveWhere",
    "Clear",
    "Pop",
    "Delete",
    "LoadAndDelete",
];

/// Spring stereotypes: their beans are singletons, so instance fields live
/// as long as the application.
const SPRING_BEANS: &[&str] = &[
    "Service",
    "Component",
    "Repository",
    "RestController",
    "Controller",
    "Configuration",
];

/// Code tokens without line breaks, with bracket depths. String literals keep
/// their slot but never match, so `"io.ReadAll("` cannot pose as a call.
struct Code<'a> {
    source: &'a str,
    tokens: Vec<&'a Token>,
    /// `{` depth before each token.
    braces: Vec<usize>,
    /// `(` and `[` depth before each token.
    parens: Vec<usize>,
}

impl<'a> Code<'a> {
    fn new(source: &'a str, all: &'a [Token]) -> Self {
        let tokens: Vec<&Token> = all.iter().filter(|t| t.text != "\n").collect();
        let (mut braces, mut parens) = (Vec::new(), Vec::new());
        let (mut b, mut p) = (0usize, 0usize);
        for t in &tokens {
            braces.push(b);
            parens.push(p);
            if t.literal {
                continue;
            }
            match t.text.as_str() {
                "{" => b += 1,
                "}" => b = b.saturating_sub(1),
                "(" | "[" => p += 1,
                ")" | "]" => p = p.saturating_sub(1),
                _ => {}
            }
        }
        Code {
            source,
            tokens,
            braces,
            parens,
        }
    }
    fn t(&self, i: usize) -> &str {
        self.tokens
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }
    fn line(&self, i: usize) -> u32 {
        self.tokens.get(i).map_or(0, |t| t.line)
    }
    fn column(&self, i: usize) -> usize {
        let start = self.tokens[i].start;
        self.source[..start]
            .rfind('\n')
            .map_or(start, |n| start - n - 1)
    }
    fn is(&self, i: usize, pattern: &[&str]) -> bool {
        pattern.iter().enumerate().all(|(k, p)| self.t(i + k) == *p)
    }
    /// Index after the bracket that closes the one at `i`.
    fn close(&self, i: usize) -> usize {
        let mut depth = 0usize;
        for j in i..self.tokens.len() {
            match self.t(j) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return j + 1;
                    }
                }
                _ => {}
            }
        }
        self.tokens.len()
    }
    /// A plain `=`: not `==`, `!=`, `<=`, `>=`, `=>`, `+=` or `:=`.
    fn assigns(&self, i: usize) -> bool {
        self.initializes(i) && (i == 0 || self.t(i - 1) != ">")
    }
    /// Like [`Self::assigns`], but `>=` may close a generic type, as in
    /// `Map<K, V> = …`. Only declaration contexts may use this.
    fn initializes(&self, i: usize) -> bool {
        self.t(i) == "="
            && !matches!(self.t(i + 1), "=" | ">")
            && (i == 0
                || !matches!(
                    self.t(i - 1),
                    "=" | "!" | "<" | "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | ":"
                ))
    }
    fn mentions(&self, word: &str) -> bool {
        (0..self.tokens.len()).any(|i| self.t(i) == word)
    }
}

/// Every memory growth candidate in `source`, ordered by line.
pub fn scan_memory(source: &str, lang: Lang) -> Vec<MemoryCandidate> {
    let flavor = match lang {
        Lang::Java | Lang::CSharp | Lang::Go => Flavor::C,
        Lang::Py => Flavor::Python,
        Lang::Ts | Lang::Js => Flavor::Js,
        Lang::Rust => Flavor::Rust,
        Lang::Php => Flavor::Php,
        Lang::Kotlin => Flavor::Kotlin,
    };
    let all = tokens(source, flavor);
    let code = Code::new(source, &all);
    let mut found = Vec::new();
    for i in 0..code.tokens.len() {
        for (pattern, confidence) in read_all_rules(lang) {
            if code.is(i, pattern) && !paged(&code, i) && !limited(&code, i, pattern.len()) {
                let name: String = pattern
                    .iter()
                    .filter(|p| !matches!(**p, "(" | ")"))
                    .copied()
                    .collect();
                found.push(candidate(MemoryKind::ReadAll, &code, i, name, *confidence));
            }
        }
        unbounded_cache(&code, i, lang, &mut found);
        variable_buffer(&code, i, lang, &mut found);
    }
    grow_only(&code, lang, &mut found);
    found.sort_by(|a, b| (a.line, a.kind, &a.construct).cmp(&(b.line, b.kind, &b.construct)));
    found.dedup();
    found
}

/// A query that loads one page, not the whole table:
/// `.Take(n).ToListAsync()`, `.setMaxResults(n).getResultList()` or
/// `findAll(pageable)`. `i` is the `.` before the method.
fn paged(c: &Code, i: usize) -> bool {
    if !matches!(
        c.t(i + 1),
        "ToListAsync" | "ToArrayAsync" | "findAll" | "getResultList"
    ) {
        return false;
    }
    let close = c.close(i + 2);
    if (i + 3..close).any(|j| c.t(j).to_ascii_lowercase().contains("page")) {
        return true;
    }
    (0..i)
        .rev()
        .take_while(|&j| !matches!(c.t(j), ";" | "{" | "}"))
        .any(|j| j > 0 && c.t(j - 1) == "." && matches!(c.t(j), "Take" | "setMaxResults"))
}

/// A read capped by its source: `reader.take(max).read_to_end(…)` or
/// `io.ReadAll(io.LimitReader(r, max))`.
fn limited(c: &Code, i: usize, len: usize) -> bool {
    let close = c.close(i + len - 1);
    let args = (i + len..close).any(|j| matches!(c.t(j), "LimitReader" | "MaxBytesReader"));
    args || (0..i)
        .rev()
        .take_while(|&j| !matches!(c.t(j), ";" | "{" | "}"))
        .any(|j| c.is(j, &[".", "take", "("]))
}

fn candidate(
    kind: MemoryKind,
    code: &Code,
    i: usize,
    construct: String,
    confidence: Confidence,
) -> MemoryCandidate {
    MemoryCandidate {
        kind,
        line: code.line(i),
        construct,
        confidence,
    }
}

fn unbounded_cache(c: &Code, i: usize, lang: Lang, f: &mut Vec<MemoryCandidate>) {
    match lang {
        Lang::Java
            if matches!(c.t(i), "Caffeine" | "CacheBuilder")
                && c.is(i + 1, &[".", "newBuilder", "("]) =>
        {
            // The builder chain ends at the statement's `;`.
            let mut built = false;
            let mut j = i;
            while j < c.tokens.len() && !(c.t(j) == ";" && c.parens[j] <= c.parens[i]) {
                match c.t(j) {
                    "maximumSize" | "maximumWeight" | "expireAfterWrite" | "expireAfterAccess"
                    | "expireAfter" | "weakKeys" | "weakValues" | "softValues" => return,
                    "build" | "buildAsync" => built = true,
                    _ => {}
                }
                j += 1;
            }
            // Without `build` the chain may continue elsewhere with a limit.
            let confidence = if built { High } else { Medium };
            let name = format!("{}.newBuilder", c.t(i));
            f.push(candidate(
                MemoryKind::UnboundedCache,
                c,
                i,
                name,
                confidence,
            ));
        }
        Lang::CSharp => {
            let site = if c.is(i, &["new", "MemoryCache", "("]) {
                "new MemoryCache"
            } else if c.is(i, &["AddMemoryCache", "("]) {
                "AddMemoryCache"
            } else {
                return;
            };
            // The limit is often configured in another statement.
            if !c.mentions("SizeLimit") {
                f.push(candidate(
                    MemoryKind::UnboundedCache,
                    c,
                    i,
                    site.into(),
                    Medium,
                ));
            }
        }
        Lang::Py if c.t(i) == "@" => {
            let at = if c.is(i + 1, &["functools", "."]) {
                i + 3
            } else {
                i + 1
            };
            let unbounded = match c.t(at) {
                "cache" => matches!(c.t(at + 1), "def" | "async" | "@"),
                "lru_cache" => {
                    c.is(at + 1, &["(", "None"])
                        || (c.t(at + 1) == "("
                            && (at + 2..c.close(at + 1))
                                .any(|j| c.is(j, &["maxsize", "=", "None"])))
                }
                _ => false,
            };
            if unbounded {
                let name = format!("@{}", c.t(at));
                f.push(candidate(MemoryKind::UnboundedCache, c, i, name, High));
            }
        }
        _ => {}
    }
}

fn variable_buffer(c: &Code, i: usize, lang: Lang, f: &mut Vec<MemoryCandidate>) {
    // `(construct, first size token)`
    let site = match lang {
        Lang::Java | Lang::CSharp if c.is(i, &["new", "byte", "["]) => Some(("new byte[]", i + 3)),
        Lang::Go if c.is(i, &["make", "(", "[", "]", "byte", ","]) => Some(("make([]byte)", i + 6)),
        // Only a numeric fill makes a byte buffer; `vec![Set::new(); n]` is a table.
        Lang::Rust
            if c.is(i, &["vec", "!", "["])
                && c.t(i + 3).starts_with(|ch: char| ch.is_ascii_digit()) =>
        {
            // `vec![value; size]`: the size follows the `;`.
            let end = c.close(i + 2);
            (i + 3..end)
                .find(|&j| c.t(j) == ";" && c.parens[j] == c.parens[i] + 1)
                .map(|semi| ("vec![_; n]", semi + 1))
        }
        Lang::Rust if c.is(i, &[":", ":", "with_capacity", "("]) => Some(("with_capacity", i + 4)),
        Lang::Py if c.is(i, &["bytearray", "("]) => Some(("bytearray", i + 2)),
        Lang::Ts | Lang::Js if c.is(i, &["Buffer", ".", "alloc", "("]) => {
            Some(("Buffer.alloc", i + 4))
        }
        Lang::Ts | Lang::Js if c.is(i, &["Buffer", ".", "allocUnsafe", "("]) => {
            Some(("Buffer.allocUnsafe", i + 4))
        }
        Lang::Ts | Lang::Js if c.is(i, &["new", "ArrayBuffer", "("]) => {
            Some(("new ArrayBuffer", i + 3))
        }
        _ => None,
    };
    let Some((construct, start)) = site else {
        return;
    };
    let depth = c.parens[start];
    let size: Vec<&str> = (start..c.tokens.len())
        .take_while(|&j| !(c.parens[j] == depth && matches!(c.t(j), "," | ")" | "]" | ";")))
        .map(|j| c.t(j))
        .collect();
    if !size.is_empty() && !constant(&size) {
        f.push(candidate(
            MemoryKind::VariableBuffer,
            c,
            i,
            construct.into(),
            Medium,
        ));
    }
}

/// A size made of literals and constants, or copied from existing data's
/// length, which does not let input inflate the allocation.
fn constant(size: &[&str]) -> bool {
    if size
        .iter()
        .any(|t| matches!(*t, "len" | "length" | "Length" | "Count"))
    {
        return true;
    }
    size.iter().all(|t| {
        t.starts_with(|c: char| c.is_ascii_digit())
            || (t.len() > 1
                && t.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            || (!t.is_empty() && !t.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
    })
}

/// A long-lived collection declaration.
struct Declaration {
    at: usize,
    name: String,
    collection: String,
}

fn grow_only(c: &Code, lang: Lang, f: &mut Vec<MemoryCandidate>) {
    let spring = lang == Lang::Java
        && (0..c.tokens.len()).any(|i| c.t(i) == "@" && SPRING_BEANS.contains(&c.t(i + 1)));
    let startup = startup(c, lang);
    let mut declarations = Vec::new();
    for i in 0..c.tokens.len() {
        if c.initializes(i) {
            if let Some(d) = declaration(c, i, lang, spring) {
                declarations.push(d);
            }
        } else if lang == Lang::Go && c.t(i) == "var" && c.braces[i] == 0 {
            // `var seen sync.Map` or `var log []string`: usable without `=`.
            let collection = if c.is(i + 2, &["sync", ".", "Map"]) {
                "sync.Map"
            } else if c.is(i + 2, &["[", "]"]) {
                "slice"
            } else {
                continue;
            };
            declarations.push(Declaration {
                at: i + 1,
                name: c.t(i + 1).into(),
                collection: collection.into(),
            });
        }
    }
    for d in declarations {
        if d.name.is_empty()
            || !d
                .name
                .starts_with(|ch: char| ch.is_alphabetic() || ch == '_')
        {
            continue;
        }
        let (mut grows, mut shrinks) = (false, false);
        for j in (0..c.tokens.len()).filter(|&j| j != d.at && c.t(j) == d.name) {
            let (g, s) = usage(c, j);
            // A lookup table filled once at load time does not grow later.
            grows |= g && !startup[j];
            shrinks |= s;
        }
        if grows && !shrinks {
            let construct = format!("{} {}", d.collection, d.name);
            f.push(candidate(
                MemoryKind::GrowOnlyCollection,
                c,
                d.at,
                construct,
                Medium,
            ));
        }
    }
}

/// Whether each token runs once at load time: Java and C# static
/// initializers, Go's `func init`, and module-level code in Python and
/// TypeScript.
fn startup(c: &Code, lang: Lang) -> Vec<bool> {
    if lang == Lang::Py {
        return (0..c.tokens.len())
            .map(|j| !in_python_function(c, j))
            .collect();
    }
    let module = matches!(lang, Lang::Ts | Lang::Js);
    let mut blocks: Vec<bool> = Vec::new();
    let mut out = Vec::with_capacity(c.tokens.len());
    for k in 0..c.tokens.len() {
        let inside = blocks.last().copied().unwrap_or(module);
        out.push(inside);
        match c.t(k) {
            "{" => {
                let opens = match lang {
                    // `static {` and `static Registry() {`
                    Lang::Java | Lang::CSharp => {
                        k > 0 && c.t(k - 1) == "static"
                            || k > 3 && c.is(k - 2, &["(", ")"]) && c.t(k - 4) == "static"
                    }
                    Lang::Go => k > 3 && c.is(k - 3, &["init", "(", ")"]) && c.t(k - 4) == "func",
                    _ => false,
                };
                // Functions declared at module level run later, not at load.
                let function = module
                    && k > 1
                    && (c.is(k - 2, &["=", ">"]) || c.t(k - 1) == ")" && !control(c, k - 1));
                blocks.push((inside && !function) || opens);
            }
            "}" => {
                blocks.pop();
            }
            _ => {}
        }
    }
    out
}

/// Whether the `)` at `close` ends the head of `if`, `for` and the like.
fn control(c: &Code, close: usize) -> bool {
    let mut depth = 0usize;
    for k in (0..=close).rev() {
        match c.t(k) {
            ")" => depth += 1,
            "(" => {
                depth -= 1;
                if depth == 0 {
                    return k > 0
                        && matches!(
                            c.t(k - 1),
                            "if" | "for" | "while" | "switch" | "catch" | "with"
                        );
                }
            }
            _ => {}
        }
    }
    false
}

/// Whether the token at `j` is inside a `def` body: a `def` above it that is
/// indented less than every line in between.
fn in_python_function(c: &Code, j: usize) -> bool {
    let mut indent = usize::MAX;
    for k in (0..=j).rev() {
        let line_start = k == 0 || c.line(k - 1) != c.line(k);
        if c.t(k) == "def" {
            // `async def` indents at `async`.
            let at = if k > 0 && c.t(k - 1) == "async" {
                k - 1
            } else {
                k
            };
            if c.column(at) < indent {
                return true;
            }
        }
        if line_start {
            indent = indent.min(c.column(k));
            if indent == 0 {
                return false;
            }
        }
    }
    false
}

/// The long-lived collection that the assignment at `eq` declares, if any.
fn declaration(c: &Code, eq: usize, lang: Lang, spring: bool) -> Option<Declaration> {
    let start = (0..eq)
        .rev()
        .find(|&j| {
            matches!(c.t(j), ";" | "{" | "}")
                || (matches!(lang, Lang::Go | Lang::Py) && c.line(j) != c.line(eq))
        })
        .map_or(0, |j| j + 1);
    let left: Vec<&str> = (start..eq).map(|j| c.t(j)).collect();
    // The first tokens of the initializer, within the statement.
    let right: Vec<&str> = (eq + 1..(eq + 15).min(c.tokens.len()))
        .take_while(|&j| {
            c.t(j) != ";"
                && (!matches!(lang, Lang::Go | Lang::Py | Lang::Ts | Lang::Js)
                    || c.line(j) == c.line(eq))
        })
        .map(|j| c.t(j))
        .collect();
    let has = |words: &[&str], w: &str| {
        // `&'static str` is a lifetime, not a `static` item.
        (0..words.len()).any(|k| words[k] == w && (k == 0 || words[k - 1] != "'"))
    };
    let typed = |left: &[&str]| -> Option<usize> {
        // `name: Type = …` names the token before the first `:`.
        left.iter()
            .position(|t| *t == ":")
            .filter(|&p| p > 0)
            .map(|p| p - 1)
    };
    let found_collection = || {
        // The constructed type is more specific than the declared one.
        right
            .iter()
            .chain(&left)
            .find(|t| COLLECTIONS.contains(t))
            .map(|t| t.to_string())
    };
    let (name_at, collection) = match lang {
        Lang::Java | Lang::CSharp => {
            let long_lived =
                has(&left, "static") || (spring && c.braces[eq] == 1 && c.parens[eq] == 0);
            let built =
                right.contains(&"new") || (lang == Lang::CSharp && right.starts_with(&["[", "]"]));
            if !long_lived || !built || c.parens[eq] != 0 {
                return None;
            }
            (eq - 1, found_collection()?)
        }
        Lang::Rust => {
            if !has(&left, "static") {
                return None;
            }
            (start + typed(&left)?, found_collection()?)
        }
        Lang::Ts | Lang::Js => {
            let module =
                c.braces[eq] == 0 && left.iter().any(|t| matches!(*t, "const" | "let" | "var"));
            if !(module || has(&left, "static")) || c.parens[eq] != 0 {
                return None;
            }
            let collection = match right.as_slice() {
                ["new", "Map" | "Set", ..] => right[1],
                ["[", "]", ..] => "[]",
                _ => return None,
            };
            let at = typed(&left).unwrap_or(left.len() - 1);
            (start + at, collection.to_string())
        }
        Lang::Kotlin => return None,
        Lang::Go => {
            if c.braces[eq] != 0 || c.parens[eq] > 1 {
                return None;
            }
            let collection = match right.as_slice() {
                ["make", "(", "map", ..] | ["map", "[", ..] => "map",
                ["make", "(", "[", ..] | ["[", "]", ..] => "slice",
                _ => return None,
            };
            let at = if left.first() == Some(&"var") { 1 } else { 0 };
            if left.len() != at + 1 {
                return None;
            }
            (start + at, collection.to_string())
        }
        Lang::Py => {
            if c.braces[eq] != 0 || c.parens[eq] != 0 || start >= eq || c.column(start) != 0 {
                return None;
            }
            let collection = match right.as_slice() {
                ["{", "}", ..] => "{}",
                ["[", "]", ..] => "[]",
                ["collections", ".", name, "(", ..] => name,
                [
                    name @ ("dict" | "set" | "list" | "defaultdict" | "OrderedDict" | "Counter"
                    | "deque"),
                    "(",
                    ..,
                ] => name,
                _ => return None,
            };
            if collection == "deque" && right.contains(&"maxlen") {
                return None;
            }
            let at = typed(&left).unwrap_or(0);
            if left.len() != 1 && typed(&left) != Some(0) {
                return None;
            }
            (start + at, collection.to_string())
        }
        Lang::Php => return None,
    };
    Some(Declaration {
        at: name_at,
        name: c.t(name_at).into(),
        collection,
    })
}

/// Whether the mention of a collection at `j` grows or shrinks it.
fn usage(c: &Code, j: usize) -> (bool, bool) {
    let (mut grows, mut shrinks) = (false, false);
    // `del cache[key]` (Python), `delete(cache, key)` and `clear(cache)` (Go).
    if j > 0 && c.t(j - 1) == "del"
        || j > 1 && c.t(j - 1) == "(" && matches!(c.t(j - 2), "delete" | "clear")
    {
        shrinks = true;
    }
    if c.assigns(j + 1) {
        // `log = append(log, x)` grows; any other reassignment resets.
        if c.t(j + 2) == "append" {
            grows = true;
        } else {
            shrinks = true;
        }
        return (grows, shrinks);
    }
    let mut k = j + 1;
    loop {
        match c.t(k) {
            "." => {
                let method = c.t(k + 1);
                grows |= GROWS.contains(&method);
                shrinks |= SHRINKS.contains(&method);
                k += 2;
                if c.t(k) == "(" {
                    k = c.close(k);
                }
            }
            // `?.` in TypeScript, `x?.` and `x!.` chains, Rust's `?`.
            "?" | "!" if c.t(k + 1) == "." || c.t(k) == "?" => k += 1,
            "[" => {
                k = c.close(k);
                if c.assigns(k) {
                    grows = true;
                }
            }
            _ => break,
        }
    }
    (grows, shrinks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str, lang: Lang) -> Vec<(MemoryKind, u32, String, Confidence)> {
        scan_memory(source, lang)
            .into_iter()
            .map(|c| (c.kind, c.line, c.construct, c.confidence))
            .collect()
    }

    fn kinds(source: &str, lang: Lang) -> Vec<(MemoryKind, u32)> {
        scan_memory(source, lang)
            .into_iter()
            .map(|c| (c.kind, c.line))
            .collect()
    }

    use MemoryKind::*;

    #[test]
    fn java_caches_collections_reads_and_buffers() {
        let source = r#"
@Service
class Prices {
    private static final Map<String, Price> CACHE = new ConcurrentHashMap<>();
    private final List<Event> events = new ArrayList<>();
    private final Map<String, Long> seen = new HashMap<>();
    private final Cache<String, X> a = Caffeine.newBuilder().build();
    private final Cache<String, X> b = Caffeine.newBuilder().maximumSize(1000).build();

    Price get(String k) {
        CACHE.computeIfAbsent(k, this::load);
        events.add(new Event(k));
        seen.put(k, 1L);
        seen.remove(k);
        byte[] body = in.readAllBytes();
        byte[] buf = new byte[header.size];
        byte[] fixed = new byte[BUFFER_SIZE];
        byte[] copy = new byte[body.length];
        return repo.findAll().get(0);
    }
}
"#;
        assert_eq!(
            found(source, Lang::Java),
            vec![
                (
                    GrowOnlyCollection,
                    4,
                    "ConcurrentHashMap CACHE".into(),
                    Medium
                ),
                (GrowOnlyCollection, 5, "ArrayList events".into(), Medium),
                (UnboundedCache, 7, "Caffeine.newBuilder".into(), High),
                (ReadAll, 15, ".readAllBytes".into(), High),
                (VariableBuffer, 16, "new byte[]".into(), Medium),
                (ReadAll, 19, ".findAll".into(), Medium),
            ]
        );
    }

    #[test]
    fn lookup_tables_filled_at_load_time_do_not_grow() {
        let java = "enum Op {\n  A;\n  private static final Map<Byte, Op> BY_CODE = new HashMap<>();\n  static {\n    for (Op o : values()) { BY_CODE.put(o.code, o); }\n  }\n}\n";
        assert_eq!(kinds(java, Lang::Java), vec![]);
        let csharp = "class Codes {\n  static readonly Dictionary<int, string> Names = new();\n  static Codes() { Names.Add(1, \"a\"); }\n}\n";
        assert_eq!(kinds(csharp, Lang::CSharp), vec![]);
        let go = "package a\nvar names = map[int]string{}\nfunc init() {\n\tnames[1] = \"a\"\n}\n";
        assert_eq!(kinds(go, Lang::Go), vec![]);
        let py = "TABLE = {}\nfor k in KEYS:\n    TABLE[k] = k\n\nclass A:\n    pass\n";
        assert_eq!(kinds(py, Lang::Py), vec![]);
        let ts =
            "const routes = [];\nroutes.push('/a');\nfor (const r of more) { routes.push(r); }\n";
        assert_eq!(kinds(ts, Lang::Ts), vec![]);
        // The same additions inside a function still count.
        let py_runtime = "TABLE = {}\nclass A:\n    def put(self, k):\n        TABLE[k] = k\n";
        assert_eq!(kinds(py_runtime, Lang::Py), vec![(GrowOnlyCollection, 1)]);
        let ts_runtime = "const routes = [];\nexport function add(r) { routes.push(r); }\n";
        assert_eq!(kinds(ts_runtime, Lang::Ts), vec![(GrowOnlyCollection, 1)]);
        let ts_arrow = "const seen = new Set();\napp.get('/', (req) => { seen.add(req.ip); });\n";
        assert_eq!(kinds(ts_arrow, Lang::Ts), vec![(GrowOnlyCollection, 1)]);
        let py_async = "_buckets: dict[str, int] = {}\n\nasync def limit(ip):\n    async with lock:\n        _buckets[ip] = 1\n";
        assert_eq!(kinds(py_async, Lang::Py), vec![(GrowOnlyCollection, 1)]);
    }

    #[test]
    fn java_instance_fields_outside_spring_beans_are_short_lived() {
        let source = "class Builder {\n  private final List<String> parts = new ArrayList<>();\n  void add(String p) { parts.add(p); }\n}\n";
        assert_eq!(kinds(source, Lang::Java), vec![]);
    }

    #[test]
    fn csharp_statics_caches_and_reads() {
        let source = r#"
static class Registry {
    static readonly ConcurrentDictionary<string, Item> Items = new();
    static readonly List<string> Log = [];
    public static void Track(string k) {
        Items.TryAdd(k, new Item());
        Log.Add(k);
        Log.Clear();
        services.AddMemoryCache();
        var text = await reader.ReadToEndAsync();
        var all = await db.Orders.ToListAsync();
    }
}
"#;
        assert_eq!(
            found(source, Lang::CSharp),
            vec![
                (
                    GrowOnlyCollection,
                    3,
                    "ConcurrentDictionary Items".into(),
                    Medium
                ),
                (UnboundedCache, 9, "AddMemoryCache".into(), Medium),
                (ReadAll, 10, ".ReadToEndAsync".into(), High),
                (ReadAll, 11, ".ToListAsync".into(), Medium),
            ]
        );
        let limited = "services.AddMemoryCache(o => o.SizeLimit = 1024);";
        assert_eq!(kinds(limited, Lang::CSharp), vec![]);
    }

    #[test]
    fn go_package_maps_slices_and_reads() {
        let source = r#"package shop

var sessions = make(map[string]Session)
var seen sync.Map
var audit []string
var counts = map[string]int{}

func handle(r *http.Request) {
	body, _ := io.ReadAll(r.Body)
	sessions[id] = Session{}
	seen.Store(id, true)
	audit = append(audit, id)
	counts[id] = 1
	delete(counts, id)
	buf := make([]byte, n)
	cp := make([]byte, len(body))
}
"#;
        assert_eq!(
            found(source, Lang::Go),
            vec![
                (GrowOnlyCollection, 3, "map sessions".into(), Medium),
                (GrowOnlyCollection, 4, "sync.Map seen".into(), Medium),
                (GrowOnlyCollection, 5, "slice audit".into(), Medium),
                (ReadAll, 9, "io.ReadAll".into(), High),
                (VariableBuffer, 15, "make([]byte)".into(), Medium),
            ]
        );
    }

    #[test]
    fn python_module_state_caches_and_reads() {
        let source = r#"import functools

_registry = {}
_events: list[str] = []
_recent = deque(maxlen=100)
LIMITS = {"a": 1}

@functools.lru_cache(maxsize=None)
def price(k):
    return k

@cache
def other(k):
    _registry[k] = k
    _events.append(k)
    _events.clear()
    _recent.append(k)
    data = request.stream.read()
    rows = cur.fetchall()
    buf = bytearray(size)
    local = {}
    local[k] = 1

@functools.lru_cache(maxsize=128)
def bounded(k):
    return k
"#;
        assert_eq!(
            found(source, Lang::Py),
            vec![
                (GrowOnlyCollection, 3, "{} _registry".into(), Medium),
                (UnboundedCache, 8, "@lru_cache".into(), High),
                (UnboundedCache, 12, "@cache".into(), High),
                (ReadAll, 18, ".read".into(), Medium),
                (ReadAll, 19, ".fetchall".into(), High),
                (VariableBuffer, 20, "bytearray".into(), Medium),
            ]
        );
    }

    #[test]
    fn typescript_module_maps_and_reads() {
        let source = r#"
export const sessions: Map<string, Session> = new Map();
const pending = new Set<string>();
let history = [];
class Store { static items = new Map<string, Item>(); }

export function handle(id: string) {
  sessions.set(id, {});
  pending.add(id);
  pending.delete(id);
  history.push(id);
  Store.items?.set(id, {});
  const local = new Map();
  local.set(id, 1);
  const raw = fs.readFileSync(path);
  const buf = Buffer.alloc(size);
  const fixed = Buffer.alloc(1024);
}
"#;
        assert_eq!(
            found(source, Lang::Ts),
            vec![
                (GrowOnlyCollection, 2, "Map sessions".into(), Medium),
                (GrowOnlyCollection, 4, "[] history".into(), Medium),
                (GrowOnlyCollection, 5, "Map items".into(), Medium),
                (ReadAll, 15, "readFileSync".into(), High),
                (VariableBuffer, 16, "Buffer.alloc".into(), Medium),
            ]
        );
    }

    #[test]
    fn rust_statics_reads_and_buffers() {
        let source = r#"
static SEEN: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NAMES: &[&str] = &["a", "b"];

fn handle(mut file: File, n: usize) -> io::Result<()> {
    SEEN.lock().unwrap().insert(key(), 1);
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    let raw = fs::read(path)?;
    let buf = vec![0u8; n];
    let fixed = vec![0u8; 4096];
    let v = Vec::with_capacity(items.len());
    Ok(())
}
"#;
        assert_eq!(
            found(source, Lang::Rust),
            vec![
                (GrowOnlyCollection, 2, "HashMap SEEN".into(), Medium),
                (ReadAll, 8, ".read_to_string".into(), High),
                (ReadAll, 9, "fs::read".into(), High),
                (VariableBuffer, 10, "vec![_; n]".into(), Medium),
            ]
        );
    }

    #[test]
    fn paged_queries_are_not_read_all() {
        let csharp = "var page = await db.Items\n    .Skip(size * index)\n    .Take(size)\n    .ToListAsync();\nvar all = await db.Items.OrderBy(x => x.Name).ToListAsync();\n";
        assert_eq!(kinds(csharp, Lang::CSharp), vec![(ReadAll, 5)]);
        let java = "class R {\n  void a() {\n    repo.findAll(PageRequest.of(0, 20));\n    repo.findAll(pageable);\n    em.createQuery(q).setMaxResults(50).getResultList();\n    repo.findAll();\n  }\n}\n";
        assert_eq!(kinds(java, Lang::Java), vec![(ReadAll, 6)]);
    }

    #[test]
    fn rust_locals_limited_reads_and_non_byte_vectors_are_not_candidates() {
        let source = r#"
fn sarif(report: &Report) -> String {
    let mut ids: Vec<&'static str> = Vec::new();
    for f in &report.findings { ids.push(f.id); }
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES).read_to_end(&mut bytes)?;
    let dag = vec![BTreeSet::new(); components];
    String::new()
}
"#;
        assert_eq!(kinds(source, Lang::Rust), vec![]);
        let go = "package a\nfunc f(r io.Reader) { b, _ := io.ReadAll(io.LimitReader(r, max)) }\n";
        assert_eq!(kinds(go, Lang::Go), vec![]);
    }

    #[test]
    fn php_reads() {
        let source =
            "<?php\n$body = file_get_contents('php://input');\n$rows = $stmt->fetchAll();\n";
        assert_eq!(kinds(source, Lang::Php), vec![(ReadAll, 2), (ReadAll, 3)]);
    }

    #[test]
    fn comments_and_strings_never_match() {
        let java = "class A {\n  // in.readAllBytes()\n  String s = \"Files.readAllLines(p)\";\n  /* new byte[n] */\n}\n";
        assert_eq!(kinds(java, Lang::Java), vec![]);
        let py = "# data = f.read()\ntext = 'cur.fetchall()'\n";
        assert_eq!(kinds(py, Lang::Py), vec![]);
        let go = "package a\n// io.ReadAll(r)\nvar s = \"os.ReadFile(p)\"\n";
        assert_eq!(kinds(go, Lang::Go), vec![]);
    }
}
