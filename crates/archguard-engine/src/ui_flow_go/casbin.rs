//! Casbin model and CSV policy evaluation per route.
use super::model::*;
use super::text::*;
use crate::extract_ctx::{gap, read_text};
use archguard_core::{ANY_METHOD, AccessState, Condition, DiagnosticSeverity, UiFlowDiagnostic};
use std::collections::BTreeSet;

/// `(request definition, policy effect, matcher)` of a model, whitespace
/// removed from the first and last.
fn parse_model(model: &str) -> (String, String, String) {
    let mut section = String::new();
    let (mut req, mut effect, mut matcher) = (String::new(), String::new(), String::new());
    for line in model.lines() {
        let l = line.trim();
        if let Some(n) = l.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            section = n.to_string();
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        match (section.as_str(), k.trim()) {
            ("request_definition", "r") => req = v.split_whitespace().collect(),
            ("policy_effect", "e") => effect = v.trim().to_string(),
            ("matchers", "m") => matcher = v.split_whitespace().collect(),
            _ => {}
        }
    }
    (req, effect, matcher)
}

/// The `p` lines (without the leading `p`) and `g` lines of a CSV policy.
fn parse_policy(policy: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let (mut ps, mut gs) = (vec![], vec![]);
    for line in policy.lines() {
        let f: Vec<String> = line.split(',').map(|x| x.trim().to_string()).collect();
        match f.first().map(String::as_str) {
            Some("p") if f.len() >= 4 => ps.push(f[1..].to_vec()),
            Some("g") if f.len() >= 3 => gs.push(f[1..].to_vec()),
            _ => {}
        }
    }
    (ps, gs)
}

fn is_deny(p: &[String]) -> bool {
    p.get(3).is_some_and(|e| e == "deny")
}

/// The object match mode and whether the action is a regex, from a matcher
/// made only of supported clauses.
fn parse_matcher(matcher: &str) -> Result<(&'static str, bool), String> {
    let (mut sub_ok, mut act_ok, mut act_re, mut obj) = (false, false, false, None);
    for c in matcher.split("&&") {
        match c {
            "g(r.sub,p.sub)" | "r.sub==p.sub" => sub_ok = true,
            "keyMatch(r.obj,p.obj)" => obj = Some("km"),
            "keyMatch2(r.obj,p.obj)" | "keyMatch3(r.obj,p.obj)" => obj = Some("km2"),
            "r.obj==p.obj" => obj = Some("eq"),
            "r.act==p.act" => act_ok = true,
            "regexMatch(r.act,p.act)" => (act_ok, act_re) = (true, true),
            _ => return Err(format!("matcher clause {c} is not supported")),
        }
    }
    match (sub_ok, act_ok, obj) {
        (true, true, Some(obj)) => Ok((obj, act_re)),
        _ => Err("matcher misses a subject, object or action clause".into()),
    }
}

fn act_hits(act: &str, method: &str, regex: bool) -> bool {
    if !regex {
        return act.eq_ignore_ascii_case(method) || act == "*";
    }
    act.split('|').any(|a| {
        let a = a.trim_matches(['(', ')']);
        matches!(a, ".*" | "*") || a.eq_ignore_ascii_case(method)
    })
}

/// The condition one allow line gives the route; none when it does not apply.
fn line_cond(
    p: &[String],
    (obj, act_re): (&str, bool),
    (method, path): (&str, &str),
    roles: &BTreeSet<&str>,
) -> Result<Option<Condition>, String> {
    match obj_match(obj, &p[1], path) {
        Some(false) => return Ok(None),
        None => return Err("a policy path is narrower than the route path".into()),
        Some(true) => {}
    }
    if !act_hits(&p[2], method, act_re) {
        return Ok(None);
    }
    let sub = &p[0];
    if sub == "*" {
        return Err("a policy subject is a wildcard".into());
    }
    Ok(Some(if roles.contains(sub.as_str()) {
        Condition::Role(sub.clone())
    } else {
        Condition::Custom(format!("user {sub}"))
    }))
}

pub(super) fn casbin_eval(
    model: &str,
    policy: &str,
    method: &str,
    path: &str,
) -> Result<(AccessState, Option<String>), String> {
    let (req, effect, matcher) = parse_model(model);
    if req != "sub,obj,act" && req != "sub,obj,act," {
        return Err("request definition is not sub, obj, act".into());
    }
    let (ps, gs) = parse_policy(policy);
    let norm: String = effect.split_whitespace().collect();
    if norm == "!some(where(p.eft==deny))" {
        if ps.iter().any(|p| is_deny(p)) {
            return Err("deny-override policy with deny lines is not evaluated".into());
        }
        let note = "casbin deny-override, no policy: every request is allowed";
        return Ok((AccessState::Undeclared, Some(note.into())));
    }
    if norm != "some(where(p.eft==allow))" {
        return Err("policy effect is not allow-override".into());
    }
    if method == ANY_METHOD {
        return Err("route accepts any method".into());
    }
    let (obj, act_re) = parse_matcher(&matcher)?;
    let roles: BTreeSet<&str> = gs
        .iter()
        .filter_map(|g| g.get(1).map(String::as_str))
        .collect();
    let mut conds: Vec<Condition> = vec![];
    for p in ps.iter().filter(|p| !is_deny(p)) {
        if let Some(c) = line_cond(p, (obj, act_re), (method, path), &roles)? {
            conds.push(c);
        }
    }
    conds.dedup();
    let cond = match conds.len() {
        0 => Condition::Never,
        1 => conds.remove(0),
        _ => Condition::Any(conds),
    };
    Ok((AccessState::Protected(cond), None))
}

pub(super) fn is_param(seg: &str) -> bool {
    seg.starts_with(':') || (seg.starts_with('{') && seg.ends_with('}')) || seg.starts_with('*')
}

/// Whether a policy object covers a route path: `Some(true)` all requests of
/// the route, `Some(false)` none, `None` only some (not decidable).
pub(super) fn obj_match(mode: &str, pol: &str, route: &str) -> Option<bool> {
    if mode == "eq" {
        return Some(pol == route);
    }
    let (ps, rs): (Vec<&str>, Vec<&str>) = (pol.split('/').collect(), route.split('/').collect());
    for (i, p) in ps.iter().enumerate() {
        if *p == "*"
            || (mode == "km"
                && p.ends_with('*')
                && rs
                    .get(i)
                    .is_some_and(|r| r.starts_with(p.trim_end_matches('*'))))
        {
            return Some(true);
        }
        let Some(r) = rs.get(i) else {
            return Some(false);
        };
        if mode == "km2" && is_param(p) {
            continue;
        }
        if is_param(r) && !is_param(p) {
            return None;
        }
        if p != r {
            return Some(false);
        }
    }
    Some(ps.len() == rs.len())
}

impl<'a> Interp<'a> {
    pub(super) fn enforcer(&mut self, src: usize, var: String, args: (usize, usize), off: usize) {
        let s = &self.srcs[src];
        let a = split_top(&s.code, args.0, args.1, b',');
        let at = self.at(src, off);
        let model = a.first().and_then(|r| s.lit(*r));
        let policy = match a.get(1) {
            Some(r) => match s.lit(*r) {
                Some(p) => Policy::Csv(p),
                None => Policy::Db,
            },
            None => Policy::Unreadable("policy argument missing".into()),
        };
        let policy = if model.is_none() {
            Policy::Unreadable("model is not a file literal".into())
        } else {
            policy
        };
        self.enforcers.push(Enforcer { model, policy, at });
        self.enf_vars.insert(var, self.enforcers.len() - 1);
    }

    pub(super) fn read_conf(&self, name: &str) -> Option<String> {
        let m = &self.module.dir;
        let own = if m.is_empty() {
            name.to_string()
        } else {
            format!("{m}/{name}")
        };
        read_text(self.ctx.root, &own, 1 << 20)
            .ok()
            .or_else(|| read_text(self.ctx.root, name, 1 << 20).ok())
    }

    pub(super) fn casbin_state(&mut self, e: usize, method: &str, path: &str) -> AccessState {
        let enf = &self.enforcers[e];
        let at = enf.at.clone();
        let unknown = |this: &mut Self, why: &str| {
            gap(
                &mut this.diags,
                &at.0,
                at.1,
                format!("Casbin policy not evaluated: {why}."),
            );
            AccessState::Protected(Condition::Unknown(format!("casbin {why}")))
        };
        let Some(model_file) = enf.model.clone() else {
            return unknown(self, "model not read");
        };
        let policy_file = match &enf.policy {
            Policy::Csv(p) => p.clone(),
            Policy::Db => return unknown(self, "policy in database"),
            Policy::Unreadable(w) => {
                let w = w.clone();
                return unknown(self, &w);
            }
        };
        let (Some(model), Some(policy)) =
            (self.read_conf(&model_file), self.read_conf(&policy_file))
        else {
            return unknown(self, "model or policy file not found");
        };
        match casbin_eval(&model, &policy, method, path) {
            Ok((st, note)) => {
                if let Some(n) = note {
                    self.diags.push(UiFlowDiagnostic {
                        file: at.0.clone(),
                        line: at.1,
                        message: n,
                        severity: DiagnosticSeverity::Note,
                    });
                }
                st
            }
            Err(why) => unknown(self, &why),
        }
    }
}
