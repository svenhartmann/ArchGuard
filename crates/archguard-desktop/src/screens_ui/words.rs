//! Conditions, calls and access states in the window's language.

use archguard_engine::core::{AccessState, Action, Call, Condition, Outcome};

use crate::tab_support::Texts;

/// `signed in and (role A or policy P) and not (custom x)`.
pub fn condition(texts: &Texts, c: &Condition) -> String {
    let join = |items: &[Condition], word: &str| {
        let parts: Vec<String> = (items.iter())
            .map(|c| match c {
                Condition::All(_) | Condition::Any(_) => format!("({})", condition(texts, c)),
                _ => condition(texts, c),
            })
            .collect();
        parts.join(texts.t(word))
    };
    let named = |key: &str, name: &str| texts.f(key, &[("name", name.to_string())]);
    match c {
        Condition::All(items) => join(items, " and "),
        Condition::Any(items) => join(items, " or "),
        Condition::Not(inner) => named("not ({name})", &condition(texts, inner)),
        Condition::Authenticated => texts.t("signed in").to_string(),
        Condition::Never => texts.t("never").to_string(),
        Condition::Role(name) => named("role {name}", name),
        Condition::Policy(name) => named("policy {name}", name),
        Condition::Custom(name) => named("custom {name}", name),
        Condition::Unknown(text) => named("unknown ({name})", text),
    }
}

/// A screen's guard in words; none reads "none".
pub fn access(texts: &Texts, c: Option<&Condition>) -> String {
    c.map_or_else(|| texts.t("none").to_string(), |c| condition(texts, c))
}

/// An endpoint's access: its condition, "open" or "undeclared".
pub fn endpoint_access(texts: &Texts, a: &AccessState) -> String {
    match a {
        AccessState::Protected(c) => condition(texts, c),
        AccessState::Open => texts.t("open (declared anonymous)").to_string(),
        AccessState::Undeclared => texts.t("undeclared").to_string(),
    }
}

pub fn outcome(texts: &Texts, o: Outcome) -> String {
    texts
        .t(match o {
            Outcome::Allowed => "allowed",
            Outcome::Denied => "denied",
            Outcome::Unknown => "unknown",
        })
        .to_string()
}

/// `GET /api/orders`, a method id, or the unread source text.
pub fn call(a: &Action) -> String {
    match &a.call {
        Call::Http { method, path } => format!("{method} {path}"),
        Call::Method(id) | Call::Unresolved(id) => id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_read_as_words_in_both_languages() {
        let c = Condition::All(vec![
            Condition::Authenticated,
            Condition::Any(vec![
                Condition::Role("A".into()),
                Condition::Policy("P".into()),
            ]),
            Condition::Not(Box::new(Condition::Custom("x".into()))),
        ]);
        assert_eq!(
            condition(&Texts::default(), &c),
            "signed in and (role A or policy P) and not (custom x)"
        );
        assert_eq!(
            condition(&Texts::german(), &c),
            "angemeldet und (Rolle A oder Policy P) und nicht (Bedingung x)"
        );
        assert_eq!(access(&Texts::german(), None), "keine");
        assert_eq!(outcome(&Texts::german(), Outcome::Denied), "verweigert");
    }
}
