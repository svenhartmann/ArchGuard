//! `sbom`: third-party components from the lockfiles with licence states (ADR 0016).
use crate::server::ArchguardTools;
use archguard_engine::Sbom;
use archguard_engine::core::LicensesConfig;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

const STATES: [&str; 5] = ["denied", "not_allowed", "unlisted", "allowed", "unknown"];

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SbomParams {
    /// Project directory inside the server root.
    pub path: Option<String>,
    /// Exact licence state: denied, not_allowed, unlisted, allowed or unknown.
    pub license_state: Option<String>,
    /// Only components that fail the [licenses] policy.
    pub violations_only: bool,
    /// Case-insensitive substring of the package name or purl.
    pub query: Option<String>,
    /// First matching component. Follow next_offset from the response.
    pub offset: Option<usize>,
    /// Requested components, 1..=200; the 4 KiB response budget may reduce the page.
    pub limit: Option<usize>,
}

pub fn validate(params: &SbomParams) -> Result<(), ErrorData> {
    if params
        .license_state
        .as_deref()
        .is_some_and(|state| !STATES.contains(&state))
    {
        return Err(crate::tools::invalid_params(
            "license_state must be denied, not_allowed, unlisted, allowed or unknown",
        ));
    }
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    if params
        .query
        .as_ref()
        .is_some_and(|query| query.len() > 512 || query.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "query must be at most 512 bytes without NUL characters",
        ));
    }
    Ok(())
}

fn size(value: &Value) -> usize {
    serde_json::to_vec(value).expect("plain JSON").len()
}

pub fn sbom_value(sbom: &Sbom, policy: &LicensesConfig, params: &SbomParams) -> Value {
    let query = params.query.as_deref().unwrap_or("").trim().to_lowercase();
    let rows: Vec<Value> = sbom
        .components
        .iter()
        .map(|c| serde_json::to_value(c).expect("component serializes"))
        .filter(|c| {
            params
                .license_state
                .as_deref()
                .is_none_or(|state| c["license_state"] == state)
                && (!params.violations_only
                    || serde_json::from_value(c["license_state"].clone())
                        .is_ok_and(|state| policy.violates(state)))
                && (query.is_empty()
                    || c["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
                    || c["purl"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query))
        })
        .collect();
    let total = rows.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(crate::server::DEFAULT_LIMIT);
    let mut reply = json!({
        "components_total": sbom.components.len(), "by_ecosystem": sbom.by_ecosystem,
        "license_states": sbom.license_states, "policy_configured": policy.is_active(),
        "violations": sbom.violations, "manifests_total": sbom.manifests.len(),
        "notes": sbom.notes.iter().take(3).collect::<Vec<_>>(), "notes_total": sbom.notes.len(),
        "total": total, "offset": offset, "returned": 0, "next_offset": null, "truncated": false,
        "components": [],
        "scope": "Offline from the lockfiles/manifests in the repository; licences only where they state them, unknown is never guessed. Full CycloneDX 1.6: archguard sbom <path> --format cyclonedx.",
    });
    let mut returned = 0;
    for row in rows.into_iter().skip(offset).take(limit) {
        reply["components"].as_array_mut().unwrap().push(row);
        if size(&reply) > crate::tools::MAX_RESPONSE_BYTES {
            reply["components"].as_array_mut().unwrap().pop();
            break;
        }
        returned += 1;
    }
    let next = offset.saturating_add(returned);
    reply["returned"] = returned.into();
    reply["next_offset"] = if next < total {
        next.into()
    } else {
        Value::Null
    };
    reply["truncated"] = (next < total).into();
    reply
}

#[tool_router(router = sbom_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Software bill of materials of third-party dependencies, built offline from the lockfiles and manifests in the repository (Cargo.lock, package-lock.json, yarn.lock, pnpm-lock.yaml, packages.lock.json/*.csproj, pom.xml, gradle.lockfile, go.mod, poetry.lock, uv.lock, requirements*.txt, composer.lock). Returns ecosystem counts, licence states against the [licenses] policy (denied, not_allowed, unlisted, allowed, unknown) and paginated components with purl, version, scope, stated licence and the manifests (projects/services) that state them. Licences come only from the files; unknown is never fetched or guessed and is no violation unless [licenses] unknown = \"deny\". Full CycloneDX 1.6: archguard sbom <path> --format cyclonedx. 4 KiB pages."
    )]
    pub(crate) async fn sbom(
        &self,
        Parameters(params): Parameters<SbomParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.path.as_deref())?;
        let loaded = crate::tools::load_config_at(&root)?;
        let policy = &loaded.config.licenses;
        let sbom = archguard_engine::sbom(&root, policy);
        Ok(sbom_value(&sbom, policy, &params).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::sbom;
    use std::path::Path;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sbom-polyglot")
    }

    #[test]
    fn violations_are_filtered_and_pages_stay_in_budget() {
        let policy = LicensesConfig {
            deny: vec!["GPL-3.0-only".into()],
            ..LicensesConfig::default()
        };
        let sbom = sbom(&fixture(), &policy);
        let reply = sbom_value(
            &sbom,
            &policy,
            &SbomParams {
                violations_only: true,
                ..Default::default()
            },
        );
        assert_eq!(reply["total"], 1);
        assert_eq!(reply["components"][0]["purl"], "pkg:npm/gpl-thing@2.0.0");
        assert_eq!(reply["components"][0]["license_state"], "denied");
        assert_eq!(reply["license_states"]["unknown"], 18);

        let all = sbom_value(
            &sbom,
            &policy,
            &SbomParams {
                limit: Some(200),
                ..Default::default()
            },
        );
        assert!(size(&all) <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(all["total"], 24);
        let returned = all["returned"].as_u64().unwrap();
        assert_eq!(all["truncated"], returned < 24);
        assert_eq!(
            all["next_offset"],
            if returned < 24 {
                json!(returned)
            } else {
                Value::Null
            }
        );
    }

    #[test]
    fn unknown_state_filter_and_validation() {
        let policy = LicensesConfig::default();
        let sbom = sbom(&fixture(), &policy);
        let reply = sbom_value(
            &sbom,
            &policy,
            &SbomParams {
                license_state: Some("unknown".into()),
                query: Some("SERDE".into()),
                ..Default::default()
            },
        );
        assert_eq!(reply["total"], 1);
        assert_eq!(reply["components"][0]["license"], Value::Null);
        assert!(
            validate(&SbomParams {
                license_state: Some("bad".into()),
                ..Default::default()
            })
            .is_err()
        );
    }
}
