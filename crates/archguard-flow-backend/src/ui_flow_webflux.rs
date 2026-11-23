//! Spring WebFlux: functional `RouterFunction` routes as endpoints. The
//! reactive `SecurityWebFilterChain` (`authorizeExchange`, `pathMatchers`,
//! `anyExchange`) is read by the servlet chain code in `ui_flow_backend`
//! (`spring::chain`), which accepts these names too. This module hooks into
//! the backend's extraction (`extract_with`) so the routes exist before the
//! chain runs and it decides them like annotated controllers.
use crate::ui_flow_backend::spring::{Conf, declared_access, family};
use crate::ui_flow_backend::{Index, Out, extract_with as backend_extract, path as join_path};
use archguard_core::{AccessState, UiFlow};
use archguard_flow_clike::decl::{Decl, Dialect, decls, each_method};
use archguard_flow_clike::lex::{self, skip_ws, skip_ws_back, split_top, words};
use archguard_flow_clike::source::Source;
use archguard_flow_common::extract_ctx::{ExtractContext, with_root_config};
use std::collections::BTreeSet;

/// The backend flow with the functional routes of the project.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    backend_extract(ctx, Some(scan))
}

/// [`extract_with`] with the root's configuration.
pub fn extract(root: &std::path::Path) -> UiFlow {
    with_root_config(root, extract_with)
}

const VERBS: [&str; 7] = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// What `scan` reads from: the file, the project's files, the index and conf.
struct Ctx<'a> {
    src: &'a Source,
    sources: &'a [Source],
    index: &'a Index<'a>,
    conf: &'a Conf,
}

/// A call `name(args)` whose name starts at `at`.
struct Call<'a> {
    name: &'a str,
    close: usize,
    args: Vec<&'a str>,
}

fn call_at(src: &Source, at: usize) -> Option<Call<'_>> {
    let b = src.bytes();
    if (at > 0 && lex::is_ident(b[at - 1])) || !b[at].is_ascii_alphabetic() {
        return None;
    }
    let end = at + b[at..].iter().take_while(|c| lex::is_ident(**c)).count();
    let open = skip_ws(b, end);
    let close = (b.get(open) == Some(&b'('))
        .then(|| src.pair(open))
        .flatten()?;
    Some(Call {
        name: &src.text[at..end],
        close,
        args: split_top(&src.text[open + 1..close]),
    })
}

/// `(method, path argument)` when the call at `at` is a route, else `None`.
fn route_of<'a>(
    src: &Source,
    dsl: &[(usize, usize)],
    at: usize,
    call: &Call<'a>,
) -> Option<Option<(&'static str, Option<&'a str>)>> {
    let b = src.bytes();
    let dot = skip_ws_back(b, at);
    let after_dot = dot > 0 && b[dot - 1] == b'.';
    if matches!(call.name, "route" | "andRoute") {
        let call_site = !call.args.is_empty() && !declaration(src, at, call.close);
        return call_site.then(|| predicate_verb(call.args[0]));
    }
    let verb = VERBS.iter().find(|v| **v == call.name)?;
    let predicate = after_dot
        && src.text[..dot - 1]
            .trim_end()
            .ends_with("RequestPredicates");
    let in_dsl = !after_dot && dsl.iter().any(|&(f, to)| f < at && at < to);
    ((after_dot && !predicate) || in_dsl).then(|| Some((*verb, call.args.first().copied())))
}

/// `RouterFunction` routes of `src` as endpoints (access from the handler
/// method's annotations; the chain applies afterwards like for controllers). A
/// route whose predicate, path or `nest` prefix is not literal is skipped with a
/// gap, never shown as a guessed endpoint.
fn scan(src: &Source, sources: &[Source], index: &Index, conf: &Conf, out: &mut Out) {
    kotlin_chain_gap(src, out);
    let dsl = dsl_blocks(src);
    if !(src.text.contains("RouterFunction") || !dsl.is_empty()) {
        return;
    }
    let ctx = Ctx {
        src,
        sources,
        index,
        conf,
    };
    for at in 0..src.text.len() {
        let Some(call) = call_at(src, at) else {
            continue;
        };
        let Some(route) = route_of(src, &dsl, at, &call) else {
            continue;
        };
        let handler = call.args.get(1..).and_then(|a| a.last()).copied();
        emit(&ctx, out, at, (call.name, route, handler));
    }
}

/// A route call as read: its name, `(method, path argument)` and handler.
type Read<'a> = (&'a str, Option<(&'a str, Option<&'a str>)>, Option<&'a str>);

/// The endpoint of one route, or a gap when it is not readable.
fn emit(ctx: &Ctx, out: &mut Out, at: usize, (name, route, handler): Read) {
    let (src, index) = (ctx.src, ctx.index);
    let path = route.and_then(|(v, a)| Some((v, index.text(a?)?)));
    let (Some((method, p)), Some(prefix)) = (path, prefix(src, index, at)) else {
        let m = format!(
            "Functional route `{name}(..)` not readable (predicate, path or nest prefix is not literal); the endpoint is skipped."
        );
        return out.at(src, at, m);
    };
    let parts: Vec<&str> = prefix
        .iter()
        .map(String::as_str)
        .chain([p.as_str()])
        .collect();
    let access = handler_access(ctx, out, at, handler);
    out.endpoint(src, at, method, join_path(&parts), &access);
}

/// `(method, path argument)` of a route predicate starting with a verb call.
fn predicate_verb(pred: &str) -> Option<(&'static str, Option<&str>)> {
    let p = pred.trim();
    let p = p.strip_prefix("RequestPredicates.").unwrap_or(p);
    let verb = VERBS
        .iter()
        .find(|v| p.strip_prefix(**v).is_some_and(|r| r.starts_with('(')))?;
    let inner = &p[verb.len() + 1..];
    let inner = &inner[..inner.find(')').unwrap_or(inner.len())];
    Some((verb, split_top(inner).first().copied()))
}

/// A declaration `route(..) {` / `fun route(..)` rather than a call.
fn declaration(src: &Source, at: usize, close: usize) -> bool {
    let b = src.bytes();
    let next = skip_ws(b, close + 1);
    b.get(next) == Some(&b'{')
        || src.text[next..].starts_with("throws")
        || src.text[..at].trim_end().ends_with("fun")
}

/// `{ .. }` spans of Kotlin `router { }` / `coRouter { }`.
fn dsl_blocks(src: &Source) -> Vec<(usize, usize)> {
    let b = src.bytes();
    ["router", "coRouter"]
        .iter()
        .flat_map(|w| words(&src.text, w).map(move |p| p + w.len()))
        .filter_map(|e| {
            let k = skip_ws(b, e);
            (b.get(k) == Some(&b'{')).then_some(())?;
            Some((k, src.pair(k)?))
        })
        .collect()
}

/// The predicate text of the `nest` whose arguments or block open at `k`:
/// `Ok(None)` when `k` opens no `nest`, `Err` when its predicate is not readable.
fn nest_predicate(src: &Source, k: usize) -> Result<Option<String>, ()> {
    let b = src.bytes();
    let before = skip_ws_back(b, k);
    let head = &src.text[..before];
    let first = |from: usize, to: usize| {
        split_top(&src.text[from..to])
            .first()
            .map(|a| a.to_string())
            .ok_or(())
    };
    if b[k] == b'(' && head.ends_with("nest") {
        // Java `nest(path("/api"), b -> ..)`
        return first(k + 1, src.pair(k).ok_or(())?).map(Some);
    }
    if b[k] != b'{' {
        return Ok(None);
    }
    if head.ends_with(')') {
        // Kotlin `nest(path("/api")) { .. }`
        let open = src.pair(before - 1).ok_or(())?;
        let nest = head[..open].trim_end().ends_with("nest");
        return if nest {
            first(open + 1, before - 1).map(Some)
        } else {
            Ok(None)
        };
    }
    if !head.ends_with("nest") {
        return Ok(None);
    }
    // Kotlin `"/api".nest { .. }`, `("/api" and accept(..)).nest { .. }`
    let dot = skip_ws_back(b, before - 4)
        .checked_sub(1)
        .filter(|d| b[*d] == b'.')
        .ok_or(())?;
    let end = skip_ws_back(b, dot);
    match b[end - 1] {
        b'"' => {
            let from = src.text[..end - 1].rfind('"').ok_or(())?;
            Ok(Some(src.text[from..end].to_string()))
        }
        b')' => {
            let open = src.pair(end - 1).ok_or(())?;
            let inner = &src.text[open + 1..end - 1];
            Ok(Some(
                inner
                    .split(" and ")
                    .next()
                    .unwrap_or(inner)
                    .trim()
                    .to_string(),
            ))
        }
        _ => Err(()),
    }
}

/// The path prefixes of the enclosing `nest`s, outermost first; `None` when one
/// is not literal.
fn prefix(src: &Source, index: &Index, at: usize) -> Option<Vec<String>> {
    let b = src.bytes();
    let mut found = Vec::new();
    for k in (0..at).rev() {
        if !matches!(b[k], b'(' | b'{') || !src.pair(k).is_some_and(|c| c > at) {
            continue;
        }
        let Some(pred) = nest_predicate(src, k).ok()? else {
            continue;
        };
        let pred = pred.trim();
        let pred = pred.strip_prefix("RequestPredicates.").unwrap_or(pred);
        let arg = (pred.strip_prefix("path(")).and_then(|r| r.strip_suffix(')'));
        found.push(index.text(arg.unwrap_or(pred))?);
    }
    found.reverse();
    Some(found)
}

/// The access of the handler method `recv::method` (its annotations); a handler
/// that is not one method while a method of that name carries security: Unknown
/// with a gap.
fn handler_access(ctx: &Ctx, out: &mut Out, at: usize, handler: Option<&str>) -> AccessState {
    let Ctx {
        src,
        sources,
        index,
        conf,
    } = *ctx;
    let Some((recv, method)) = handler.and_then(|h| h.trim().split_once("::")) else {
        return AccessState::Undeclared; // a lambda carries no method security
    };
    let (recv, method) = (recv.trim(), method.trim());
    let class = receiver_type(src, at, recv).and_then(|t| {
        let all: Vec<(&Source, Decl)> = sources
            .iter()
            .flat_map(|s| {
                (decls(s, Dialect::jvm(s)).into_iter())
                    .filter(|d| d.class && d.name == t)
                    .map(move |d| (s, d))
            })
            .collect();
        <[_; 1]>::try_from(all).ok().map(|[one]| one)
    });
    if let Some((csrc, class)) = class {
        let mut own: Vec<Decl> = vec![];
        each_method(csrc, Dialect::jvm(csrc), |c, d| {
            if c.open == class.open && d.name == method {
                own.push(d.clone());
            }
        });
        if own.len() <= 1 {
            let plain = Decl {
                class: false,
                name: method.into(),
                attrs: vec![],
                open: class.open,
                end: 0,
                bases: vec![],
                flag: true,
            };
            let d = own.first().unwrap_or(&plain);
            return declared_access(conf, index, out, (csrc, &class, d), 1).0;
        }
    }
    let secured = sources.iter().any(|s| {
        let mut hit = false;
        each_method(s, Dialect::jvm(s), |_, d| {
            hit |= d.name == method && d.attrs.iter().any(|a| family(&a.name).is_some());
        });
        hit
    });
    if !secured {
        return AccessState::Undeclared;
    }
    let text =
        format!("handler `{recv}::{method}` of a functional route is not resolved to one method");
    AccessState::Protected(out.unknown(src, at, &text))
}

/// The simple type name of `recv` in `recv::method`: a type itself, `this`, or a
/// variable declared with exactly one type in the file.
fn receiver_type(src: &Source, at: usize, recv: &str) -> Option<String> {
    let upper = |t: &str| t.starts_with(|c: char| c.is_ascii_uppercase());
    if upper(recv) {
        return Some(recv.to_string());
    }
    if recv == "this" {
        let classes = decls(src, Dialect::jvm(src));
        let own = (classes.iter()).rfind(|d| d.class && d.open < at && at <= d.end)?;
        return Some(own.name.clone());
    }
    let b = src.bytes();
    let mut types = BTreeSet::new();
    for p in words(&src.text, recv) {
        let after = skip_ws(b, p + recv.len());
        let (s, e) = if b.get(after) == Some(&b':') && b.get(after + 1) != Some(&b':') {
            // Kotlin `recv: Type`
            let s = skip_ws(b, after + 1);
            (
                s,
                s + b[s..].iter().take_while(|c| lex::is_ident(**c)).count(),
            )
        } else if b
            .get(after)
            .is_some_and(|c| matches!(c, b';' | b',' | b')' | b'='))
        {
            // Java `Type recv`
            lex::ident_before(b, skip_ws_back(b, p))
        } else {
            continue;
        };
        if upper(&src.text[s..e]) {
            types.insert(src.text[s..e].to_string());
        }
    }
    let types: Vec<String> = types.into_iter().collect();
    <[_; 1]>::try_from(types).ok().map(|[one]| one)
}

/// Kotlin `authorizeExchange { .. }` (no parentheses) is a DSL this does not read.
fn kotlin_chain_gap(src: &Source, out: &mut Out) {
    let b = src.bytes();
    for p in words(&src.text, "authorizeExchange") {
        let open = skip_ws(b, p + "authorizeExchange".len());
        // A method reference or `Customizer` argument holds rules elsewhere.
        let elsewhere = b.get(open) == Some(&b'(')
            && src.pair(open).is_some_and(|c| {
                let args = src.text[open + 1..c].trim();
                !args.is_empty() && !args.contains("->")
            });
        if b.get(open) != Some(&b'(') || elsewhere {
            let m = "WebFlux `authorizeExchange { .. }` DSL is not read; endpoints it protects stay undeclared.";
            out.at(src, p, m.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Condition, DiagnosticSeverity};

    const ENABLE: &str = "@EnableReactiveMethodSecurity\npublic class Enable {}\n";
    const CTRL: &str = "@RestController\npublic class C {\n  @GetMapping(\"/a\") String a() { return \"\"; }\n  @GetMapping(\"/b\") String b() { return \"\"; }\n}\n";

    fn flow(files: &[(&str, &str)]) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        let mut all = vec![
            ("pom.xml", "<project/>"),
            ("src/main/java/Enable.java", ENABLE),
        ];
        all.extend_from_slice(files);
        for (path, text) in all {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }
    fn find<'a>(f: &'a UiFlow, m: &str, p: &str) -> Option<&'a AccessState> {
        f.endpoints
            .iter()
            .find(|e| e.method == m && e.path == p)
            .map(|e| &e.access)
    }
    fn role(n: &str) -> AccessState {
        AccessState::Protected(Condition::Role(format!("ROLE_{n}")))
    }
    fn chain(body: &str) -> String {
        format!(
            "@Configuration\npublic class Sec {{\n  @Bean SecurityWebFilterChain c(ServerHttpSecurity http) {{\n    {body}\n    return http.build();\n  }}\n}}\n"
        )
    }
    const F39: &str = "http.authorizeExchange(e -> e.pathMatchers(\"/a\").permitAll().anyExchange().hasRole(\"U\"));";
    fn gaps(f: &UiFlow) -> usize {
        f.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .count()
    }

    #[test]
    fn f39_authorize_exchange_first_match() {
        let c = chain(F39);
        let f = flow(&[
            ("src/main/java/C.java", CTRL),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(
            find(&f, "GET", "/a"),
            Some(&AccessState::Open),
            "{:#?}",
            f.diagnostics
        );
        assert_eq!(find(&f, "GET", "/b"), Some(&role("U")));
    }

    #[test]
    fn method_pattern_and_deny_all() {
        let c = chain(
            "http.authorizeExchange(e -> e.pathMatchers(HttpMethod.POST, \"/a\").denyAll().anyExchange().authenticated());",
        );
        let f = flow(&[
            ("src/main/java/C.java", CTRL),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(
            find(&f, "GET", "/a"),
            Some(&AccessState::Protected(Condition::Authenticated))
        );
    }

    const HANDLER: &str = "@Component\npublic class H {\n  @PreAuthorize(\"hasRole('ADMIN')\")\n  public Mono<ServerResponse> p(ServerRequest r) { return null; }\n  public Mono<ServerResponse> q(ServerRequest r) { return null; }\n}\n";
    fn routes(body: &str) -> String {
        format!(
            "@Configuration\npublic class R {{\n  private final H h;\n  @Bean RouterFunction<ServerResponse> r() {{\n    {body}\n  }}\n}}\n"
        )
    }

    #[test]
    fn f40_router_function_route_gets_the_chain() {
        let r = routes("return RouterFunctions.route(GET(\"/f\"), h::q);");
        let c = chain(F39);
        let f = flow(&[
            ("src/main/java/H.java", HANDLER),
            ("src/main/java/R.java", &r),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(
            find(&f, "GET", "/f"),
            Some(&role("U")),
            "{:#?}",
            f.diagnostics
        );
    }

    #[test]
    fn route_without_chain_is_undeclared_and_handler_annotation_counts() {
        let r = routes("return route(GET(\"/f\"), h::q).andRoute(POST(\"/g\"), h::p);");
        let f = flow(&[
            ("src/main/java/H.java", HANDLER),
            ("src/main/java/R.java", &r),
        ]);
        assert_eq!(
            find(&f, "GET", "/f"),
            Some(&AccessState::Undeclared),
            "{:#?}",
            f.diagnostics
        );
        // andRoute reads like route; the handler's @PreAuthorize counts
        assert_eq!(find(&f, "POST", "/g"), Some(&role("ADMIN")));
    }

    #[test]
    fn handler_method_security_survives_a_permit_all_chain() {
        let r = routes("return route(GET(\"/p\"), h::p);");
        let c = chain("http.authorizeExchange(e -> e.anyExchange().permitAll());");
        let f = flow(&[
            ("src/main/java/H.java", HANDLER),
            ("src/main/java/R.java", &r),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(
            find(&f, "GET", "/p"),
            Some(&role("ADMIN")),
            "{:#?}",
            f.diagnostics
        );
    }

    #[test]
    fn builder_routes_with_nest_prefix() {
        let r = routes(
            "return RouterFunctions.route().GET(\"/x\", h::q).nest(RequestPredicates.path(\"/api\"), b -> b.POST(\"/y\", h::q)).build();",
        );
        let f = flow(&[
            ("src/main/java/H.java", HANDLER),
            ("src/main/java/R.java", &r),
        ]);
        assert!(find(&f, "GET", "/x").is_some(), "{:#?}", f.endpoints);
        assert!(find(&f, "POST", "/api/y").is_some(), "{:#?}", f.endpoints);
    }

    #[test]
    fn kotlin_co_router_with_string_nest() {
        let r = "class R(private val h: H) {\n  fun r() = coRouter {\n    GET(\"/k\", h::q)\n    \"/api\".nest {\n      POST(\"/p\", h::q)\n    }\n  }\n}\n";
        let f = flow(&[("src/main/java/H.java", HANDLER), ("src/main/java/R.kt", r)]);
        assert!(find(&f, "GET", "/k").is_some(), "{:#?}", f.diagnostics);
        assert!(
            find(&f, "POST", "/api/p").is_some(),
            "{:#?} {:#?}",
            f.endpoints,
            f.diagnostics
        );
    }

    #[test]
    fn unreadable_path_is_a_gap_without_endpoint() {
        let r = routes("return route(GET(PATH), h::q);");
        let c = chain("http.authorizeExchange(e -> e.anyExchange().permitAll());");
        let f = flow(&[
            ("src/main/java/H.java", HANDLER),
            ("src/main/java/R.java", &r),
            ("src/main/java/Sec.java", &c),
        ]);
        assert!(
            f.endpoints.is_empty() || f.endpoints.iter().all(|e| e.access != AccessState::Open),
            "{:#?}",
            f.endpoints
        );
        assert!(gaps(&f) >= 1);
    }

    #[test]
    fn unresolved_handler_with_security_elsewhere_is_unknown() {
        let r = "@Configuration\npublic class R {\n  @Bean RouterFunction<ServerResponse> r(Object o) {\n    return route(GET(\"/p\"), o::p);\n  }\n}\n";
        let c = chain("http.authorizeExchange(e -> e.anyExchange().permitAll());");
        let f = flow(&[
            ("src/main/java/H.java", HANDLER),
            ("src/main/java/R.java", r),
            ("src/main/java/Sec.java", &c),
        ]);
        assert!(
            matches!(
                find(&f, "GET", "/p"),
                Some(AccessState::Protected(Condition::Unknown(_)))
            ),
            "{:#?}",
            f.endpoints
        );
        assert!(gaps(&f) >= 1);
    }

    #[test]
    fn security_matcher_literal_scope() {
        let c = chain(
            "http.securityMatcher(new PathPatternParserServerWebExchangeMatcher(\"/a/**\")).authorizeExchange(e -> e.anyExchange().hasRole(\"U\"));",
        );
        let ctrl = "@RestController\npublic class C {\n  @GetMapping(\"/a/x\") String a() { return \"\"; }\n  @GetMapping(\"/web\") String b() { return \"\"; }\n}\n";
        let f = flow(&[
            ("src/main/java/C.java", ctrl),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(
            find(&f, "GET", "/a/x"),
            Some(&role("U")),
            "{:#?}",
            f.diagnostics
        );
        assert_eq!(find(&f, "GET", "/web"), Some(&AccessState::Undeclared));
    }

    #[test]
    fn unreadable_rule_never_becomes_open() {
        let c = chain(
            "http.authorizeExchange(e -> e.matchers(custom).access(m).pathMatchers(\"/a\").permitAll());",
        );
        let f = flow(&[
            ("src/main/java/C.java", CTRL),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(find(&f, "GET", "/a"), Some(&AccessState::Undeclared));
    }

    #[test]
    fn handed_out_server_http_security_is_not_evaluated() {
        let c = chain(&format!("{F39} customize(http);"));
        let f = flow(&[
            ("src/main/java/C.java", CTRL),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_eq!(
            find(&f, "GET", "/a"),
            Some(&AccessState::Undeclared),
            "{:#?}",
            f.diagnostics
        );
    }

    #[test]
    fn rules_given_as_method_reference_are_a_gap_not_open() {
        let c = chain("http.authorizeExchange(this::rules);");
        let f = flow(&[
            ("src/main/java/C.java", CTRL),
            ("src/main/java/Sec.java", &c),
        ]);
        assert_ne!(find(&f, "GET", "/a"), Some(&AccessState::Open));
        assert!(gaps(&f) >= 1, "{:#?}", f.diagnostics);
    }

    #[test]
    fn kotlin_authorize_exchange_dsl_is_a_gap() {
        let k = "@Bean fun c(http: ServerHttpSecurity): SecurityWebFilterChain {\n  http { authorizeExchange { authorize(anyExchange, permitAll) } }\n  return http.build()\n}\n";
        let f = flow(&[("src/main/java/C.java", CTRL), ("src/main/java/Sec.kt", k)]);
        assert_ne!(find(&f, "GET", "/a"), Some(&AccessState::Open));
        assert!(gaps(&f) >= 1, "{:#?}", f.diagnostics);
    }
}
