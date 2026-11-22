//! Ruby on Rails endpoints and their access for ui-flow/v1 (ADR 0021).
//!
//! A text-level reader of the `.rb` files below the root (test and spec files
//! left out). Comments are cut, continuation lines joined and `do`/`end`
//! blocks nested into a statement tree; it fills only `endpoints` and
//! `diagnostics`, and only for a `config/routes.rb` (or `config/routes/*.rb`
//! reached by `draw`).
//!
//! Routes: `resources`/`resource` (`only:`/`except:`/`controller:`/`path:`/
//! `param:`/`concerns:`, nested, `member`, `collection`), `namespace`,
//! `scope`, `concern`/`concerns`, verbs (`get "x", to: "c#a"`, `get "x" =>
//! "c#a"`, `on:`), `match` (`via:`), `root`, `mount`, `draw`, the Devise
//! blocks `authenticate`/`authenticated`/`unauthenticated` and `constraints`
//! (block or option). An engine's `isolate_namespace` is the module prefix.
//!
//! Controllers: classes and modules with their superclass, `include`d
//! concerns (`included do`, `class_methods do`), and the ordered callback
//! list. For each action the chain is built as Rails does: ancestors first,
//! re-registering a callback replaces the earlier one (Note), `prepend_`
//! goes first, `skip_before_action` removes exactly the callback it names.
//! A callback's meaning: `[views.access]` first; Devise `authenticate_<scope>!`
//! for a scope read from `devise_for` (the first one is the default, another
//! scope adds `custom(scope:<s>)`); the generated `require_authentication`
//! (its definition calls `resume_session`); any other project method has no
//! meaning (Note), except that a method a `skip_before_action` names is an
//! access position and becomes `unknown` with a gap. `if:`/`unless:`, a
//! non-literal `only:`/`except:` or a block make the callback `unknown`
//! with a gap. `Open` needs an explicit marker (a skip of a known guard,
//! `allow_unauthenticated_access` as defined in the project, `skip_authorization`)
//! and never shows next to a callback that may halt and was not read.
//!
//! Authorization: CanCanCan `load_and_authorize_resource`/`authorize_resource`
//! (`only:`/`except:`/`class:`, `skip_authorize_resource`), `authorize!`
//! (CanCanCan or Action Policy, chosen by `Gemfile.lock`; both or none gives
//! `unknown`), Pundit `authorize` in the action body, `http_basic_authenticate_with`
//! (a project redefinition gives `unknown`). Only statements at the top of an
//! action count; a modifier or block is a Note. `verify_authorized`,
//! `policy_scope`, `check_authorization` protect nothing.
//! Not read: views and links, the Ability and policy catalogues, `Devise`
//! controllers, `devise_group`, `shallow`, `direct`/`resolve`.
use crate::extract_ctx::{ExtractContext, finish, gap, read_text, walk, with_root_config};
use crate::services::support::test_path;
use archguard_core::{DiagnosticSeverity, UiFlow, UiFlowDiagnostic};

const MAX_FILES: usize = 20_000;

pub fn extract(root: &std::path::Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// The `.rb` files and the `Gemfile.lock` text below `ctx.root`.
fn read_files(ctx: &ExtractContext, flow: &mut UiFlow) -> Option<(Vec<(String, String)>, String)> {
    let walked = match walk(ctx.root) {
        Ok(w) => w,
        Err(e) => {
            let message = format!("Discovery exclusions not read; no Rails app analyzed: {e}");
            gap(&mut flow.diagnostics, "", 0, message);
            return None;
        }
    };
    let skipped = |p: &str| matches!(p, "spec" | "vendor" | "node_modules");
    let mut paths: Vec<String> = (walked.files.into_iter())
        .filter(|f| f.ends_with(".rb") || f.ends_with("Gemfile.lock"))
        .filter(|f| !test_path(f) && !f.split('/').any(skipped))
        .collect();
    paths.sort();
    let (mut files, mut lock) = (Vec::new(), String::new());
    for rel in paths.into_iter().take(MAX_FILES) {
        match read_text(ctx.root, &rel, 1 << 20) {
            Ok(text) if rel.ends_with("Gemfile.lock") => lock.push_str(&text),
            Ok(text) => files.push((rel, text)),
            Err(e) => {
                let message = format!("File not read ({e}); its routes and callbacks are unknown.");
                gap(&mut flow.diagnostics, &rel, 0, message);
            }
        }
    }
    Some((files, lock))
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    if let Some((files, lock)) = read_files(ctx, &mut flow)
        && files.iter().any(|(r, _)| r.ends_with("config/routes.rb"))
    {
        if let Some(e) = &ctx.config_error {
            let message = format!("Configuration not read; [views.access] not applied: {e}");
            note(&mut flow.diagnostics, ".", 0, message);
        }
        let mut p = proj::Proj::new(&ctx.config.views.access, &files, &lock);
        p.run(ctx.root, &files);
        flow.diagnostics.append(&mut p.diags);
        flow.endpoints.append(&mut p.endpoints);
    }
    finish(&mut flow);
    flow
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: impl Into<String>) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Note,
    });
}

/// Text level: comments cut, lines joined, blocks nested, call arguments read.
mod lex {
    /// One statement; `kids` for a block opener.
    #[derive(Clone, Debug)]
    pub(super) struct Stmt {
        pub(super) line: u32,
        pub(super) text: String,
        pub(super) kids: Vec<Stmt>,
        pub(super) block: bool,
    }

    /// Code of `raw` without comment, the bracket balance, and a heredoc id.
    fn scan(raw: &str) -> (String, i32, Option<String>) {
        let (mut out, mut delta, mut quote) = (String::new(), 0i32, None::<char>);
        let mut chars = raw.chars();
        while let Some(c) = chars.next() {
            if let Some(q) = quote {
                out.push(c);
                if c == '\\' {
                    out.extend(chars.next());
                } else if c == q {
                    quote = None;
                }
                continue;
            }
            match c {
                '#' => break,
                '"' | '\'' | '`' => quote = Some(c),
                '(' | '[' | '{' => delta += 1,
                ')' | ']' | '}' => delta -= 1,
                _ => {}
            }
            out.push(c);
        }
        let heredoc = heredoc_id(&out);
        (out, delta, heredoc)
    }

    fn heredoc_id(code: &str) -> Option<String> {
        ["<<~", "<<-"].iter().find_map(|m| {
            let at = code.find(m)? + 3;
            let rest = code[at..].trim_start_matches(['"', '\'']);
            let id: String = (rest.chars())
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!id.is_empty()).then_some(id)
        })
    }

    /// Lines to ignore: heredoc bodies and `=begin`/`=end` comments.
    #[derive(Default)]
    struct Skipper {
        heredoc: Option<String>,
        comment: bool,
    }

    impl Skipper {
        fn skips(&mut self, raw: &str) -> bool {
            if let Some(id) = &self.heredoc {
                if raw.trim() == id {
                    self.heredoc = None;
                }
                return true;
            }
            if raw.starts_with("=begin") {
                self.comment = true;
            }
            if self.comment {
                self.comment = !raw.starts_with("=end");
                return true;
            }
            false
        }
    }

    fn continues(code: &str, depth: i32) -> bool {
        depth > 0
            || [",", "\\", "&&", "||", "=>", "+"]
                .iter()
                .any(|s| code.ends_with(s))
    }

    pub(super) fn logical_lines(text: &str) -> Vec<(u32, String)> {
        let (mut out, mut cur, mut start, mut depth) = (Vec::new(), String::new(), 0u32, 0i32);
        let mut skip = Skipper::default();
        for (i, raw) in text.lines().enumerate() {
            if skip.skips(raw) {
                continue;
            }
            if raw.trim() == "__END__" {
                break;
            }
            let (code, delta, heredoc) = scan(raw);
            let code = code.trim();
            if code.is_empty() && cur.is_empty() {
                continue;
            }
            if cur.is_empty() {
                start = i as u32 + 1;
            } else {
                cur.push(' ');
            }
            cur.push_str(code);
            depth = (depth + delta).max(0);
            skip.heredoc = heredoc.or(skip.heredoc.take());
            if !continues(code, depth) {
                out.push((start, std::mem::take(&mut cur)));
                depth = 0;
            }
        }
        if !cur.is_empty() {
            out.push((start, cur));
        }
        out
    }

    pub(super) fn first_word(t: &str) -> &str {
        let ident = |(i, c): &(usize, char)| {
            c.is_alphanumeric() || *c == '_' || ((*c == '?' || *c == '!') && *i > 0)
        };
        let end = t
            .char_indices()
            .find(|ic| !ident(ic))
            .map_or(t.len(), |(i, _)| i);
        &t[..end]
    }

    fn ends_with_do(t: &str) -> bool {
        let pipes = t.ends_with('|') && (t.contains(" do |") || t.contains(" do|"));
        t == "do" || t.ends_with(" do") || pipes
    }

    /// `def x = expr` and `def x(a) = expr` open no block.
    fn endless_def(t: &str) -> bool {
        let after = t[3..].trim_start();
        let name_end = after.find(['(', ' ', '=']).unwrap_or(after.len());
        let mut rest = after[name_end..].trim_start();
        if let Some(r) = rest.strip_prefix('(').and_then(|r| r.split_once(')')) {
            rest = r.1.trim_start();
        }
        rest.starts_with('=') && !rest.starts_with("==")
    }

    fn opens(t: &str) -> bool {
        if t.ends_with("; end") || t.ends_with(" end") {
            return false;
        }
        match first_word(t) {
            "class" | "module" | "if" | "unless" | "while" | "until" | "case" | "begin" | "for" => {
                true
            }
            "def" => !endless_def(t),
            _ => ends_with_do(t),
        }
    }

    fn is_end(t: &str) -> bool {
        t == "end"
            || ["end ", "end.", "end)", "end;"]
                .iter()
                .any(|p| t.starts_with(p))
    }

    fn build(lines: &[(u32, String)], i: &mut usize) -> Vec<Stmt> {
        let mut out = Vec::new();
        while *i < lines.len() {
            let (line, t) = &lines[*i];
            *i += 1;
            if is_end(t) {
                break;
            }
            let block = opens(t);
            let kids = if block { build(lines, i) } else { Vec::new() };
            out.push(Stmt {
                line: *line,
                text: t.clone(),
                kids,
                block,
            });
        }
        out
    }

    pub(super) fn tree(text: &str) -> Vec<Stmt> {
        let lines = logical_lines(text);
        let (mut i, mut out) = (0, Vec::new());
        while i < lines.len() {
            out.extend(build(&lines, &mut i));
        }
        out
    }

    /// Byte offset of the first top-level `pat` (outside quotes and brackets).
    fn find_top(s: &str, pat: &str) -> Option<usize> {
        let (b, mut depth, mut quote) = (s.as_bytes(), 0i32, None::<u8>);
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            match quote {
                Some(_) if c == b'\\' => i += 1,
                Some(q) => quote = (c != q).then_some(q),
                None => match c {
                    b'"' | b'\'' => quote = Some(c),
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => depth -= 1,
                    _ if depth == 0 && s[i..].starts_with(pat) => return Some(i),
                    _ => {}
                },
            }
            i += 1;
        }
        None
    }

    fn split_top(s: &str) -> Vec<String> {
        let (mut out, mut rest) = (Vec::new(), s);
        while let Some(at) = find_top(rest, ",") {
            out.push(rest[..at].trim().to_string());
            rest = &rest[at + 1..];
        }
        if !rest.trim().is_empty() {
            out.push(rest.trim().to_string());
        }
        out
    }

    #[derive(Clone, Debug)]
    pub(super) enum V {
        Sym(String),
        Str(String),
        List(Vec<V>),
        Const(String),
        Lambda(String),
        Bool(bool),
        Raw(String),
    }

    impl V {
        fn parse_sym(s: &str) -> Option<V> {
            let n = s
                .strip_prefix(':')
                .filter(|n| !n.is_empty() && !n.starts_with(':'))?;
            Some(V::Sym(n.trim_matches(['"', '\'']).into()))
        }

        fn parse_str(s: &str) -> Option<V> {
            let quoted = |q: char| s.len() >= 2 && s.starts_with(q) && s.ends_with(q);
            let plain = (quoted('"') || quoted('\'')) && !s.contains("#{");
            plain.then(|| V::Str(s[1..s.len() - 1].into()))
        }

        fn parse_list(s: &str) -> Option<V> {
            if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                return Some(V::List(
                    split_top(inner).iter().map(|p| V::parse(p)).collect(),
                ));
            }
            let words = s
                .strip_prefix("%i[")
                .or(s.strip_prefix("%w["))?
                .strip_suffix(']')?;
            let sym = s.starts_with("%i");
            let item = |w: &str| {
                if sym {
                    V::Sym(w.into())
                } else {
                    V::Str(w.into())
                }
            };
            Some(V::List(words.split_whitespace().map(item).collect()))
        }

        fn is_const(s: &str) -> bool {
            let ok = |c: char| c.is_alphanumeric() || "_:.()".contains(c);
            s.starts_with(|c: char| c.is_ascii_uppercase()) && s.chars().all(ok)
        }

        pub(super) fn parse(s: &str) -> V {
            let s = s.trim();
            let lambda = ["->", "lambda", "proc", "Proc.new"]
                .iter()
                .any(|p| s.starts_with(p));
            if lambda {
                return V::Lambda(s.into());
            }
            if let Some(v) = Self::parse_sym(s)
                .or_else(|| Self::parse_str(s))
                .or_else(|| Self::parse_list(s))
            {
                return v;
            }
            match s {
                "true" | "false" => V::Bool(s == "true"),
                _ if Self::is_const(s) => V::Const(s.split(".new").next().unwrap_or(s).into()),
                _ => V::Raw(s.into()),
            }
        }

        pub(super) fn text(&self) -> String {
            match self {
                V::Sym(s) => format!(":{s}"),
                V::Str(s) => format!("\"{s}\""),
                V::List(l) => {
                    format!("[{}]", l.iter().map(V::text).collect::<Vec<_>>().join(", "))
                }
                V::Const(s) | V::Lambda(s) | V::Raw(s) => s.clone(),
                V::Bool(b) => b.to_string(),
            }
        }

        pub(super) fn name(&self) -> Option<&str> {
            match self {
                V::Sym(s) | V::Str(s) => Some(s),
                _ => None,
            }
        }

        /// The names of a symbol/string or a list of them.
        pub(super) fn names(&self) -> Option<Vec<String>> {
            match self {
                V::List(l) => l.iter().map(|v| v.name().map(String::from)).collect(),
                v => v.name().map(|n| vec![n.to_string()]),
            }
        }
    }

    #[derive(Default, Debug)]
    pub(super) struct Args {
        pub(super) pos: Vec<V>,
        pub(super) opts: Vec<(String, V)>,
        pub(super) pairs: Vec<(V, V)>,
    }

    impl Args {
        pub(super) fn opt(&self, k: &str) -> Option<&V> {
            self.opts.iter().find(|(n, _)| n == k).map(|(_, v)| v)
        }
    }

    pub(super) struct Call {
        pub(super) name: String,
        pub(super) args: Args,
        pub(super) block: bool,
        /// Text of a trailing `if`/`unless` modifier.
        pub(super) modifier: Option<String>,
    }

    /// Offset of the `:` of a `key: value` piece.
    fn key_colon(piece: &str) -> Option<usize> {
        let k = piece.find(':').filter(|&k| k > 0)?;
        let key = piece[..k]
            .chars()
            .all(|c| c.is_alphanumeric() || "_?!".contains(c));
        let after = &piece[k + 1..];
        let spaced = after
            .as_bytes()
            .first()
            .is_none_or(|c| c.is_ascii_whitespace());
        (key && !after.starts_with(':') && spaced).then_some(k)
    }

    fn parse_args(s: &str) -> Args {
        let mut a = Args::default();
        for piece in split_top(s) {
            let pair = find_top(&piece, "=>").filter(|_| !piece.starts_with("->"));
            if let Some(at) = pair {
                a.pairs
                    .push((V::parse(&piece[..at]), V::parse(&piece[at + 2..])));
            } else if let Some(k) = key_colon(&piece) {
                a.opts
                    .push((piece[..k].to_string(), V::parse(&piece[k + 1..])));
            } else {
                a.pos.push(V::parse(&piece));
            }
        }
        a
    }

    /// `t` without a trailing `do` / `do |x|`, and whether it had one.
    fn strip_do(t: &str) -> (&str, bool) {
        if let Some(s) = t.strip_suffix(" do").or(t.strip_suffix(" do |")) {
            return (s, true);
        }
        match t
            .rfind(" do |")
            .or(t.rfind(" do|"))
            .filter(|_| t.ends_with('|'))
        {
            Some(at) => (&t[..at], true),
            None => (t, false),
        }
    }

    fn strip_modifier(t: &str) -> (&str, Option<String>) {
        for kw in [" if ", " unless "] {
            if let Some(at) = find_top(t, kw) {
                return (t[..at].trim(), Some(t[at + 1..].to_string()));
            }
        }
        (t, None)
    }

    pub(super) fn call(t: &str) -> Option<Call> {
        let (t, mut block) = strip_do(t.trim());
        let (t, modifier) = strip_modifier(t);
        let name = first_word(t);
        let tail = &t[name.len()..];
        if name.is_empty() || !(tail.is_empty() || tail.starts_with(['(', ' '])) {
            return None;
        }
        let mut rest = tail.trim_start();
        if tail.starts_with('(') && rest.ends_with(')') {
            rest = &rest[1..rest.len() - 1];
        }
        if rest.starts_with('{') {
            block = true;
            rest = "";
        }
        Some(Call {
            name: name.into(),
            args: parse_args(rest),
            block,
            modifier,
        })
    }

    /// All statements below `stmts`, depth first.
    pub(super) fn flat(stmts: &[Stmt]) -> Vec<&Stmt> {
        let mut out = Vec::new();
        for s in stmts {
            out.push(s);
            out.extend(flat(&s.kids));
        }
        out
    }
}

/// Inflection and path helpers.
mod names {
    pub(super) fn singular(s: &str) -> String {
        match s {
            "people" => return "person".into(),
            "children" => return "child".into(),
            _ => {}
        }
        if ["sses", "xes", "ches", "shes", "uses"]
            .iter()
            .any(|suf| s.ends_with(suf))
        {
            return s[..s.len() - 2].into();
        }
        if let Some(r) = s.strip_suffix("ies") {
            return format!("{r}y");
        }
        match s.strip_suffix('s') {
            Some(r) if !s.ends_with("ss") => r.into(),
            _ => s.into(),
        }
    }

    pub(super) fn pluralize(s: &str) -> String {
        let vowel = |r: &str| r.ends_with(['a', 'e', 'i', 'o', 'u']);
        match s.strip_suffix('y').filter(|r| !vowel(r)) {
            Some(r) => format!("{r}ies"),
            None if s.ends_with(['s', 'x']) || s.ends_with("ch") || s.ends_with("sh") => {
                format!("{s}es")
            }
            None => format!("{s}s"),
        }
    }

    fn capitalize(w: &str) -> String {
        let mut c = w.chars();
        c.next().map_or(String::new(), |f| {
            f.to_uppercase().collect::<String>() + c.as_str()
        })
    }

    pub(super) fn camelize(path: &str) -> String {
        let seg = |s: &str| s.split('_').map(capitalize).collect::<String>();
        let segs: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(seg).collect();
        segs.join("::")
    }

    pub(super) fn key_of(name: &str) -> String {
        name.replace('_', "").to_lowercase()
    }

    pub(super) fn join_path(a: &str, b: &str) -> String {
        let (a, b) = (a.trim_matches('/'), b.trim_matches('/'));
        match (a.is_empty(), b.is_empty()) {
            (true, true) => String::new(),
            (true, false) => format!("/{b}"),
            (false, true) => format!("/{a}"),
            _ => format!("/{a}/{b}"),
        }
    }

    /// `join_path` without the leading slash: a module or controller path.
    pub(super) fn join_mod(a: &str, b: &str) -> String {
        join_path(a, b).trim_start_matches('/').to_string()
    }

    /// `/a/(.:format)` and other optional groups dropped.
    pub(super) fn clean_path(p: &str) -> String {
        let (mut out, mut depth) = (String::new(), 0);
        for c in p.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        let out = out.trim_end_matches('/');
        if out.is_empty() {
            "/".into()
        } else {
            out.into()
        }
    }
}

/// Classes, modules and the callback events of their bodies.
mod model {
    use super::lex::{Args, Call, Stmt, V, call, first_word};
    use super::names::camelize;
    use std::collections::HashMap;

    #[derive(Clone, Debug, Default)]
    pub(super) struct Opts {
        pub(super) only: Option<Vec<String>>,
        pub(super) except: Option<Vec<String>>,
        /// `if:`/`unless:` text, or a conditional registration.
        pub(super) cond: Option<String>,
        /// `only:`/`except:` is not a literal list.
        pub(super) unreadable: bool,
    }

    impl Opts {
        pub(super) fn read(a: &Args, conditional: bool) -> Opts {
            let mut o = Opts::default();
            for (k, v) in &a.opts {
                match (k.as_str(), v.names()) {
                    ("only", Some(n)) => o.only = Some(n),
                    ("except", Some(n)) => o.except = Some(n),
                    ("only" | "except", None) => o.unreadable = true,
                    ("if" | "unless", _) => o.cond = Some(format!("{k}: {}", v.text())),
                    _ => {}
                }
            }
            if conditional && o.cond.is_none() {
                o.cond = Some("registered under a condition".into());
            }
            o
        }

        /// Whether the option list covers `action`; `None` when it cannot be read.
        pub(super) fn applies(&self, action: &str) -> Option<bool> {
            if self.unreadable {
                return None;
            }
            let only = self
                .only
                .as_ref()
                .is_none_or(|l| l.iter().any(|a| a == action));
            let except = self
                .except
                .as_ref()
                .is_some_and(|l| l.iter().any(|a| a == action));
            Some(only && !except)
        }
    }

    #[derive(Clone, Debug)]
    pub(super) enum Model {
        Default,
        Named(String),
        Unreadable(String),
    }

    #[derive(Clone, Debug)]
    pub(super) enum Kind {
        Include(String),
        Before {
            names: Vec<String>,
            o: Opts,
            prepend: bool,
        },
        Skip {
            names: Vec<String>,
            o: Opts,
        },
        Unauth {
            o: Opts,
        },
        Authz {
            o: Opts,
            model: Model,
        },
        SkipAuthz {
            o: Opts,
        },
        Basic {
            o: Opts,
        },
    }

    #[derive(Clone, Debug)]
    pub(super) struct Ev {
        pub(super) kind: Kind,
        pub(super) file: String,
        pub(super) line: u32,
    }

    #[derive(Clone, Debug)]
    pub(super) struct Method {
        pub(super) kids: Vec<Stmt>,
    }

    #[derive(Default, Debug)]
    pub(super) struct Class {
        pub(super) name: String,
        pub(super) superclass: Option<String>,
        pub(super) lexical: Vec<String>,
        pub(super) events: Vec<Ev>,
        pub(super) methods: HashMap<String, Method>,
        pub(super) class_methods: HashMap<String, Vec<Stmt>>,
    }

    fn const_name(s: &str) -> String {
        (s.chars())
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
            .collect()
    }

    fn read_class_head(t: &str) -> Option<(String, Option<String>)> {
        let rest = t.split_once(char::is_whitespace)?.1.trim();
        let name = const_name(rest);
        if name.is_empty() {
            return None;
        }
        let sup = rest[name.len()..]
            .trim_start()
            .strip_prefix('<')
            .map(|s| const_name(s.trim()));
        Some((name, sup.filter(|s| !s.is_empty())))
    }

    pub(super) fn collect_classes(
        stmts: &[Stmt],
        file: &str,
        ns: &mut Vec<String>,
        out: &mut Vec<Class>,
    ) {
        for s in stmts.iter().filter(|s| s.block) {
            let w = first_word(&s.text);
            let head = read_class_head(&s.text).filter(|_| w == "class" || w == "module");
            let Some((name, superclass)) = head else {
                collect_classes(&s.kids, file, ns, out);
                continue;
            };
            let mark = ns.len();
            let lexical = ns.clone();
            ns.extend(name.split("::").map(String::from));
            let mut c = Class {
                name: ns.join("::"),
                superclass,
                lexical,
                ..Default::default()
            };
            read_body(&s.kids, file, false, &mut c);
            out.push(c);
            collect_classes(&s.kids, file, ns, out);
            ns.truncate(mark);
        }
    }

    fn def_name(text: &str) -> &str {
        text[3..].trim().split(['(', ' ', ';']).next().unwrap_or("")
    }

    fn add_def(s: &Stmt, c: &mut Class) {
        let name = def_name(&s.text);
        match name.strip_prefix("self.") {
            Some(n) => c.class_methods.insert(n.into(), s.kids.clone()).map(drop),
            None => c
                .methods
                .insert(
                    name.into(),
                    Method {
                        kids: s.kids.clone(),
                    },
                )
                .map(drop),
        };
    }

    fn add_class_methods(s: &Stmt, c: &mut Class) {
        for d in s.kids.iter().filter(|d| first_word(&d.text) == "def") {
            c.class_methods
                .insert(def_name(&d.text).into(), d.kids.clone());
        }
    }

    /// `private def x` and friends.
    fn add_visibility_def(s: &Stmt, c: &mut Class) {
        let rest = s.text.strip_prefix(first_word(&s.text)).map(str::trim);
        if let Some(rest) = rest.filter(|r| r.starts_with("def ")) {
            c.methods.insert(
                def_name(rest).into(),
                Method {
                    kids: s.kids.clone(),
                },
            );
        }
    }

    pub(super) fn read_body(kids: &[Stmt], file: &str, conditional: bool, c: &mut Class) {
        for s in kids {
            read_stmt(s, file, conditional, c);
        }
    }

    fn read_stmt(s: &Stmt, file: &str, conditional: bool, c: &mut Class) {
        match first_word(&s.text) {
            "def" => add_def(s, c),
            "class_methods" if s.block => add_class_methods(s, c),
            "included" if s.block => read_body(&s.kids, file, conditional, c),
            "if" | "unless" | "case" | "begin" if s.block => read_body(&s.kids, file, true, c),
            "class" | "module" => {}
            "private" | "protected" | "public" => add_visibility_def(s, c),
            _ => {
                if let Some(kind) = event_kind(s, conditional) {
                    c.events.push(Ev {
                        kind,
                        file: file.into(),
                        line: s.line,
                    });
                }
            }
        }
    }

    fn names_of(cl: &Call) -> Vec<String> {
        cl.args
            .pos
            .iter()
            .filter_map(|v| v.name().map(String::from))
            .collect()
    }

    fn before_kind(cl: &Call, o: Opts) -> Kind {
        let mut names = names_of(cl);
        if cl.block || names.is_empty() {
            names = vec!["<block>".into()];
        }
        let cond = o.cond.or_else(|| cl.block.then(|| "block".to_string()));
        Kind::Before {
            names,
            o: Opts { cond, ..o },
            prepend: cl.name.starts_with("prepend"),
        }
    }

    fn authz_kind(cl: &Call, o: Opts) -> Kind {
        let first = cl.args.pos.first().and_then(V::name);
        let model = match cl.args.opt("class") {
            Some(V::Const(n) | V::Str(n)) => Model::Named(n.clone()),
            Some(v) => Model::Unreadable(format!("class: {}", v.text())),
            None => first.map_or(Model::Default, |n| Model::Named(camelize(n))),
        };
        Kind::Authz { o, model }
    }

    fn event_kind(s: &Stmt, conditional: bool) -> Option<Kind> {
        let cl = call(&s.text)?;
        let o = Opts::read(&cl.args, conditional);
        Some(match cl.name.as_str() {
            "include" | "prepend" => Kind::Include(cl.args.pos.first()?.text()),
            "before_action"
            | "prepend_before_action"
            | "append_before_action"
            | "before_filter"
            | "around_action" => before_kind(&cl, o),
            "skip_before_action" | "skip_before_filter" | "skip_action_callback" => Kind::Skip {
                names: names_of(&cl),
                o,
            },
            "allow_unauthenticated_access" => Kind::Unauth { o },
            "http_basic_authenticate_with" => Kind::Basic { o },
            "load_and_authorize_resource" | "authorize_resource" => authz_kind(&cl, o),
            "skip_authorize_resource" | "skip_load_and_authorize_resource" => Kind::SkipAuthz { o },
            _ => return None,
        })
    }
}

/// The project: classes, Devise scopes, libraries and the route records.
mod proj {
    use super::eval::Eval;
    use super::lex::{Stmt, V, call, flat, tree};
    use super::model::{Class, Ev, Kind, collect_classes};
    use super::names::{camelize, key_of, singular};
    use super::routes::Ctx;
    use super::{note, routes};
    use crate::extract_ctx::dir_name;
    use archguard_core::{AccessMeaning, AccessState, Condition, EndpointAccess, UiFlowDiagnostic};
    use std::collections::{BTreeMap, HashMap, HashSet};

    /// One route as read: controller path (with module) and action.
    pub(super) struct Rec {
        pub(super) method: String,
        pub(super) path: String,
        pub(super) target: Option<(String, String)>,
        pub(super) layer: AccessState,
        pub(super) file: String,
        pub(super) line: u32,
        pub(super) source: String,
        pub(super) app: String,
    }

    pub(super) struct Libs {
        pub(super) pundit: bool,
        pub(super) cancan: bool,
        pub(super) action_policy: bool,
    }

    pub(super) struct Proj<'a> {
        pub(super) access: &'a BTreeMap<String, AccessMeaning>,
        pub(super) classes: Vec<Class>,
        by_key: HashMap<String, usize>,
        /// Devise scopes in `devise_for` order; the first is the default.
        pub(super) scopes: Vec<String>,
        pub(super) libs: Libs,
        pub(super) skip_named: HashSet<String>,
        pub(super) methods: HashMap<String, Vec<Vec<Stmt>>>,
        pub(super) recs: Vec<Rec>,
        pub(super) concerns: HashMap<String, Vec<Stmt>>,
        pub(super) diags: Vec<UiFlowDiagnostic>,
        pub(super) endpoints: Vec<EndpointAccess>,
        cache: HashMap<(usize, String), AccessState>,
    }

    fn read_libs(lock: &str, classes: &[Class]) -> Libs {
        let has = |g: &str| {
            lock.lines()
                .any(|l| l.trim_start().starts_with(&format!("{g} (")))
        };
        let includes_pundit = |c: &Class| {
            (c.events.iter())
                .any(|e| matches!(&e.kind, Kind::Include(n) if n.starts_with("Pundit")))
        };
        Libs {
            pundit: has("pundit") || classes.iter().any(includes_pundit),
            cancan: has("cancancan"),
            action_policy: has("action_policy"),
        }
    }

    /// Names a `skip_before_action` anywhere in the project names.
    fn skip_names(classes: &[Class]) -> HashSet<String> {
        let mut out = HashSet::new();
        let in_macro = classes
            .iter()
            .flat_map(|c| c.class_methods.values())
            .flatten();
        for s in in_macro {
            if let Some(cl) = call(&s.text).filter(|cl| cl.name == "skip_before_action") {
                out.extend(
                    cl.args
                        .pos
                        .iter()
                        .filter_map(|v| v.name().map(String::from)),
                );
            }
        }
        for ev in classes.iter().flat_map(|c| &c.events) {
            if let Kind::Skip { names, .. } = &ev.kind {
                out.extend(names.iter().cloned());
            }
        }
        out
    }

    fn mentions_controller(rel: &str, text: &str) -> bool {
        let routes = rel.ends_with("config/routes.rb") || rel.contains("config/routes/");
        !routes
            && ["Controller", "Concern", "def self."]
                .iter()
                .any(|m| text.contains(m))
    }

    fn isolated_module(files: &[(String, String)], app_dir: &str) -> String {
        let named = |t: &str| {
            let at = t.find("isolate_namespace")? + 17;
            let n = (t[at..].trim_start().chars())
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect::<String>();
            (!n.is_empty()).then(|| n.replace("::", "/").to_lowercase())
        };
        let own = |r: &String| r.starts_with(app_dir) && !r.ends_with("config/routes.rb");
        (files.iter())
            .filter(|(r, _)| own(r))
            .find_map(|(_, t)| named(t))
            .unwrap_or_default()
    }

    impl<'a> Proj<'a> {
        pub(super) fn new(
            access: &'a BTreeMap<String, AccessMeaning>,
            files: &[(String, String)],
            lock: &str,
        ) -> Self {
            let mut classes = Vec::new();
            for (rel, text) in files.iter().filter(|(r, t)| mentions_controller(r, t)) {
                collect_classes(&tree(text), rel, &mut Vec::new(), &mut classes);
            }
            let mut methods: HashMap<String, Vec<Vec<Stmt>>> = HashMap::new();
            for (n, m) in classes.iter().flat_map(|c| &c.methods) {
                methods.entry(n.clone()).or_default().push(m.kids.clone());
            }
            Proj {
                access,
                by_key: (classes.iter().enumerate())
                    .map(|(i, c)| (key_of(&c.name), i))
                    .collect(),
                scopes: Vec::new(),
                libs: read_libs(lock, &classes),
                skip_named: skip_names(&classes),
                methods,
                classes,
                recs: Vec::new(),
                concerns: HashMap::new(),
                diags: Vec::new(),
                endpoints: Vec::new(),
                cache: HashMap::new(),
            }
        }

        pub(super) fn run(&mut self, root: &std::path::Path, files: &[(String, String)]) {
            let routes: Vec<(String, Vec<Stmt>)> = (files.iter())
                .filter(|(r, _)| r.ends_with("config/routes.rb"))
                .map(|(r, t)| (r.clone(), tree(t)))
                .collect();
            for (_, t) in &routes {
                self.read_devise(t);
            }
            for (rel, stmts) in &routes {
                let app_dir = rel.strip_suffix("config/routes.rb").unwrap_or("");
                let app_dir = app_dir.trim_end_matches('/');
                let module = isolated_module(files, app_dir);
                let ctx = Ctx::root(module);
                let app = dir_name(root, app_dir);
                routes::Walker::new(self, rel, app, app_dir, files).walk(stmts, &ctx);
            }
            self.resolve_recs();
        }

        fn resolve_recs(&mut self) {
            for r in std::mem::take(&mut self.recs) {
                let (handler, access) = self.rec_access(&r);
                self.endpoints.push(EndpointAccess {
                    method: r.method,
                    path: r.path,
                    access: r.layer.and(access),
                    application: Some(r.app),
                    file: r.file,
                    line: r.line,
                    source: r.source,
                    handler,
                    sends: Vec::new(),
                });
            }
        }

        fn rec_access(&mut self, r: &Rec) -> (Option<String>, AccessState) {
            let Some((ctrl, action)) = &r.target else {
                return (None, AccessState::Undeclared);
            };
            let name = format!("{}Controller", camelize(ctrl));
            let state = match self.lookup(&name) {
                Some(i) => self.cached(i, action),
                None => {
                    let msg = format!(
                        "Controller {name} not found in the project; only the route's own conditions apply."
                    );
                    note(&mut self.diags, &r.file, r.line, msg);
                    AccessState::Undeclared
                }
            };
            (Some(format!("{name}#{action}")), state)
        }

        fn read_devise(&mut self, stmts: &[Stmt]) {
            for s in stmts {
                if let Some(cl) = call(&s.text).filter(|c| c.name == "devise_for") {
                    let single = cl.args.opt("singular").and_then(V::name).map(String::from);
                    for n in cl.args.pos.iter().filter_map(V::name) {
                        let scope = single.clone().unwrap_or_else(|| singular(n));
                        if !self.scopes.contains(&scope) {
                            self.scopes.push(scope);
                        }
                    }
                }
                self.read_devise(&s.kids);
            }
        }

        /// The class `name` (e.g. `Admin::PostsController`): exact, else the one
        /// class whose name ends with it.
        pub(super) fn lookup(&self, name: &str) -> Option<usize> {
            let k = key_of(name);
            self.by_key.get(&k).copied().or_else(|| {
                let mut ends = self.by_key.iter().filter(|(c, _)| c.ends_with(&k));
                let first = ends.next()?;
                ends.next().is_none().then_some(*first.1)
            })
        }

        pub(super) fn resolve(&self, name: &str, lexical: &[String]) -> Option<usize> {
            (0..=lexical.len()).rev().find_map(|i| {
                let parts = lexical[..i].iter().map(String::as_str).chain([name]);
                self.by_key
                    .get(&key_of(&parts.collect::<Vec<_>>().join("::")))
                    .copied()
            })
        }

        fn cached(&mut self, cls: usize, action: &str) -> AccessState {
            let key = (cls, action.to_string());
            if let Some(s) = self.cache.get(&key) {
                return s.clone();
            }
            let mut e = Eval::new(&*self);
            let state = e.run(cls, action);
            let mut diags = e.diags;
            self.diags.append(&mut diags);
            self.cache.insert(key, state.clone());
            state
        }

        /// A listed meaning as a state; `Err` for a `$arg` form (no arguments at a callback).
        pub(super) fn listed(&self, name: &str) -> Option<Result<AccessState, ()>> {
            use AccessState::Protected as P;
            Some(match self.access.get(name)?.at_use_site(false) {
                AccessMeaning::Open => Ok(AccessState::Open),
                AccessMeaning::Authenticated => Ok(P(Condition::Authenticated)),
                AccessMeaning::Never => Ok(P(Condition::Never)),
                AccessMeaning::Role(n) => Ok(P(Condition::Role(n.clone()))),
                AccessMeaning::Policy(n) => Ok(P(Condition::Policy(n.clone()))),
                _ => Err(()),
            })
        }

        pub(super) fn devise_state(&self, scope: &str) -> AccessState {
            let auth = Condition::Authenticated;
            let default = self.scopes.first().is_some_and(|d| d == scope);
            let other = || Condition::Custom(format!("scope:{scope}"));
            AccessState::Protected(if default {
                auth
            } else {
                Condition::All(vec![auth, other()])
            })
        }

        /// The meaning of a guard when it is known to be one (listed non-open,
        /// Devise, generated).
        pub(super) fn known_guard(&self, name: &str) -> Option<AccessState> {
            if self.access.contains_key(name) {
                return self.listed(name)?.ok().filter(|s| *s != AccessState::Open);
            }
            let scope = name
                .strip_prefix("authenticate_")
                .and_then(|s| s.strip_suffix('!'));
            if let Some(s) = scope.filter(|s| self.scopes.iter().any(|x| x == s)) {
                return Some(self.devise_state(s));
            }
            let bodies = self
                .methods
                .get(name)
                .filter(|_| name == "require_authentication")?;
            let resumes = |k: &Vec<Stmt>| flat(k).iter().any(|s| s.text.contains("resume_session"));
            bodies
                .iter()
                .any(resumes)
                .then_some(AccessState::Protected(Condition::Authenticated))
        }

        pub(super) fn may_halt(&self, name: &str) -> bool {
            let Some(bodies) = self.methods.get(name) else {
                return true;
            };
            let halts = [
                "redirect_to",
                "redirect_back",
                "render",
                "head",
                "raise",
                "throw",
                "halt",
                "abort",
            ];
            let tokens = |s: &Stmt| {
                (s.text.split(|c: char| !(c.is_alphanumeric() || c == '_')))
                    .any(|t| halts.contains(&t))
            };
            let calls_project = |s: &Stmt| {
                let w = super::lex::first_word(&s.text);
                w != name && self.methods.contains_key(w)
            };
            bodies
                .iter()
                .any(|k| flat(k).iter().any(|s| tokens(s) || calls_project(s)))
        }

        /// Names a project macro `allow_unauthenticated_access` skips.
        pub(super) fn unauth_names(&self) -> Vec<String> {
            let macros = self
                .classes
                .iter()
                .filter_map(|c| c.class_methods.get("allow_unauthenticated_access"));
            let skips = macros.flatten().filter_map(|s| call(&s.text));
            let mut out = Vec::new();
            for cl in skips.filter(|cl| cl.name == "skip_before_action") {
                out.extend(
                    cl.args
                        .pos
                        .iter()
                        .filter_map(|v| v.name().map(String::from)),
                );
            }
            out
        }

        /// The events of `cls` in Rails order: superclass, then own and included.
        pub(super) fn events<'b>(
            &'b self,
            cls: usize,
            out: &mut Vec<&'b Ev>,
            seen: &mut HashSet<usize>,
            depth: usize,
        ) {
            let c = &self.classes[cls];
            if depth > 32 || !seen.insert(cls) {
                return;
            }
            let sup = c
                .superclass
                .as_deref()
                .and_then(|s| self.resolve(s, &c.lexical));
            if let Some(sup) = sup {
                self.events(sup, out, seen, depth + 1);
            }
            for ev in &c.events {
                match &ev.kind {
                    Kind::Include(n) => {
                        if let Some(m) = self.resolve(n, &c.lexical) {
                            self.events(m, out, seen, depth + 1);
                        }
                    }
                    _ => out.push(ev),
                }
            }
        }

        fn ancestors(&self, cls: usize, out: &mut Vec<usize>) {
            let c = &self.classes[cls];
            if out.len() > 64 || out.contains(&cls) {
                return;
            }
            out.push(cls);
            for ev in c.events.iter().rev() {
                if let Kind::Include(n) = &ev.kind
                    && let Some(m) = self.resolve(n, &c.lexical)
                {
                    self.ancestors(m, out);
                }
            }
            let sup = c
                .superclass
                .as_deref()
                .and_then(|s| self.resolve(s, &c.lexical));
            if let Some(sup) = sup {
                self.ancestors(sup, out);
            }
        }

        pub(super) fn find_method(&self, cls: usize, name: &str) -> Option<&super::model::Method> {
            let mut chain = Vec::new();
            self.ancestors(cls, &mut chain);
            chain
                .iter()
                .find_map(|&i| self.classes[i].methods.get(name))
        }
    }
}

/// Per-action evaluation of the callback chain and the action body.
mod eval {
    use super::lex::{Call, Stmt, V, call};
    use super::model::{Ev, Kind, Model, Opts};
    use super::names::{camelize, singular};
    use super::proj::Proj;
    use archguard_core::{AccessState, Condition, UiFlowDiagnostic};
    use std::collections::HashSet;

    enum Item {
        Cb { name: String, o: Opts, ev: Ev },
        Authz { model: Model, o: Opts, ev: Ev },
        Basic { o: Opts, ev: Ev },
    }

    /// What folding the chain found.
    #[derive(Default)]
    struct Acc {
        state: AccessState,
        listed_open: bool,
        halting: Vec<String>,
        callbacks: Vec<String>,
    }

    pub(super) struct Eval<'p, 'a> {
        p: &'p Proj<'a>,
        pub(super) diags: Vec<UiFlowDiagnostic>,
    }

    fn unknown(text: impl Into<String>) -> AccessState {
        AccessState::Protected(Condition::Unknown(text.into()))
    }

    fn remove_cb(chain: &mut Vec<Item>, n: &str) -> bool {
        let before = chain.len();
        chain.retain(|i| !matches!(i, Item::Cb { name, .. } if name == n));
        chain.len() != before
    }

    impl<'p, 'a> Eval<'p, 'a> {
        pub(super) fn new(p: &'p Proj<'a>) -> Self {
            Eval {
                p,
                diags: Vec::new(),
            }
        }

        fn gap(&mut self, ev: &Ev, msg: impl Into<String>) {
            crate::extract_ctx::gap(&mut self.diags, &ev.file, ev.line, msg);
        }

        fn note(&mut self, ev: &Ev, msg: impl Into<String>) {
            super::note(&mut self.diags, &ev.file, ev.line, msg);
        }

        pub(super) fn run(&mut self, cls: usize, action: &str) -> AccessState {
            let mut refs = Vec::new();
            self.p.events(cls, &mut refs, &mut HashSet::new(), 0);
            let evs: Vec<Ev> = refs.into_iter().cloned().collect();
            let (chain, skipped) = self.build_chain(&evs, action);
            let ctrl = self.p.classes[cls].name.clone();
            let mut acc = self.fold(&chain, &ctrl, action);
            let known = |n: &String| self.p.known_guard(n).is_some();
            let mut marker = acc.listed_open || skipped.iter().any(known);
            let (authz, skip_authz) = self.body(cls, action, &acc.callbacks, evs.first());
            acc.state = acc.state.and(authz);
            marker |= skip_authz;
            if acc.state != AccessState::Undeclared || !marker {
                return acc.state;
            }
            if acc.halting.is_empty() {
                return AccessState::Open;
            }
            let names = acc.halting.join(", ");
            if let Some(ev) = evs.first() {
                let msg = format!(
                    "An explicit open marker applies to {ctrl}#{action}, but callbacks {names} may halt and are not read; not shown as open."
                );
                self.gap(ev, msg);
            }
            unknown(names)
        }

        fn build_chain(&mut self, evs: &[Ev], action: &str) -> (Vec<Item>, Vec<String>) {
            let (mut chain, mut skipped) = (Vec::new(), Vec::new());
            for ev in evs {
                match &ev.kind {
                    Kind::Before { names, o, prepend } => {
                        self.register(&mut chain, ev, (names.as_slice(), o, *prepend), action)
                    }
                    Kind::Skip { names, o } => {
                        self.skip(&mut chain, &mut skipped, (ev, action), names, o)
                    }
                    Kind::Unauth { o } => self.unauth(&mut chain, &mut skipped, ev, o, action),
                    Kind::Authz { o, model } if o.applies(action) != Some(false) => {
                        let item = Item::Authz {
                            model: model.clone(),
                            o: o.clone(),
                            ev: ev.clone(),
                        };
                        chain.push(item);
                    }
                    Kind::SkipAuthz { o } if o.applies(action) == Some(true) => {
                        chain.retain(|i| !matches!(i, Item::Authz { .. }))
                    }
                    Kind::Basic { o } if o.applies(action) != Some(false) => {
                        chain.push(Item::Basic {
                            o: o.clone(),
                            ev: ev.clone(),
                        })
                    }
                    _ => {}
                }
            }
            (chain, skipped)
        }

        fn register(
            &mut self,
            chain: &mut Vec<Item>,
            ev: &Ev,
            reg: (&[String], &Opts, bool),
            action: &str,
        ) {
            let (names, o, prepend) = reg;
            for n in names {
                if remove_cb(chain, n) {
                    let msg = format!(
                        "before_action :{n} re-registered here replaces the earlier registration (ancestor or concern); its other actions lose that callback."
                    );
                    self.note(ev, msg);
                }
                if o.applies(action) != Some(false) {
                    let it = Item::Cb {
                        name: n.clone(),
                        o: o.clone(),
                        ev: ev.clone(),
                    };
                    if prepend {
                        chain.insert(0, it)
                    } else {
                        chain.push(it)
                    }
                }
            }
        }

        fn skip(
            &mut self,
            chain: &mut Vec<Item>,
            skipped: &mut Vec<String>,
            at: (&Ev, &str),
            names: &[String],
            o: &Opts,
        ) {
            let (ev, action) = at;
            for n in names {
                match (o.applies(action), o.cond.is_none()) {
                    (Some(true), true) => {
                        if remove_cb(chain, n) {
                            skipped.push(n.clone());
                        }
                    }
                    (Some(false), _) => {}
                    _ => {
                        let msg = format!(
                            "skip_before_action :{n} has a condition or a non-literal list; the callback is kept."
                        );
                        self.gap(ev, msg);
                    }
                }
            }
        }

        fn unauth(
            &mut self,
            chain: &mut Vec<Item>,
            skipped: &mut Vec<String>,
            ev: &Ev,
            o: &Opts,
            action: &str,
        ) {
            let names = self.p.unauth_names();
            if names.is_empty() {
                self.gap(ev, "allow_unauthenticated_access is not defined by a skip_before_action in the project; not read.");
            } else {
                self.skip(chain, skipped, (ev, action), &names, o);
            }
        }

        fn fold(&mut self, chain: &[Item], ctrl: &str, action: &str) -> Acc {
            let mut acc = Acc::default();
            for it in chain {
                match it {
                    Item::Cb { name, o, ev } => self.callback(&mut acc, name, o, ev, action),
                    Item::Authz { model, o, ev } => {
                        let s = self.authz(model, o, ev, ctrl, action);
                        acc.state = std::mem::take(&mut acc.state).and(s);
                    }
                    Item::Basic { o, ev } => {
                        let s = self.basic(o, ev, action);
                        acc.state = std::mem::take(&mut acc.state).and(s);
                    }
                }
            }
            acc
        }

        /// The meaning a callback has by `[views.access]`, Devise or generator.
        fn meaning(&mut self, name: &str, ev: &Ev) -> Option<AccessState> {
            match self.p.listed(name) {
                Some(Ok(s)) => Some(s),
                Some(Err(())) => {
                    let msg = format!(
                        "[views.access] entry for {name} needs arguments; a callback has none."
                    );
                    self.gap(ev, msg);
                    Some(unknown(name))
                }
                None => self.p.known_guard(name),
            }
        }

        fn callback(&mut self, acc: &mut Acc, name: &str, o: &Opts, ev: &Ev, action: &str) {
            acc.callbacks.push(name.to_string());
            let unreadable = o.applies(action).is_none();
            let cond = o
                .cond
                .clone()
                .or_else(|| unreadable.then(|| "only/except not a literal list".to_string()));
            let Some(meaning) = self.meaning(name, ev) else {
                return self.unread(acc, name, ev);
            };
            match (meaning, cond) {
                (_, Some(c)) => {
                    let msg = format!(
                        "before_action :{name} has {c}; whether it guards this action is not readable."
                    );
                    self.gap(ev, msg);
                    acc.state = std::mem::take(&mut acc.state).and(unknown(format!("{name} {c}")));
                }
                (AccessState::Open, None) => acc.listed_open = true,
                (s, None) => acc.state = std::mem::take(&mut acc.state).and(s),
            }
        }

        /// A callback without a meaning: unknown when a skip names it; a gap when it may halt.
        fn unread(&mut self, acc: &mut Acc, name: &str, ev: &Ev) {
            if self.p.skip_named.contains(name) {
                let msg = format!(
                    "before_action :{name} is skipped elsewhere, so it is a guard whose meaning is unknown; list it in [views.access]."
                );
                self.gap(ev, msg);
                acc.state = std::mem::take(&mut acc.state).and(unknown(name));
            } else if self.p.may_halt(name) {
                let msg = format!(
                    "before_action :{name} has no access meaning (project method or block); list it in [views.access] if it guards."
                );
                self.gap(ev, msg);
                acc.halting.push(name.to_string());
            }
        }

        fn authz(
            &mut self,
            model: &Model,
            o: &Opts,
            ev: &Ev,
            ctrl: &str,
            action: &str,
        ) -> AccessState {
            let m = match model {
                Model::Named(n) => Ok(n.clone()),
                Model::Default => Ok(singular(
                    ctrl.rsplit("::")
                        .next()
                        .unwrap_or("")
                        .trim_end_matches("Controller"),
                )),
                Model::Unreadable(t) => Err(t.clone()),
            };
            match (m, o.applies(action)) {
                (Ok(m), Some(true)) if !m.is_empty() => {
                    AccessState::Protected(Condition::Policy(format!("{action} {m}")))
                }
                (m, _) => {
                    let why = m
                        .err()
                        .unwrap_or_else(|| "only/except not a literal list".into());
                    self.gap(
                        ev,
                        format!("authorize_resource: {why}; the checked subject is not readable."),
                    );
                    unknown("authorize_resource")
                }
            }
        }

        fn basic(&mut self, o: &Opts, ev: &Ev, action: &str) -> AccessState {
            let defines = |n: &str| {
                self.p
                    .classes
                    .iter()
                    .any(|c| c.class_methods.contains_key(n) || c.methods.contains_key(n))
            };
            if defines("http_basic_authenticate_with")
                || o.applies(action).is_none()
                || o.cond.is_some()
            {
                self.gap(ev, "http_basic_authenticate_with is redefined in the project or has a condition; its effect is not read.");
                unknown("http_basic_authenticate_with")
            } else {
                AccessState::Protected(Condition::Authenticated)
            }
        }

        /// Authorization read from the action body: (state, `skip_authorization`).
        fn body(
            &mut self,
            cls: usize,
            action: &str,
            callbacks: &[String],
            at: Option<&Ev>,
        ) -> (AccessState, bool) {
            let none = (AccessState::Undeclared, false);
            let p = self.p;
            let Some(m) = p.find_method(cls, action) else {
                return none;
            };
            let mut sources = vec![m.kids.clone()];
            sources.extend(
                callbacks
                    .iter()
                    .filter_map(|n| p.find_method(cls, n))
                    .map(|c| c.kids.clone()),
            );
            let ev = at.cloned().unwrap_or(Ev {
                kind: Kind::Include(String::new()),
                file: String::new(),
                line: 0,
            });
            let (mut state, mut skip) = (AccessState::Undeclared, false);
            for (cl, nested) in body_calls(&m.kids) {
                let conditional = nested || cl.modifier.is_some();
                match cl.name.as_str() {
                    "skip_authorization" if !conditional => skip = true,
                    "authorize" | "authorize!" if conditional => {
                        let msg = format!(
                            "{} in {}#{action} sits under a condition; not read as a guard.",
                            cl.name, self.p.classes[cls].name
                        );
                        self.note(&ev, msg);
                    }
                    "authorize" | "authorize!" => {
                        let s = self.authorize(&cl, action, &sources, &ev, cls);
                        state = state.and(s);
                    }
                    _ => {}
                }
            }
            (state, skip)
        }

        fn authorize(
            &mut self,
            cl: &Call,
            action: &str,
            sources: &[Vec<Stmt>],
            ev: &Ev,
            cls: usize,
        ) -> AccessState {
            let text = format!(
                "{} {}",
                cl.name,
                cl.args
                    .pos
                    .iter()
                    .map(V::text)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let a = Auth {
                cl,
                action,
                sources,
            };
            let policy = if cl.name == "authorize" {
                pundit(self.p, &a)
            } else {
                bang(self.p, &a)
            };
            match policy {
                Ok(p) => AccessState::Protected(Condition::Policy(p)),
                Err(why) => {
                    let msg = format!("{text} in {}#{action}: {why}", self.p.classes[cls].name);
                    crate::extract_ctx::gap(&mut self.diags, &ev.file, ev.line, msg);
                    unknown(text)
                }
            }
        }
    }

    /// The calls of an action body, with whether each sits under a block.
    fn body_calls(kids: &[Stmt]) -> Vec<(Call, bool)> {
        let mut stack: Vec<(&Stmt, bool)> = kids.iter().rev().map(|s| (s, false)).collect();
        let mut found = Vec::new();
        while let Some((s, nested)) = stack.pop() {
            stack.extend(s.kids.iter().rev().map(|k| (k, true)));
            found.extend(call(&s.text).map(|cl| (cl, nested || s.block)));
        }
        found
    }

    struct Auth<'x> {
        cl: &'x Call,
        action: &'x str,
        sources: &'x [Vec<Stmt>],
    }

    /// The class `@ivar = Const.…` assigns, when every assignment agrees.
    fn assigned_class(sources: &[Vec<Stmt>], ivar: &str) -> Option<String> {
        let mut found: Vec<String> = Vec::new();
        for s in sources.iter().flat_map(|k| super::lex::flat(k)) {
            let Some(rest) = s.text.strip_prefix(ivar).map(str::trim_start) else {
                continue;
            };
            let eq = rest
                .strip_prefix("||=")
                .or_else(|| rest.strip_prefix('=').filter(|r| !r.starts_with('=')));
            let Some(rhs) = eq.map(str::trim) else {
                continue;
            };
            let c: String = rhs
                .chars()
                .take_while(|c| c.is_alphanumeric() || "_:".contains(*c))
                .collect();
            if !(c.starts_with(|c: char| c.is_ascii_uppercase()) && rhs[c.len()..].starts_with('.'))
            {
                return None;
            }
            found.push(c);
        }
        found.dedup();
        (found.len() == 1).then(|| found.remove(0))
    }

    fn class_of(a: &Auth, v: &V) -> Option<String> {
        match v {
            V::Const(c) => Some(c.clone()),
            V::Raw(r) if r.starts_with('@') => assigned_class(a.sources, r),
            _ => None,
        }
    }

    /// A record argument's class or a symbol's name.
    fn subject_class(a: &Auth, v: &V) -> Option<String> {
        match v {
            V::Sym(s) => Some(camelize(s)),
            v => class_of(a, v),
        }
    }

    fn query(a: &Auth, i: usize) -> String {
        match a.cl.args.pos.get(i) {
            Some(V::Sym(q)) => q.clone(),
            _ => format!("{}?", a.action),
        }
    }

    /// The policy class a Pundit `authorize` checks.
    fn pundit_policy(a: &Auth) -> Option<String> {
        if let Some(v) = a.cl.args.opt("policy_class") {
            return if let V::Const(c) = v {
                Some(c.clone())
            } else {
                None
            };
        }
        match a.cl.args.pos.first()? {
            V::List(items) if items.len() >= 2 => {
                let ns: Option<Vec<String>> = items[..items.len() - 1]
                    .iter()
                    .map(|v| v.name().map(camelize))
                    .collect();
                let last = subject_class(a, items.last()?)?;
                Some(format!("{}::{last}Policy", ns?.join("::")))
            }
            v => subject_class(a, v).map(|c| format!("{c}Policy")),
        }
    }

    fn pundit(p: &Proj<'_>, a: &Auth) -> Result<String, String> {
        if !p.libs.pundit {
            return Err("Pundit is not among the gems or includes; the library behind authorize is unknown.".into());
        }
        let policy = pundit_policy(a).ok_or("the checked record's class is not readable.")?;
        Ok(format!("{policy}#{}", query(a, 1)))
    }

    fn bang(p: &Proj<'_>, a: &Auth) -> Result<String, String> {
        match (p.libs.cancan, p.libs.action_policy) {
            (true, false) => cancan(a),
            (false, true) => action_policy(a),
            (true, true) => Err("both cancancan and action_policy are in the Gemfile.lock; authorize! is ambiguous.".into()),
            (false, false) => Err("neither cancancan nor action_policy is in the Gemfile.lock; authorize! is ambiguous.".into()),
        }
    }

    fn cancan(a: &Auth) -> Result<String, String> {
        let subject = a.cl.args.pos.get(1).and_then(|v| subject_class(a, v));
        match (a.cl.args.pos.first().and_then(V::name), subject) {
            (Some(act), Some(s)) => Ok(format!("{act} {s}")),
            _ => Err("the action or subject is not readable.".into()),
        }
    }

    fn action_policy(a: &Auth) -> Result<String, String> {
        let policy = match a.cl.args.opt("with") {
            Some(V::Const(c)) => Some(c.clone()),
            Some(_) => None,
            None => {
                a.cl.args
                    .pos
                    .first()
                    .and_then(|v| class_of(a, v))
                    .map(|c| format!("{c}Policy"))
            }
        };
        let rule = match a.cl.args.opt("to") {
            Some(V::Sym(q)) => q.clone(),
            _ => format!("{}?", a.action),
        };
        policy
            .map(|p| format!("{p}#{rule}"))
            .ok_or_else(|| "the checked record's class is not readable.".into())
    }
}

/// Reading `config/routes.rb`.
mod routes {
    use super::lex::{Args, Call, Stmt, V, call, tree};
    use super::names::{clean_path, join_mod, join_path, pluralize, singular};
    use super::proj::{Proj, Rec};
    use archguard_core::{ANY_METHOD, AccessState, Condition};

    #[derive(Clone)]
    pub(super) struct Res {
        ctrl: String,
        coll: String,
        member: String,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum On {
        None,
        Member,
        Collection,
    }

    #[derive(Clone)]
    pub(super) struct Ctx {
        path: String,
        module: String,
        controller: Option<String>,
        layer: AccessState,
        res: Option<Res>,
        on: On,
    }

    /// `"posts#index"` below the context's module as (controller path, action).
    fn split_target(t: &str, ctx: &Ctx) -> Option<(String, String)> {
        let (c, a) = t.split_once('#')?;
        Some((join_mod(&ctx.module, c), a.to_string()))
    }

    impl Ctx {
        /// The context inside `namespace`, `scope`, `member` or `collection`.
        fn scoped(mut self, cl: &Call) -> Ctx {
            let opt = |k: &str| cl.args.opt(k).and_then(V::name);
            let first = cl.args.pos.first().and_then(V::name);
            match cl.name.as_str() {
                "namespace" => {
                    let n = first.unwrap_or("");
                    self.path = join_path(&self.path, opt("path").unwrap_or(n));
                    self.module = join_mod(&self.module, opt("module").unwrap_or(n));
                }
                "scope" => {
                    if let Some(p) = first.or(opt("path")) {
                        self.path = join_path(&self.path, p);
                    }
                    if let Some(m) = opt("module") {
                        self.module = join_mod(&self.module, m);
                    }
                    self.controller = opt("controller").map(String::from).or(self.controller);
                }
                name => {
                    let member = name == "member";
                    if let Some(r) = self.res.clone() {
                        self.path = if member { r.member } else { r.coll };
                        self.on = if member { On::Member } else { On::Collection };
                    }
                }
            }
            self
        }

        pub(super) fn root(module: String) -> Ctx {
            Ctx {
                path: String::new(),
                module,
                controller: None,
                layer: AccessState::Undeclared,
                res: None,
                on: On::None,
            }
        }
    }

    pub(super) struct Walker<'p, 'a> {
        p: &'p mut Proj<'a>,
        file: String,
        app: String,
        app_dir: String,
        files: &'p [(String, String)],
    }

    /// `only:`/`except:` of a `resources` call.
    struct Selection {
        only: Option<Vec<String>>,
        except: Option<Vec<String>>,
        unreadable: bool,
    }

    impl Selection {
        fn read(a: &Args) -> Selection {
            let list = |k: &str| a.opt(k).map(|v| v.names());
            let (only, except) = (list("only"), list("except"));
            let unreadable = matches!(only, Some(None)) || matches!(except, Some(None));
            Selection {
                only: only.flatten(),
                except: except.flatten(),
                unreadable,
            }
        }

        fn wants(&self, action: &str) -> bool {
            let has = |l: &Option<Vec<String>>, n: &str| {
                l.as_ref().is_some_and(|l| l.iter().any(|x| x == n))
            };
            let only = self
                .only
                .as_ref()
                .is_none_or(|l| l.iter().any(|x| x == action || x == "all"));
            only && !has(&self.only, "none") && !has(&self.except, action)
        }
    }

    /// Methods and path of one standard action of a resource.
    fn standard(res: &Res, action: &str) -> (&'static [&'static str], String) {
        match action {
            "index" => (&["GET"], res.coll.clone()),
            "create" => (&["POST"], res.coll.clone()),
            "new" => (&["GET"], join_path(&res.coll, "new")),
            "edit" => (&["GET"], join_path(&res.member, "edit")),
            "show" => (&["GET"], res.member.clone()),
            "update" => (&["PATCH", "PUT"], res.member.clone()),
            _ => (&["DELETE"], res.member.clone()),
        }
    }

    fn is_draw(t: &str) -> bool {
        t.contains(".routes.draw") || t.ends_with("add_routes do") || t == "draw do"
    }

    impl<'p, 'a> Walker<'p, 'a> {
        pub(super) fn new(
            p: &'p mut Proj<'a>,
            file: &str,
            app: String,
            app_dir: &str,
            files: &'p [(String, String)],
        ) -> Self {
            Walker {
                p,
                file: file.into(),
                app,
                app_dir: app_dir.into(),
                files,
            }
        }

        fn gap(&mut self, line: u32, msg: impl Into<String>) {
            crate::extract_ctx::gap(&mut self.p.diags, &self.file, line, msg);
        }

        pub(super) fn walk(&mut self, stmts: &[Stmt], ctx: &Ctx) {
            for s in stmts {
                self.stmt(s, ctx);
            }
        }

        fn stmt(&mut self, s: &Stmt, ctx: &Ctx) {
            let Some(cl) = call(&s.text).filter(|_| !is_draw(&s.text)) else {
                return self.walk(&s.kids, ctx);
            };
            if !self.scoping(&cl, s, ctx) {
                self.leaf(&cl, s, ctx);
            }
        }

        /// Statements that open a scope; false for any other.
        fn scoping(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) -> bool {
            let line = s.line;
            match cl.name.as_str() {
                "namespace" | "scope" | "member" | "collection" => {
                    let c = self.with_constraints(&cl.args, ctx, line);
                    let c = c.scoped(cl);
                    self.walk(&s.kids, &c);
                }
                "constraints" => {
                    let mut c = ctx.clone();
                    for v in &cl.args.pos {
                        let st = self.constraint(v, line);
                        c.layer = c.layer.and(st);
                    }
                    self.walk(&s.kids, &c);
                }
                "authenticate" | "authenticated" | "unauthenticated" => {
                    let c = self.devise_block(cl, ctx, line);
                    self.walk(&s.kids, &c);
                }
                "concern" | "concerns" | "draw" => self.inline(cl, s, ctx),
                "devise_scope" | "shallow" | "defaults" | "with_options" => {
                    if cl.name == "shallow" {
                        self.gap(line, "shallow routes are not read as shallow; nested paths are listed in full.");
                    }
                    self.walk(&s.kids, ctx);
                }
                _ => return false,
            }
            true
        }

        fn leaf(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) {
            match cl.name.as_str() {
                "resources" | "resource" => self.resources(cl, s, ctx),
                "root" => self.root(cl, s, ctx),
                "get" | "post" | "put" | "patch" | "delete" | "match" => self.verb(cl, s, ctx),
                "mount" => self.mount(cl, s, ctx),
                "devise_for" | "direct" | "resolve" | "devise_group" => {}
                _ if s.block => self.walk(&s.kids, ctx),
                _ => {}
            }
        }

        /// The state of a constraint/guard value: a listed class, an unlisted
        /// class or a lambda (`unknown` with a gap); a hash or nothing adds none.
        fn constraint(&mut self, v: &V, line: u32) -> AccessState {
            let unknown = |t: &str| AccessState::Protected(Condition::Unknown(t.into()));
            match v {
                V::Const(n) => match self.p.listed(n) {
                    Some(Ok(s)) => s,
                    _ => {
                        self.gap(line, format!("Route constraint {n} is a class whose access meaning is unknown; list it in [views.access]."));
                        unknown(n)
                    }
                },
                V::Lambda(t) => {
                    self.gap(line, format!("Route constraint {t} is a lambda; not read."));
                    unknown(t)
                }
                V::Raw(r) if r.starts_with('{') && !r.contains("->") => AccessState::Undeclared,
                V::Raw(r) => {
                    self.gap(line, format!("Route constraint {r} is not readable."));
                    unknown(r)
                }
                _ => AccessState::Undeclared,
            }
        }

        fn with_constraints(&mut self, a: &Args, ctx: &Ctx, line: u32) -> Ctx {
            let mut c = ctx.clone();
            if let Some(v) = a.opt("constraints") {
                let s = self.constraint(v, line);
                c.layer = c.layer.and(s);
            }
            c
        }

        /// The state of a Devise block, from its scope and optional lambda.
        fn devise_block(&mut self, cl: &Call, ctx: &Ctx, line: u32) -> Ctx {
            use AccessState::Protected as P;
            let scope = cl.args.pos.iter().find_map(|v| v.name().map(String::from));
            let base = match &scope {
                Some(s) if self.p.scopes.iter().any(|x| x == s) => self.p.devise_state(s),
                Some(s) => {
                    self.gap(
                        line,
                        format!(
                            "Devise scope {s} is not declared by devise_for; its guard is unknown."
                        ),
                    );
                    P(Condition::All(vec![
                        Condition::Authenticated,
                        Condition::Unknown(format!("scope:{s}")),
                    ]))
                }
                None => match self.p.scopes.first() {
                    Some(d) => self.p.devise_state(&d.clone()),
                    None => P(Condition::Authenticated),
                },
            };
            let mut state = base;
            if let Some(l) = cl.args.pos.iter().find(|v| matches!(v, V::Lambda(_))) {
                self.gap(
                    line,
                    format!("Devise {} lambda {} is not read.", cl.name, l.text()),
                );
                state = state.and(P(Condition::Unknown(l.text())));
            }
            if let (true, P(c)) = (cl.name == "unauthenticated", &state) {
                state = P(Condition::Not(Box::new(c.clone())));
            }
            let mut c = ctx.clone();
            c.layer = c.layer.and(state);
            c
        }

        /// `concern`, `concerns` and `draw`: statements from elsewhere.
        fn inline(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) {
            match cl.name.as_str() {
                "concern" => {
                    if let Some(n) = cl.args.pos.first().and_then(V::name) {
                        self.p.concerns.insert(n.to_string(), s.kids.clone());
                    }
                }
                "concerns" => {
                    for n in cl.args.pos.iter().filter_map(V::name) {
                        self.concern(n, ctx, s.line);
                    }
                }
                _ => self.draw(cl, ctx, s.line),
            }
        }

        fn concern(&mut self, n: &str, ctx: &Ctx, line: u32) {
            match self.p.concerns.get(n).cloned() {
                Some(k) => self.walk(&k, ctx),
                None => self.gap(
                    line,
                    format!("Routing concern {n} not found; its routes are unknown."),
                ),
            }
        }

        fn draw(&mut self, cl: &Call, ctx: &Ctx, line: u32) {
            let Some(n) = cl.args.pos.first().and_then(V::name) else {
                return;
            };
            let want = format!("{}/config/routes/{n}.rb", self.app_dir);
            let want = want.trim_start_matches('/');
            match self.files.iter().find(|(r, _)| r == want) {
                Some((_, text)) => self.walk(&tree(text), ctx),
                None => self.gap(line, format!("Route file config/routes/{n}.rb not found.")),
            }
        }

        fn push(
            &mut self,
            method: &str,
            path: String,
            target: Option<(String, String)>,
            ctx: &Ctx,
            at: (u32, String),
        ) {
            self.p.recs.push(Rec {
                method: method.into(),
                path: clean_path(&path),
                target,
                layer: ctx.layer.clone(),
                file: self.file.clone(),
                line: at.0,
                source: at.1,
                app: self.app.clone(),
            });
        }

        fn root(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) {
            let c = self.with_constraints(&cl.args, ctx, s.line);
            let to = cl.args.opt("to").or(cl.args.pos.first());
            let target = to.and_then(V::name).and_then(|t| split_target(t, &c));
            let path = if c.path.is_empty() {
                "/".into()
            } else {
                c.path.clone()
            };
            self.push("GET", path, target, &c, (s.line, s.text.clone()));
        }

        fn mount(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) {
            let c = self.with_constraints(&cl.args, ctx, s.line);
            let at = cl.args.pairs.first().map(|(_, r)| r).or(cl.args.opt("at"));
            if c.layer == AccessState::Undeclared {
                let what = cl.args.pos.first().map_or("an engine".into(), V::text);
                self.gap(
                    s.line,
                    format!("mount {what} outside any guard: the engine's own access is not read."),
                );
            }
            match at.and_then(V::name) {
                Some(p) => self.push(
                    ANY_METHOD,
                    join_path(&c.path, p),
                    None,
                    &c,
                    (s.line, s.text.clone()),
                ),
                None => self.gap(
                    s.line,
                    "mount path is not a literal; the mounted app is unknown.",
                ),
            }
        }

        fn resources(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) {
            let c0 = self.with_constraints(&cl.args, ctx, s.line);
            let sel = Selection::read(&cl.args);
            if sel.unreadable {
                self.gap(
                    s.line,
                    format!(
                        "{} only:/except: is not a literal list; every standard action is listed.",
                        cl.name
                    ),
                );
            }
            for name in cl.args.pos.iter().filter_map(V::name) {
                self.resource(cl, s, &c0, &sel, name);
            }
        }

        fn resource(&mut self, cl: &Call, s: &Stmt, c0: &Ctx, sel: &Selection, name: &str) {
            let single = cl.name == "resource";
            let opt = |k: &str| cl.args.opt(k).and_then(V::name);
            let module = opt("module").map_or(c0.module.clone(), |m| join_mod(&c0.module, m));
            let plural = if single {
                pluralize(name)
            } else {
                name.to_string()
            };
            let ctrl = join_mod(&module, opt("controller").unwrap_or(&plural));
            let param = opt("param").unwrap_or("id");
            let coll = join_path(&c0.path, opt("path").unwrap_or(name));
            let member = if single {
                coll.clone()
            } else {
                join_path(&coll, &format!(":{param}"))
            };
            let nested = if single {
                coll.clone()
            } else {
                join_path(&coll, &format!(":{}_{param}", singular(name)))
            };
            let res = Res { ctrl, coll, member };
            let all: &[&str] = if single {
                &["show", "new", "create", "edit", "update", "destroy"]
            } else {
                &[
                    "index", "show", "new", "create", "edit", "update", "destroy",
                ]
            };
            for a in all.iter().filter(|a| sel.wants(a)) {
                let (methods, base) = standard(&res, a);
                for m in methods {
                    let target = Some((res.ctrl.clone(), a.to_string()));
                    self.push(
                        m,
                        base.clone(),
                        target,
                        c0,
                        (s.line, format!("{} :{name}", cl.name)),
                    );
                }
            }
            let child = Ctx {
                path: nested,
                module,
                res: Some(res),
                on: On::None,
                ..c0.clone()
            };
            if let Some(v) = cl.args.opt("concerns") {
                for n in v.names().unwrap_or_default() {
                    self.concern(&n, &child, s.line);
                }
            }
            self.walk(&s.kids, &child);
        }

        fn verb_methods(&mut self, cl: &Call, line: u32) -> Vec<String> {
            let methods: Vec<String> = match (cl.name.as_str(), cl.args.opt("via")) {
                ("match", Some(V::Sym(a))) if a == "all" => vec![ANY_METHOD.into()],
                ("match", Some(v)) => v
                    .names()
                    .unwrap_or_default()
                    .iter()
                    .map(|m| m.to_uppercase())
                    .collect(),
                ("match", None) => Vec::new(),
                _ => vec![cl.name.to_uppercase()],
            };
            if methods.is_empty() {
                self.gap(
                    line,
                    "match without a literal via:; the verbs are unknown (listed as any).",
                );
                return vec![ANY_METHOD.to_string()];
            }
            methods
        }

        /// The (controller path, action) a verb route points at.
        fn verb_target(
            &mut self,
            cl: &Call,
            c: &Ctx,
            to: Option<&V>,
            name: &str,
            line: u32,
        ) -> Option<(String, String)> {
            let last = name.rsplit('/').next().unwrap_or("").to_string();
            let opt = |k: &str| cl.args.opt(k).and_then(V::name).map(String::from);
            let (ctl, act) = (opt("controller"), opt("action"));
            let ctrl_of = |k: &str| join_mod(&c.module, k);
            match to {
                Some(V::Str(t)) => return split_target(t, c),
                Some(_) => return None,
                None => {}
            }
            if ctl.is_some() || act.is_some() {
                let ctrl = ctl
                    .or_else(|| c.controller.clone())
                    .or_else(|| c.res.as_ref().map(|r| r.ctrl.clone()));
                return ctrl.map(|k| (ctrl_of(&k), act.unwrap_or(last)));
            }
            if let Some(r) = &c.res {
                return Some((r.ctrl.clone(), last));
            }
            if let Some(k) = &c.controller {
                return Some((ctrl_of(k), last));
            }
            if let Some((k, a)) = name.rsplit_once('/') {
                return Some((ctrl_of(k), a.to_string()));
            }
            self.gap(
                line,
                format!(
                    "route \"{name}\" has no target (to:/controller:); its controller is unknown."
                ),
            );
            None
        }

        fn verb(&mut self, cl: &Call, s: &Stmt, ctx: &Ctx) {
            let line = s.line;
            let c = self.with_constraints(&cl.args, ctx, line);
            let methods = self.verb_methods(cl, line);
            let (path_v, to_v) = match cl.args.pairs.first() {
                Some((l, r)) => (Some(l), Some(r)),
                None => (cl.args.pos.first(), cl.args.opt("to")),
            };
            let Some(path) = path_v.and_then(V::name).map(String::from) else {
                return self.gap(
                    line,
                    format!(
                        "{} route path is not a literal; the route is unknown.",
                        cl.name
                    ),
                );
            };
            let on = match cl.args.opt("on").and_then(V::name) {
                Some("member") => On::Member,
                Some("collection") => On::Collection,
                _ => c.on,
            };
            let base = match (&c.res, on) {
                (Some(r), On::Member) => r.member.clone(),
                (Some(r), On::Collection) => r.coll.clone(),
                _ => c.path.clone(),
            };
            let target = self.verb_target(cl, &c, to_v, path.trim_matches('/'), line);
            let full = join_path(&base, &path);
            for m in methods {
                self.push(&m, full.clone(), target.clone(), &c, (line, s.text.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState as A, Condition as C, Config, DiagnosticSeverity};

    fn run(files: &[(&str, &str)], access: &str) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let config: Config = toml::from_str(&format!("[views.access]\n{access}")).unwrap();
        extract_with(&ExtractContext::new(dir.path(), &config))
    }

    fn state(flow: &UiFlow, method: &str, path: &str) -> A {
        let found: Vec<_> = (flow.endpoints.iter())
            .filter(|e| e.method == method && e.path == path)
            .collect();
        assert_eq!(found.len(), 1, "{method} {path} in {:?}", flow.endpoints);
        found[0].access.clone()
    }

    fn has(flow: &UiFlow, severity: DiagnosticSeverity, part: &str) -> bool {
        (flow.diagnostics.iter()).any(|d| d.severity == severity && d.message.contains(part))
    }

    fn auth() -> A {
        A::Protected(C::Authenticated)
    }

    fn pol(n: &str) -> A {
        A::Protected(C::Policy(n.into()))
    }

    fn unknown(n: &str) -> A {
        A::Protected(C::Unknown(n.into()))
    }

    const APP: &str = "class ApplicationController < ActionController::Base\n  before_action :authenticate_user!\nend\n";

    fn routes(body: &str) -> String {
        format!("Rails.application.routes.draw do\n  devise_for :users\n{body}end\n")
    }

    #[test]
    fn f01_devise_before_action() {
        let r = routes("  resources :posts, only: [:index]\n");
        let flow = run(
            &[
                ("config/routes.rb", &r),
                ("app/controllers/application_controller.rb", APP),
                (
                    "app/controllers/posts_controller.rb",
                    "class PostsController < ApplicationController\n  def index\n  end\nend\n",
                ),
            ],
            "",
        );
        assert_eq!(state(&flow, "GET", "/posts"), auth());
        assert_eq!(
            flow.endpoints[0].handler.as_deref(),
            Some("PostsController#index")
        );
    }

    const PAGES_ROUTES: &str =
        "  get \"home\", to: \"pages#home\"\n  get \"about\", to: \"pages#about\"\n";

    #[test]
    fn f02_skip_before_action_opens_only_listed() {
        let r = routes(PAGES_ROUTES);
        let flow = run(
            &[
                ("config/routes.rb", &r),
                ("app/controllers/application_controller.rb", APP),
                (
                    "app/controllers/pages_controller.rb",
                    "class PagesController < ApplicationController\n  skip_before_action :authenticate_user!, only: [:home]\nend\n",
                ),
            ],
            "",
        );
        assert_eq!(state(&flow, "GET", "/home"), A::Open);
        assert_eq!(state(&flow, "GET", "/about"), auth());
    }

    #[test]
    fn f03_reregistration_replaces_parent_callback() {
        let r = routes("  resources :posts, only: [:index, :edit]\n");
        let flow = run(
            &[
                ("config/routes.rb", &r),
                ("app/controllers/application_controller.rb", APP),
                (
                    "app/controllers/posts_controller.rb",
                    "class PostsController < ApplicationController\n  before_action :authenticate_user!, only: [:edit]\nend\n",
                ),
            ],
            "",
        );
        assert_eq!(state(&flow, "GET", "/posts/:id/edit"), auth());
        assert_eq!(state(&flow, "GET", "/posts"), A::Undeclared);
        assert!(has(&flow, DiagnosticSeverity::Note, "replaces"));
    }

    #[test]
    fn f04_project_defined_guard_has_no_meaning_unless_listed() {
        let files = [
            (
                "config/routes.rb",
                "Rails.application.routes.draw do\n  get \"x\", to: \"pages#x\"\nend\n",
            ),
            (
                "app/controllers/application_controller.rb",
                "class ApplicationController\n  before_action :authenticate_user!\n  def authenticate_user!\n    redirect_to \"/login\" unless session[:u]\n  end\nend\n",
            ),
            (
                "app/controllers/pages_controller.rb",
                "class PagesController < ApplicationController\nend\n",
            ),
        ];
        let flow = run(&files, "");
        assert_eq!(state(&flow, "GET", "/x"), A::Undeclared);
        assert!(has(&flow, DiagnosticSeverity::Gap, "authenticate_user!"));
        let flow = run(&files, "\"authenticate_user!\" = \"authenticated\"\n");
        assert_eq!(state(&flow, "GET", "/x"), auth());
    }

    #[test]
    fn f05_conditional_guard_is_unknown() {
        let flow = run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  get \"x\", to: \"pages#x\"\nend\n",
                ),
                (
                    "app/controllers/pages_controller.rb",
                    "class PagesController < ApplicationController\n  before_action :require_admin, if: -> { params[:scope] == \"all\" }\nend\n",
                ),
            ],
            "require_admin = \"role:admin\"\n",
        );
        assert!(
            matches!(state(&flow, "GET", "/x"), A::Protected(C::Unknown(t)) if t.contains("require_admin"))
        );
        assert!(has(&flow, DiagnosticSeverity::Gap, "require_admin"));
        assert!(!flow.complete);
    }

    #[test]
    fn f06_devise_authenticate_block() {
        let r = routes(
            "  authenticate :user do\n    resources :reports, only: [:index]\n  end\n  authenticate :user, ->(u) { u.admin? } do\n    mount Sidekiq::Web => \"/sidekiq\"\n  end\n",
        );
        let flow = run(&[("config/routes.rb", &r)], "");
        assert_eq!(state(&flow, "GET", "/reports"), auth());
        match state(&flow, "ANY", "/sidekiq") {
            A::Protected(C::All(items)) => {
                assert_eq!(items[0], C::Authenticated);
                assert!(matches!(&items[1], C::Unknown(t) if t.contains("u.admin?")));
            }
            other => panic!("{other:?}"),
        }
        assert!(has(&flow, DiagnosticSeverity::Gap, "u.admin?"));
    }

    #[test]
    fn f07_authenticated_and_unauthenticated_roots() {
        let r = routes(
            "  authenticated :user do\n    root to: \"dashboard#show\", as: :authenticated_root\n  end\n  root to: \"landing#show\"\n",
        );
        let flow = run(&[("config/routes.rb", &r)], "");
        let roots: Vec<_> = (flow.endpoints.iter())
            .filter(|e| e.path == "/")
            .map(|e| (e.handler.clone().unwrap(), e.access.clone()))
            .collect();
        assert!(roots.contains(&("DashboardController#show".into(), auth())));
        assert!(roots.contains(&("LandingController#show".into(), A::Undeclared)));
    }

    #[test]
    fn f08_constraints() {
        let r = routes(
            "  constraints(Constraints::Admin) do\n    resources :flags, only: [:index]\n  end\n  constraints(subdomain: \"api\") do\n    resources :things, only: [:index]\n  end\n",
        );
        let flow = run(&[("config/routes.rb", &r)], "");
        assert_eq!(state(&flow, "GET", "/flags"), unknown("Constraints::Admin"));
        assert_eq!(state(&flow, "GET", "/things"), A::Undeclared);
        assert!(has(&flow, DiagnosticSeverity::Gap, "Constraints::Admin"));
        let flow = run(
            &[("config/routes.rb", &r)],
            "\"Constraints::Admin\" = \"role:admin\"\n",
        );
        assert_eq!(
            state(&flow, "GET", "/flags"),
            A::Protected(C::Role("admin".into()))
        );
    }

    const AUTH_CONCERN: &str = "module Authentication\n  extend ActiveSupport::Concern\n  included do\n    before_action :require_authentication\n  end\n  class_methods do\n    def allow_unauthenticated_access(**options)\n      skip_before_action :require_authentication, **options\n    end\n  end\n  private\n    def require_authentication\n      resume_session || request_authentication\n    end\nend\n";

    #[test]
    fn f09_rails8_authentication_concern() {
        let flow = run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  resource :session, only: [:new, :create, :destroy]\nend\n",
                ),
                ("app/controllers/concerns/authentication.rb", AUTH_CONCERN),
                (
                    "app/controllers/application_controller.rb",
                    "class ApplicationController < ActionController::Base\n  include Authentication\nend\n",
                ),
                (
                    "app/controllers/sessions_controller.rb",
                    "class SessionsController < ApplicationController\n  allow_unauthenticated_access only: %i[ new create ]\nend\n",
                ),
            ],
            "",
        );
        assert_eq!(state(&flow, "GET", "/session/new"), A::Open);
        assert_eq!(state(&flow, "POST", "/session"), A::Open);
        assert_eq!(state(&flow, "DELETE", "/session"), auth());
    }

    const PUNDIT_APP: &str = "class ApplicationController < ActionController::Base\n  include Pundit::Authorization\nend\n";

    fn pundit(controller: &str, extra_routes: &str) -> UiFlow {
        let r = format!("Rails.application.routes.draw do\n{extra_routes}end\n");
        run(
            &[
                ("config/routes.rb", &r),
                ("Gemfile.lock", "    pundit (2.3.0)\n"),
                ("app/controllers/application_controller.rb", PUNDIT_APP),
                ("app/controllers/posts_controller.rb", controller),
            ],
            "",
        )
    }

    #[test]
    fn f10_pundit_authorize() {
        let flow = pundit(
            "class PostsController < ApplicationController\n  def update\n    @post = Post.find(params[:id])\n    authorize @post\n  end\n  def publish\n    @post = Post.find(params[:id])\n    authorize @post, :publish?\n  end\n  def dash\n    authorize :dashboard, :show?\n  end\n  def adm\n    @post = Post.find(params[:id])\n    authorize [:admin, @post]\n  end\n  def mystery\n    authorize something\n  end\nend\n",
            "  patch \"p\", to: \"posts#update\"\n  get \"pub\", to: \"posts#publish\"\n  get \"dash\", to: \"posts#dash\"\n  get \"adm\", to: \"posts#adm\"\n  get \"mys\", to: \"posts#mystery\"\n",
        );
        assert_eq!(state(&flow, "PATCH", "/p"), pol("PostPolicy#update?"));
        assert_eq!(state(&flow, "GET", "/pub"), pol("PostPolicy#publish?"));
        assert_eq!(state(&flow, "GET", "/dash"), pol("DashboardPolicy#show?"));
        assert_eq!(state(&flow, "GET", "/adm"), pol("Admin::PostPolicy#adm?"));
        assert!(matches!(
            state(&flow, "GET", "/mys"),
            A::Protected(C::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    #[test]
    fn f11_policy_scope_and_verify_authorized_protect_nothing() {
        let flow = pundit(
            "class PostsController < ApplicationController\n  after_action :verify_authorized, except: :index\n  def index\n    @posts = policy_scope(Post)\n  end\nend\n",
            "  get \"posts\", to: \"posts#index\"\n",
        );
        assert_eq!(state(&flow, "GET", "/posts"), A::Undeclared);
    }

    #[test]
    fn f12_skip_authorization_is_open_only_without_callbacks() {
        let body = "def show\n    skip_authorization\n  end\n";
        let bare = pundit(
            &format!("class PostsController < ApplicationController\n  {body}end\n"),
            "  get \"s\", to: \"posts#show\"\n",
        );
        assert_eq!(state(&bare, "GET", "/s"), A::Open);
        let guarded = pundit(
            &format!(
                "class PostsController < ApplicationController\n  before_action :authenticate_user!\n  {body}end\n"
            ),
            "  devise_for :users\n  get \"s\", to: \"posts#show\"\n",
        );
        assert_eq!(state(&guarded, "GET", "/s"), auth());
    }

    #[test]
    fn f13_conditional_authorize_is_no_condition() {
        let flow = pundit(
            "class PostsController < ApplicationController\n  def show\n    @post = Post.find(1)\n    authorize @post if params[:strict]\n  end\nend\n",
            "  get \"s\", to: \"posts#show\"\n",
        );
        assert_eq!(state(&flow, "GET", "/s"), A::Undeclared);
        assert!(has(&flow, DiagnosticSeverity::Note, "authorize"));
    }

    fn cancan(controller: &str) -> UiFlow {
        run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  resources :articles, only: [:index, :show]\nend\n",
                ),
                ("Gemfile.lock", "    cancancan (3.5.0)\n"),
                ("app/controllers/articles_controller.rb", controller),
            ],
            "",
        )
    }

    #[test]
    fn f14_cancan_resource_macros() {
        let flow = cancan(
            "class ArticlesController < ApplicationController\n  load_and_authorize_resource\nend\n",
        );
        assert_eq!(state(&flow, "GET", "/articles"), pol("index Article"));
        assert_eq!(state(&flow, "GET", "/articles/:id"), pol("show Article"));
        let flow = cancan(
            "class ArticlesController < ApplicationController\n  load_and_authorize_resource except: :index\nend\n",
        );
        assert_eq!(state(&flow, "GET", "/articles"), A::Undeclared);
        assert_eq!(state(&flow, "GET", "/articles/:id"), pol("show Article"));
        let flow =
            cancan("class ArticlesController < ApplicationController\n  load_resource\nend\n");
        assert_eq!(state(&flow, "GET", "/articles"), A::Undeclared);
    }

    #[test]
    fn f15_cancan_authorize_bang() {
        let flow = cancan(
            "class ArticlesController < ApplicationController\n  def show\n    @article = Article.find(params[:id])\n    authorize! :edit, @article\n  end\n  def index\n    authorize! :edit, @thing\n  end\nend\n",
        );
        assert_eq!(state(&flow, "GET", "/articles/:id"), pol("edit Article"));
        assert!(matches!(
            state(&flow, "GET", "/articles"),
            A::Protected(C::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    #[test]
    fn f18_authorize_bang_needs_one_library() {
        let with = |lock| -> UiFlow {
            run(
                &[
                    (
                        "config/routes.rb",
                        "Rails.application.routes.draw do\n  get \"s\", to: \"posts#show\"\nend\n",
                    ),
                    ("Gemfile.lock", lock),
                    (
                        "app/controllers/posts_controller.rb",
                        "class PostsController < ApplicationController\n  def show\n    @post = Post.find(1)\n    authorize! @post\n  end\nend\n",
                    ),
                ],
                "",
            )
        };
        assert_eq!(
            state(&with("    action_policy (0.7.0)\n"), "GET", "/s"),
            pol("PostPolicy#show?")
        );
        let both = with("    action_policy (0.7.0)\n    cancancan (3.5.0)\n");
        assert!(matches!(
            state(&both, "GET", "/s"),
            A::Protected(C::Unknown(_))
        ));
        assert!(!both.complete);
    }

    #[test]
    fn f19_http_basic_authenticate_with() {
        let with = |app: &str| {
            run(
                &[
                    (
                        "config/routes.rb",
                        "Rails.application.routes.draw do\n  resources :events, only: [:index, :create]\nend\n",
                    ),
                    ("app/controllers/application_controller.rb", app),
                    (
                        "app/controllers/events_controller.rb",
                        "class EventsController < ApplicationController\n  http_basic_authenticate_with name: ENV[\"U\"], password: ENV[\"P\"], except: :index\nend\n",
                    ),
                ],
                "",
            )
        };
        let flow = with("class ApplicationController\nend\n");
        assert_eq!(state(&flow, "GET", "/events"), A::Undeclared);
        assert_eq!(state(&flow, "POST", "/events"), auth());
        let flow = with(
            "class ApplicationController\n  def self.http_basic_authenticate_with(**options)\n  end\nend\n",
        );
        assert_eq!(state(&flow, "GET", "/events"), A::Undeclared);
        assert!(matches!(
            state(&flow, "POST", "/events"),
            A::Protected(C::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    #[test]
    fn f20_guard_inherited_through_class_and_concern() {
        let flow = run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  devise_for :users\n  namespace :admin do\n    resources :users, only: [:index]\n  end\nend\n",
                ),
                (
                    "app/controllers/concerns/guarded.rb",
                    "module Guarded\n  extend ActiveSupport::Concern\n  included do\n    before_action :authenticate_user!\n  end\nend\n",
                ),
                (
                    "app/controllers/admin/base_controller.rb",
                    "module Admin\n  class BaseController < ApplicationController\n    include Guarded\n  end\nend\n",
                ),
                (
                    "app/controllers/admin/users_controller.rb",
                    "class Admin::UsersController < Admin::BaseController\nend\n",
                ),
            ],
            "",
        );
        assert_eq!(state(&flow, "GET", "/admin/users"), auth());
        assert_eq!(
            flow.endpoints[0].handler.as_deref(),
            Some("Admin::UsersController#index")
        );
    }

    #[test]
    fn skip_never_opens_next_to_an_unreadable_guard() {
        let r = routes(PAGES_ROUTES);
        let flow = run(
            &[
                ("config/routes.rb", &r),
                (
                    "app/controllers/application_controller.rb",
                    "class ApplicationController\n  before_action :authenticate_user!\n  before_action :require_admin\n  def require_admin\n    redirect_to root_path unless current_user.admin?\n  end\nend\n",
                ),
                (
                    "app/controllers/pages_controller.rb",
                    "class PagesController < ApplicationController\n  skip_before_action :authenticate_user!, only: [:home]\nend\n",
                ),
            ],
            "",
        );
        assert_ne!(state(&flow, "GET", "/home"), A::Open);
        assert!(has(&flow, DiagnosticSeverity::Gap, "require_admin"));
    }

    #[test]
    fn nested_resources_member_collection_and_scopes() {
        let r = "Rails.application.routes.draw do\n  scope \"/v1\" do\n    resources :posts, only: [:show] do\n      resources :comments, only: [:index]\n      member do\n        post :publish\n      end\n      collection do\n        get :search\n      end\n    end\n  end\nend\n";
        let flow = run(&[("config/routes.rb", r)], "");
        for (m, p) in [
            ("GET", "/v1/posts/:id"),
            ("GET", "/v1/posts/:post_id/comments"),
            ("POST", "/v1/posts/:id/publish"),
            ("GET", "/v1/posts/search"),
        ] {
            state(&flow, m, p);
        }
    }

    fn halting(callback: &str, defs: &str) -> UiFlow {
        run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  get \"x\", to: \"pages#x\"\nend\n",
                ),
                (
                    "app/controllers/pages_controller.rb",
                    &format!(
                        "class PagesController < ApplicationController\n  {callback}\n{defs}end\n"
                    ),
                ),
            ],
            "",
        )
    }

    #[test]
    fn halting_callback_not_named_by_a_skip_is_a_gap() {
        let flow = halting(
            "before_action :require_login",
            "  def require_login\n    redirect_to login_path unless current_user\n  end\n",
        );
        assert_eq!(state(&flow, "GET", "/x"), A::Undeclared);
        assert!(has(&flow, DiagnosticSeverity::Gap, "require_login"));
        assert!(!flow.complete);
        let flow = halting(
            "before_action { redirect_to \"/\" unless current_user }",
            "",
        );
        assert!(has(&flow, DiagnosticSeverity::Gap, "block"));
    }

    #[test]
    fn around_action_is_read_like_before_action() {
        let flow = halting(
            "around_action :guard",
            "  def guard\n    return head(:forbidden) unless admin?\n    yield\n  end\n",
        );
        assert_eq!(state(&flow, "GET", "/x"), A::Undeclared);
        assert!(has(&flow, DiagnosticSeverity::Gap, "guard"));
    }

    #[test]
    fn mount_outside_a_guard_is_a_gap_inside_one_is_not() {
        let r = routes(
            "  mount Blazer::Engine, at: \"/blazer\"\n  authenticate :user do\n    mount Sidekiq::Web => \"/sidekiq\"\n  end\n",
        );
        let flow = run(&[("config/routes.rb", &r)], "");
        assert_eq!(state(&flow, "ANY", "/blazer"), A::Undeclared);
        assert_eq!(state(&flow, "ANY", "/sidekiq"), auth());
        let gaps: Vec<_> = flow
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .collect();
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].message.contains("Blazer::Engine"));
    }
}
