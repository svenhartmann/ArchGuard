//! Quarkus and Jakarta REST endpoints and their access for ui-flow/v1
//! (ADR 0021, 0022).
//!
//! An endpoint is a method with `@GET`/`@POST`/... in a class with `@Path`
//! (or in an interface with `@Path`, then each implementing class is one
//! endpoint); its path is `quarkus.http.root-path` + `@ApplicationPath` (else
//! `quarkus.rest.path`/`quarkus.resteasy.path`) + class `@Path` + method
//! `@Path`. Java and Kotlin sources are read with the declaration reader of
//! `ui_flow_backend`; the access of Spring controllers is read there.
//!
//! Access is `and(HTTP layer, annotation layer)`; an `Open` layer never lifts
//! a protected one (`@PermitAll` does not lift `quarkus.http.auth.permission`).
//! - HTTP layer: `quarkus.http.auth.permission.<name>.{paths,methods,policy,
//!   enabled,shared}` from `application.properties`/`.yaml`/`.yml`. The set
//!   with the longest matching path wins, among those one naming the request
//!   method, several winners are ANDed, a path hit without a method hit is
//!   `never`; `shared=true` sets always apply. `permit` = open, `deny` =
//!   never, `authenticated`, any other name = `policy(name)` (its
//!   `roles-allowed` is emitted as a policy definition).
//! - Annotation layer: `@RolesAllowed`/`@PermitAll`/`@DenyAll`/`@Authenticated`
//!   (method replaces class; `**` = signed in; `${key}` resolved from the
//!   configuration), `@PermissionsAllowed` (values: any, `inclusive` or
//!   repeated: all) and `@AuthorizationPolicy`; unannotated endpoints get
//!   `deny-unannotated-endpoints`, `default-roles-allowed` and
//!   `deny-unannotated-members`; else `undeclared`. On an interface only the
//!   implementing class counts (a note names the ignored annotation).
//!
//! Everything recognised but unreadable (`${…}` in a key, a key with a
//! `%profile.` prefix or `application-<profile>.*` file, an unread attribute
//! such as `auth-mechanism`, a path parameter against a literal permission
//! path, a non-literal role, conflicting annotations, a configuration file
//! that does not parse) is `unknown` with a gap on every endpoint it may
//! reach, never `open`. Not read: meta-annotations over the security
//! annotations, `HttpSecurity` observers, `roles-mapping` (a gap), `web.xml`,
//! the active profile, service-level checks on CDI beans.
use archguard_core::{
    AccessState, Condition, DiagnosticSeverity, EndpointAccess, PolicyDef, UiFlow, UiFlowDiagnostic,
};
use archguard_flow_clike::decl::{Attr, Decl, Dialect, class_header, decls, each_method};
use archguard_flow_clike::lex::{self, literal, literals, named, positional, split_top};
use archguard_flow_clike::quarkus_build::quarkus_build;
use archguard_flow_clike::source::Source;
use archguard_flow_clike::supertypes::{jdk_type, names_interface};
use archguard_flow_common::extract_ctx::{
    ExtractContext, ancestors, dir_name, finish, gap, read_text, walk, with_root_config,
};
use archguard_flow_common::support::test_path;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 20_000;
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const VERBS: [&str; 7] = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
const STANDARD: [&str; 4] = ["RolesAllowed", "PermitAll", "DenyAll", "Authenticated"];
const PERMISSION: &str = "quarkus.http.auth.permission.";
const POLICY: &str = "quarkus.http.auth.policy.";
const SET_ATTRS: [&str; 6] = [
    "paths",
    "methods",
    "policy",
    "enabled",
    "shared",
    "applies-to",
];

/// Endpoints with their access and diagnostics.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut out = Out(UiFlow::default());
    match walk(ctx.root) {
        Ok(walked) => scan(ctx.root, walked.files, &mut out),
        Err(e) => out.gap(".", 0, format!("Discovery settings not read: {e}")),
    }
    finish(&mut out.0);
    out.0
}

struct Out(UiFlow);
impl Out {
    fn gap(&mut self, file: &str, line: u32, message: impl Into<String>) {
        gap(&mut self.0.diagnostics, file, line, message);
    }
    fn note(&mut self, file: &str, line: u32, message: impl Into<String>) {
        self.0.diagnostics.push(UiFlowDiagnostic {
            file: file.into(),
            line,
            message: message.into(),
            severity: DiagnosticSeverity::Note,
        });
    }
    fn unknown(&mut self, file: &str, line: u32, text: &str) -> Condition {
        self.gap(file, line, format!("Access condition not readable: {text}"));
        Condition::Unknown(text.into())
    }
}

/// What one directory with a build file holds.
#[derive(Default)]
struct Project {
    sources: Vec<String>,
    configs: Vec<String>,
}

fn scan(root: &Path, files: Vec<String>, out: &mut Out) {
    let builds: BTreeSet<String> = (files.iter())
        .filter_map(|f| {
            let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
            BUILD_FILES.contains(&name).then(|| dir.to_string())
        })
        .collect();
    let projects = group(&files, &builds, out);
    let texts = read_sources(root, &projects, out);
    let relevant = relevant(&texts);
    let (mut srcs, mut protos) = (Vec::new(), Vec::new());
    for (dir, project) in projects.iter().filter(|(d, _)| relevant.contains(d)) {
        for (_, rel, text) in texts.iter().filter(|(d, _, _)| d == &dir) {
            srcs.push(Src {
                src: Source::new(rel.clone(), text),
                proj: protos.len(),
            });
        }
        let application = builds.contains(dir).then(|| dir_name(root, dir));
        let probe = if dir.is_empty() {
            "x".to_string()
        } else {
            format!("{dir}/x")
        };
        let quarkus = ancestors(&probe).any(|d| quarkus_build(root, d));
        let props = Props::read(root, &project.configs, out);
        protos.push((application, quarkus, server_build(root, dir), props));
    }
    emit(srcs, protos, out);
}

/// Whether the build file of `dir` itself depends on a Quarkus REST server
/// (not only on a REST client).
fn server_build(root: &Path, dir: &str) -> bool {
    (BUILD_FILES.iter()).any(|b| {
        let rel = if dir.is_empty() {
            (*b).to_string()
        } else {
            format!("{dir}/{b}")
        };
        read_text(root, &rel, MAX_BYTES).is_ok_and(|t| {
            let t = t
                .replace("quarkus-rest-client", "")
                .replace("quarkus-resteasy-client", "");
            t.contains("quarkus-rest") || t.contains("quarkus-resteasy")
        })
    })
}

// ---- configuration ----

mod parse {

    /// `Some(Some(profile))` for `application-<profile>.<ext>`, `Some(None)` for
    /// the plain `application.<ext>`; `None` for any other file name.
    pub(super) fn profile_of(name: &str) -> Option<Option<String>> {
        let stem = ["properties", "yaml", "yml"]
            .iter()
            .find_map(|e| name.strip_suffix(&format!(".{e}")))?;
        match stem.strip_prefix("application") {
            Some("") => Some(None),
            Some(p) => p.strip_prefix('-').map(|p| Some(p.to_string())),
            None => None,
        }
    }

    /// `name.attr` -> (name, attr); a quoted name may contain dots.
    pub(super) fn split_name(rest: &str) -> Option<(&str, &str)> {
        let end = match rest.strip_prefix('"') {
            Some(r) => r.find('"')? + 2,
            None => rest.find('.')?,
        };
        let attr = rest.get(end..)?.strip_prefix('.')?;
        Some((&rest[..end], attr))
    }

    pub(super) fn parse_properties(text: &str) -> Vec<(String, String, u32)> {
        let mut out = Vec::new();
        let mut lines = text.lines().enumerate();
        while let Some((i, raw)) = lines.next() {
            let mut line = raw.trim_start().to_string();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            while line.ends_with('\\') && !line.ends_with("\\\\") {
                line.pop();
                match lines.next() {
                    Some((_, next)) => line.push_str(next.trim_start()),
                    None => break,
                }
            }
            if let Some((k, v)) = line.split_once('=') {
                out.push((k.trim().to_string(), v.trim().to_string(), i as u32 + 1));
            }
        }
        out
    }

    pub(super) fn parse_yaml(text: &str) -> Result<Vec<(String, String, u32)>, String> {
        use serde_yaml_ng::Value;
        fn flat(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
            match v {
                Value::Mapping(m) => {
                    for (k, v) in m {
                        let k = match k {
                            Value::String(s) => s.clone(),
                            other => serde_yaml_ng::to_string(other)
                                .unwrap_or_default()
                                .trim()
                                .into(),
                        };
                        let key = if prefix.is_empty() {
                            k
                        } else {
                            format!("{prefix}.{k}")
                        };
                        flat(&key, v, out);
                    }
                }
                Value::Sequence(items) => {
                    let parts: Vec<String> = (items.iter()).map(scalar).collect();
                    out.push((prefix.into(), parts.join(",")));
                }
                v => out.push((prefix.into(), scalar(v))),
            }
        }
        fn scalar(v: &Value) -> String {
            match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => serde_yaml_ng::to_string(other)
                    .unwrap_or_default()
                    .trim()
                    .into(),
            }
        }
        let doc: Value = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
        let mut pairs = Vec::new();
        flat("", &doc, &mut pairs);
        Ok((pairs.into_iter())
            .map(|(k, v)| {
                let last = k.rsplit('.').next().unwrap_or(&k);
                let line = text
                    .lines()
                    .position(|l| l.contains(last))
                    .map_or(1, |p| p as u32 + 1);
                (k, v, line)
            })
            .collect())
    }
}
use parse::{parse_properties, parse_yaml, profile_of, split_name};

mod discover {
    use super::*;

    /// The source and configuration files by project directory.
    pub(super) fn group(
        files: &[String],
        builds: &BTreeSet<String>,
        out: &mut Out,
    ) -> BTreeMap<String, Project> {
        let mut projects: BTreeMap<String, Project> = BTreeMap::new();
        let mut count = 0;
        for rel in files.iter().filter(|f| !test_path(f)) {
            let name = rel.rsplit('/').next().unwrap_or(rel);
            let is_source = name.ends_with(".java") || name.ends_with(".kt");
            if !is_source && profile_of(name).is_none() {
                continue;
            }
            let dir = ancestors(rel).find(|d| d.is_empty() || builds.contains(*d));
            let project = projects.entry(dir.unwrap_or_default().into()).or_default();
            count += usize::from(is_source);
            if !is_source {
                project.configs.push(rel.clone());
            } else if count <= MAX_FILES {
                project.sources.push(rel.clone());
            }
        }
        if count > MAX_FILES {
            let message = format!("More than {MAX_FILES} source files; the rest were not scanned.");
            out.gap(".", 0, message);
        }
        projects
    }

    /// `(project dir, file, text)` of every source that can be read.
    pub(super) fn read_sources<'a>(
        root: &Path,
        projects: &'a BTreeMap<String, Project>,
        out: &mut Out,
    ) -> Vec<(&'a String, String, String)> {
        let mut texts = Vec::new();
        for (dir, project) in projects {
            for rel in &project.sources {
                match read_text(root, rel, MAX_BYTES) {
                    Ok(text) => texts.push((dir, rel.clone(), text)),
                    Err(e) => out.gap(rel, 0, format!("Source file not read (limit 1 MiB): {e}")),
                }
            }
        }
        texts
    }

    /// Projects that mention Jakarta REST, or a REST interface declared in one
    /// that does: such an interface is served by a class that need not mention
    /// Jakarta REST, possibly in another project.
    pub(super) fn relevant<'a>(texts: &[(&'a String, String, String)]) -> BTreeSet<&'a String> {
        let ifaces: BTreeSet<&str> = (texts.iter())
            .filter(|(_, _, t)| t.contains("ws.rs"))
            .flat_map(|(_, _, t)| interface_names(t))
            .collect();
        (texts.iter())
            .filter(|(_, _, t)| {
                t.contains("ws.rs")
                    || t.contains("setServiceBeans")
                    || ifaces.iter().any(|n| t.contains(n))
            })
            .map(|(d, _, _)| *d)
            .collect()
    }

    /// Names declared with `interface` in `text`.
    pub(super) fn interface_names(text: &str) -> Vec<&str> {
        let words: Vec<&str> = (text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
            .filter(|w| !w.is_empty())
            .collect();
        (words.windows(2))
            .filter(|w| w[0] == "interface")
            .map(|w| w[1])
            .collect()
    }
}
use discover::{group, read_sources, relevant};

struct Entry {
    key: String,
    profile: Option<String>,
    value: String,
    file: String,
    line: u32,
}

impl Entry {
    /// The entries of one configuration file.
    fn load(root: &Path, rel: &str) -> Result<Vec<Entry>, String> {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let text = read_text(root, rel, MAX_BYTES).map_err(|e| e.to_string())?;
        let pairs = match name.ends_with(".properties") {
            true => parse_properties(&text),
            false => parse_yaml(&text)?,
        };
        let file_profile = profile_of(name).flatten();
        let entries = pairs.into_iter().filter_map(|(raw, value, line)| {
            let (profile, key) = match raw.strip_prefix('%') {
                Some(r) => {
                    let (p, k) = r.split_once('.')?;
                    (Some(p.to_string()), k.to_string())
                }
                None => (file_profile.clone(), raw),
            };
            let file = rel.to_string();
            Some(Entry {
                key,
                profile,
                value,
                file,
                line,
            })
        });
        Ok(entries.collect())
    }
}

enum Read {
    Absent,
    Value(String, String, u32),
    Unreadable(String, String, u32),
}

#[derive(Default)]
struct Props {
    items: Vec<Entry>,
    /// Configuration files that did not parse.
    broken: Vec<String>,
}

impl Props {
    fn read(root: &Path, files: &[String], out: &mut Out) -> Props {
        let mut props = Props::default();
        for rel in files {
            match Entry::load(root, rel) {
                Ok(entries) => props.items.extend(entries),
                Err(e) => {
                    out.gap(rel, 0, format!("Configuration file not read: {e}"));
                    props.broken.push(rel.clone());
                }
            }
        }
        props
    }

    fn get(&self, key: &str) -> Read {
        if let Some(file) = self.broken.first() {
            return Read::Unreadable("a configuration file was not read".into(), file.clone(), 0);
        }
        let hits: Vec<&Entry> = self.items.iter().filter(|e| e.key == key).collect();
        let Some(first) = hits.first() else {
            return Read::Absent;
        };
        let (file, line) = (first.file.clone(), first.line);
        if let Some(p) = hits.iter().find_map(|e| e.profile.as_ref()) {
            return Read::Unreadable(format!("depends on the profile `{p}`"), file, line);
        }
        if hits.iter().any(|e| e.value != first.value) {
            return Read::Unreadable("several different values".into(), file, line);
        }
        if first.value.contains("${") {
            return Read::Unreadable("an unresolved expression".into(), file, line);
        }
        Read::Value(first.value.clone(), file, line)
    }

    /// `(name, attribute)` pairs of the keys below `prefix`.
    fn below(&self, prefix: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for e in &self.items {
            if let Some((name, attr)) = e.key.strip_prefix(prefix).and_then(split_name) {
                map.entry(name.into()).or_default().insert(attr.into());
            }
        }
        map
    }

    /// File and line of the first key below `prefix` + `name`.
    fn site(&self, prefix: &str) -> (String, u32) {
        (self.items.iter())
            .find(|e| e.key.starts_with(prefix))
            .map_or((String::new(), 0), |e| (e.file.clone(), e.line))
    }
}

// ---- HTTP permissions ----

/// `**` = signed in; one role is itself; several are any of them.
fn roles_condition<'a>(names: impl Iterator<Item = &'a str>) -> Option<Condition> {
    let names: Vec<&str> = names.filter(|n| !n.is_empty()).collect();
    if names.is_empty() {
        None
    } else if names.contains(&"**") {
        Some(Condition::Authenticated)
    } else {
        let mut roles: Vec<Condition> =
            names.iter().map(|n| Condition::Role((*n).into())).collect();
        Some(if roles.len() == 1 {
            roles.remove(0)
        } else {
            Condition::Any(roles)
        })
    }
}

use http::{Set, http_layer, read_sets};

mod http {
    use super::*;

    #[derive(Clone, PartialEq, Eq)]
    enum Hit {
        No,
        Maybe,
        Yes,
    }

    enum PolicyRef {
        Named(String),
        Unread(String),
    }

    pub(super) struct Set {
        name: String,
        /// Paths with the root path applied; none when unreadable.
        paths: Option<Vec<String>>,
        methods: Option<BTreeSet<String>>,
        shared: bool,
        policy: PolicyRef,
        /// Why the set may apply to more than it is read to.
        unsure: Option<String>,
    }

    fn why(unsure: &mut Option<String>, reason: String) {
        unsure.get_or_insert(reason);
    }

    fn attr_of(key: &str) -> &str {
        key.rsplit('.').next().unwrap_or(key)
    }

    fn bool_prop(props: &Props, key: &str, default: bool, unsure: &mut Option<String>) -> bool {
        match props.get(key) {
            Read::Absent => default,
            Read::Value(v, ..) if v == "true" || v == "false" => v == "true",
            Read::Value(v, ..) => {
                why(unsure, format!("`{}` is `{v}`", attr_of(key)));
                default
            }
            Read::Unreadable(w, ..) => {
                why(unsure, format!("`{}` is {w}", attr_of(key)));
                default
            }
        }
    }

    fn paths_prop(
        props: &Props,
        key: &str,
        root: &str,
        unsure: &mut Option<String>,
    ) -> Option<Vec<String>> {
        match props.get(key) {
            Read::Value(v, ..) => Some(
                (v.split(',').map(str::trim).filter(|p| !p.is_empty()))
                    .map(|p| match p.starts_with('/') {
                        true => p.to_string(),
                        false => format!("{root}/{p}"),
                    })
                    .collect(),
            ),
            Read::Absent => {
                why(unsure, "it has no `paths`".into());
                None
            }
            Read::Unreadable(w, ..) => {
                why(unsure, format!("`paths` is {w}"));
                None
            }
        }
    }

    fn methods_prop(
        props: &Props,
        key: &str,
        unsure: &mut Option<String>,
    ) -> Option<BTreeSet<String>> {
        match props.get(key) {
            Read::Absent => None,
            Read::Value(v, ..) => Some(v.split(',').map(|m| m.trim().to_uppercase()).collect()),
            Read::Unreadable(w, ..) => {
                why(unsure, format!("`methods` is {w}"));
                None
            }
        }
    }

    fn policy_prop(props: &Props, name: &str, unsure: &mut Option<String>) -> PolicyRef {
        match props.get(&format!("{PERMISSION}{name}.policy")) {
            Read::Value(v, ..) if !v.is_empty() => PolicyRef::Named(v),
            Read::Value(..) | Read::Absent => {
                why(unsure, "it has no `policy`".into());
                PolicyRef::Unread("no policy".into())
            }
            Read::Unreadable(w, ..) => {
                PolicyRef::Unread(format!("`policy` of permission `{name}` is {w}"))
            }
        }
    }

    fn read_set(
        props: &Props,
        (name, attrs): (String, BTreeSet<String>),
        root: &str,
        out: &mut Out,
    ) -> Option<Set> {
        let key = |a: &str| format!("{PERMISSION}{name}.{a}");
        let (file, line) = props.site(&format!("{PERMISSION}{name}."));
        let mut unsure: Option<String> = None;
        if !bool_prop(props, &key("enabled"), true, &mut unsure) {
            out.note(&file, line, format!("HTTP permission `{name}` is disabled"));
            return None;
        }
        let paths = paths_prop(props, &key("paths"), root, &mut unsure);
        let methods = methods_prop(props, &key("methods"), &mut unsure);
        let shared = bool_prop(props, &key("shared"), false, &mut unsure);
        let policy = policy_prop(props, &name, &mut unsure);
        if let Some(a) = attrs.iter().find(|a| !SET_ATTRS.contains(&a.as_str())) {
            why(&mut unsure, format!("attribute `{a}` is not read"));
        }
        if let PolicyRef::Unread(w) = &policy {
            out.gap(&file, line, format!("HTTP permission `{name}`: {w}"));
        }
        if let Some(w) = &unsure {
            let message = format!("HTTP permission `{name}` is not fully readable: {w}");
            out.gap(&file, line, message);
        }
        Some(Set {
            name,
            paths,
            methods,
            shared,
            policy,
            unsure,
        })
    }

    /// `quarkus.http.auth.policy.<name>.roles-allowed` as a policy definition.
    fn read_policies(props: &Props, application: &Option<String>, out: &mut Out) {
        for (name, attrs) in props.below(POLICY) {
            let (file, line) = props.site(&format!("{POLICY}{name}."));
            let only_roles = attrs.len() == 1 && attrs.contains("roles-allowed");
            let requires = match props.get(&format!("{POLICY}{name}.roles-allowed")) {
                Read::Value(v, ..) if only_roles => roles_condition(v.split(',').map(str::trim)),
                _ => None,
            };
            let requires = requires.unwrap_or_else(|| {
                let message = format!("HTTP policy `{name}` is not fully readable");
                out.gap(&file, line, message);
                Condition::Unknown(format!("policy {name}"))
            });
            out.0.policies.push(PolicyDef {
                name,
                requires,
                application: application.clone(),
                file,
                line,
                source: "quarkus".into(),
            });
        }
    }

    /// The permission sets; the policy definitions go to the flow.
    pub(super) fn read_sets(
        props: &Props,
        root: &str,
        application: &Option<String>,
        out: &mut Out,
    ) -> Vec<Set> {
        let mut sets = Vec::new();
        for entry in props.below(PERMISSION) {
            sets.extend(read_set(props, entry, root, out));
        }
        read_policies(props, application, out);
        sets
    }

    /// How `pattern` matches the endpoint path `ep` (segments), and how specific
    /// the pattern is. `*` alone is one segment, a trailing `*` any number.
    fn path_hit(pattern: &str, ep: &[&str]) -> (Hit, usize) {
        let segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        let Some((last, init)) = segs.split_last() else {
            return (if ep.is_empty() { Hit::Yes } else { Hit::No }, 0);
        };
        let (tail, last_lit) = match last.strip_suffix('*') {
            Some("") => (true, None),
            Some(p) => (true, Some(p)),
            None => (false, Some(*last)),
        };
        let fixed: Vec<&str> = init.iter().copied().chain(last_lit).collect();
        let score = (fixed.iter())
            .filter(|s| **s != "*")
            .map(|s| 1000 + s.len())
            .sum();
        if ep.len() < fixed.len() || (!tail && ep.len() != fixed.len()) {
            return (Hit::No, score);
        }
        let mut hit = Hit::Yes;
        for (p, e) in fixed.iter().zip(ep) {
            if *p == "*" {
                continue;
            }
            if p.contains('*') || e.contains('{') {
                hit = Hit::Maybe; // a wildcard inside a segment, or a parameter value, is not read
            } else if p != e {
                return (Hit::No, score);
            }
        }
        (hit, score)
    }

    /// The sets whose path surely matches (with their specificity), and the
    /// names of those that may match.
    fn classify<'a>(sets: &'a [Set], ep: &[&str]) -> (Vec<(&'a Set, usize)>, Vec<&'a str>) {
        let (mut hits, mut maybe) = (Vec::new(), Vec::new());
        for s in sets {
            let Some(paths) = &s.paths else {
                maybe.push(s.name.as_str());
                continue;
            };
            let best = (paths.iter())
                .map(|p| path_hit(p, ep))
                .filter(|(h, _)| *h != Hit::No)
                .max_by_key(|(_, score)| *score);
            match best {
                None => {}
                Some((Hit::Yes, score)) if s.unsure.is_none() => hits.push((s, score)),
                Some(_) => maybe.push(s.name.as_str()),
            }
        }
        (hits, maybe)
    }

    /// The sets that apply (winners by longest path, naming the method first,
    /// plus the shared ones) by name, and whether no winner allows the method.
    fn applying<'a>(hits: &[(&'a Set, usize)], method: &str) -> (Vec<&'a Set>, bool) {
        let best = hits.iter().map(|(_, score)| *score).max().unwrap_or(0);
        let allows = |s: &Set| s.methods.as_ref().is_none_or(|m| m.contains(method));
        let top: Vec<&Set> = (hits.iter())
            .filter(|(_, sc)| *sc == best)
            .map(|(s, _)| *s)
            .collect();
        let mut winners: Vec<&Set> = (top.iter())
            .copied()
            .filter(|s| s.methods.is_some() && allows(s))
            .collect();
        if winners.is_empty() {
            winners = (top.iter())
                .copied()
                .filter(|s| s.methods.is_none())
                .collect();
        }
        let mut layers: Vec<&Set> = (hits.iter())
            .map(|(s, _)| *s)
            .filter(|s| s.shared && allows(s))
            .collect();
        for w in winners.iter().copied() {
            if !layers.iter().any(|l| l.name == w.name) {
                layers.push(w);
            }
        }
        layers.sort_by(|a, b| a.name.cmp(&b.name));
        (layers, winners.is_empty())
    }

    fn state_of(policy: &PolicyRef) -> AccessState {
        use AccessState::{Open, Protected};
        match policy {
            PolicyRef::Unread(w) => Protected(Condition::Unknown(w.clone())),
            PolicyRef::Named(p) => match p.as_str() {
                "permit" => Open,
                "deny" => Protected(Condition::Never),
                "authenticated" => Protected(Condition::Authenticated),
                name => Protected(Condition::Policy(name.into())),
            },
        }
    }

    /// The HTTP layer for `method` `path`: the longest path wins, those naming
    /// the method before those without, all winners and the shared sets must
    /// allow.
    pub(super) fn http_layer(
        sets: &[Set],
        method: &str,
        path: &str,
        site: (&str, u32),
        out: &mut Out,
    ) -> AccessState {
        let ep: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let (hits, maybe) = classify(sets, &ep);
        if !maybe.is_empty() {
            let text = format!("HTTP permission {} may apply", maybe.join(", "));
            out.gap(site.0, site.1, format!("{method} {path}: {text}"));
            return AccessState::Protected(Condition::Unknown(text));
        }
        if hits.is_empty() {
            return AccessState::Undeclared;
        }
        let (layers, miss) = applying(&hits, method);
        let mut state = match miss {
            true => AccessState::Protected(Condition::Never), // path hit, no method hit
            false => AccessState::Undeclared,
        };
        for s in layers {
            state = state.and(state_of(&s.policy));
        }
        state
    }
}

// ---- annotations ----

/// What the annotations of one element say, per family.
#[derive(Default)]
struct Sec {
    std: Option<AccessState>,
    permissions: Option<Condition>,
    policy: Option<Condition>,
}

impl Sec {
    fn is_empty(&self) -> bool {
        self.std.is_none() && self.permissions.is_none() && self.policy.is_none()
    }
    /// The method's own annotation of a family replaces the class's.
    fn over(self, class: Sec) -> Sec {
        Sec {
            std: self.std.or(class.std),
            permissions: self.permissions.or(class.permissions),
            policy: self.policy.or(class.policy),
        }
    }
}

/// The items of an annotation value: one, or `{…}`, `[…]`, `arrayOf(…)`.
fn list_items(raw: &str) -> Option<Vec<&str>> {
    let s = raw.trim();
    let inner = [("arrayOf(", ')'), ("{", '}'), ("[", ']')]
        .iter()
        .find_map(|(open, close)| s.strip_prefix(open)?.strip_suffix(*close));
    let items = match inner {
        Some(inner) => split_top(inner),
        None => vec![s],
    };
    (!items.iter().any(|i| i.is_empty()) && !items.is_empty()).then_some(items)
}

/// `static final String NAME = "…"` (Java) and `const val NAME = "…"` (Kotlin)
/// by `Class.NAME`.
fn constants(srcs: &[Src], classes: &[(usize, Decl)]) -> HashMap<String, Option<String>> {
    let mut map: HashMap<String, Option<String>> = HashMap::new();
    for (si, s) in srcs.iter().enumerate() {
        let text = &s.src.text;
        for marker in ["static final String ", "const val "] {
            for (at, _) in text.match_indices(marker) {
                let rest = &text[at + marker.len()..];
                let name: String = (rest.chars())
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let after = rest[name.len()..].trim_start();
                let Some(value) = after.strip_prefix('=') else {
                    continue;
                };
                let value = value.split([';', '\n']).next().unwrap_or("").trim();
                let Some(class) = (classes.iter())
                    .filter(|(csi, c)| *csi == si && c.open <= at && at <= c.end)
                    .max_by_key(|(_, c)| c.open)
                else {
                    continue;
                };
                let key = format!("{}.{name}", class.1.name);
                let value = literal(value);
                match map.get(&key) {
                    Some(old) if *old != value => {
                        map.insert(key, None);
                    }
                    _ => {
                        map.insert(key, value);
                    }
                }
            }
        }
    }
    map
}

fn has_security(attrs: &[Attr]) -> bool {
    attrs.iter().any(|a| {
        STANDARD.contains(&a.name.as_str())
            || matches!(
                a.name.as_str(),
                "PermissionsAllowed" | "AuthorizationPolicy"
            )
    })
}

fn value_of(a: &Attr) -> &str {
    named(&a.args, "value")
        .or_else(|| positional(&a.args))
        .unwrap_or("")
}

fn all_of(mut items: Vec<Condition>) -> Option<Condition> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(Condition::All(items)),
    }
}

struct Reader<'a> {
    props: &'a Props,
    src: &'a Source,
    /// `Class.NAME` -> literal value of a string constant; none when two
    /// classes of that name disagree.
    consts: &'a HashMap<String, Option<String>>,
}

impl Reader<'_> {
    fn unknown(&self, out: &mut Out, a: &Attr, text: &str) -> Condition {
        out.unknown(&self.src.rel, self.src.line(a.at), text)
    }

    fn read(&self, attrs: &[Attr], out: &mut Out) -> Sec {
        let std: Vec<&Attr> = attrs
            .iter()
            .filter(|a| STANDARD.contains(&a.name.as_str()))
            .collect();
        let std = match std.as_slice() {
            [] => None,
            [a] => Some(self.standard(a, out)),
            [a, ..] => {
                let c = self.unknown(out, a, "several security annotations on one element");
                Some(AccessState::Protected(c))
            }
        };
        let permissions: Vec<Condition> = (attrs.iter())
            .filter(|a| a.name == "PermissionsAllowed")
            .map(|a| self.permission(a, out))
            .collect();
        let policy = attrs
            .iter()
            .find(|a| a.name == "AuthorizationPolicy")
            .map(|a| match named(&a.args, "name").and_then(literal) {
                Some(n) if !n.contains("${") => Condition::Policy(n),
                _ => self.unknown(out, a, &a.args),
            });
        Sec {
            std,
            permissions: all_of(permissions),
            policy,
        }
    }

    fn standard(&self, a: &Attr, out: &mut Out) -> AccessState {
        use AccessState::Protected;
        match a.name.as_str() {
            "PermitAll" => AccessState::Open,
            "DenyAll" => Protected(Condition::Never),
            "Authenticated" => Protected(Condition::Authenticated),
            _ => {
                let raw = value_of(a);
                let cond = list_items(raw).and_then(|items| {
                    let mut names = Vec::new();
                    for item in items {
                        match literal(item) {
                            Some(v) => names.extend(self.expand(&v)?),
                            None => names.push(self.constant(item)?),
                        }
                    }
                    roles_condition(names.iter().map(String::as_str))
                });
                Protected(cond.unwrap_or_else(|| self.unknown(out, a, raw)))
            }
        }
    }

    /// `Class.NAME` (qualified names are cut to the last two segments).
    fn constant(&self, item: &str) -> Option<String> {
        let segs: Vec<&str> = item.trim().split('.').map(str::trim).collect();
        let [.., class, name] = segs.as_slice() else {
            return None;
        };
        let ident =
            |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
        if !(ident(class) && ident(name)) {
            return None;
        }
        self.consts
            .get(&format!("{class}.{name}"))
            .cloned()
            .flatten()
    }

    /// A role literal: `${key}` is the configured value (a list splits).
    fn expand(&self, v: &str) -> Option<Vec<String>> {
        let Some(inner) = v.strip_prefix("${").and_then(|r| r.strip_suffix('}')) else {
            return (!v.contains("${")).then(|| vec![v.to_string()]);
        };
        match self.props.get(inner) {
            Read::Value(value, ..) if !inner.contains(['$', ':']) => {
                Some(value.split(',').map(|s| s.trim().to_string()).collect())
            }
            _ => None,
        }
    }

    fn permission(&self, a: &Attr, out: &mut Out) -> Condition {
        if named(&a.args, "permission").is_some() || named(&a.args, "params").is_some() {
            return Condition::Custom(a.args.trim().into());
        }
        let vals =
            literals(value_of(a)).filter(|v| !v.is_empty() && !v.iter().any(|x| x.contains("${")));
        let Some(vals) = vals else {
            return self.unknown(out, a, &a.args);
        };
        let mut atoms: Vec<Condition> = vals.into_iter().map(Condition::Policy).collect();
        if atoms.len() == 1 {
            atoms.remove(0)
        } else if named(&a.args, "inclusive") == Some("true") {
            Condition::All(atoms)
        } else {
            Condition::Any(atoms)
        }
    }

    /// The annotation layer of a method (`attrs`) in `class`: annotations
    /// (method before class), else the configured defaults.
    fn layer(
        &self,
        quarkus: bool,
        (attrs, class): (&[Attr], &Decl),
        others: bool,
        out: &mut Out,
    ) -> AccessState {
        let sec = self.read(attrs, out).over(self.read(&class.attrs, out));
        let mut layer = sec.std.clone().unwrap_or_default();
        if sec.is_empty() && quarkus {
            layer = self.defaults(others, out);
        }
        for c in [sec.permissions, sec.policy].into_iter().flatten() {
            layer = layer.and(AccessState::Protected(c));
        }
        layer
    }

    /// A boolean Quarkus property; absent is false.
    fn flag(&self, key: &str, out: &mut Out) -> Result<bool, Condition> {
        match self.props.get(key) {
            Read::Absent => Ok(false),
            Read::Value(v, ..) if v == "true" || v == "false" => Ok(v == "true"),
            Read::Value(v, f, l) => Err(out.unknown(&f, l, &format!("{key}={v}"))),
            Read::Unreadable(w, f, l) => Err(out.unknown(&f, l, &format!("{key} is {w}"))),
        }
    }

    /// What an endpoint without any security annotation gets.
    fn defaults(&self, others_annotated: bool, out: &mut Out) -> AccessState {
        use AccessState::{Protected, Undeclared};
        match self.flag("quarkus.security.jaxrs.deny-unannotated-endpoints", out) {
            Ok(true) => return Protected(Condition::Never),
            Err(c) => return Protected(c),
            Ok(false) => {}
        }
        if others_annotated {
            match self.flag("quarkus.security.deny-unannotated-members", out) {
                Ok(true) => return Protected(Condition::Never),
                Err(c) => return Protected(c),
                Ok(false) => {}
            }
        }
        let key = "quarkus.security.jaxrs.default-roles-allowed";
        match self.props.get(key) {
            Read::Absent => Undeclared,
            Read::Value(v, f, l) => match roles_condition(v.split(',').map(str::trim)) {
                Some(c) => Protected(c),
                None => Protected(out.unknown(&f, l, &format!("{key}={v}"))),
            },
            Read::Unreadable(w, f, l) => Protected(out.unknown(&f, l, &format!("{key} is {w}"))),
        }
    }
}

// ---- endpoints ----

/// One path part of an annotation (`@Path`, `@ApplicationPath`): a literal,
/// regex of a parameter dropped (`{id:\d+}` -> `{id}`).
fn path_of(a: &Attr) -> Option<String> {
    let mut text = literal(value_of(a))?;
    while let Some(open) = text.find('{') {
        let close = text[open..].find('}')? + open;
        let name = text[open + 1..close]
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        text.replace_range(open..=close, &format!("\u{1}{name}\u{2}"));
    }
    Some(text.replace('\u{1}', "{").replace('\u{2}', "}"))
}

fn join(parts: &[&str]) -> String {
    let segs: Vec<&str> = (parts.iter())
        .flat_map(|p| p.split('/'))
        .filter(|s| !s.is_empty())
        .collect();
    format!("/{}", segs.join("/"))
}

/// A source file with the index of its project.
struct Src {
    src: Source,
    proj: usize,
}

/// What the configuration of one project says about the URL and the HTTP layer.
struct Pctx {
    application: Option<String>,
    quarkus: bool,
    /// Whether the project serves REST resources: a server dependency, an
    /// `@ApplicationPath` or a class with `@Path`.
    serves: bool,
    props: Props,
    sets: Vec<Set>,
    prefix: String,
    prefix_unknown: Option<String>,
}

impl Pctx {
    fn build(
        index: usize,
        (application, quarkus, dep, props): Proto,
        srcs: &[Src],
        classes: &[(usize, Decl)],
        out: &mut Out,
    ) -> Pctx {
        let serves = Pctx::serves(dep, index, srcs, classes);
        let mut ctx = Pctx {
            application,
            quarkus,
            serves,
            props,
            sets: Vec::new(),
            prefix: String::new(),
            prefix_unknown: None,
        };
        if !quarkus {
            return ctx;
        }
        let root = match ctx.props.get("quarkus.http.root-path") {
            Read::Absent => "/".to_string(),
            Read::Value(v, ..) => v,
            Read::Unreadable(w, f, l) => {
                let message =
                    format!("quarkus.http.root-path is {w}; endpoint paths are incomplete");
                out.gap(&f, l, message);
                ctx.prefix_unknown = Some(format!("root path is {w}"));
                "/".into()
            }
        };
        ctx.sets = read_sets(&ctx.props, &root, &ctx.application, out);
        let mut app_paths = BTreeSet::new();
        for (si, class) in classes.iter().filter(|(si, _)| srcs[*si].proj == index) {
            let Some(a) = class.attrs.iter().find(|a| a.name == "ApplicationPath") else {
                continue;
            };
            match path_of(a) {
                Some(p) => {
                    app_paths.insert(p);
                }
                None => {
                    let src = &srcs[*si].src;
                    let message =
                        "@ApplicationPath is not a literal; endpoint paths are incomplete";
                    out.gap(&src.rel, src.line(a.at), message);
                    ctx.prefix_unknown = Some("@ApplicationPath is not a literal".into());
                }
            }
        }
        let rest = if app_paths.len() > 1 {
            ctx.prefix_unknown = Some("several @ApplicationPath values".into());
            let message =
                "Several different @ApplicationPath values; endpoint paths are incomplete";
            out.gap(".", 0, message);
            String::new()
        } else if let Some(p) = app_paths.into_iter().next() {
            p
        } else {
            let mut found = String::new();
            for key in ["quarkus.rest.path", "quarkus.resteasy.path"] {
                match ctx.props.get(key) {
                    Read::Value(v, ..) => found = v,
                    Read::Unreadable(w, f, l) => {
                        out.gap(
                            &f,
                            l,
                            format!("{key} is {w}; endpoint paths are incomplete"),
                        );
                        ctx.prefix_unknown = Some(format!("{key} is {w}"));
                    }
                    Read::Absent => {}
                }
            }
            found
        };
        ctx.prefix = join(&[&root, &rest]);
        ctx
    }

    /// Whether project `index` serves REST resources: a server dependency, an
    /// `@ApplicationPath` or a concrete class with `@Path`.
    fn serves(dep: bool, index: usize, srcs: &[Src], classes: &[(usize, Decl)]) -> bool {
        dep || (classes.iter())
            .filter(|(si, _)| srcs[*si].proj == index)
            .any(|(_, c)| {
                find(&c.attrs, "ApplicationPath").is_some()
                    || (!c.flag && find(&c.attrs, "Path").is_some())
            })
    }

    /// The HTTP layer of `method` `path` in this project.
    fn http(&self, method: &str, path: &str, site: (&str, u32), out: &mut Out) -> AccessState {
        if !self.quarkus || (self.sets.is_empty() && self.prefix_unknown.is_none()) {
            return AccessState::Undeclared;
        }
        match &self.prefix_unknown {
            Some(w) => AccessState::Protected(Condition::Unknown(w.clone())),
            None => http_layer(&self.sets, method, path, site, out),
        }
    }

    /// The first endpoint of a project: what applies to the whole of it.
    fn announce(&self, file: &str, line: u32, out: &mut Out) {
        if !self.quarkus {
            let message = "Jakarta REST without Quarkus: annotations are read as declared; whether the container enforces them is not verified";
            out.note(file, line, message);
        }
        let mapping = (self.props.items.iter())
            .find(|e| e.key.starts_with("quarkus.http.auth.roles-mapping."));
        if let (true, Some(e)) = (self.quarkus, mapping) {
            let message = "quarkus.http.auth.roles-mapping is not applied: role names are compared as written";
            out.gap(&e.file, e.line, message);
        }
    }
}

/// Who carries the security annotations of a route: the source index, the
/// class, its method annotations, and the class key.
struct Target {
    si: usize,
    class: Decl,
    attrs: Vec<Attr>,
    key: Option<(usize, usize)>,
    /// An implementation of an interface route.
    implementation: bool,
    /// An implementation in a project without a REST server: whether and how
    /// it is registered is configuration this scan does not read.
    unregistered: bool,
    /// Registered by a CXF `setServiceBeans`; the server's security chain is not evaluated.
    chain: bool,
}

type Proto = (Option<String>, bool, bool, Props);

struct Member {
    si: usize,
    class: Decl,
    method: Decl,
}

/// One HTTP method declaration of a resource, before it is placed in a project.
struct Route<'a> {
    class_path: String,
    method_path: String,
    verbs: Vec<&'a str>,
    method: &'a Decl,
    file: String,
    line: u32,
}

/// Whether `attrs` carry a JAX-RS HTTP method or `@Path`.
fn has_jaxrs(attrs: &[Attr]) -> bool {
    attrs
        .iter()
        .any(|a| VERBS.contains(&a.name.as_str()) || a.name == "Path")
}

fn find<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a Attr> {
    attrs.iter().find(|a| a.name == name)
}

/// The classes and their annotated methods of all sources.
fn collect(srcs: &[Src]) -> (Vec<Member>, Vec<(usize, Decl)>) {
    let (mut members, mut classes) = (Vec::new(), Vec::new());
    for (si, s) in srcs.iter().enumerate() {
        let declared = decls(&s.src, Dialect::jvm(&s.src))
            .into_iter()
            .filter(|d| d.class);
        classes.extend(declared.map(|d| (si, d)));
        each_method(&s.src, Dialect::jvm(&s.src), |c, m| {
            members.push(Member {
                si,
                class: c.clone(),
                method: m.clone(),
            });
        });
    }
    (members, classes)
}

fn emit(srcs: Vec<Src>, protos: Vec<Proto>, out: &mut Out) {
    let (members, classes) = collect(&srcs);
    let ctxs: Vec<Pctx> = (protos.into_iter().enumerate())
        .map(|(i, p)| Pctx::build(i, p, &srcs, &classes, out))
        .collect();
    let mut seen: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, m) in members.iter().enumerate() {
        seen.entry((m.si, m.class.open)).or_default().push(i);
    }
    let world = World {
        consts: constants(&srcs, &classes),
        jaxrs: srcs.iter().map(|s| s.src.text.contains("ws.rs")).collect(),
        srcs: &srcs,
        ctxs,
        classes,
        members,
        seen,
        checked: Default::default(),
        by_name: Default::default(),
        by_base: Default::default(),
        implementors: Default::default(),
        registered: Default::default(),
    };
    let mut world = world;
    for (i, (_, c)) in world.classes.iter().enumerate() {
        world.by_name.entry(c.name.clone()).or_default().push(i);
        for b in &c.bases {
            world.by_base.entry(b.clone()).or_default().push(i);
        }
    }
    world.registered = world.cxf_registrations();
    let world = world;
    let mut announced = BTreeSet::new();
    let first = out.0.endpoints.len();
    for m in &world.members {
        world.member(m, &mut announced, out);
    }
    // one endpoint per implementation method, however many interfaces declare it
    let mut seen_keys = BTreeSet::new();
    let mut index = 0;
    out.0.endpoints.retain(|e| {
        index += 1;
        index <= first
            || seen_keys.insert((
                e.application.clone(),
                e.method.clone(),
                e.path.clone(),
                e.handler.clone(),
            ))
    });
}

/// Reading of CXF `setServiceBeans` registrations.
mod cxf {
    use super::*;

    /// The type names of the arguments of the `setServiceBeans(args)` at `at`.
    pub(super) fn registration(src: &Source, at: usize) -> Option<Vec<&str>> {
        let (text, b) = (&src.text, src.text.as_bytes());
        let open = at + "setServiceBeans".len();
        let close = src.pair(open).filter(|_| b.get(open) == Some(&b'('))?;
        let arg = &text[open + 1..close];
        let mut types = Vec::new();
        let mut k = 0;
        while k < arg.len() {
            let t = ident_at(&arg[k..]);
            if t.is_empty() {
                k += arg[k..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            let (before, after) = (arg[..k].trim_end(), arg[k + t.len()..].trim_start());
            if t == "new" {
                types.push(ident_at(after));
            } else if t.starts_with(|c: char| c.is_lowercase())
                && !before.ends_with(['.', ':'])
                && !after.starts_with('(')
            {
                types.extend(declared_type(src, at, t));
            }
            k += t.len();
        }
        Some(types)
    }

    /// The identifier at the start of `s`.
    fn ident_at(s: &str) -> &str {
        &s[..s
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(s.len())]
    }

    /// The declared type of the name `t` before `at`: `T t` or `List<T> t`, only
    /// when every declaration found names the same type.
    fn declared_type<'a>(src: &'a Source, at: usize, t: &str) -> Option<&'a str> {
        let (text, b) = (&src.text, src.text.as_bytes());
        let found: BTreeSet<&str> = lex::words(&text[..at], t)
            .filter(|p| {
                text[p + t.len()..]
                    .trim_start()
                    .starts_with([',', ')', '=', ';'])
            })
            .filter_map(|p| {
                let k = lex::skip_ws_back(b, p);
                let ty = if k > 0 && b[k - 1] == b'>' {
                    let lt = text[..k].rfind('<')?;
                    text[lt + 1..k - 1]
                        .rsplit(|c: char| !c.is_alphanumeric())
                        .next()?
                } else {
                    let (a, z) = lex::ident_before(b, k);
                    &text[a..z]
                };
                ty.starts_with(|c: char| c.is_uppercase()).then_some(ty)
            })
            .collect();
        let mut it = found.into_iter();
        it.next().filter(|_| it.next().is_none())
    }
}
use cxf::registration;

/// Everything the scan found, to place each resource method.
struct World<'a> {
    srcs: &'a [Src],
    ctxs: Vec<Pctx>,
    classes: Vec<(usize, Decl)>,
    members: Vec<Member>,
    /// Members by (source, class) key.
    seen: HashMap<(usize, usize), Vec<usize>>,
    consts: HashMap<String, Option<String>>,
    jaxrs: Vec<bool>,
    /// Classes whose supertypes were checked: (source, class key).
    checked: std::cell::RefCell<BTreeSet<(usize, usize)>>,
    /// Indices into `classes` by simple name.
    by_name: HashMap<String, Vec<usize>>,
    /// Indices into `classes` by the simple names in their base list.
    by_base: HashMap<String, Vec<usize>>,
    /// The implementing classes of an interface (source, class key), by `classes` index.
    implementors: std::cell::RefCell<HashMap<(usize, usize), Vec<usize>>>,
    /// `classes` indices of classes registered by `setServiceBeans`.
    registered: BTreeSet<usize>,
}

/// The class and method path parts; a gap when one is not a literal.
fn path_parts(
    class: &Decl,
    method: &Decl,
    site: (&str, u32),
    out: &mut Out,
) -> Option<(String, String)> {
    let Some(cp) = find(&class.attrs, "Path").and_then(path_of) else {
        let message = format!(
            "@Path of `{}` is not a literal; its endpoints are not listed",
            class.name
        );
        out.gap(site.0, site.1, message);
        return None;
    };
    let mp = match find(&method.attrs, "Path") {
        None => String::new(),
        Some(a) => path_of(a).or_else(|| {
            let message = format!(
                "@Path of `{}.{}` is not a literal; the endpoint is not listed",
                class.name, method.name
            );
            out.gap(site.0, site.1, message);
            None
        })?,
    };
    Some((cp, mp))
}

impl World<'_> {
    fn member(&self, m: &Member, announced: &mut BTreeSet<usize>, out: &mut Out) {
        let (class, method) = (&m.class, &m.method);
        if find(&class.attrs, "Path").is_none() || !self.jaxrs[m.si] {
            return;
        }
        if find(&class.attrs, "RegisterRestClient").is_some() {
            return; // the client side of an API, not a served resource
        }
        let src = &self.srcs[m.si].src;
        let at = (method.attrs.iter()).map(|a| a.at).min();
        let (file, line) = (src.rel.clone(), src.line(at.unwrap_or(method.open)));
        let verbs: Vec<&str> = (method.attrs.iter())
            .map(|a| a.name.as_str())
            .filter(|n| VERBS.contains(n))
            .collect();
        if verbs.is_empty() {
            if find(&method.attrs, "Path").is_some() {
                let message = format!(
                    "Sub-resource locator `{}.{}` is not followed; the endpoints behind it are not listed",
                    class.name, method.name
                );
                out.gap(&file, line, message);
            }
            return;
        }
        let Some((class_path, method_path)) = path_parts(class, method, (&file, line), out) else {
            return;
        };
        let route = Route {
            class_path,
            method_path,
            verbs,
            method,
            file,
            line,
        };
        for target in self.targets(m, &route, out) {
            self.emit_target(&route, target, announced, out);
        }
    }

    /// Who carries the security annotations: the class itself, or for an
    /// interface each implementation (in any project).
    fn targets(&self, m: &Member, route: &Route, out: &mut Out) -> Vec<Target> {
        let (class, method) = (&m.class, &m.method);
        if !class.flag {
            return vec![Target {
                si: m.si,
                class: class.clone(),
                attrs: method.attrs.clone(),
                key: Some((m.si, class.open)),
                implementation: false,
                unregistered: false,
                chain: false,
            }];
        }
        let (file, line) = (route.file.as_str(), route.line);
        if has_security(&class.attrs) || has_security(&method.attrs) {
            let message = format!(
                "Security annotation on interface `{}` is not applied: Quarkus reads them on the implementation class",
                class.name
            );
            out.note(file, line, message);
        }
        if self.used_as_client(&class.name) {
            let message = format!(
                "`{}` is used as a REST client (RestClientBuilder or @RestClient), not served",
                class.name
            );
            out.note(file, line, message);
            return vec![];
        }
        let api = &self.srcs[m.si].src.text;
        let key = (m.si, class.open);
        let cached = self.implementors.borrow().get(&key).cloned();
        let found = cached.unwrap_or_else(|| {
            let found: Vec<usize> = (self.subtypes(&class.name, api).into_iter())
                .filter(|i| {
                    let (isi, c) = &self.classes[*i];
                    let src = &self.srcs[*isi].src;
                    !c.flag
                        && !class_header(src, c)
                            .split_whitespace()
                            .any(|w| w == "abstract")
                        && !src.text.contains("WebTarget") // a client-side implementation
                })
                .collect();
            self.implementors.borrow_mut().insert(key, found.clone());
            found
        });
        let serving = |i: &usize| self.ctxs[self.srcs[self.classes[*i].0].proj].serves;
        let (served, rest): (Vec<usize>, Vec<usize>) = found.into_iter().partition(serving);
        let unregistered = served.is_empty();
        let impls = if unregistered { rest } else { served };
        if impls.is_empty() {
            let message = format!(
                "No serving implementation of `{}` in the scan; it is not listed as an endpoint",
                class.name
            );
            out.note(file, line, message);
            return vec![]; // nothing serves it: a client-side declaration
        }
        (impls.into_iter())
            .map(|i| {
                let (isi, ic) = &self.classes[i];
                Target {
                    si: *isi,
                    class: ic.clone(),
                    attrs: self.own_attrs(*isi, ic, &method.name),
                    key: Some((*isi, ic.open)),
                    implementation: true,
                    unregistered,
                    chain: self.registered.contains(&i),
                }
            })
            .collect()
    }

    /// The one class called `name` that source text `from` can name (same
    /// package or imported); none when absent or ambiguous.
    fn named_from(&self, from: &str, name: &str) -> Option<usize> {
        let mut hits = (self.by_name.get(name).into_iter().flatten())
            .copied()
            .filter(|i| names_interface(from, &self.srcs[self.classes[*i].0].src.text, name));
        hits.next().filter(|_| hits.next().is_none())
    }

    /// Classes tied to a CXF server by `x.setServiceBeans(args)` in a Java
    /// source: the types named by the argument, with everything below them.
    /// Spring XML `<jaxrs:server>` is not read; such classes stay unregistered.
    fn cxf_registrations(&self) -> BTreeSet<usize> {
        let mut found = BTreeSet::new();
        for s in self.srcs {
            for at in lex::words(&s.src.text, "setServiceBeans") {
                let Some(types) = registration(&s.src, at) else {
                    continue;
                };
                for i in types.iter().filter_map(|t| self.named_from(&s.src.text, t)) {
                    let ty = &self.classes[i].1.name;
                    let api = &self.srcs[self.classes[i].0].src.text;
                    for j in self.subtypes(ty, api).into_iter().chain([i]) {
                        found.insert(j);
                    }
                }
            }
        }
        found
    }

    /// The classes and interfaces (indices into `classes`) below interface
    /// `name` of the source with text `api`: direct implementors that can name
    /// it, then everything below those.
    fn subtypes(&self, name: &str, api: &str) -> BTreeSet<usize> {
        let below = |n: &str| self.by_base.get(n).into_iter().flatten().copied();
        let mut found: BTreeSet<usize> = below(name)
            .filter(|i| names_interface(&self.srcs[self.classes[*i].0].src.text, api, name))
            .collect();
        let mut todo: Vec<usize> = found.iter().copied().collect();
        while let Some(i) = todo.pop() {
            let (pi, parent) = &self.classes[i];
            let parent_text = &self.srcs[*pi].src.text;
            for k in below(&parent.name) {
                let text = &self.srcs[self.classes[k].0].src.text;
                if names_interface(text, parent_text, &parent.name) && found.insert(k) {
                    todo.push(k);
                }
            }
        }
        found
    }

    /// A gap for each supertype of a serving class that is not in the scan
    /// exactly once: it may hold more routes.
    fn supertype_gaps(&self, t: &Target, out: &mut Out) {
        if !self.checked.borrow_mut().insert((t.si, t.class.open)) {
            return;
        }
        let src = &self.srcs[t.si].src;
        let line = src.line(t.class.attrs.first().map_or(t.class.open, |a| a.at));
        let mut seen = BTreeSet::new();
        let mut todo: Vec<(usize, &str)> =
            t.class.bases.iter().map(|b| (t.si, b.as_str())).collect();
        while let Some((from, name)) = todo.pop() {
            let known = self.by_name.contains_key(name);
            if !seen.insert(name) || (!known && jdk_type(&self.srcs[from].src, name)) {
                continue;
            }
            let mut found: Vec<&(usize, Decl)> = (self.by_name.get(name).into_iter().flatten())
                .map(|i| &self.classes[*i])
                .collect();
            if found.len() > 1 {
                // several of that name: the one the subtype's source can name
                let from_text = &self.srcs[from].src.text;
                found.retain(|(si, _)| names_interface(from_text, &self.srcs[*si].src.text, name));
            }
            match found.as_slice() {
                [(si, d)] => todo.extend(d.bases.iter().map(|b| (*si, b.as_str()))),
                _ => {
                    let message = format!(
                        "A route may be inherited from supertype {name}, which is not in the scan."
                    );
                    out.gap(&src.rel, line, message);
                }
            }
        }
    }

    /// Whether some source builds interface `name` with `RestClientBuilder…
    /// .build(Name.class)` or injects it with `@RestClient`.
    fn used_as_client(&self, name: &str) -> bool {
        let built = format!("build({name}.class)");
        (self.srcs.iter()).any(|s| {
            let t = &s.src.text;
            t.contains(&built)
                || (t.match_indices("@RestClient"))
                    .any(|(i, _)| t[i + 11..].split_whitespace().take(4).any(|w| w == name))
        })
    }

    /// The annotations of method `name` in `class`, if it has any.
    fn own_attrs(&self, si: usize, class: &Decl, name: &str) -> Vec<Attr> {
        (self.seen.get(&(si, class.open)).into_iter().flatten())
            .map(|i| &self.members[*i])
            .find(|x| x.method.name == name)
            .map_or(vec![], |x| x.method.attrs.clone())
    }

    /// Whether another method of the class carries a security annotation.
    fn others(&self, key: Option<(usize, usize)>, name: &str) -> bool {
        (key.and_then(|k| self.seen.get(&k)).into_iter().flatten())
            .map(|i| &self.members[*i])
            .any(|x| x.method.name != name && has_security(&x.method.attrs))
    }

    /// The route an implementation method declares itself: JAX-RS 3.6 (the
    /// annotations of an implementation method with any JAX-RS annotation of
    /// its own replace those of the interface method).
    fn own_route<'r>(&self, route: &Route<'r>, t: &Target, out: &mut Out) -> Option<Route<'r>> {
        let src = &self.srcs[t.si].src;
        let at = (t.attrs.iter()).map(|a| a.at).min().unwrap_or(t.class.open);
        let (file, line) = (src.rel.clone(), src.line(at));
        let verbs: Vec<&'static str> = (VERBS.iter())
            .copied()
            .filter(|v| find(&t.attrs, v).is_some())
            .collect();
        if verbs.is_empty() {
            let message = format!(
                "Sub-resource locator `{}.{}` is not followed; the endpoints behind it are not listed",
                t.class.name, route.method.name
            );
            out.gap(&file, line, message);
            return None;
        }
        let method_path = match find(&t.attrs, "Path") {
            None => String::new(),
            Some(a) => path_of(a).or_else(|| {
                let message = format!(
                    "@Path of `{}.{}` is not a literal; the endpoint is not listed",
                    t.class.name, route.method.name
                );
                out.gap(&file, line, message);
                None
            })?,
        };
        Some(Route {
            class_path: route.class_path.clone(),
            method_path,
            verbs,
            method: route.method,
            file,
            line,
        })
    }

    fn emit_target(
        &self,
        route: &Route,
        t: Target,
        announced: &mut BTreeSet<usize>,
        out: &mut Out,
    ) {
        let (tsi, key) = (t.si, t.key);
        let own;
        let route = if t.implementation && has_jaxrs(&t.attrs) {
            let Some(r) = self.own_route(route, &t, out) else {
                return;
            };
            own = r;
            &own
        } else {
            route
        };
        self.supertype_gaps(&t, out);
        let (tclass, tattrs) = (t.class, t.attrs);
        let ctx = &self.ctxs[self.srcs[tsi].proj];
        if announced.insert(self.srcs[tsi].proj) {
            ctx.announce(&route.file, route.line, out);
        }
        let reader = Reader {
            consts: &self.consts,
            props: &ctx.props,
            src: &self.srcs[tsi].src,
        };
        let others = self.others(key, &route.method.name);
        let layer = reader.layer(ctx.quarkus, (&tattrs, &tclass), others, out);
        let (file, line) = if t.unregistered {
            let src = &self.srcs[tsi].src;
            let at = (tattrs.iter()).map(|a| a.at).min().unwrap_or(tclass.open);
            (src.rel.clone(), src.line(at))
        } else {
            (route.file.clone(), route.line)
        };
        let layer = if t.unregistered {
            let why = if t.chain {
                format!(
                    "`{}` is registered by `setServiceBeans`; the server's providers, interceptors and filters are not evaluated",
                    tclass.name
                )
            } else {
                format!(
                    "`{}` is not registered where a REST server is configured; its registration is not read",
                    tclass.name
                )
            };
            out.gap(
                &file,
                line,
                format!("{why}: access of {} not read", route.method.name),
            );
            AccessState::Protected(Condition::Unknown(why))
        } else {
            layer
        };
        let path = join(&[&ctx.prefix, &route.class_path, &route.method_path]);
        let handler = format!("{}.{}", tclass.name, route.method.name);
        for verb in &route.verbs {
            let http = ctx.http(verb, &path, (&file, line), out);
            out.0.endpoints.push(EndpointAccess {
                method: (*verb).into(),
                path: path.clone(),
                access: http.and(layer.clone()),
                application: ctx.application.clone(),
                file: file.clone(),
                line,
                source: if ctx.quarkus { "quarkus" } else { "jaxrs" }.into(),
                handler: Some(handler.clone()),
                sends: vec![],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AccessState::{Open, Protected, Undeclared};
    use archguard_core::{AccessState, Condition, DiagnosticSeverity};

    const POM: &str = "<project><dependencies><dependency><artifactId>quarkus-rest</artifactId></dependency></dependencies></project>";
    const PROPS: &str = "src/main/resources/application.properties";

    fn run(files: &[(&str, &str)]) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        let mut all = vec![("pom.xml", POM)];
        all.extend_from_slice(files);
        for (path, text) in all {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    /// One resource class `R` at `/a` with the given class annotations and methods.
    fn res(class: &str, body: &str) -> String {
        format!(
            "import jakarta.ws.rs.*;\nimport jakarta.annotation.security.*;\n{class}\n@Path(\"/a\")\npublic class R {{\n{body}\n}}\n"
        )
    }
    fn go(java: &str, props: &str) -> UiFlow {
        run(&[("src/main/java/R.java", java), (PROPS, props)])
    }
    fn at(flow: &UiFlow, method: &str, path: &str) -> AccessState {
        let hits: Vec<_> = (flow.endpoints.iter())
            .filter(|e| e.method == method && e.path == path)
            .collect();
        assert_eq!(hits.len(), 1, "{method} {path} in {:?}", flow.endpoints);
        hits[0].access.clone()
    }
    fn role(r: &str) -> Condition {
        Condition::Role(r.into())
    }
    fn gaps(flow: &UiFlow) -> usize {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .count()
    }
    const M: &str = "public String m() { return \"\"; }";

    #[test]
    fn q01_roles_allowed_any_of() {
        let src = res("", &format!("@GET @RolesAllowed({{\"x\",\"y\"}}) {M}"));
        let f = go(&src, "");
        assert_eq!(
            at(&f, "GET", "/a"),
            Protected(Condition::Any(vec![role("x"), role("y")]))
        );
        assert!(f.complete);
        assert_eq!(f.endpoints[0].handler.as_deref(), Some("R.m"));
    }

    #[test]
    fn q02_method_beats_class() {
        let src = res("@RolesAllowed(\"admin\")", &format!("@GET @PermitAll {M}"));
        assert_eq!(at(&go(&src, ""), "GET", "/a"), Open);
    }

    #[test]
    fn q03_class_authenticated() {
        let src = res("@io.quarkus.security.Authenticated", &format!("@GET {M}"));
        assert_eq!(
            at(&go(&src, ""), "GET", "/a"),
            Protected(Condition::Authenticated)
        );
    }

    #[test]
    fn q04_unannotated_is_undeclared() {
        let f = go(&res("", &format!("@GET {M}")), "");
        assert_eq!(at(&f, "GET", "/a"), Undeclared);
        assert!(f.complete);
    }

    #[test]
    fn q05_deny_unannotated() {
        let src = res("", &format!("@GET {M}"));
        let f = go(
            &src,
            "quarkus.security.jaxrs.deny-unannotated-endpoints=true\n",
        );
        assert_eq!(at(&f, "GET", "/a"), Protected(Condition::Never));
    }

    #[test]
    fn q06_default_roles_allowed() {
        let src = res("", &format!("@GET {M}"));
        let f = go(&src, "quarkus.security.jaxrs.default-roles-allowed=**\n");
        assert_eq!(at(&f, "GET", "/a"), Protected(Condition::Authenticated));
        let f = go(&src, "quarkus.security.jaxrs.default-roles-allowed=a,b\n");
        assert_eq!(
            at(&f, "GET", "/a"),
            Protected(Condition::Any(vec![role("a"), role("b")]))
        );
        // deny wins when both are set
        let both = "quarkus.security.jaxrs.default-roles-allowed=a\nquarkus.security.jaxrs.deny-unannotated-endpoints=true\n";
        assert_eq!(
            at(&go(&src, both), "GET", "/a"),
            Protected(Condition::Never)
        );
    }

    const Q07: &str = "quarkus.http.auth.permission.a.paths=/*\nquarkus.http.auth.permission.a.policy=authenticated\nquarkus.http.auth.permission.b.paths=/q/*\nquarkus.http.auth.permission.b.policy=permit\n";

    fn two(q: &str, y: &str) -> String {
        format!(
            "import jakarta.ws.rs.*;\nimport jakarta.annotation.security.*;\n@Path(\"/q\") public class Q {{ @GET @Path(\"/x\") {q} {M} }}\n@Path(\"/y\") public class Y {{ @GET {y} {M} }}\n"
        )
    }

    #[test]
    fn q07_longest_path_wins() {
        let f = go(&two("", ""), Q07);
        assert_eq!(at(&f, "GET", "/q/x"), Open);
        assert_eq!(at(&f, "GET", "/y"), Protected(Condition::Authenticated));
    }

    #[test]
    fn q08_permit_all_cannot_lift_http_layer() {
        let f = go(&two("", "@PermitAll"), Q07);
        assert_eq!(at(&f, "GET", "/y"), Protected(Condition::Authenticated));
    }

    #[test]
    fn q09_http_permit_plus_annotation() {
        let f = go(&two("@RolesAllowed(\"admin\")", ""), Q07);
        assert_eq!(at(&f, "GET", "/q/x"), Protected(role("admin")));
    }

    fn api(method: &str) -> String {
        format!(
            "import jakarta.ws.rs.*;\n@Path(\"/api\") public class A {{ @{method} @Path(\"/x\") {M} @GET @Path(\"/public/a\") public String a() {{ return \"\"; }} }}\n"
        )
    }

    #[test]
    fn q10_several_winners_are_anded() {
        let p = "quarkus.http.auth.permission.p1.paths=/api/*\nquarkus.http.auth.permission.p1.policy=user\nquarkus.http.auth.permission.p2.paths=/api/*\nquarkus.http.auth.permission.p2.policy=admin\n";
        let f = go(&api("GET"), p);
        assert_eq!(
            at(&f, "GET", "/api/x"),
            Protected(Condition::All(vec![
                Condition::Policy("user".into()),
                Condition::Policy("admin".into())
            ]))
        );
    }

    #[test]
    fn q10_roles_allowed_policy_is_defined() {
        let p = "quarkus.http.auth.permission.p1.paths=/api/*\nquarkus.http.auth.permission.p1.policy=user\nquarkus.http.auth.policy.user.roles-allowed=u,v\n";
        let f = go(&api("GET"), p);
        assert_eq!(
            at(&f, "GET", "/api/x"),
            Protected(Condition::Policy("user".into()))
        );
        assert_eq!(f.policies.len(), 1);
        assert_eq!(
            f.policies[0].requires,
            Condition::Any(vec![role("u"), role("v")])
        );
    }

    #[test]
    fn q11_path_hit_method_miss_is_never() {
        let p = "quarkus.http.auth.permission.p.paths=/api/*\nquarkus.http.auth.permission.p.methods=GET\nquarkus.http.auth.permission.p.policy=authenticated\n";
        let f = go(&api("POST"), p);
        assert_eq!(at(&f, "POST", "/api/x"), Protected(Condition::Never));
        assert_eq!(
            at(&f, "GET", "/api/public/a"),
            Protected(Condition::Authenticated)
        );
    }

    #[test]
    fn q12_longest_path_permit_beats_deny() {
        let p = "quarkus.http.auth.permission.p1.paths=/api/*\nquarkus.http.auth.permission.p1.policy=deny\nquarkus.http.auth.permission.p2.paths=/api/public/*\nquarkus.http.auth.permission.p2.policy=permit\n";
        let f = go(&api("GET"), p);
        assert_eq!(at(&f, "GET", "/api/public/a"), Open);
        assert_eq!(at(&f, "GET", "/api/x"), Protected(Condition::Never));
    }

    #[test]
    fn q13_wildcard_needs_a_segment_boundary() {
        let src = format!(
            "import jakarta.ws.rs.*;\n@Path(\"/public-info\") public class P {{ @GET {M} }}\n@Path(\"/public\") public class Q {{ @GET @Path(\"/z\") {M} }}\n"
        );
        let p = "quarkus.http.auth.permission.p.paths=/public*\nquarkus.http.auth.permission.p.policy=deny\n";
        let f = go(&src, p);
        assert_eq!(at(&f, "GET", "/public-info"), Undeclared);
        assert_eq!(at(&f, "GET", "/public/z"), Protected(Condition::Never));
    }

    #[test]
    fn q14_permissions_allowed() {
        let src = res(
            "",
            &format!(
                "@GET @PermissionsAllowed({{\"see:all\",\"read\"}}) {M}\n@POST @Path(\"/w\") @PermissionsAllowed(\"create\") @PermissionsAllowed(\"update\") {M}"
            ),
        );
        let f = go(&src, "");
        let p = |n: &str| Condition::Policy(n.into());
        assert_eq!(
            at(&f, "GET", "/a"),
            Protected(Condition::Any(vec![p("see:all"), p("read")]))
        );
        assert_eq!(
            at(&f, "POST", "/a/w"),
            Protected(Condition::All(vec![p("create"), p("update")]))
        );
    }

    #[test]
    fn q15_property_expression_in_roles_allowed() {
        let src = res("", &format!("@GET @RolesAllowed(\"${{admin-role}}\") {M}"));
        let f = go(&src, "admin-role=boss\n");
        assert_eq!(at(&f, "GET", "/a"), Protected(role("boss")));
        assert!(f.complete);
        let f = go(&src, "");
        assert!(matches!(
            at(&f, "GET", "/a"),
            Protected(Condition::Unknown(_))
        ));
        assert!(!f.complete);
    }

    #[test]
    fn q16_profile_key_is_not_a_fact() {
        let src = res("", &format!("@GET @PermitAll {M}"));
        let f = go(&src, "%prod.quarkus.http.auth.permission.x.policy=deny\n");
        assert!(
            matches!(at(&f, "GET", "/a"), Protected(Condition::Unknown(_))),
            "{:?}",
            f.endpoints
        );
        assert!(gaps(&f) > 0 && !f.complete);
    }

    #[test]
    fn q17_interface_annotation_is_not_applied() {
        let src = "import jakarta.ws.rs.*;\nimport jakarta.annotation.security.*;\n@Path(\"/i\") @RolesAllowed(\"admin\") public interface Api { @GET String g(); }\npublic class Impl implements Api { public String g() { return \"\"; } }\n";
        let f = go(src, "");
        assert_eq!(at(&f, "GET", "/i"), Undeclared);
        assert!((f.diagnostics.iter()).any(|d| d.message.contains("implementation")));
        assert_eq!(f.endpoints[0].handler.as_deref(), Some("Impl.g"));
        // the implementation's own annotation is read
        let src = src.replace(
            "public class Impl",
            "@RolesAllowed(\"u\") public class Impl",
        );
        assert_eq!(at(&go(&src, ""), "GET", "/i"), Protected(role("u")));
    }

    #[test]
    fn interface_in_an_api_module_is_served_by_the_backend_module() {
        let api = "import jakarta.ws.rs.*;\n@Path(\"/api/x\") public interface XApi { @GET String g(); }\n";
        let imp = "import jakarta.annotation.security.*;\n@RolesAllowed(\"u\") public class XImpl implements XApi { public String g() { return \"\"; } }\n";
        let f = run(&[
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/XApi.java", api),
            ("backend/pom.xml", POM),
            ("backend/src/main/java/XImpl.java", imp),
            (
                "backend/src/main/resources/application.properties",
                "quarkus.http.auth.permission.a.paths=/api/*\nquarkus.http.auth.permission.a.policy=authenticated\n",
            ),
        ]);
        let e = &f.endpoints[0];
        assert_eq!(e.application.as_deref(), Some("backend"));
        assert_eq!(e.handler.as_deref(), Some("XImpl.g"));
        assert_eq!(
            e.access,
            Protected(Condition::All(vec![Condition::Authenticated, role("u")]))
        );
    }

    // W6a: a CXF registration (`setServiceBeans`) ties the implementation to a server.
    fn cxf(chain: &str, annotated: bool) -> UiFlow {
        cxf_with(chain, annotated, &[])
    }

    fn cxf_with(chain: &str, annotated: bool, extra: &[(&str, &str)]) -> UiFlow {
        let api = "import jakarta.ws.rs.*;\n@Path(\"/api/x\") public interface XApi extends JAXRSService { @GET String g(); }\n";
        let ann = if annotated {
            "@jakarta.annotation.security.PermitAll "
        } else {
            ""
        };
        let imp = format!(
            "public class XImpl implements XApi {{ {ann}public String g() {{ return \"\"; }} }}\n"
        );
        let ctx = format!(
            "class Ctx {{ @Bean Server c(final List<JAXRSService> services) {{ var s = new SpringJAXRSServerFactoryBean(); s.setServiceBeans(services.stream().map(Object.class::cast).toList());{chain} return s.create(); }} }}\n"
        );
        let mut files = vec![
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/XApi.java", api),
            (
                "api/src/main/java/JAXRSService.java",
                "public interface JAXRSService {}\n",
            ),
            ("core/pom.xml", "<project/>"),
            ("core/src/main/java/XImpl.java", imp.as_str()),
            ("core/src/main/java/Ctx.java", ctx.as_str()),
        ];
        files.extend_from_slice(extra);
        run(&files)
    }

    #[test]
    fn cxf_registration_without_chain_stays_unknown_and_names_the_registration() {
        let f = cxf("", false);
        assert!(unknown_reason(&f, "XImpl.g").contains("setServiceBeans"));
    }

    #[test]
    fn cxf_registration_with_providers_stays_unknown_and_says_why() {
        let f = cxf(" s.setProviders(List.of(filter));", true);
        let e = f.endpoints.iter().find(|e| e.path == "/api/x").unwrap();
        assert!(matches!(e.access, Protected(Condition::Unknown(_))));
        assert!(
            f.diagnostics
                .iter()
                .any(|d| d.message.contains("setServiceBeans"))
        );
    }

    // W6a review: registrations the reader cannot prove chain-free stay Unknown.
    fn unknown_reason(f: &UiFlow, h: &str) -> String {
        match handler_access(f, h) {
            Protected(Condition::Unknown(why)) => why,
            other => panic!("{h}: {other:?}"),
        }
    }

    fn handler_access(f: &UiFlow, h: &str) -> AccessState {
        (f.endpoints.iter())
            .find(|e| e.handler.as_deref() == Some(h))
            .unwrap_or_else(|| panic!("no {h} in {:?}", f.endpoints))
            .access
            .clone()
    }

    #[test]
    fn cxf_interceptor_added_through_a_getter_is_not_chain_free() {
        let f = cxf(" s.getInInterceptors().add(new Jaas());", true);
        assert!(matches!(
            handler_access(&f, "XImpl.g"),
            Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn cxf_alias_of_the_factory_is_not_chain_free() {
        let f = cxf(" var t = s; t.setProviders(List.of());", true);
        assert!(matches!(
            handler_access(&f, "XImpl.g"),
            Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn cxf_assigned_create_result_is_not_chain_free() {
        let f = cxf(
            " Server srv = s.create(); srv.getEndpoint().getInInterceptors().add(new Jaas());",
            true,
        );
        assert!(matches!(
            handler_access(&f, "XImpl.g"),
            Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn cxf_chained_call_after_create_is_not_chain_free() {
        let f = cxf(
            " s.create().getEndpoint().getInInterceptors().add(new Jaas());",
            true,
        );
        assert!(matches!(
            handler_access(&f, "XImpl.g"),
            Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn cxf_bus_configuration_anywhere_is_not_chain_free() {
        let bus = "class B { void f(Bus bus) { bus.getInInterceptors().add(new Jaas()); } }\n";
        let f = cxf_with("", true, &[("core/src/main/java/B.java", bus)]);
        assert!(matches!(
            handler_access(&f, "XImpl.g"),
            Protected(Condition::Unknown(_))
        ));
        let xml = "<beans><cxf:bus/></beans>";
        let f = cxf_with("", true, &[("core/src/main/resources/cxf.xml", xml)]);
        assert!(matches!(
            handler_access(&f, "XImpl.g"),
            Protected(Condition::Unknown(_))
        ));
    }

    const CAPI: &str = "package api;\nimport jakarta.ws.rs.*;\n@Path(\"/api/x\") public interface XApi extends JAXRSService { @GET String g(); }\n";
    fn two_impls(ctx: &str) -> UiFlow {
        let imp = |p: &str, n: &str| {
            format!(
                "package {p};\nimport api.XApi;\npublic class {n} implements XApi {{ public String g() {{ return \"\"; }} }}\n"
            )
        };
        run(&[
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/api/XApi.java", CAPI),
            (
                "api/src/main/java/api/JAXRSService.java",
                "package api; public interface JAXRSService {}\n",
            ),
            ("core/pom.xml", "<project/>"),
            ("core/src/main/java/a/XImpl.java", &imp("a", "XImpl")),
            ("other/pom.xml", "<project/>"),
            (
                "other/src/main/java/b/OtherImpl.java",
                &imp("b", "OtherImpl"),
            ),
            ("core/src/main/java/c/Ctx.java", ctx),
        ])
    }

    #[test]
    fn cxf_registered_implementation_does_not_drop_the_others() {
        let f = two_impls(
            "package c;\nimport a.XImpl;\nclass Ctx { Object c() { var s = new SpringJAXRSServerFactoryBean(); s.setServiceBeans(List.of(new XImpl())); return s.create(); } }\n",
        );
        assert!(unknown_reason(&f, "XImpl.g").contains("setServiceBeans"));
        assert!(!unknown_reason(&f, "OtherImpl.g").contains("setServiceBeans"));
    }

    #[test]
    fn cxf_argument_names_resolve_only_real_arguments() {
        // `stream` in `services.stream()` is a method call, not the parameter `stream`
        let ctx = "package c;\nimport a.XImpl;\nimport b.OtherImpl;\nclass Ctx { Object c(final List<XImpl> services, final OtherImpl stream) { var s = new SpringJAXRSServerFactoryBean(); s.setServiceBeans(services.stream().toList()); return s.create(); } }\n";
        let f = two_impls(ctx);
        assert!(unknown_reason(&f, "XImpl.g").contains("setServiceBeans"));
        assert!(!unknown_reason(&f, "OtherImpl.g").contains("setServiceBeans"));
    }

    #[test]
    fn cxf_type_name_without_import_is_ambiguous() {
        let imp = |p: &str| {
            format!(
                "package {p};\nimport api.XApi;\npublic class Dup implements XApi {{ public String g() {{ return \"\"; }} }}\n"
            )
        };
        let ctx = "package c;\nclass Ctx { Object c() { var s = new SpringJAXRSServerFactoryBean(); s.setServiceBeans(List.of(new Dup())); return s.create(); } }\n";
        let f = run(&[
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/api/XApi.java", CAPI),
            ("core/pom.xml", "<project/>"),
            ("core/src/main/java/a/Dup.java", &imp("a")),
            ("core/src/main/java/b/Dup.java", &imp("b")),
            ("core/src/main/java/c/Ctx.java", ctx),
        ]);
        assert!(!unknown_reason(&f, "Dup.g").contains("setServiceBeans"));
    }

    // W5a: an implementation outside any REST server project (CXF registers it
    // by configuration) is listed with unknown access and a gap, not dropped.
    #[test]
    fn interface_implemented_outside_a_server_project_is_unknown_with_a_gap() {
        let api = "import jakarta.ws.rs.*;\n@Path(\"/api/x\") public interface XApi { @GET String g(); }\n";
        let imp = "public class XImpl implements XApi { public String g() { return \"\"; } }\n";
        let f = run(&[
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/XApi.java", api),
            ("core/pom.xml", "<project/>"),
            ("core/src/main/java/XImpl.java", imp),
        ]);
        let e = (f.endpoints.iter())
            .find(|e| e.path == "/api/x")
            .unwrap_or_else(|| panic!("no endpoint in {:?}", f.endpoints));
        assert!(matches!(e.access, Protected(Condition::Unknown(_))));
        assert_eq!(e.file, "core/src/main/java/XImpl.java");
        assert_eq!(e.handler.as_deref(), Some("XImpl.g"));
        assert!((f.diagnostics.iter()).any(|d| {
            d.severity == DiagnosticSeverity::Gap && d.message.contains("registration")
        }));
        assert!(!f.complete);
    }

    // An interface that extends the REST interface, and a class below an
    // abstract implementation, still serve its methods.
    #[test]
    fn interface_routes_reach_implementations_through_the_hierarchy() {
        let api = "import jakarta.ws.rs.*;\n@Path(\"/p\") public interface P { @GET String g(); }\npublic interface C extends P {}\n";
        let imp = "public abstract class Base implements C { public String g() { return \"\"; } }\npublic class Impl extends Base {}\n";
        let f = go(&format!("{api}{imp}"), "");
        assert_eq!(at(&f, "GET", "/p"), Undeclared);
    }

    // JAX-RS 3.6: an implementation method with JAX-RS annotations of its own
    // replaces those of the interface method.
    #[test]
    fn implementation_method_with_own_annotations_hides_the_interface_ones() {
        let api = "import jakarta.ws.rs.*;\n@Path(\"/p\") public interface P { @GET String g(); @GET @Path(\"h\") String h(); }\n";
        let imp = "import jakarta.ws.rs.*;\npublic class Impl implements P { @GET @Path(\"other\") public String g() { return \"\"; } public String h() { return \"\"; } }\n";
        let f = go(&format!("{api}{imp}"), "");
        assert_eq!(at(&f, "GET", "/p/h"), Undeclared);
        assert!(
            !f.endpoints.iter().any(|e| e.path == "/p"),
            "{:?}",
            f.endpoints
        );
    }

    // Below the first level a subtype must name its parent by package or
    // import too: `b.Foo` extends `b.AbstractSvc`, not the one of package a.
    #[test]
    fn same_named_abstract_base_in_another_package_is_no_implementation() {
        let api = "package a;\nimport jakarta.ws.rs.*;\n@Path(\"/p\") public interface P { @GET String g(); }\n";
        let base = "package a;\npublic abstract class AbstractSvc implements P {}\n";
        let imp = "package a;\npublic class Impl extends AbstractSvc { public String g() { return \"\"; } }\n";
        let other = "package b;\npublic abstract class AbstractSvc {}\n";
        let foo = "package b;\nimport jakarta.annotation.security.*;\npublic class Foo extends AbstractSvc { @PermitAll public String g() { return \"\"; } }\n";
        let f = run(&[
            ("src/main/java/a/P.java", api),
            ("src/main/java/a/AbstractSvc.java", base),
            ("src/main/java/a/Impl.java", imp),
            ("src/main/java/b/AbstractSvc.java", other),
            ("src/main/java/b/Foo.java", foo),
        ]);
        assert_eq!(at(&f, "GET", "/p"), Undeclared);
        assert_eq!(f.endpoints[0].handler.as_deref(), Some("Impl.g"));
    }

    // A project type named like a JDK root type is still a supertype.
    #[test]
    fn project_type_named_record_is_not_skipped() {
        let src = res("", "@GET public String g() { return \"\"; }")
            .replace("class R {", "class R extends Record {");
        let f = run(&[
            ("src/main/java/R.java", &src),
            ("src/main/java/Record.java", "public class Record {}\n"),
        ]);
        assert_eq!(at(&f, "GET", "/a"), Undeclared);
        assert!(f.complete, "{:?}", f.diagnostics);
    }

    #[test]
    fn unreadable_supertype_of_a_resource_is_a_gap() {
        let src = res("", "@GET public String g() { return \"\"; }")
            .replace("class R {", "class R extends Lib {");
        let f = go(&src, "");
        assert_eq!(at(&f, "GET", "/a"), Undeclared);
        assert!((f.diagnostics.iter()).any(|d| {
            d.severity == DiagnosticSeverity::Gap
                && d.message.contains("inherited from supertype Lib")
        }));
        assert!(!f.complete);
    }

    #[test]
    fn role_constants_are_read_and_rest_clients_are_not_resources() {
        let roles = "public class Roles { public static final String ADMIN = \"admin\"; public static final String OPS = \"ops\"; }\n";
        let src = res(
            "",
            &format!(
                "@GET @RolesAllowed({{ Roles.ADMIN, Roles.OPS }}) {M}\n@POST @RolesAllowed(Roles.NOPE) {M}"
            ),
        );
        let client = "import jakarta.ws.rs.*;\n@Path(\"/c\") @RegisterRestClient public interface C { @GET String g(); }\n";
        let f = run(&[
            ("src/main/java/Roles.java", roles),
            ("src/main/java/R.java", &src),
            ("src/main/java/C.java", client),
        ]);
        assert_eq!(
            at(&f, "GET", "/a"),
            Protected(Condition::Any(vec![role("admin"), role("ops")]))
        );
        assert!(matches!(
            at(&f, "POST", "/a"),
            Protected(Condition::Unknown(_))
        ));
        assert_eq!(f.endpoints.len(), 2);
    }

    #[test]
    fn same_simple_name_interfaces_resolve_by_package() {
        let a = "package a;\nimport jakarta.ws.rs.*;\n@Path(\"/a\") public interface RunService { @GET String g(); }\n";
        let b = "package b;\nimport jakarta.ws.rs.*;\n@Path(\"/b\") public interface RunService { @GET String g(); }\n";
        let imp = "package s;\nimport a.RunService;\nimport jakarta.annotation.security.*;\n@RolesAllowed(\"u\") public class Impl implements RunService { public String g() { return \"\"; } }\n";
        let f = run(&[
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/a/RunService.java", a),
            ("api/src/main/java/b/RunService.java", b),
            ("server/pom.xml", POM),
            ("server/src/main/java/s/Impl.java", imp),
        ]);
        // /b has no serving implementation: not an endpoint
        assert_eq!(f.endpoints.len(), 1, "{:?}", f.endpoints);
        assert_eq!(at(&f, "GET", "/a"), Protected(role("u")));
    }

    #[test]
    fn client_side_implementation_serves_nothing() {
        let api = "package a;\nimport jakarta.ws.rs.*;\n@Path(\"/a\") public interface Api { @GET String g(); }\n";
        let client = "package c;\nimport a.Api;\npublic class ClientImpl implements Api { ResteasyWebTarget t; public String g() { return \"\"; } }\n";
        let f = run(&[
            ("api/pom.xml", "<project/>"),
            ("api/src/main/java/a/Api.java", api),
            (
                "client/pom.xml",
                "<project><artifactId>resteasy-client</artifactId></project>",
            ),
            ("client/src/main/java/c/ClientImpl.java", client),
        ]);
        assert!(f.endpoints.is_empty(), "{:?}", f.endpoints);
    }

    #[test]
    fn interface_built_as_rest_client_is_not_an_endpoint() {
        let api = "package a;\nimport jakarta.ws.rs.*;\n@Path(\"/hello\") public interface Hello { @GET String g(); }\n";
        let user = "package a;\nimport jakarta.ws.rs.*;\n@Path(\"/u\") public class U { @GET public String g() { return RestClientBuilder.newBuilder().build(Hello.class).g(); } }\n";
        let imp = "package a;\npublic class HelloImpl implements Hello { public String g() { return \"\"; } }\n";
        let f = run(&[
            ("src/main/java/a/Hello.java", api),
            ("src/main/java/a/U.java", user),
            ("src/main/java/a/HelloImpl.java", imp),
        ]);
        assert_eq!(f.endpoints.len(), 1, "{:?}", f.endpoints);
        assert_eq!(f.endpoints[0].path, "/u");
    }

    #[test]
    fn deny_all_yaml_and_profile_variants() {
        let src = res("", &format!("@GET @DenyAll {M}\n@POST {M}"));
        let yaml = "quarkus:\n  http:\n    auth:\n      permission:\n        a:\n          paths: /a\n          policy: authenticated\n";
        let f = run(&[
            ("src/main/java/R.java", &src),
            ("src/main/resources/application.yaml", yaml),
        ]);
        assert_eq!(
            at(&f, "GET", "/a"),
            Protected(Condition::All(vec![
                Condition::Authenticated,
                Condition::Never
            ]))
        );
        assert_eq!(at(&f, "POST", "/a"), Protected(Condition::Authenticated));
        // a profile variant of a read key makes it unreadable
        let f = go(
            &src,
            "quarkus.http.auth.permission.a.paths=/a\nquarkus.http.auth.permission.a.policy=permit\n%dev.quarkus.http.auth.permission.a.policy=deny\n",
        );
        assert!(matches!(
            at(&f, "POST", "/a"),
            Protected(Condition::Unknown(_))
        ));
        assert!(!f.complete);
    }

    #[test]
    fn unreadable_forms_are_unknown_never_open() {
        let src = res(
            "",
            &format!(
                "@GET @PermitAll {M}\n@POST @Path(\"/{{id}}\") @PermitAll {M}\n@PUT @RolesAllowed(Roles.ADMIN) {M}\n@Path(\"/sub\") public Object sub() {{ return null; }}"
            ),
        );
        let p = "quarkus.http.auth.permission.k.paths=/a/1\nquarkus.http.auth.permission.k.policy=deny\nquarkus.http.auth.permission.e.paths=${p}\nquarkus.http.auth.permission.e.policy=deny\n";
        let f = go(&src, p);
        assert_eq!(f.endpoints.len(), 3);
        for e in &f.endpoints {
            assert_ne!(e.access, Open, "{e:?}");
        }
        assert!((f.diagnostics.iter()).any(|d| d.message.contains("Sub-resource")));
        assert!(!f.complete);
    }
}
