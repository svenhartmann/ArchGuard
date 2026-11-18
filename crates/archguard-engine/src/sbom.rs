//! SBOM of third-party dependencies and the `[licenses]` gate (ADR 0016).
//!
//! Built offline from the lockfiles and manifests in the repository (the
//! same parse as the dependency inventory). Licences are the ones the
//! lock/manifest states (`package-lock.json`, `composer.lock`); everything
//! else stays unknown — no registry is asked and nothing is guessed.
//! First-party packages (workspace members, path/editable/project
//! references) are not components.
use std::collections::BTreeMap;
use std::path::Path;

use archguard_core::{
    Finding, FindingKind, LicenseState, LicensesConfig, Ref, Severity, is_spdx_expression,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::dep_inventory::{depth_note, manifest_deps};

/// One third-party package, deduplicated by purl across manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SbomComponent {
    pub ecosystem: String,
    pub name: String,
    pub version: Option<String>,
    pub purl: String,
    /// The stated licence as one expression (several stated licences are
    /// joined with `OR`); `None` = unknown.
    pub license: Option<String>,
    pub license_state: LicenseState,
    /// `required`, `optional` (dev/test only) or `None` when not stated.
    pub scope: Option<String>,
    /// Project-relative manifests stating the package: its projects/services.
    pub manifests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SbomManifest {
    pub path: String,
    pub ecosystem: String,
    /// Third-party packages the manifest states.
    pub components: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Sbom {
    pub components: Vec<SbomComponent>,
    pub manifests: Vec<SbomManifest>,
    pub by_ecosystem: BTreeMap<String, usize>,
    pub license_states: BTreeMap<LicenseState, usize>,
    /// Components that fail the `[licenses]` policy.
    pub violations: usize,
    pub notes: Vec<String>,
    /// The manifest search hit its depth bound: deeper lockfiles are missing.
    pub search_limited: bool,
}

/// Percent-encode one purl segment (RFC 3986 unreserved stay as is).
fn encode(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Package URL of a dependency (purl spec types cargo, npm, pypi,
/// composer, maven, nuget, golang).
pub fn purl(ecosystem: &str, name: &str, version: Option<&str>) -> String {
    let name = match ecosystem {
        // purl spec, type pypi: lowercase, `_` becomes `-` (`.` stays).
        "pypi" => name.to_lowercase().replace('_', "-"),
        "maven" => name.replacen(':', "/", 1),
        _ => name.to_string(),
    };
    let path = name.split('/').map(encode).collect::<Vec<_>>().join("/");
    match version {
        Some(version) => format!("pkg:{ecosystem}/{path}@{}", encode(version)),
        None => format!("pkg:{ecosystem}/{path}"),
    }
}

/// One expression for the stated licences; `None` when nothing is stated.
fn license_expression(licenses: &[String]) -> Option<String> {
    let licenses: Vec<&str> = licenses
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    match licenses.as_slice() {
        [] => None,
        [one] => Some(one.to_string()),
        many => Some(
            many.iter()
                .map(|l| {
                    if l.contains(' ') {
                        format!("({l})")
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" OR "),
        ),
    }
}

fn scope_rank(scope: Option<&str>) -> u8 {
    match scope {
        Some("optional") => 0,
        None => 1,
        _ => 2,
    }
}

/// Build the SBOM for `root` and evaluate every component against `policy`.
pub fn sbom(root: &Path, policy: &LicensesConfig) -> Sbom {
    let mut by_purl: BTreeMap<String, SbomComponent> = BTreeMap::new();
    let (manifests, search_limited) = manifest_deps(root);
    let mut result = Sbom {
        search_limited,
        ..Sbom::default()
    };
    if search_limited {
        result.notes.push(depth_note());
    }
    for (ecosystem, path, deps) in manifests {
        let third_party: Vec<_> = deps.into_iter().filter(|d| !d.local).collect();
        if third_party.is_empty() {
            result.notes.push(format!(
                "{path}: no third-party dependencies stated (empty, unparseable or first-party only)"
            ));
        }
        result.manifests.push(SbomManifest {
            path: path.clone(),
            ecosystem: ecosystem.clone(),
            components: third_party.len(),
        });
        for dep in third_party {
            let purl = purl(&ecosystem, &dep.name, dep.version.as_deref());
            let license = license_expression(&dep.licenses);
            let component = by_purl
                .entry(purl.clone())
                .or_insert_with(|| SbomComponent {
                    ecosystem: ecosystem.clone(),
                    name: dep.name.clone(),
                    version: dep.version.clone(),
                    purl,
                    license: None,
                    license_state: LicenseState::Unknown,
                    scope: dep.scope.map(str::to_string),
                    manifests: Vec::new(),
                });
            // The first manifest that states a licence wins.
            if component.license.is_none() {
                component.license = license;
            }
            // Required anywhere beats unknown beats optional everywhere.
            if scope_rank(dep.scope) > scope_rank(component.scope.as_deref()) {
                component.scope = dep.scope.map(str::to_string);
            }
            if !component.manifests.contains(&path) {
                component.manifests.push(path.clone());
            }
        }
    }
    if result.manifests.is_empty() {
        result.notes.push(
            "no supported lockfile or manifest found (cargo, npm, pypi, composer, maven, nuget, golang)"
                .to_string(),
        );
    }
    for mut component in by_purl.into_values() {
        component.license_state = policy.evaluate(component.license.as_deref());
        *result
            .by_ecosystem
            .entry(component.ecosystem.clone())
            .or_default() += 1;
        *result
            .license_states
            .entry(component.license_state)
            .or_default() += 1;
        result.violations += usize::from(policy.violates(component.license_state));
        result.components.push(component);
    }
    result
}

fn finding(id: String, severity: Severity, message: String, evidence: Vec<Ref>) -> Finding {
    Finding {
        kind: FindingKind::License,
        id,
        severity,
        message: Some(message),
        evidence,
        nodes: Vec::new(),
        edge_count: 0,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: Vec::new(),
    }
}

/// The `[licenses]` violations as `check` findings (one error per
/// component), plus one warning when an active policy could not see every
/// lockfile (none found, or the search hit its depth bound), so the gate
/// never passes silently on nothing.
pub fn license_findings(sbom: &Sbom, policy: &LicensesConfig) -> Vec<Finding> {
    if !policy.is_active() {
        return Vec::new();
    }
    let coverage = if sbom.manifests.is_empty() {
        Some(
            "[licenses] is configured but no supported lockfile or manifest was found; no licence was checked."
                .to_string(),
        )
    } else if sbom.search_limited {
        Some(format!("[licenses]: {}.", depth_note()))
    } else {
        None
    };
    let coverage = coverage.map(|message| {
        finding(
            "license-coverage".into(),
            Severity::Warning,
            message,
            Vec::new(),
        )
    });
    sbom.components
        .iter()
        .filter(|c| policy.violates(c.license_state))
        .map(|c| {
            let label = format!(
                "{} {} ({})",
                c.name,
                c.version.as_deref().unwrap_or("(version unknown)"),
                c.ecosystem
            );
            let reason = match c.license_state {
                LicenseState::Denied => "is on the deny list",
                LicenseState::NotAllowed => "is not on the allow list",
                _ => "is not stated in the lockfile/manifest and [licenses] unknown = \"deny\"",
            };
            finding(
                format!("license:{}", c.purl),
                Severity::Error,
                format!(
                    "{label}: licence {} {reason}.",
                    c.license.as_deref().unwrap_or("unknown")
                ),
                c.manifests
                    .iter()
                    .map(|m| Ref {
                        file: m.clone(),
                        line: 0,
                        raw: format!("{m}: {label}"),
                    })
                    .collect(),
            )
        })
        .chain(coverage)
        .collect()
}

fn cyclonedx_licenses(license: &str) -> Value {
    if is_spdx_expression(license) {
        json!([{ "expression": license }])
    } else {
        json!([{ "license": { "name": license } }])
    }
}

/// CycloneDX 1.6 JSON. Deterministic: no serial number or timestamp.
pub fn to_cyclonedx(sbom: &Sbom, project: &str) -> Value {
    let components: Vec<Value> = sbom
        .components
        .iter()
        .map(|c| {
            let mut component = json!({
                "type": "library",
                "bom-ref": c.purl,
                "name": c.name,
                "purl": c.purl,
            });
            if c.ecosystem == "maven"
                && let Some((group, artifact)) = c.name.split_once(':')
            {
                component["group"] = group.into();
                component["name"] = artifact.into();
            }
            if let Some(version) = &c.version {
                component["version"] = version.as_str().into();
            }
            if let Some(scope) = &c.scope {
                component["scope"] = scope.as_str().into();
            }
            if let Some(license) = &c.license {
                component["licenses"] = cyclonedx_licenses(license);
            }
            component["properties"] = c
                .manifests
                .iter()
                .map(|m| json!({ "name": "archguard:manifest", "value": m }))
                .collect();
            component
        })
        .collect();
    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "version": 1,
        "metadata": {
            "tools": { "components": [{
                "type": "application",
                "name": "archguard",
                "version": env!("CARGO_PKG_VERSION"),
            }]},
            "component": { "type": "application", "bom-ref": "project", "name": project },
        },
        "components": components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::UnknownLicense;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sbom-polyglot")
    }

    fn by_purl(sbom: &Sbom) -> BTreeMap<&str, &SbomComponent> {
        sbom.components
            .iter()
            .map(|c| (c.purl.as_str(), c))
            .collect()
    }

    #[test]
    fn purls_follow_the_spec_per_ecosystem() {
        assert_eq!(
            purl("npm", "@types/node", Some("20.11.0")),
            "pkg:npm/%40types/node@20.11.0"
        );
        assert_eq!(
            purl("pypi", "Typing_Extensions", Some("4.10.0")),
            "pkg:pypi/typing-extensions@4.10.0"
        );
        assert_eq!(
            purl("pypi", "zope.interface", Some("6.0")),
            "pkg:pypi/zope.interface@6.0",
            "the purl spec lowercases and replaces `_` only"
        );
        assert_eq!(
            purl("maven", "com.google.guava:guava", Some("33.0.0-jre")),
            "pkg:maven/com.google.guava/guava@33.0.0-jre"
        );
        assert_eq!(
            purl("golang", "github.com/gin-gonic/gin", Some("v1.9.1")),
            "pkg:golang/github.com/gin-gonic/gin@v1.9.1"
        );
        assert_eq!(
            purl("cargo", "x", Some("1.0.0+meta")),
            "pkg:cargo/x@1.0.0%2Bmeta"
        );
        assert_eq!(
            purl("nuget", "Central.Managed", None),
            "pkg:nuget/Central.Managed"
        );
    }

    #[test]
    fn every_supported_lockfile_yields_third_party_components() {
        let sbom = sbom(&fixture(), &LicensesConfig::default());
        let purls: Vec<&str> = sbom.components.iter().map(|c| c.purl.as_str()).collect();
        assert_eq!(
            purls,
            [
                "pkg:cargo/serde@1.0.219",
                "pkg:composer/dual/licensed@1.0.0",
                "pkg:composer/monolog/monolog@3.5.0",
                "pkg:composer/phpunit/phpunit@10.5.0",
                "pkg:golang/github.com/gin-gonic/gin@v1.9.1",
                "pkg:golang/golang.org/x/net@v0.21.0",
                "pkg:maven/com.google.guava/guava@33.0.0-jre",
                "pkg:maven/org.junit.jupiter/junit-jupiter-api@5.10.2",
                "pkg:npm/%40babel/core@7.24.0",
                "pkg:npm/%40types/node@20.11.0",
                "pkg:npm/%40vue/shared@3.4.21",
                "pkg:npm/gpl-thing@2.0.0",
                "pkg:npm/lodash@4.17.21",
                "pkg:npm/no-licence@0.1.0",
                "pkg:npm/react@18.3.1",
                "pkg:npm/vue@3.4.21",
                "pkg:nuget/Central.Managed",
                "pkg:nuget/Dapper@2.1.35",
                "pkg:nuget/Newtonsoft.Json@13.0.3",
                "pkg:nuget/Polly@8.3.0",
                "pkg:nuget/Serilog@3.1.1",
                "pkg:pypi/numpy@1.26.4",
                "pkg:pypi/pytest@8.1.1",
                "pkg:pypi/typing-extensions@4.10.0",
            ],
            "first-party packages (api, etl, shared-ui, Billing.Domain) and superseded manifests are left out"
        );
        assert!(
            !sbom
                .manifests
                .iter()
                .any(|m| m.path.ends_with("composer.json")
                    || m.path.ends_with("pyproject.toml")
                    || m.path.ends_with("Billing.csproj")),
            "{:?}",
            sbom.manifests
        );
    }

    #[test]
    fn licences_and_scopes_come_only_from_the_files() {
        let sbom = sbom(&fixture(), &LicensesConfig::default());
        let c = by_purl(&sbom);
        assert_eq!(c["pkg:npm/react@18.3.1"].license.as_deref(), Some("MIT"));
        assert_eq!(c["pkg:npm/react@18.3.1"].scope.as_deref(), Some("required"));
        assert_eq!(
            c["pkg:npm/%40types/node@20.11.0"].scope.as_deref(),
            Some("optional")
        );
        assert_eq!(
            c["pkg:composer/dual/licensed@1.0.0"].license.as_deref(),
            Some("MIT OR GPL-3.0-only")
        );
        assert_eq!(
            c["pkg:composer/phpunit/phpunit@10.5.0"].scope.as_deref(),
            Some("optional")
        );
        assert_eq!(
            c["pkg:pypi/pytest@8.1.1"].scope.as_deref(),
            Some("optional")
        );
        assert_eq!(
            c["pkg:maven/org.junit.jupiter/junit-jupiter-api@5.10.2"]
                .scope
                .as_deref(),
            Some("optional")
        );
        // Cargo.lock states neither licence nor scope: unknown, not guessed.
        assert_eq!(c["pkg:cargo/serde@1.0.219"].license, None);
        assert_eq!(c["pkg:cargo/serde@1.0.219"].scope, None);
        assert_eq!(
            c["pkg:cargo/serde@1.0.219"].license_state,
            LicenseState::Unknown
        );
        assert_eq!(
            c["pkg:cargo/serde@1.0.219"].manifests,
            ["services/api/Cargo.lock"]
        );
    }

    #[test]
    fn policy_yields_one_error_finding_per_violating_component() {
        let policy = LicensesConfig {
            allow: vec!["MIT".into(), "BSD-3-Clause".into()],
            deny: vec!["GPL-3.0-only".into()],
            unknown: UnknownLicense::Allow,
        };
        let sbom = sbom(&fixture(), &policy);
        let findings = license_findings(&sbom, &policy);
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            ["license:pkg:npm/gpl-thing@2.0.0"],
            "dual licence MIT OR GPL passes"
        );
        assert_eq!(sbom.violations, 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].kind, FindingKind::License);
        assert_eq!(
            findings[0].evidence[0].file,
            "services/web/package-lock.json"
        );
        assert_eq!(sbom.license_states[&LicenseState::Unknown], 18);

        let strict = LicensesConfig {
            unknown: UnknownLicense::Deny,
            ..policy
        };
        let sbom = super::sbom(&fixture(), &strict);
        assert_eq!(license_findings(&sbom, &strict).len(), 19);
    }

    #[test]
    fn cyclonedx_document_has_the_required_shape() {
        let sbom = sbom(&fixture(), &LicensesConfig::default());
        let bom = to_cyclonedx(&sbom, "sbom-polyglot");
        assert_eq!(bom["bomFormat"], "CycloneDX");
        assert_eq!(bom["specVersion"], "1.6");
        let components = bom["components"].as_array().expect("components");
        assert_eq!(components.len(), 24);
        let guava = components
            .iter()
            .find(|c| c["name"] == "guava")
            .expect("guava");
        assert_eq!(guava["group"], "com.google.guava");
        assert_eq!(guava["scope"], "required");
        let dual = components
            .iter()
            .find(|c| c["purl"] == "pkg:composer/dual/licensed@1.0.0")
            .expect("dual");
        assert_eq!(
            dual["licenses"],
            json!([{ "expression": "MIT OR GPL-3.0-only" }])
        );
        let serde = components
            .iter()
            .find(|c| c["name"] == "serde")
            .expect("serde");
        assert!(serde.get("licenses").is_none(), "unknown stays absent");
        assert_eq!(
            cyclonedx_licenses("SEE LICENSE IN LICENSE"),
            json!([{ "license": { "name": "SEE LICENSE IN LICENSE" } }])
        );
    }

    #[test]
    fn an_active_policy_without_any_lockfile_warns_instead_of_passing_silently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let policy = LicensesConfig {
            deny: vec!["GPL-3.0-only".into()],
            ..LicensesConfig::default()
        };
        let findings = license_findings(&sbom(dir.path(), &policy), &policy);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].severity, Severity::Warning);
        assert_eq!(findings[0].id, "license-coverage");

        let deep = dir.path().join(["d"; 13].join("/"));
        std::fs::create_dir_all(&deep).expect("mkdir");
        std::fs::write(
            deep.join("package-lock.json"),
            r#"{"packages":{"node_modules/x":{"version":"1.0.0","license":"MIT"}}}"#,
        )
        .expect("lock");
        std::fs::write(dir.path().join("package-lock.json"), "{}").expect("lock");
        let result = sbom(dir.path(), &policy);
        assert!(result.search_limited);
        let findings = license_findings(&result, &policy);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0]
                .message
                .as_deref()
                .is_some_and(|m| m.contains("deeper than")),
            "{findings:?}"
        );
        assert!(
            license_findings(&result, &LicensesConfig::default()).is_empty(),
            "no policy, no gate"
        );
    }

    #[test]
    fn empty_project_has_a_coverage_note() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sbom = sbom(dir.path(), &LicensesConfig::default());
        assert!(sbom.components.is_empty());
        assert!(
            sbom.notes[0].contains("no supported lockfile"),
            "{:?}",
            sbom.notes
        );
    }
}
