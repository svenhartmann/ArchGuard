//! Requests in `.http`/`.rest` files and curl calls in `*.sh`, `Dockerfile` and `Makefile`
//! become artifacts (ticket 25). Rules of this format, by the name position of the API:
//! `PUT /<index>` -> index; `PUT|POST /_index_template/<n>` and `/_template/<n>` -> index_template;
//! `/_component_template/<n>`; `/_ilm/policy/<n>` -> ilm_policy; `/_plugins/_ism/policies/<n>` and
//! `/_opendistro/_ism/policies/<n>` -> ism_policy (flavor opensearch); `/_ingest/pipeline/<n>`;
//! `POST /_aliases` -> one alias per `add` action, referencing its index or indices. Scheme, host
//! and query are ignored; GET, DELETE, `_search` and every other path create nothing. A name with a
//! variable (`${INDEX}`, `{{index}}`), a body that is no JSON object, an `-d @file` that is absolute,
//! outside the root or unreadable, and an unreadable file are gaps. The body goes through
//! `search_definition` with the kind from the path; `@file` is read relative to the script, then to
//! the root, and named in the evidence (the merge then drops the `json:` artifact of that file). A curl counts only as the literal first word of a command.
//! `scope` = `http:<folder>`, evidence = the request line. Trust boundary: files below the root
//! through the safe reader, at most 5 000 files, no test paths, no network, nothing executed.
// shortcut: curl options without a value are skipped by prefix, unknown value options (`--proxy x`)
// would make their value the URL; combined short flags (`-sd`) are not split.
use super::http_parse::{Body, Request, http_requests, shell_requests};
use super::json::search_definition;
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, SearchReference, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{relative, test_path};
use serde_json::{Map, Value};
use std::path::Path;

const MAX_FILES: usize = 5_000;

#[derive(Clone, Copy)]
enum Mode {
    Http,
    Shell,
}

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        es_gap(&mut out, EsGap::DefinitionScanStop, ".", None, &[]);
    }
    for (rel, mode) in files {
        let Some(text) = (ctx.read)(ctx.root, &ctx.root.join(&rel)) else {
            es_gap(&mut out, EsGap::DefinitionUnread, &rel, None, &[]);
            continue;
        };
        let requests = match mode {
            Mode::Http => http_requests(&text),
            Mode::Shell => shell_requests(&text),
        };
        let mut scan = Scan::new(ctx, &rel, &mut out, "http");
        requests.iter().for_each(|r| scan.request(r));
    }
    SourceOutput::from_scratch(out)
}

fn mode(rel: &str) -> Option<Mode> {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    let ext = Path::new(file).extension().and_then(|x| x.to_str());
    match ext {
        Some("http" | "rest") => Some(Mode::Http),
        Some("sh" | "bash" | "mk" | "dockerfile") => Some(Mode::Shell),
        _ => matches!(file, "Makefile" | "makefile" | "GNUmakefile")
            .then_some(Mode::Shell)
            .or_else(|| file.starts_with("Dockerfile").then_some(Mode::Shell)),
    }
}

fn candidates(ctx: &SearchContext) -> Vec<(String, Mode)> {
    let mut files: Vec<_> = ctx
        .discovery
        .walk(ctx.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(ctx.root, e.path()))
        .filter(|rel| !test_path(rel) && !rel.split('/').any(|p| p == ".terraform"))
        .filter_map(|rel| mode(&rel).map(|m| (rel, m)))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Segments of the path of `url` without scheme, host (variables allowed) and query.
fn segments(url: &str) -> Vec<&str> {
    let rest = ["http://", "https://"]
        .iter()
        .find_map(|s| url.strip_prefix(s))
        .unwrap_or(url);
    let path = if rest.starts_with('/') {
        rest
    } else {
        &rest[slash(rest)..]
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// The first `/` outside `{...}` (a host variable may hold one), or the end.
fn slash(s: &str) -> usize {
    let mut depth = 0_i32;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '/' if depth <= 0 => return i,
            _ => {}
        }
    }
    s.len()
}

/// Kind and name segment of a write to an API with a name in its path.
fn route<'a>(method: &str, segs: &[&'a str]) -> Option<(&'static str, &'a str)> {
    let write = matches!(method, "PUT" | "POST");
    match segs {
        [n] if method == "PUT" && !n.starts_with('_') => Some(("index", n)),
        ["_index_template" | "_template", n] if write => Some(("index_template", n)),
        ["_component_template", n] if write => Some(("component_template", n)),
        ["_ilm", "policy", n] if write => Some(("ilm_policy", n)),
        ["_plugins" | "_opendistro", "_ism", "policies", n] if write => Some(("ism_policy", n)),
        ["_ingest", "pipeline", n] if write => Some(("ingest_pipeline", n)),
        _ => None,
    }
}

fn literal(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
}

/// `dir` + `path` without `.`/`..`; `None` for absolute paths, variables and a way out of the root.
fn below_root(dir: &str, path: &str) -> Option<String> {
    if path.starts_with(['/', '~']) || path.contains(['$', '\\', ':']) {
        return None;
    }
    let mut parts: Vec<&str> = dir
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Turns the requests of one file into artifacts; `prefix` starts the `scope` (`http`, `devtools`).
pub(super) struct Scan<'a, 'b> {
    ctx: &'a SearchContext<'b>,
    rel: &'a str,
    out: &'a mut ServiceTopology,
    prefix: &'a str,
    n: usize,
}

impl<'a, 'b> Scan<'a, 'b> {
    pub(super) fn new(
        ctx: &'a SearchContext<'b>,
        rel: &'a str,
        out: &'a mut ServiceTopology,
        prefix: &'a str,
    ) -> Self {
        Scan {
            ctx,
            rel,
            out,
            prefix,
            n: 0,
        }
    }

    fn folder(&self) -> &str {
        self.rel.rsplit_once('/').map_or(".", |(d, _)| d)
    }

    fn gap(&mut self, kind: EsGap, parts: &[&str]) {
        es_gap(self.out, kind, self.rel, None, parts);
    }

    pub(super) fn request(&mut self, req: &Request) {
        let segs = segments(&req.url);
        if req.method == "POST" && segs == ["_aliases"] {
            return self.aliases(req);
        }
        let Some((kind, name)) = route(&req.method, &segs) else {
            // A write whose path is empty or starts with a variable may name anything.
            let hidden = segs.first().is_none_or(|s| s.contains(['$', '{']));
            if hidden && matches!(req.method.as_str(), "PUT" | "POST") {
                self.gap(EsGap::IndexNotLiteral, &[&req.line.to_string()]);
            }
            return;
        };
        if !literal(name) {
            return self.gap(EsGap::IndexNotLiteral, &[&req.line.to_string()]);
        }
        let Some((value, file)) = self.body(&req.body) else {
            return self.gap(EsGap::DefinitionUnread, &[&req.line.to_string()]);
        };
        let Some(def) = search_definition(&value, Some(kind)) else {
            return self.gap(EsGap::DefinitionUnread, &[&req.line.to_string()]);
        };
        let mut a = self.artifact(req, kind, name);
        (a.flavor, a.patterns) = (def.flavor.into(), def.patterns);
        (a.references, a.detail) = (def.references, def.detail);
        a.evidence.extend(file.map(|f| ServiceEvidence {
            file: f,
            pointer: String::new(),
            line: None,
            description: "Body read from this file; no cluster access.".into(),
        }));
        self.out.elasticsearch.artifacts.push(a);
    }

    fn artifact(&mut self, req: &Request, kind: &str, name: &str) -> SearchArtifact {
        self.n += 1;
        SearchArtifact {
            kind: kind.into(),
            name: Some(name.into()),
            address: format!("{}#{}", self.rel, self.n),
            scope: format!("{}:{}", self.prefix, self.folder()),
            evidence: vec![ServiceEvidence {
                file: self.rel.into(),
                pointer: format!("{} {}", req.method, req.url),
                line: Some(req.line),
                description: format!(
                    "{kind} {name} declared by a {} request; read as data, never sent.",
                    req.method
                ),
            }],
            patterns: vec![],
            references: vec![],
            origins: vec![],
            detail: None,
            flavor: String::new(),
        }
    }

    /// The parsed body (an empty object without one) and the file it came from.
    fn body(&self, body: &Body) -> Option<(Value, Option<String>)> {
        let (text, file) = match body {
            Body::None => return Some((Value::Object(Map::new()), None)),
            Body::Inline(t) => (t.clone(), None),
            Body::File(p) => {
                let rel = [self.folder(), ""]
                    .iter()
                    .filter_map(|dir| below_root(dir, p))
                    .find(|rel| {
                        (self.ctx.read)(self.ctx.root, &self.ctx.root.join(rel)).is_some()
                    })?;
                let text = (self.ctx.read)(self.ctx.root, &self.ctx.root.join(&rel))?;
                (text, Some(rel))
            }
        };
        serde_json::from_str(&text).ok().map(|v| (v, file))
    }

    /// `POST /_aliases`: each `add` action with literal names is an alias over its indices.
    fn aliases(&mut self, req: &Request) {
        let Some((value, _)) = self.body(&req.body) else {
            return self.gap(EsGap::DefinitionUnread, &[&req.line.to_string()]);
        };
        let Some(actions) = value.get("actions").and_then(Value::as_array) else {
            return self.gap(EsGap::DefinitionUnread, &[&req.line.to_string()]);
        };
        for add in actions.iter().filter_map(|a| a.get("add")) {
            let (indices, aliases) = (
                names(add, "index", "indices"),
                names(add, "alias", "aliases"),
            );
            let (Some(indices), Some(aliases)) = (indices, aliases) else {
                self.gap(EsGap::DefinitionUnread, &[&req.line.to_string()]);
                continue;
            };
            for alias in aliases {
                let mut a = self.artifact(req, "alias", &alias);
                a.references = indices
                    .iter()
                    .map(|i| SearchReference {
                        kind: "index".into(),
                        name: i.clone(),
                    })
                    .collect();
                self.out.elasticsearch.artifacts.push(a);
            }
        }
    }
}

/// The literal names under `one` or `many` (a string or a list of strings), else `None`.
fn names(action: &Value, one: &str, many: &str) -> Option<Vec<String>> {
    let v = action.get(one).or_else(|| action.get(many))?;
    let list: Vec<&str> = match v {
        Value::String(s) => vec![s],
        Value::Array(a) => a.iter().map(Value::as_str).collect::<Option<_>>()?,
        _ => return None,
    };
    let ok = !list.is_empty() && list.iter().all(|n| literal(n));
    ok.then(|| list.into_iter().map(str::to_owned).collect())
}
