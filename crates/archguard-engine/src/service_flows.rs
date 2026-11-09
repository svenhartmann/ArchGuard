//! Service flows for arc42 chapter 6 (ADR 0006): which HTTP routes each
//! service's source handles, which other services a route calls in which
//! order (static, from the source), which errors it raises, and which
//! timeouts, retries, circuit breakers and fallbacks the service or its
//! imported client code sets up. Target services come from the configured
//! endpoints (`PAYMENTS_API` → payments) or from URLs naming a service.

mod events;

use archguard_core::{
    RouteFailure, RouteStep, ServiceDependencyKind, ServiceResilience, ServiceRoute,
    ServiceTopology,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SOURCES: [&str; 12] = [
    "js", "mjs", "cjs", "ts", "py", "go", "java", "kt", "cs", "rb", "php", "rs",
];
const VERBS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
/// Receivers of `.get('/x', …)`-style route registrations.
const ROUTERS: [&str; 9] = [
    "app", "router", "server", "api", "r", "e", "mux", "bp", "routes",
];
/// Calls whose argument list runs its steps together.
const PARALLEL: [&str; 7] = [
    "Promise.all(",
    "Promise.allSettled(",
    "asyncio.gather(",
    "Task.WhenAll(",
    "join!(",
    "try_join!(",
    "CompletableFuture.allOf(",
];
const MAX_BYTES: u64 = 1 << 20;

pub(crate) fn augment(root: &Path, report: &mut ServiceTopology) {
    let mut routes = Vec::new();
    let mut resilience: Vec<ServiceResilience> = Vec::new();
    for service in &report.services {
        let Some(context) = &service.build_context else {
            continue;
        };
        let targets = Targets::of(report, &service.id);
        for (file, own) in files(root, &root.join(context)) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let rel = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if own {
                routes.extend(
                    routes_in(&text)
                        .into_iter()
                        .map(|r| route(&service.id, &rel, &text, r, &targets)),
                );
            }
            for r in resilience_in(&text, &service.id, &rel) {
                if !resilience
                    .iter()
                    .any(|x| (&x.service, &x.kind, &x.detail) == (&r.service, &r.kind, &r.detail))
                {
                    resilience.push(r);
                }
            }
        }
    }
    events::connect(&mut routes);
    report.routes = routes;
    report.resilience = resilience;
}

/// Where a service's calls go: endpoint keys and service names.
struct Targets {
    keys: BTreeMap<String, String>,
    names: BTreeMap<String, String>,
}

impl Targets {
    fn of(report: &ServiceTopology, service: &str) -> Self {
        let keys = report
            .dependencies
            .iter()
            .filter(|d| d.from == service && d.kind == ServiceDependencyKind::ConfiguredHttp)
            .flat_map(|d| d.evidence.iter().map(move |e| (e, d)))
            .filter_map(|(e, d)| Some((e.pointer.rsplit('/').next()?.to_string(), d.to.clone())))
            .collect();
        let names = report
            .services
            .iter()
            .map(|s| (s.name.clone(), s.id.clone()))
            .collect();
        Targets { keys, names }
    }
}

/// The service's own source files, then files they import relatively
/// (shared client code), marked `false`.
fn files(root: &Path, dir: &Path) -> Vec<(PathBuf, bool)> {
    let own: Vec<PathBuf> = ignore::WalkBuilder::new(dir)
        .build()
        .flatten()
        // The walk does not follow links: a symlink is no regular file here,
        // so it can never lead out of the root.
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES))
        .map(|e| e.into_path())
        // Only the part below the root names tests; the root's own name may not.
        .filter(|p| source(p) && !below(root, p).contains("test"))
        .collect();
    let mut out: Vec<(PathBuf, bool)> = own.iter().cloned().map(|p| (p, true)).collect();
    for file in &own {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for import in relative_imports(&text) {
            let Some(path) = file.parent().map(|d| d.join(&import)) else {
                continue;
            };
            let path = path.canonicalize().unwrap_or(path);
            if path.starts_with(root.canonicalize().unwrap_or(root.to_path_buf()))
                && path.is_file()
                && !out.iter().any(|(p, _)| p == &path)
            {
                out.push((path, false));
            }
        }
    }
    out
}

/// `path` relative to `root`, as text.
fn below(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn source(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| SOURCES.contains(&x))
}

/// `'../../runtime/http.js'`-style literals in import and require lines.
fn relative_imports(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("import") || l.contains("require(") || l.contains("from "))
        .filter_map(|l| {
            let at = l.find(['\'', '"'])?;
            let lit = literal(&l[at..])?;
            (lit.starts_with("./") || lit.starts_with("../")).then(|| lit.to_string())
        })
        .collect()
}

/// The quoted literal at the start of `text` (after its quote).
fn literal(text: &str) -> Option<&str> {
    let quote = text
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let body = &text[1..];
    body.find(quote).map(|end| &body[..end])
}

/// At most `n` bytes of `text`, ending on a character boundary.
fn cut(text: &str, n: usize) -> &str {
    let mut end = n.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn line_of(text: &str, at: usize) -> u32 {
    text[..at.min(text.len())].matches('\n').count() as u32 + 1
}

/// A found route: method, path and where its handler starts.
struct Found {
    method: String,
    path: String,
    at: usize,
}

fn route(
    service: &str,
    file: &str,
    text: &str,
    r: (Found, usize),
    targets: &Targets,
) -> ServiceRoute {
    let (found, end) = r;
    let segment = &text[found.at..end];
    ServiceRoute {
        service: service.into(),
        method: found.method,
        path: found.path,
        file: file.into(),
        line: line_of(text, found.at),
        steps: calls_in(segment, targets)
            .into_iter()
            .map(|mut s| {
                s.line += line_of(text, found.at) - 1;
                s
            })
            .collect(),
        catches: catches(segment),
        failures: failures_in(segment)
            .into_iter()
            .map(|mut f| {
                f.line += line_of(text, found.at) - 1;
                f
            })
            .collect(),
    }
}

/// Routes in source order, each with the end of its handler (the next
/// route or the end of the file).
fn routes_in(text: &str) -> Vec<(Found, usize)> {
    let mut found: Vec<Found> = Vec::new();
    found.extend(compared_routes(text));
    found.extend(registered_routes(text));
    found.extend(events::consumed(text).into_iter().map(|(topic, at)| Found {
        method: events::EVENT.into(),
        path: topic,
        at,
    }));
    found.sort_by_key(|f| f.at);
    found.dedup_by(|a, b| a.at == b.at);
    let ends: Vec<usize> = found
        .iter()
        .skip(1)
        .map(|f| f.at)
        .chain([text.len()])
        .collect();
    found.into_iter().zip(ends).collect()
}

/// `method === 'POST' && path === '/checkout'`: a verb literal, then a
/// path literal with only a comparison between them.
fn compared_routes(text: &str) -> Vec<Found> {
    let mut out = Vec::new();
    for verb in VERBS {
        for q in ['\'', '"'] {
            for (at, _) in text.match_indices(&format!("{q}{verb}{q}")) {
                let after = &text[at + verb.len() + 2..];
                let Some(start) = after.find(['\'', '"']).filter(|&s| s < 40) else {
                    continue;
                };
                if after[..start].contains(['(', ')', '{', '}', ';', ',']) {
                    continue;
                }
                if let Some(path) = literal(&after[start..]).filter(|p| p.starts_with('/')) {
                    out.push(Found {
                        method: verb.into(),
                        path: path.into(),
                        at,
                    });
                }
            }
        }
    }
    out
}

/// A route whose path literal starts `rest`; `"POST /x"` names its own method.
fn registered(method: &str, rest: &str, at: usize) -> Option<Found> {
    let path = literal(rest.trim_start())?;
    let (method, path) = match path.split_once(' ') {
        Some((m, p)) if VERBS.contains(&m) => (m.to_string(), p.to_string()),
        _ => (method.to_string(), path.to_string()),
    };
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    Some(Found { method, path, at })
}

/// `app.post('/x', …)` on a router-like receiver, `@PostMapping("/x")`,
/// `[HttpPost("x")]` for one verb.
fn verb_routes(text: &str, verb: &str) -> Vec<Found> {
    let lower = verb.to_ascii_lowercase();
    let title = format!("{}{}", &verb[..1], &lower[1..]);
    let call = format!(".{lower}(");
    let receiver = |at: usize| {
        text[..at]
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("")
    };
    let calls = text
        .match_indices(&call)
        .filter(|(at, _)| ROUTERS.contains(&receiver(*at)))
        .filter_map(|(at, _)| registered(verb, &text[at + call.len()..], at));
    let annotated = [format!("@{title}Mapping("), format!("[Http{title}(")]
        .into_iter()
        .flat_map(|marker| {
            text.match_indices(&marker)
                .filter_map(|(at, _)| registered(verb, &text[at + marker.len()..], at))
                .collect::<Vec<_>>()
        });
    calls.chain(annotated).collect()
}

/// Registered routes of every verb, plus `@app.route('/x')` and
/// `HandleFunc("POST /x", …)`.
fn registered_routes(text: &str) -> Vec<Found> {
    let mut out: Vec<Found> = VERBS.iter().flat_map(|v| verb_routes(text, v)).collect();
    for marker in [".route(", "HandleFunc("] {
        out.extend(
            text.match_indices(marker)
                .filter_map(|(at, _)| registered("GET", &text[at + marker.len()..], at)),
        );
    }
    out
}

/// Calls to other services in `segment`, in source order.
fn calls_in(segment: &str, targets: &Targets) -> Vec<RouteStep> {
    let parallel = parallel_ranges(segment);
    let mut steps: Vec<(usize, RouteStep)> = Vec::new();
    for (key, to) in &targets.keys {
        for marker in [
            format!("'{key}'"),
            format!("\"{key}\""),
            format!("${{{key}}}"),
            format!("env.{key}"),
        ] {
            for (at, _) in segment.match_indices(&marker) {
                if let Some(step) = step_at(segment, at + marker.len(), to) {
                    steps.push((at, step));
                }
            }
        }
    }
    for (name, to) in &targets.names {
        for (at, _) in segment.match_indices(&format!("://{name}")) {
            if let Some(step) = step_at(segment, at, to) {
                steps.push((at, step));
            }
        }
    }
    steps.extend(
        events::published(segment)
            .into_iter()
            .map(|(topic, at)| (at, events::step(topic))),
    );
    steps.sort_by_key(|(at, _)| *at);
    steps
        .into_iter()
        .map(|(at, mut s)| {
            s.line = line_of(segment, at);
            s.parallel = parallel.iter().any(|(a, b)| (*a..*b).contains(&at));
            s
        })
        .collect()
}

/// The path and method of the call whose target is named just before `at`.
fn step_at(segment: &str, at: usize, to: &str) -> Option<RouteStep> {
    let rest = &segment[at..];
    let window = cut(rest, 160);
    let path = window.find('/').filter(|&p| p < 60).map(|p| {
        let tail = &window[p..];
        let end = tail
            .find(['\'', '"', '`', '?', ' ', ')', ','])
            .unwrap_or(tail.len());
        tail[..end].replace("${", "{")
    })?;
    let call_end = window.find(");").unwrap_or(window.len());
    let method = VERBS
        .iter()
        .find(|v| {
            let w = &window[..call_end];
            w.contains(&format!("method: '{v}'"))
                || w.contains(&format!("method: \"{v}\""))
                || w.contains(&format!("method=\"{v}\""))
        })
        .unwrap_or(&"GET");
    Some(RouteStep {
        to: to.into(),
        method: (*method).into(),
        path,
        line: 0,
        parallel: false,
    })
}

/// Byte ranges inside `Promise.all(…)` and its kin.
fn parallel_ranges(segment: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for marker in PARALLEL {
        for (at, _) in segment.match_indices(marker) {
            let start = at + marker.len();
            let mut depth = 1;
            let end = segment[start..]
                .char_indices()
                .find_map(|(i, c)| {
                    depth += match c {
                        '(' => 1,
                        ')' => -1,
                        _ => 0,
                    };
                    (depth == 0).then_some(start + i)
                })
                .unwrap_or(segment.len());
            out.push((start, end));
        }
    }
    out
}

/// The handler catches errors itself: `catch`, `except`, `rescue` as
/// words, `recover()` or `err != nil`.
fn catches(segment: &str) -> bool {
    let word = |w: &str| {
        segment.match_indices(w).any(|(at, _)| {
            let before = segment[..at].chars().next_back();
            let after = segment[at + w.len()..].chars().next();
            let edge = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            edge(before) && edge(after)
        })
    };
    ["catch", "except", "rescue"].into_iter().any(word)
        || segment.contains("recover()")
        || segment.contains("err != nil")
}

/// `HttpError(409, 'Insufficient stock')`, `HTTPException(status_code=404, …)`,
/// `http.Error(w, "msg", 409)`: a 4xx/5xx status with its message.
fn failures_in(segment: &str) -> Vec<RouteFailure> {
    let mut out: Vec<RouteFailure> = Vec::new();
    for marker in ["Error(", "Exception(", "abort(", "status(", "StatusCode("] {
        for (at, _) in segment.match_indices(marker) {
            let args = &segment[at + marker.len()..];
            let args = cut(&args[..args.find(')').unwrap_or(args.len())], 160);
            let status = args
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|n| n.parse::<u16>().ok())
                .find(|n| (400..600).contains(n));
            let message = args
                .find(['\'', '"', '`'])
                .and_then(|q| literal(&args[q..]));
            if let (Some(status), Some(message)) = (status, message)
                && !out
                    .iter()
                    .any(|f| f.status == status && f.message == message)
            {
                out.push(RouteFailure {
                    status,
                    message: message.into(),
                    line: line_of(segment, at),
                });
            }
        }
    }
    out.sort_by_key(|f| f.line);
    out
}

/// Timeouts, retries, circuit breakers and fallbacks in `text`.
fn resilience_in(text: &str, service: &str, file: &str) -> Vec<ServiceResilience> {
    let mut out = Vec::new();
    let mut add = |kind: &str, detail: String, at: usize| {
        out.push(ServiceResilience {
            service: service.into(),
            kind: kind.into(),
            detail,
            file: file.into(),
            line: line_of(text, at),
        });
    };
    for (marker, unit) in [
        ("AbortSignal.timeout(", "ms"),
        ("from_secs(", "s"),
        ("FromSeconds(", "s"),
        ("timeout=", "s"),
        ("timeout: ", "ms"),
    ] {
        for (at, _) in text.match_indices(marker) {
            let digits: String = text[at + marker.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                add("timeout", format!("{digits} {unit}"), at);
            }
        }
    }
    let kinds: [(&str, &[&str]); 3] = [
        (
            "retry",
            &[
                "axios-retry",
                "tenacity",
                "@Retry",
                "AddRetry",
                "RetryPolicy",
                "retryWhen",
                "max_retries",
                "backoff",
            ],
        ),
        (
            "circuit_breaker",
            &[
                "CircuitBreaker",
                "opossum",
                "circuit_breaker",
                "AddCircuitBreaker",
            ],
        ),
        ("fallback", &["fallbackMethod", "Fallback(", "fallback:"]),
    ];
    for (kind, markers) in kinds {
        for marker in markers {
            if let Some(at) = text.find(marker) {
                add(kind, (*marker).to_string(), at);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str = r#"
export function createHandler({call}) {
  return async ({method, path}) => {
    if (method === 'GET' && path === '/history') return [];
    if (method === 'POST' && path === '/checkout') {
      const stock = await call('INVENTORY_API', `/stock?sku=${sku}`);
      if (stock.available < 1) throw new HttpError(409, 'Insufficient stock');
      const [a, b] = await Promise.all([
        call('PAYMENTS_API', '/authorize', {method: 'POST', body: {id}}),
        call('SHIPPING_API', '/shipments', {method: 'POST'}),
      ]);
      return a;
    }
  };
}
"#;

    fn targets() -> Targets {
        let keys = [
            ("INVENTORY_API", "inv"),
            ("PAYMENTS_API", "pay"),
            ("SHIPPING_API", "ship"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        Targets {
            keys: keys.into_iter().collect(),
            names: BTreeMap::new(),
        }
    }

    #[test]
    fn a_root_whose_name_contains_test_keeps_its_files_and_skips_test_dirs() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("latest-shop");
        std::fs::create_dir_all(root.join("orders/test")).unwrap();
        std::fs::write(root.join("orders/server.js"), "app.get('/x', h);").unwrap();
        std::fs::write(root.join("orders/test/server.test.js"), "").unwrap();
        let found: Vec<PathBuf> = files(&root, &root.join("orders"))
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(found, [root.join("orders/server.js")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_root_is_not_read() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.js"), "app.get('/leak', h);").unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("orders");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.js"), dir.join("link.js")).unwrap();
        assert!(files(root.path(), &dir).is_empty());
    }

    #[test]
    fn routes_carry_their_calls_in_order_with_parallel_steps_and_failures() {
        let routes: Vec<ServiceRoute> = routes_in(ORDERS)
            .into_iter()
            .map(|r| route("orders", "server.js", ORDERS, r, &targets()))
            .collect();
        let names: Vec<(&str, &str)> = routes
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(names, [("GET", "/history"), ("POST", "/checkout")]);
        let checkout = &routes[1];
        let steps: Vec<(&str, &str, &str, bool)> = checkout
            .steps
            .iter()
            .map(|s| {
                (
                    s.to.as_str(),
                    s.method.as_str(),
                    s.path.as_str(),
                    s.parallel,
                )
            })
            .collect();
        assert_eq!(
            steps,
            [
                ("inv", "GET", "/stock", false),
                ("pay", "POST", "/authorize", true),
                ("ship", "POST", "/shipments", true)
            ]
        );
        assert_eq!(checkout.failures.len(), 1);
        assert_eq!(
            (
                checkout.failures[0].status,
                checkout.failures[0].message.as_str()
            ),
            (409, "Insufficient stock")
        );
        assert_eq!(checkout.failures[0].line, 7);
        assert!(routes[0].steps.is_empty());
    }

    #[test]
    fn registered_routes_in_several_frameworks() {
        let text = "app.post('/orders', h);\n@GetMapping(\"/items\")\n[HttpDelete(\"items/{id}\")]\nmux.HandleFunc(\"PUT /stock\", h)\nclient.get('/not-a-route')";
        let found: Vec<(String, String)> = routes_in(text)
            .into_iter()
            .map(|(f, _)| (f.method, f.path))
            .collect();
        assert_eq!(
            found,
            [
                ("POST".to_string(), "/orders".to_string()),
                ("GET".into(), "/items".into()),
                ("DELETE".into(), "/items/{id}".into()),
                ("PUT".into(), "/stock".into())
            ]
        );
    }

    #[test]
    fn timeouts_retries_and_breakers_are_named() {
        let text = "fetch(u, {signal: AbortSignal.timeout(3000)});\nconst breaker = new CircuitBreaker(fn);\nimport axiosRetry from 'axios-retry';";
        let found: Vec<(String, String)> = resilience_in(text, "s", "f.js")
            .into_iter()
            .map(|r| (r.kind, r.detail))
            .collect();
        assert_eq!(
            found,
            [
                ("timeout".to_string(), "3000 ms".to_string()),
                ("retry".into(), "axios-retry".into()),
                ("circuit_breaker".into(), "CircuitBreaker".into())
            ]
        );
    }

    #[test]
    fn multibyte_text_near_the_windows_does_not_split_a_character() {
        let filler = "ü".repeat(100);
        let text = format!(
            "if (method === 'GET' && path === '/x') {{ call('PAYMENTS_API', '/pay{filler}'); throw new HttpError(409, '{filler}'); }}"
        );
        let targets = Targets {
            keys: [("PAYMENTS_API".to_string(), "pay".to_string())]
                .into_iter()
                .collect(),
            names: BTreeMap::new(),
        };
        let routes: Vec<ServiceRoute> = routes_in(&text)
            .into_iter()
            .map(|r| route("s", "f.js", &text, r, &targets))
            .collect();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].steps.len(), 1);
    }

    #[test]
    fn a_handler_that_catches_errors_is_marked() {
        let cases = [
            (
                "try { await call('PAYMENTS_API', '/p'); } catch (e) { return {}; }",
                true,
            ),
            ("try:\n  call()\nexcept Timeout:\n  pass", true),
            ("res, err := get(); if err != nil { return }", true),
            ("defer func() { recover() }()", true),
            (
                "await call('PAYMENTS_API', '/p'); // catchy name: catchAll",
                false,
            ),
            (
                "const exceptional = 1; await call('PAYMENTS_API', '/p');",
                false,
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(catches(body), expected, "{body}");
        }
    }
}
