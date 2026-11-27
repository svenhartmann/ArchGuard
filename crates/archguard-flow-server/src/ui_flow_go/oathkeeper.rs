//! Ory Oathkeeper access rules.
use archguard_core::{
    ANY_METHOD, AccessState, Condition, DiagnosticSeverity, EndpointAccess, UiFlow,
    UiFlowDiagnostic,
};
use archguard_flow_common::extract_ctx::{ExtractContext, dir_name, read_text};
use serde_yaml_ng::Value as Y;

/// Where the diagnostics of one rule go.
struct Site<'a> {
    file: &'a str,
    line: u32,
}

impl Site<'_> {
    fn add(&self, flow: &mut UiFlow, severity: DiagnosticSeverity, message: String) {
        flow.diagnostics.push(UiFlowDiagnostic {
            file: self.file.into(),
            line: self.line,
            message,
            severity,
        });
    }
}

pub(super) fn oathkeeper(ctx: &ExtractContext, files: &[String], flow: &mut UiFlow) {
    let yaml = |f: &&String| [".yml", ".yaml", ".json"].iter().any(|e| f.ends_with(e));
    for rel in files.iter().filter(yaml).take(5_000) {
        let text = match read_text(ctx.root, rel, 1 << 20) {
            Ok(text) => text,
            Err(e) => {
                // shortcut: a note, not a gap: every YAML/JSON file is a candidate and a
                // lock file over 1 MiB would make each Go flow incomplete; upgrade to a gap
                // once the candidate test (keywords) can run before the read.
                flow.diagnostics.push(UiFlowDiagnostic {
                    file: rel.clone(),
                    line: 0,
                    message: format!(
                        "File not read ({e}); an Oathkeeper rule in it would be unknown."
                    ),
                    severity: DiagnosticSeverity::Note,
                });
                continue;
            }
        };
        if !["authenticators", "authorizer", "match"]
            .iter()
            .all(|k| text.contains(k))
        {
            continue;
        }
        let Ok(doc) = serde_yaml_ng::from_str::<Y>(&text) else {
            continue;
        };
        let rules: Vec<&Y> = match &doc {
            Y::Sequence(s) => s.iter().collect(),
            m @ Y::Mapping(_) => vec![m],
            _ => continue,
        };
        let app = dir_name(ctx.root, rel.rsplit_once('/').map_or("", |x| x.0));
        for rule in rules {
            rule_endpoints(rule, rel, &text, &app, flow);
        }
    }
}

fn methods_of(m: &Y) -> Vec<String> {
    match m.get("methods").and_then(Y::as_sequence) {
        Some(ms) if !ms.is_empty() => ms
            .iter()
            .filter_map(Y::as_str)
            .map(str::to_ascii_uppercase)
            .collect(),
        _ => vec![ANY_METHOD.into()],
    }
}

fn rule_endpoints(rule: &Y, rel: &str, text: &str, app: &str, flow: &mut UiFlow) {
    let (Some(m), Some(authn)) = (rule.get("match"), rule.get("authenticators")) else {
        return;
    };
    let Some(url) = m.get("url").and_then(Y::as_str) else {
        return;
    };
    let line = text
        .lines()
        .position(|l| l.contains(url))
        .map_or(1, |p| p as u32 + 1);
    let site = Site { file: rel, line };
    let state = oathkeeper_state(rule, authn, &site, flow);
    for method in methods_of(m) {
        flow.endpoints.push(EndpointAccess {
            method,
            path: url_path(url),
            access: state.clone(),
            application: Some(app.into()),
            file: rel.into(),
            line,
            source: "oathkeeper rule".into(),
            handler: None,
            sends: vec![],
        });
    }
}

fn url_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |x| x.1);
    let mut depth = 0;
    for (i, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            '/' if depth == 0 => return rest[i..].to_string(),
            _ => {}
        }
    }
    "/".into()
}

/// `(handler, required scopes)` of each authenticator.
fn handlers_of(authn: &Y) -> Vec<(&str, Vec<String>)> {
    let scopes = |h: &Y| -> Vec<String> {
        let list = h.get("config").and_then(|c| c.get("required_scope"));
        let list = list.and_then(Y::as_sequence);
        let all = list.map(|s| s.iter().filter_map(Y::as_str).map(String::from));
        all.map(|i| i.collect()).unwrap_or_default()
    };
    let items = authn.as_sequence().map(|s| s.iter()).into_iter().flatten();
    items
        .filter_map(|h| Some((h.get("handler")?.as_str()?, scopes(h))))
        .collect()
}

fn anonymous(h: &str) -> bool {
    matches!(h, "noop" | "anonymous")
}

/// What the authenticators alone decide.
fn authn_state(handlers: &[(&str, Vec<String>)], site: &Site, flow: &mut UiFlow) -> AccessState {
    use AccessState::Protected as P;
    if handlers.is_empty() {
        let m = "Oathkeeper authenticators are not readable.";
        site.add(flow, DiagnosticSeverity::Gap, m.into());
        return P(Condition::Unknown("oathkeeper authenticators".into()));
    }
    if handlers.iter().any(|(h, _)| anonymous(h)) {
        if handlers.iter().any(|(h, _)| !anonymous(h)) {
            let m = "Oathkeeper anonymous fallback: a request without credentials passes; the rule is open.";
            site.add(flow, DiagnosticSeverity::Note, m.into());
        }
        return AccessState::Open;
    }
    let mut conds = vec![Condition::Authenticated];
    for (h, scopes) in handlers {
        match *h {
            "jwt"
            | "bearer_token"
            | "cookie_session"
            | "oauth2_introspection"
            | "oauth2_client_credentials" => {
                conds.extend(scopes.iter().map(|s| Condition::Policy(s.clone())));
            }
            "unauthorized" => conds.push(Condition::Never),
            other => {
                let m = format!("Oathkeeper authenticator {other} is not known.");
                site.add(flow, DiagnosticSeverity::Gap, m);
                conds.push(Condition::Unknown(format!("oathkeeper:{other}")));
            }
        }
    }
    conds.dedup();
    P(if conds.len() == 1 {
        conds.remove(0)
    } else {
        Condition::All(conds)
    })
}

fn oathkeeper_state(rule: &Y, authn: &Y, site: &Site, flow: &mut UiFlow) -> AccessState {
    use AccessState::Protected as P;
    let state = authn_state(&handlers_of(authn), site, flow);
    let authorizer = rule.get("authorizer").and_then(|a| a.get("handler"));
    match authorizer.and_then(Y::as_str) {
        Some("allow") => state,
        Some("deny") => state.and(P(Condition::Never)),
        Some(h @ ("remote" | "remote_json" | "keto_engine_acp_ory")) => {
            state.and(P(Condition::Custom(format!("oathkeeper:{h}"))))
        }
        other => {
            let m = format!(
                "Oathkeeper authorizer {} is not readable.",
                other.unwrap_or("(missing)")
            );
            site.add(flow, DiagnosticSeverity::Gap, m);
            state.and(P(Condition::Unknown("oathkeeper authorizer".into())))
        }
    }
}
