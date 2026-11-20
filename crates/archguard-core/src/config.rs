//! The parsed `archguard.toml` root that composes the sections of every feature.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::architecture::ArchitectureConfig;
use crate::calls::SemanticConfig;
use crate::diagrams::Arc42Config;
use crate::licenses::LicensesConfig;
use crate::performance::PerformanceConfig;
use crate::review::ReviewConfig;
use crate::rules::{DependencyRule, OwnershipRule};
use crate::runtime::RuntimeConfig;
use crate::settings::{LayerConfig, ModulesConfig, ProjectConfig, RulesConfig, StoreConfig};

// ------------------------------------------------------- configuration (Kap. 11)

/// Parsed `archguard.toml` (Kap. 11).
///
/// Every section and every field is optional (`#[serde(default)]`
/// throughout); [`Default`] yields exactly the defaults documented in
/// Kap. 11, so an empty file parses to the default configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Top-level switch of the layer derivation (Kap. 11); `false` keeps
    /// only the layers written in `archguard.toml`.
    pub derive_layers: bool,
    pub project: ProjectConfig,
    pub modules: ModulesConfig,
    /// The `[[layers]]` entries in file order; no layers by default.
    pub layers: Vec<LayerConfig>,
    pub dependency_rules: Vec<DependencyRule>,
    pub ownership_rules: Vec<OwnershipRule>,
    pub rules: RulesConfig,
    pub review: ReviewConfig,
    pub performance: PerformanceConfig,
    /// `[runtime]`: thresholds of the runtime findings (ADR 0004).
    pub runtime: RuntimeConfig,
    pub architecture: ArchitectureConfig,
    pub semantic: SemanticConfig,
    pub store: StoreConfig,
    /// `[arc42]`: which catalogue diagrams the document shows (ADR 0006).
    pub arc42: Arc42Config,
    /// `[licenses]`: licence policy of third-party dependencies (ADR 0016).
    pub licenses: LicensesConfig,
    /// `[views]`: conventions `check` enforces on the UI-flow and domain
    /// views (ADR 0021 and 0022, decision 6); none by default.
    pub views: ViewsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            derive_layers: true,
            project: ProjectConfig::default(),
            modules: ModulesConfig::default(),
            layers: Vec::new(),
            dependency_rules: Vec::new(),
            ownership_rules: Vec::new(),
            rules: RulesConfig::default(),
            review: ReviewConfig::default(),
            performance: PerformanceConfig::default(),
            runtime: RuntimeConfig::default(),
            architecture: ArchitectureConfig::default(),
            semantic: SemanticConfig::default(),
            store: StoreConfig::default(),
            arc42: Arc42Config::default(),
            licenses: LicensesConfig::default(),
            views: ViewsConfig::default(),
        }
    }
}

/// `[views]`: opt-in conventions; an empty list checks nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewsConfig {
    pub conventions: Vec<ViewConvention>,
    /// `[views.run.<application>]`: where the locally running application
    /// answers and who signs in per role (ADR 0021, decision 7).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub run: BTreeMap<String, RunTarget>,
    /// `[views.access]`: what a project-specific guard, guard function or
    /// access attribute means, by its name (`mustBeAuthenticatedGuard =
    /// "authenticated"`, `ApiPermission = "policy:$arg"`). Extractors map a
    /// listed name to this meaning instead of an opaque custom condition and
    /// never guess from a name that is not listed (ADR 0021, decision 6).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub access: BTreeMap<String, AccessMeaning>,
}

/// The meaning of one `[views.access]` entry. Written as `open`,
/// `authenticated`, `never`, `role:<name>`, `policy:<name>`, or
/// `role:$arg` / `policy:$arg` for the literal arguments at the use site
/// (several arguments mean any of them). A `$arg` form may name a fallback
/// for use sites without arguments: `policy:$arg|authenticated`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum AccessMeaning {
    Open,
    Authenticated,
    Never,
    Role(String),
    Policy(String),
    RoleFromArguments,
    PolicyFromArguments,
    /// `<$arg form>|<fallback>`: the first with arguments, the second (never
    /// a `$arg` form) at a use site without arguments.
    FromArgumentsOr(Box<AccessMeaning>, Box<AccessMeaning>),
}

impl TryFrom<String> for AccessMeaning {
    type Error = String;
    fn try_from(text: String) -> Result<Self, String> {
        if let Some((with, without)) = text.split_once('|') {
            let with = Self::single(with)?;
            let without = Self::single(without)?;
            return match (&with, &without) {
                (Self::RoleFromArguments | Self::PolicyFromArguments, w)
                    if !matches!(w, Self::RoleFromArguments | Self::PolicyFromArguments) =>
                {
                    Ok(Self::FromArgumentsOr(Box::new(with), Box::new(without)))
                }
                _ => Err(format!(
                    "`{text}`: a fallback (`|…`) follows only role:$arg or policy:$arg \
                     and is itself no $arg form"
                )),
            };
        }
        Self::single(&text)
    }
}

impl AccessMeaning {
    /// The meaning at one use site: a fallback form resolved by whether the
    /// site passes arguments; every other meaning as it is.
    pub fn at_use_site(&self, has_arguments: bool) -> &AccessMeaning {
        match self {
            Self::FromArgumentsOr(with, _) if has_arguments => with,
            Self::FromArgumentsOr(_, without) => without,
            other => other,
        }
    }

    /// One meaning without a fallback.
    fn single(text: &str) -> Result<Self, String> {
        let named =
            |rest: &str, from_args: AccessMeaning, fixed: fn(String) -> AccessMeaning| match rest {
                "$arg" => Ok(from_args),
                "" => Err(format!("`{text}` names no role or policy")),
                name => Ok(fixed(name.to_string())),
            };
        match text {
            "open" => Ok(Self::Open),
            "authenticated" => Ok(Self::Authenticated),
            "never" => Ok(Self::Never),
            t => match t.split_once(':') {
                Some(("role", rest)) => named(rest, Self::RoleFromArguments, Self::Role),
                Some(("policy", rest)) => named(rest, Self::PolicyFromArguments, Self::Policy),
                _ => Err(format!(
                    "unknown access meaning `{text}`; expected open, authenticated, never, \
                     role:<name>, policy:<name>, role:$arg or policy:$arg"
                )),
            },
        }
    }
}

impl From<AccessMeaning> for String {
    fn from(meaning: AccessMeaning) -> String {
        match meaning {
            AccessMeaning::Open => "open".into(),
            AccessMeaning::Authenticated => "authenticated".into(),
            AccessMeaning::Never => "never".into(),
            AccessMeaning::Role(n) => format!("role:{n}"),
            AccessMeaning::Policy(n) => format!("policy:{n}"),
            AccessMeaning::RoleFromArguments => "role:$arg".into(),
            AccessMeaning::PolicyFromArguments => "policy:$arg".into(),
            AccessMeaning::FromArgumentsOr(with, without) => {
                format!("{}|{}", String::from(*with), String::from(*without))
            }
        }
    }
}

/// One locally running application of the UI flow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunTarget {
    /// Base URL such as `https://localhost:7298`; the screen route is appended.
    pub base_url: Option<String>,
    /// Route of the sign-in form the replay fills; the start screen when absent.
    pub login_path: Option<String>,
    /// Test user per role name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub users: BTreeMap<String, RunUser>,
}

/// A test user; the password comes from the environment or the keychain,
/// never from the configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawRunUser")]
pub struct RunUser {
    pub login: String,
    /// Environment variable that holds the password.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    /// Keychain service name (macOS `security find-generic-password -s`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_keychain: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRunUser {
    login: String,
    password_env: Option<String>,
    password_keychain: Option<String>,
    /// Only read to reject it with a clear message.
    password: Option<serde::de::IgnoredAny>,
}

impl TryFrom<RawRunUser> for RunUser {
    type Error = String;

    fn try_from(raw: RawRunUser) -> Result<Self, String> {
        if raw.password.is_some() {
            return Err("a password must not be written into the configuration; \
                 set password_env or password_keychain"
                .into());
        }
        if raw.password_env.is_some() == raw.password_keychain.is_some() {
            return Err("set exactly one of password_env and password_keychain".into());
        }
        Ok(Self {
            login: raw.login,
            password_env: raw.password_env,
            password_keychain: raw.password_keychain,
        })
    }
}

/// One entry of `[views] conventions`; an unknown name is a config error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewConvention {
    NavigationLiteral,
    EndpointAccessDeclared,
    HttpUrlLiteral,
    FrontendBackendAgree,
    StateChangesInEntity,
    EventsThroughRaiseFunction,
}

impl ViewConvention {
    /// The configuration name; also the rule part of a finding id. The engine
    /// matches on it because `archguard_core` re-exports only [`Config`].
    pub fn name(self) -> &'static str {
        match self {
            Self::NavigationLiteral => "navigation_literal",
            Self::EndpointAccessDeclared => "endpoint_access_declared",
            Self::HttpUrlLiteral => "http_url_literal",
            Self::FrontendBackendAgree => "frontend_backend_agree",
            Self::StateChangesInEntity => "state_changes_in_entity",
            Self::EventsThroughRaiseFunction => "events_through_raise_function",
        }
    }
}

#[cfg(test)]
mod access_tests {
    use super::{AccessMeaning, Config};

    #[test]
    fn views_access_reads_every_meaning_and_rejects_unknown_ones() {
        let config: Config = toml::from_str(
            "[views.access]\nauthGuard = \"authenticated\"\npublicGuard = \"open\"\n\
             DenyAll = \"never\"\nAdminGuard = \"role:Admin\"\nCanPurge = \"policy:CanPurge\"\n\
             ApiPermission = \"policy:$arg\"\nRequireRole = \"role:$arg\"\n",
        )
        .unwrap();
        let access = &config.views.access;
        assert_eq!(access["authGuard"], AccessMeaning::Authenticated);
        assert_eq!(access["publicGuard"], AccessMeaning::Open);
        assert_eq!(access["DenyAll"], AccessMeaning::Never);
        assert_eq!(access["AdminGuard"], AccessMeaning::Role("Admin".into()));
        assert_eq!(access["CanPurge"], AccessMeaning::Policy("CanPurge".into()));
        assert_eq!(access["ApiPermission"], AccessMeaning::PolicyFromArguments);
        assert_eq!(access["RequireRole"], AccessMeaning::RoleFromArguments);
        let text = toml::to_string(&config.views).unwrap();
        assert!(text.contains("ApiPermission = \"policy:$arg\""), "{text}");
        for bad in ["\"admin\"", "\"role:\"", "\"group:x\""] {
            let err =
                toml::from_str::<Config>(&format!("[views.access]\nx = {bad}\n")).unwrap_err();
            assert!(err.to_string().contains("x"), "{bad}: {err}");
        }
    }

    #[test]
    fn views_access_reads_a_fallback_for_use_sites_without_arguments() {
        let config: Config = toml::from_str(
            "[views.access]\nApiPermission = \"policy:$arg|authenticated\"\n\
             ApiPermissionOrAnonymous = \"policy:$arg|open\"\nHasRole = \"role:$arg|role:User\"\n",
        )
        .unwrap();
        let access = &config.views.access;
        let permission = &access["ApiPermission"];
        assert_eq!(
            permission,
            &AccessMeaning::FromArgumentsOr(
                Box::new(AccessMeaning::PolicyFromArguments),
                Box::new(AccessMeaning::Authenticated)
            )
        );
        assert_eq!(
            permission.at_use_site(true),
            &AccessMeaning::PolicyFromArguments
        );
        assert_eq!(permission.at_use_site(false), &AccessMeaning::Authenticated);
        assert_eq!(
            access["ApiPermissionOrAnonymous"].at_use_site(false),
            &AccessMeaning::Open
        );
        assert_eq!(
            access["HasRole"].at_use_site(false),
            &AccessMeaning::Role("User".into())
        );
        assert_eq!(AccessMeaning::Open.at_use_site(false), &AccessMeaning::Open);
        // Serde round trip.
        let text = toml::to_string(&config.views).unwrap();
        assert!(
            text.contains("ApiPermission = \"policy:$arg|authenticated\""),
            "{text}"
        );
        assert_eq!(
            toml::from_str::<super::ViewsConfig>(&text).unwrap(),
            config.views
        );
        // A fallback follows a `$arg` form only, and is none itself.
        for bad in [
            "\"authenticated|open\"",
            "\"policy:X|open\"",
            "\"policy:$arg|role:$arg\"",
            "\"policy:$arg|maybe\"",
            "\"policy:$arg|open|never\"",
        ] {
            let err =
                toml::from_str::<Config>(&format!("[views.access]\nx = {bad}\n")).unwrap_err();
            assert!(err.to_string().contains("x"), "{bad}: {err}");
        }
    }
}
