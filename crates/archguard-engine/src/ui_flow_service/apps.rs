//! What the Maven applications that include a module read for the service
//! layers (the `@EnableMethodSecurity` switch may sit in another module), and
//! the gaps for calls the compiler could not resolve.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use archguard_core::{AccessState, CallAnalysis, CallClass, CallMethod, MethodCall};
use archguard_flow_backend::ui_flow_backend::spring::{Conf, declared_access};
use archguard_flow_backend::ui_flow_backend::{Index, Out, Project, applications_of};
use archguard_flow_clike::source::Source;
use archguard_flow_common::extract_ctx::ExtractContext;

use archguard_flow_clike::decl::{Decl, Dialect, decls};
use archguard_flow_clike::lex::named;

/// `(file, line, message)` of something read but not readable.
pub(super) type Gap = (String, u32, String);

/// How `@EnableMethodSecurity(mode = ...)` applies the advice.
#[derive(Clone)]
pub(super) enum Advice {
    Proxy,
    Aspectj,
    /// `mode` is not a literal `PROXY` or `ASPECTJ`.
    Unreadable(Gap),
}

pub(super) fn advice_of(srcs: &[Source]) -> Advice {
    let (mut aspectj, mut proxy) = (false, false);
    for src in srcs.iter().filter(|s| !s.rel.ends_with(".cs")) {
        for a in (decls(src, Dialect::jvm(src)).iter())
            .filter(|d| d.class)
            .flat_map(|d| &d.attrs)
            .filter(|a| {
                matches!(
                    a.name.as_str(),
                    "EnableMethodSecurity" | "EnableGlobalMethodSecurity"
                )
            })
        {
            match named(&a.args, "mode").map(|v| v.trim().rsplit('.').next().unwrap_or("")) {
                None | Some("PROXY") => proxy = true,
                Some("ASPECTJ") => aspectj = true,
                Some(_) => {
                    let message = format!(
                        "`@{}` attribute `mode` is not readable, so whether self-invocation is advised is unknown: {}",
                        a.name, a.args
                    );
                    return Advice::Unreadable((src.rel.clone(), src.line(a.at), message));
                }
            }
        }
    }
    // one proxy-mode annotation next to an AspectJ one: assume the weaker
    if aspectj && !proxy {
        Advice::Aspectj
    } else {
        Advice::Proxy
    }
}

/// The method-security conventions an application's modules set together.
pub(super) struct AppConf<'a> {
    pub(super) modules: Vec<&'a str>,
    pub(super) conf: Conf,
    pub(super) advice: Advice,
}

/// By project dir: what each application that includes it reads (else its own).
pub(super) fn read_apps<'a>(
    ctx: &ExtractContext,
    projects: &'a BTreeMap<String, Project>,
    index: &Index,
    scratch: &mut Out,
) -> HashMap<&'a str, Vec<AppConf<'a>>> {
    let apps = applications_of(ctx.root, projects);
    let mut read: HashMap<(Vec<&str>, bool), (Conf, Advice)> = HashMap::new();
    let mut out: HashMap<&str, Vec<AppConf>> = HashMap::new();
    for (dir, (_, _, quarkus)) in projects {
        let groups: Vec<Vec<&str>> = match apps.get(dir) {
            Some(a) if !a.is_empty() => (a.iter())
                .map(|app| {
                    (app.iter())
                        .filter_map(|d| projects.get_key_value(d.as_str()))
                        .map(|(k, _)| k.as_str())
                        .collect()
                })
                .collect(),
            _ => vec![vec![dir.as_str()]],
        };
        for group in groups {
            let (conf, advice) = read.entry((group.clone(), *quarkus)).or_insert_with(|| {
                let all: Vec<Source> = (group.iter().filter_map(|d| projects.get(*d)))
                    .flat_map(|p| p.1.iter().cloned())
                    .collect();
                let mut conf = Conf::read(&all, index, *quarkus, scratch);
                conf.deferred_off = true;
                (conf, advice_of(&all))
            });
            let entry = AppConf {
                modules: group,
                conf: conf.clone(),
                advice: advice.clone(),
            };
            out.entry(dir.as_str()).or_default().push(entry);
        }
    }
    out
}

/// The advice mode the applications agree on, else the weaker proxy mode (an
/// unreadable one stays unreadable).
pub(super) fn advice_for(apps: &[&AppConf]) -> Option<Advice> {
    let unreadable = apps
        .iter()
        .map(|a| &a.advice)
        .find(|a| matches!(a, Advice::Unreadable(_)));
    let aspectj = !apps.is_empty() && apps.iter().all(|a| matches!(a.advice, Advice::Aspectj));
    Some(match (unreadable, aspectj) {
        (Some(gap), _) => gap.clone(),
        (None, true) => Advice::Aspectj,
        (None, false) => Advice::Proxy,
    })
}

/// A gap for every part of the Java call model that failed while others were read.
pub(crate) fn failed_scopes(analysis: &CallAnalysis) -> Vec<Gap> {
    let failed =
        (analysis.analyzers.iter()).filter(|a| a.language == "java" && a.status == "unavailable");
    (failed.map(|a| {
        let why: String = (a.diagnostics.first().map_or("", String::as_str))
            .chars()
            .take(200)
            .collect();
        let message = format!(
            "Checks in {} Java file(s) were not read: the call analysis of that part failed ({why}), so a check there is unknown.",
            a.files
        );
        (".".to_string(), 0, message)
    }))
    .collect()
}

/// The simple type name of the field or parameter `name` declared in `src`.
fn declared_type<'s>(src: &'s Source, name: &str) -> Option<&'s str> {
    let text = src.text.as_str();
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(name).find_map(|(at, _)| {
        let after = text[at + name.len()..].trim_start().chars().next()?;
        let before = text[..at].trim_end();
        if !matches!(after, ';' | '=' | ',' | ')')
            || text[..at].chars().next_back().is_some_and(ident)
            || text[at + name.len()..].starts_with(ident)
        {
            return None;
        }
        // `Type<..> name`: skip the generic arguments
        let head = match before.strip_suffix('>') {
            Some(_) => &before[..before.rfind('<')?],
            None => before,
        };
        let start = head.rfind(|c: char| !ident(c)).map_or(0, |p| p + 1);
        let ty = &head[start..];
        (!ty.is_empty() && ty.starts_with(char::is_uppercase)).then_some(ty)
    })
}

fn word(w: &str) -> bool {
    !w.is_empty() && w.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
}

/// A gap for every call in `methods` the compiler left unresolved on a field
/// or parameter whose type is a project class that can carry a check: a check
/// there is unknown.
pub(super) fn unresolved_gaps(
    calls: &HashMap<&str, Vec<&MethodCall>>,
    carriers: &BTreeSet<&str>,
    methods: &[(&Source, &CallMethod)],
) -> Vec<Gap> {
    let mut gaps = Vec::new();
    for (src, m) in methods {
        let calls = calls.get(m.id.as_str()).into_iter().flatten();
        for c in calls.filter(|c| c.resolution == "unresolved" && c.to_class.is_none()) {
            // `receiver.method` exactly, not a call on the result of another call
            let Some((receiver, method)) = c.target_label.split_once('.') else {
                continue;
            };
            let ty = declared_type(src, receiver).filter(|_| word(receiver) && word(method));
            if let Some(ty) = ty.filter(|t| carriers.contains(t)) {
                let message = format!(
                    "Check of `{receiver}.{method}` not resolved: the compiler could not resolve the call on `{ty}` (a project class), so a check there is unknown."
                );
                gaps.push((c.file.clone(), c.line, message));
            }
        }
    }
    gaps
}

/// Whether `call` is made from another module than the one declaring `class`.
pub(super) fn crosses(
    (sources, dir_of): (&HashMap<String, &Source>, &HashMap<&str, &str>),
    call: &MethodCall,
    class: &CallClass,
) -> bool {
    let dir = |file: &str| dir_of.get(sources.get(file)?.rel.as_str()).copied();
    dir(&call.file) != dir(&class.file) || dir(&class.file).is_none()
}

/// What the applications read of a method's check: `(state, ignored, why unknown)`.
/// The shared applications (those that include the endpoint's module) must
/// agree; with none, or when they differ, the check is unknown and `why` says so.
pub(super) fn read_state(
    (shared, own): (&[&AppConf], &[&AppConf]),
    (index, scratch): (&Index, &mut Out),
    at: (&Source, &Decl, &Decl),
    arity: usize,
) -> Option<(AccessState, bool, Option<&'static str>)> {
    let unshared = shared.is_empty();
    let reads: Vec<_> = (if unshared { own } else { shared })
        .iter()
        .map(|a| declared_access(&a.conf, index, scratch, at, arity))
        .collect();
    let (state, ignored) = reads.first().cloned()?;
    let why = if unshared {
        Some(
            "no application includes both the endpoint's module and this module, so whether method security is enabled for the call is unknown",
        )
    } else if reads.iter().any(|r| *r != (state.clone(), ignored)) {
        Some(
            "the applications that include this module differ in whether method security is enabled",
        )
    } else {
        None
    };
    // unknown: no check
    Some(match why {
        Some(_) => (AccessState::Undeclared, true, why),
        None => (state, ignored, None),
    })
}
