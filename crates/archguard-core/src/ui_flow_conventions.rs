//! Role conventions: how ONE application writes and compares role and
//! permission names, as its extractor read it, and the merge to one entry per
//! application: per field a value read from the project beats a framework
//! default; two different read values are a gap and the field stays unread
//! (exact comparison).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How ONE application writes and compares role and permission names, as
/// its extractor read it. Nothing read means exact comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoleConvention {
    /// Same key as `Screen::application`; none is the whole flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    /// Prefix the framework puts before role names (Spring `ROLE_`); only
    /// used to canonicalize user input and shorten labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_prefix: Option<String>,
    /// Prefix for token scopes and authorities (Spring `SCOPE_`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_prefix: Option<String>,
    /// Claim that carries the roles; information only, never compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_claim: Option<String>,
    #[serde(default)]
    pub role_case: NameCase,
    #[serde(default)]
    pub policy_case: NameCase,
    #[serde(default)]
    pub permissions: PermissionFormat,
    /// Action aliases (CanCanCan `read` -> `index`, `show`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_aliases: Vec<(String, Vec<String>)>,
    /// Where each field came from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<ConventionOrigin>,
}

/// Where one [`RoleConvention`] field was read: a file position or
/// `framework default <name> <version>` in `source`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConventionOrigin {
    pub field: String,
    pub file: String,
    pub line: u32,
    pub source: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameCase {
    #[default]
    Exact,
    Insensitive,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionFormat {
    #[default]
    Exact,
    /// Dotted ids; a held id covers `wanted` when its segments are a
    /// prefix of `wanted`'s and each is equal or `wildcard` (Squidex:
    /// `app.{app}.*` and `app.{app}` cover `app.{app}.contents.read`).
    Dotted { wildcard: String },
    /// `"<action> <subject>"` (CASL, CanCanCan).
    ActionSubject {
        any_action: String,
        any_subject: String,
    },
}

/// A conflict: file, line and message of a field read with different values.
pub(crate) type Clash = (String, u32, String);

/// One convention per application, and the conflicts found.
pub(crate) fn merge_conventions(all: Vec<RoleConvention>) -> (Vec<RoleConvention>, Vec<Clash>) {
    let mut by_app: BTreeMap<Option<String>, Vec<RoleConvention>> = BTreeMap::new();
    for c in all {
        by_app.entry(c.application.clone()).or_default().push(c);
    }
    let mut clashes = Vec::new();
    let merged = (by_app.into_values())
        .map(|group| match group.as_slice() {
            [one] => one.clone(),
            _ => merge_group(&group, &mut clashes),
        })
        .collect();
    (merged, clashes)
}

fn merge_group(group: &[RoleConvention], clashes: &mut Vec<Clash>) -> RoleConvention {
    let mut out = RoleConvention {
        application: group[0].application.clone(),
        ..RoleConvention::default()
    };
    let mut pick = |field: &str, get: &dyn Fn(&RoleConvention) -> Option<String>| {
        pick(group, field, get, clashes)
    };
    out.role_prefix = pick("role_prefix", &|c| c.role_prefix.clone());
    out.scope_prefix = pick("scope_prefix", &|c| c.scope_prefix.clone());
    out.role_claim = pick("role_claim", &|c| c.role_claim.clone());
    out.role_case = case(pick("role_case", &|c| non_exact(c.role_case)));
    out.policy_case = case(pick("policy_case", &|c| non_exact(c.policy_case)));
    out.permissions = (group.iter())
        .map(|c| &c.permissions)
        .find(|p| **p != PermissionFormat::Exact)
        .cloned()
        .unwrap_or_default();
    for c in group {
        out.action_aliases.extend(c.action_aliases.iter().cloned());
        out.origins.extend(c.origins.iter().cloned());
    }
    out.action_aliases.sort();
    out.action_aliases.dedup();
    out.origins.sort();
    out.origins.dedup();
    out
}

fn non_exact(c: NameCase) -> Option<String> {
    (c != NameCase::Exact).then(|| "insensitive".to_string())
}

fn case(v: Option<String>) -> NameCase {
    v.map_or(NameCase::Exact, |_| NameCase::Insensitive)
}

/// Whether every origin of `field` in `c` is a framework default.
fn is_default(c: &RoleConvention, field: &str) -> bool {
    let mut origins = c.origins.iter().filter(|o| o.field == field).peekable();
    origins.peek().is_some() && origins.all(|o| o.source.starts_with("framework default"))
}

/// The value of `field`: the read one (all read values equal), else a framework default.
fn pick(
    group: &[RoleConvention],
    field: &str,
    get: &dyn Fn(&RoleConvention) -> Option<String>,
    clashes: &mut Vec<Clash>,
) -> Option<String> {
    let mut read: Vec<(String, Option<&ConventionOrigin>)> = Vec::new();
    let mut defaults = Vec::new();
    for c in group {
        let Some(v) = get(c) else { continue };
        match is_default(c, field) {
            true => defaults.push(v),
            false => read.push((v, c.origins.iter().find(|o| o.field == field))),
        }
    }
    let Some((first, site)) = read.first() else {
        return defaults.into_iter().next();
    };
    if read.iter().all(|(v, _)| v == first) {
        return Some(first.clone());
    }
    let values: Vec<&str> = read.iter().map(|(v, _)| v.as_str()).collect();
    clashes.push((
        site.map(|o| o.file.clone()).unwrap_or_default(),
        site.map_or(0, |o| o.line),
        format!(
            "Role convention `{field}` read with different values ({}); not applied",
            values.join(", ")
        ),
    ));
    None
}
