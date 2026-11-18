//! `[licenses]`: licence policy for third-party dependencies (ADR 0016).
//!
//! Licences are compared as SPDX ids (case-insensitive). A stated licence
//! that is an SPDX expression is evaluated: `OR` takes the best branch,
//! `AND` the worst, `WITH <exception>` counts as its licence. Text that is
//! no expression (e.g. `SEE LICENSE IN LICENSE`) is one opaque id that no
//! list matches. A component without any stated licence is `unknown`.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LicensesConfig {
    /// SPDX ids that are allowed. Non-empty: every other licence violates.
    pub allow: Vec<String>,
    /// SPDX ids that always violate, even when also allowed.
    pub deny: Vec<String>,
    /// Whether a dependency without a stated licence violates.
    pub unknown: UnknownLicense,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnknownLicense {
    /// Reported as `unknown`, not a violation.
    #[default]
    Allow,
    Deny,
}

/// Policy result of one dependency. Ordered from worst to best so that
/// `OR` is `max` and `AND` is `min`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseState {
    Denied,
    /// An allow list exists and the licence is not on it.
    NotAllowed,
    /// No list mentions the licence and there is no allow list.
    Unlisted,
    Allowed,
    /// No licence stated in the local lock/manifest data.
    Unknown,
}

impl LicensesConfig {
    /// True when any rule is configured; an empty section checks nothing.
    pub fn is_active(&self) -> bool {
        !self.allow.is_empty() || !self.deny.is_empty() || self.unknown == UnknownLicense::Deny
    }

    /// Evaluate the stated licence expression (`None`: nothing stated).
    pub fn evaluate(&self, expression: Option<&str>) -> LicenseState {
        let Some(expression) = expression.map(str::trim).filter(|e| !e.is_empty()) else {
            return LicenseState::Unknown;
        };
        let tokens = tokenize(expression);
        let mut pos = 0;
        match self.or(&tokens, &mut pos) {
            Some(state) if pos == tokens.len() => state,
            // Not an SPDX expression: one opaque licence name.
            _ => self.id(expression),
        }
    }

    /// True when `state` fails the gate.
    pub fn violates(&self, state: LicenseState) -> bool {
        match state {
            LicenseState::Denied | LicenseState::NotAllowed => true,
            LicenseState::Unknown => self.unknown == UnknownLicense::Deny,
            LicenseState::Unlisted | LicenseState::Allowed => false,
        }
    }

    fn id(&self, id: &str) -> LicenseState {
        let listed = |list: &[String]| list.iter().any(|l| l.eq_ignore_ascii_case(id));
        if listed(&self.deny) {
            LicenseState::Denied
        } else if listed(&self.allow) {
            LicenseState::Allowed
        } else if self.allow.is_empty() {
            LicenseState::Unlisted
        } else {
            LicenseState::NotAllowed
        }
    }

    fn or(&self, tokens: &[String], pos: &mut usize) -> Option<LicenseState> {
        let mut state = self.and(tokens, pos)?;
        while is_op(tokens.get(*pos), "OR") {
            *pos += 1;
            state = state.max(self.and(tokens, pos)?);
        }
        Some(state)
    }

    fn and(&self, tokens: &[String], pos: &mut usize) -> Option<LicenseState> {
        let mut state = self.atom(tokens, pos)?;
        while is_op(tokens.get(*pos), "AND") {
            *pos += 1;
            state = state.min(self.atom(tokens, pos)?);
        }
        Some(state)
    }

    fn atom(&self, tokens: &[String], pos: &mut usize) -> Option<LicenseState> {
        let token = tokens.get(*pos)?;
        *pos += 1;
        let state = if token == "(" {
            let inner = self.or(tokens, pos)?;
            (tokens.get(*pos)? == ")").then_some(())?;
            *pos += 1;
            inner
        } else if token == ")"
            || ["AND", "OR", "WITH"]
                .iter()
                .any(|op| is_op(Some(token), op))
        {
            return None;
        } else {
            self.id(token)
        };
        if is_op(tokens.get(*pos), "WITH") {
            // The exception only widens the licence; it is not judged.
            let exception = tokens.get(*pos + 1)?;
            (exception != "(" && exception != ")").then_some(())?;
            *pos += 2;
        }
        Some(state)
    }
}

/// True when `text` parses as an SPDX licence expression whose ids use
/// only SPDX id characters (letters, digits, `.`, `-`, `+`, `:`).
pub fn is_spdx_expression(text: &str) -> bool {
    let tokens = tokenize(text);
    let mut pos = 0;
    LicensesConfig::default().or(&tokens, &mut pos).is_some()
        && pos == tokens.len()
        && tokens.iter().all(|t| {
            t.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | ':' | '(' | ')')
            })
        })
}

fn is_op(token: Option<&String>, op: &str) -> bool {
    token.is_some_and(|t| t.eq_ignore_ascii_case(op))
}

fn tokenize(expression: &str) -> Vec<String> {
    expression
        .replace('(', " ( ")
        .replace(')', " ) ")
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow: &[&str], deny: &[&str]) -> LicensesConfig {
        LicensesConfig {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
            unknown: UnknownLicense::Allow,
        }
    }

    #[test]
    fn parses_from_toml_with_defaults() {
        let config: LicensesConfig =
            toml::from_str("allow = [\"MIT\"]\nunknown = \"deny\"").expect("parses");
        assert_eq!(config.allow, ["MIT"]);
        assert!(config.deny.is_empty());
        assert_eq!(config.unknown, UnknownLicense::Deny);
        assert!(!LicensesConfig::default().is_active());
        assert!(toml::from_str::<LicensesConfig>("unknown = \"warn\"").is_err());
    }

    #[test]
    fn unknown_is_its_own_state_and_only_violates_when_configured() {
        let mut config = policy(&["MIT"], &[]);
        assert_eq!(config.evaluate(None), LicenseState::Unknown);
        assert_eq!(config.evaluate(Some("  ")), LicenseState::Unknown);
        assert!(!config.violates(LicenseState::Unknown));
        config.unknown = UnknownLicense::Deny;
        assert!(config.violates(LicenseState::Unknown));
    }

    #[test]
    fn allow_and_deny_lists_compare_spdx_ids_case_insensitively() {
        let config = policy(&["MIT", "Apache-2.0"], &["GPL-3.0-only"]);
        assert_eq!(config.evaluate(Some("mit")), LicenseState::Allowed);
        assert_eq!(config.evaluate(Some("GPL-3.0-only")), LicenseState::Denied);
        assert_eq!(
            config.evaluate(Some("BSD-3-Clause")),
            LicenseState::NotAllowed
        );
        let deny_only = policy(&[], &["GPL-3.0-only"]);
        assert_eq!(
            deny_only.evaluate(Some("BSD-3-Clause")),
            LicenseState::Unlisted
        );
        assert!(!deny_only.violates(LicenseState::Unlisted));
    }

    #[test]
    fn or_takes_the_best_branch_and_takes_the_worst() {
        let config = policy(&["MIT"], &["GPL-3.0-only"]);
        assert_eq!(
            config.evaluate(Some("(MIT OR GPL-3.0-only)")),
            LicenseState::Allowed
        );
        assert_eq!(
            config.evaluate(Some("MIT AND GPL-3.0-only")),
            LicenseState::Denied
        );
        assert_eq!(
            config.evaluate(Some("Apache-2.0 OR (MIT AND GPL-3.0-only)")),
            LicenseState::NotAllowed
        );
        assert_eq!(
            config.evaluate(Some("MIT WITH LLVM-exception")),
            LicenseState::Allowed
        );
    }

    #[test]
    fn spdx_expression_syntax_is_recognised() {
        assert!(is_spdx_expression("MIT"));
        assert!(is_spdx_expression("(MIT OR Apache-2.0) AND GPL-2.0+"));
        assert!(is_spdx_expression("Apache-2.0 WITH LLVM-exception"));
        assert!(!is_spdx_expression("SEE LICENSE IN LICENSE.md"));
        assert!(!is_spdx_expression("MIT/X11"));
        assert!(!is_spdx_expression(""));
    }

    #[test]
    fn text_that_is_no_expression_matches_no_list() {
        let config = policy(&["MIT"], &[]);
        assert_eq!(
            config.evaluate(Some("SEE LICENSE IN LICENSE.md")),
            LicenseState::NotAllowed
        );
        assert_eq!(config.evaluate(Some("MIT OR")), LicenseState::NotAllowed);
    }
}
