//! `navigation_resolves` and `endpoint_called` (ADR 0021 decision 6). Both say
//! "nothing was found", so each is judged only where the extractors read
//! everything it needs; elsewhere it is one info note, never a guess.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::{
    ANY_METHOD, Call, DiagnosticSeverity, EndpointAccess, Finding, FindingKind, Origin, Ref,
    Severity, Target, UiFlow, UiFlowDiagnostic, join_actions_to_endpoints, path_patterns_match,
};

/// One `convention` error finding, id `view-<rule>:<file>:<subject>`.
pub(super) fn finding(
    rule: &str,
    file: &str,
    line: u32,
    subject: &str,
    message: String,
) -> Finding {
    Finding {
        kind: FindingKind::Convention,
        id: format!("view-{rule}:{file}:{subject}"),
        severity: Severity::Error,
        message: Some(format!("[views] {rule}: {message}")),
        evidence: vec![Ref {
            file: file.to_string(),
            line,
            raw: subject.to_string(),
        }],
        nodes: Vec::new(),
        edge_count: 0,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: Vec::new(),
    }
}

const UNKNOWN: &str = "(unknown application)";

/// Info-level finding of the `view-conventions:` family: shown by `check`,
/// never failing, never baselined.
fn note(subject: &str, message: String) -> Finding {
    Finding {
        severity: Severity::Info,
        ..finding("conventions", "", 0, subject, message)
    }
}

fn application_of<'a>(flow: &'a UiFlow, origin: &'a Origin) -> &'a str {
    match origin {
        Origin::Screen(id) => (flow.screens.iter())
            .find(|s| &s.id == id)
            .and_then(|s| s.application.as_deref()),
        Origin::AnyScreen { application } => Some(application.as_str()).filter(|a| !a.is_empty()),
    }
    .unwrap_or(UNKNOWN)
}

/// The directory all files of `app` share (empty: the whole repository).
fn app_dir(flow: &UiFlow, app: &str) -> String {
    let screens = (flow.screens.iter())
        .filter(|s| s.application.as_deref().unwrap_or(UNKNOWN) == app)
        .map(|s| s.file.as_str());
    let transitions = (flow.transitions.iter())
        .filter(|t| application_of(flow, &t.from) == app)
        .map(|t| t.file.as_str());
    let actions = (flow.actions.iter())
        .filter(|a| application_of(flow, &a.screen) == app)
        .map(|a| a.file.as_str());
    let mut common: Option<Vec<&str>> = None;
    for file in screens.chain(transitions).chain(actions) {
        let mut dir: Vec<&str> = file.split('/').collect();
        dir.pop();
        common = Some(match common {
            None => dir,
            Some(c) => (c.iter().zip(&dir))
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect(),
        });
    }
    common.unwrap_or_default().join("/")
}

/// Gap diagnostics inside `app`'s directory whose text names one of `words`
/// (lower case); navigation-target diagnostics are the findings themselves.
fn gaps<'a>(
    flow: &'a UiFlow,
    app: &str,
    words: &'a [&str],
) -> impl Iterator<Item = &'a UiFlowDiagnostic> {
    let dir = app_dir(flow, app);
    flow.diagnostics.iter().filter(move |d| {
        let message = d.message.to_lowercase();
        d.severity == DiagnosticSeverity::Gap
            && !d.message.starts_with("Navigation target")
            && (dir.is_empty() || d.file.starts_with(&format!("{dir}/")))
            && words.iter().any(|w| message.contains(w))
    })
}

/// `<base href>` of the Blazor host page, else the documented default `/`.
fn base_href(root: &Path) -> String {
    const HOSTS: [&str; 3] = ["App.razor", "index.html", "_Host.cshtml"];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                if !["bin", "obj", "node_modules", ".git", "target"].contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if HOSTS.contains(&name.as_str())
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Some(at) = text.find("<base href=\"")
                && let Some(href) = text[at + 12..].split('"').next()
            {
                return format!("{}/", href.trim_end_matches('/'));
            }
        }
    }
    "/".into()
}

/// The path of a navigation target text that is a literal, else `None`: a
/// quoted string, or an unquoted attribute value of that source line; a
/// base-relative path gets `base`.
fn literal(root: &Path, file: &str, line: u32, text: &str, base: &str) -> Option<String> {
    let quoted = text.strip_prefix('"').and_then(|t| t.strip_suffix('"'));
    let path = match quoted {
        Some(inner) if !inner.contains('"') => inner,
        Some(_) => return None,
        None => {
            let source = std::fs::read_to_string(root.join(file)).ok()?;
            let at = source.lines().nth(line.checked_sub(1)? as usize)?;
            at.contains(&format!("=\"{text}\"")).then_some(text)?
        }
    };
    let shaped = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_alphanumeric() || "/-_.{}:*".contains(c))
        && !path.contains("//");
    shaped.then(|| match path.starts_with('/') {
        true => path.to_string(),
        false => format!("{base}{path}"),
    })
}

/// Source files under `dir` (relative to `root`), capped, build output skipped.
fn files_under(root: &Path, dir: &str) -> Vec<std::path::PathBuf> {
    let (mut stack, mut out) = (vec![root.join(dir)], Vec::new());
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !["bin", "obj", "node_modules", ".git", "target", "dist"]
                    .contains(&name.as_str())
                {
                    stack.push(path);
                }
            } else if out.len() < 5000 {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Whether sources of `dir` use an HTTP client. Unknown languages count as
/// usage: with no source file of a known language read, nothing is ruled out.
fn uses_http_client(root: &Path, dir: &str) -> bool {
    const KNOWN: [&str; 14] = [
        "ts", "tsx", "js", "jsx", "vue", "svelte", "cs", "razor", "py", "java", "kt", "php", "go",
        "rs",
    ];
    const CLIENTS: [&str; 10] = [
        "HttpClient",
        "fetch(",
        "axios",
        "XMLHttpRequest",
        "$http",
        "WebClient",
        "RestTemplate",
        "requests.",
        "reqwest",
        "Client(",
    ];
    let mut read = false;
    for file in files_under(root, dir) {
        let known = file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| KNOWN.contains(&e));
        let Some(text) = known.then(|| std::fs::read_to_string(&file).ok()).flatten() else {
            continue;
        };
        read = true;
        if CLIENTS.iter().any(|c| text.contains(c)) {
            return true;
        }
    }
    !read
}

/// A generated-client configuration file (NSwag, OpenAPI Generator), if any.
fn generated_client_config(root: &Path) -> Option<String> {
    files_under(root, "").into_iter().find_map(|f| {
        let name = f.file_name()?.to_str()?;
        let hit = name == "nswag.json" || name == "openapitools.json" || name.ends_with(".nswag");
        hit.then(|| f.strip_prefix(root).unwrap_or(&f).display().to_string())
    })
}

/// Whether a link to `path` opens the endpoint (a browser sends GET).
fn link_reaches(e: &EndpointAccess, path: &str) -> bool {
    (e.method == "GET" || e.method == ANY_METHOD) && path_patterns_match(&e.path, path)
}

type Misses<'a> = BTreeMap<String, Vec<(&'a UiFlowDiagnostic, String)>>;

/// The literal navigation targets that match no route, by application.
fn misses<'a>(root: &Path, flow: &'a UiFlow) -> Misses<'a> {
    let base = base_href(root);
    let mut misses = Misses::new();
    for d in &flow.diagnostics {
        let Some(rest) = d.message.strip_prefix("Navigation target ") else {
            continue;
        };
        let (app, target) =
            if let Some((t, app)) = rest.split_once(" matches no route of application ") {
                (app.trim_end_matches('.').to_string(), t.to_string())
            } else if let Some((t, _)) = rest.split_once(" is not a route") {
                let Some(path) = literal(root, &d.file, d.line, t, &base) else {
                    continue;
                };
                let from = (flow.transitions.iter())
                    .find(|tr| tr.file == d.file && tr.line == d.line)
                    .map(|tr| application_of(flow, &tr.from));
                (from.unwrap_or(UNKNOWN).to_string(), path)
            } else {
                continue;
            };
        if !flow.endpoints.iter().any(|e| link_reaches(e, &target)) {
            misses.entry(app).or_default().push((d, target));
        }
    }
    misses
}

pub(super) fn navigation_resolves(root: &Path, flow: &UiFlow) -> Vec<Finding> {
    let misses = misses(root, flow);
    let mut out = Vec::new();
    for (app, found) in misses {
        let routed = (flow.screens.iter())
            .any(|s| s.route.is_some() && s.application.as_deref().unwrap_or(UNKNOWN) == app);
        if routed && gaps(flow, &app, &["route", "screen"]).next().is_none() {
            out.extend(found.iter().map(|(d, target)| {
                finding("navigation_resolves", &d.file, d.line, target, format!(
                    "Navigation target `{target}` matches no screen route of {app}. Fix: point it at an existing route, or add the screen."
                ))
            }));
        } else {
            let n = found.len();
            let s = if n == 1 { "" } else { "s" };
            out.push(note(&format!("navigation_resolves {app}"), format!(
                "navigation_resolves not evaluated for {app}: routes not fully read ({n} literal target{s} unchecked)."
            )));
        }
    }
    out
}

pub(super) fn endpoint_called(root: &Path, flow: &UiFlow) -> Vec<Finding> {
    let frontends: BTreeSet<&str> = (flow.screens.iter())
        .map(|s| s.application.as_deref().unwrap_or(UNKNOWN))
        .chain(flow.actions.iter().map(|a| application_of(flow, &a.screen)))
        .collect();
    let mut unread = Vec::new();
    for app in frontends {
        let calls = (flow.actions.iter()).filter(|a| application_of(flow, &a.screen) == app);
        let unresolved = calls
            .filter(|a| matches!(a.call, Call::Unresolved(_)))
            .count();
        let unseen = gaps(flow, app, &["call", "client", "import", "url"]).count();
        let resolved = (flow.actions.iter())
            .filter(|a| {
                application_of(flow, &a.screen) == app && matches!(a.call, Call::Http { .. })
            })
            .count();
        if unresolved + unseen > 0 {
            let n = unresolved + unseen;
            unread.push(format!("{app} has {n} unresolved/unread calls"));
        } else if resolved == 0 && uses_http_client(root, &app_dir(flow, app)) {
            unread.push(format!(
                "{app} has 0 resolved calls but uses an HTTP client"
            ));
        }
    }
    let any_http = flow
        .actions
        .iter()
        .any(|a| matches!(a.call, Call::Http { .. }));
    if let (true, Some(marker)) = (!any_http, generated_client_config(root)) {
        unread.push(format!(
            "{marker} configures a generated client whose calls were not read"
        ));
    }
    if !unread.is_empty() {
        return vec![note(
            "endpoint_called",
            format!(
                "endpoint_called not evaluated: {}; a caller could be hidden there (generated client, unread URL).",
                unread.join(", ")
            ),
        )];
    }
    if flow.screens.is_empty() && flow.actions.is_empty() {
        return Vec::new();
    }
    let base = base_href(root);
    let called: BTreeSet<*const EndpointAccess> = join_actions_to_endpoints(flow)
        .into_iter()
        .flat_map(|(_, hits)| hits)
        .map(|e| e as *const _)
        .collect();
    let links: Vec<String> = flow
        .transitions
        .iter()
        .filter_map(|t| match &t.target {
            Target::Unresolved(text) => literal(root, &t.file, t.line, text, &base),
            Target::Screen(_) => None,
        })
        .collect();
    let reached = |e: &EndpointAccess| {
        called.contains(&(e as *const _)) || links.iter().any(|p| link_reaches(e, p))
    };
    (flow.endpoints.iter()).filter(|e| !reached(e)).map(|e| {
        let subject = format!("{} {}", e.method, e.path);
        finding("endpoint_called", &e.file, e.line, &subject, format!(
            "Endpoint {subject} is called by no action of the frontend. Fix: no reader found a caller; check generated clients, redirects and external callers before removing it."
        ))
    }).collect()
}
