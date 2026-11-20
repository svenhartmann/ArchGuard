//! Import of `.archguard/ui-flow.json` (ADR 0021): the way in for frameworks
//! without an extractor.
use std::path::Path;

use crate::extract_ctx::{ExtractContext, found_root_config, gap, with_root_config};

use archguard_core::{
    ArchguardError, UI_FLOW_DEFINITION, UI_FLOW_IMPORT_SOURCE, UiFlow, merge_ui_flows,
};

pub const UI_FLOW_FILE: &str = ".archguard/ui-flow.json";
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The imported flow with every entry's source set to `import`; `Ok(None)`
/// without the file. An unreadable, oversized or invalid file is an error.
pub fn read_ui_flow(root: &Path) -> Result<Option<UiFlow>, ArchguardError> {
    let bytes = match crate::project_files::read_repo_file(root, Path::new(UI_FLOW_FILE), MAX_BYTES)
    {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ArchguardError::Analysis(format!(
                "Cannot read {UI_FLOW_FILE}: {e}"
            )));
        }
    };
    let mut flow: UiFlow = serde_json::from_slice(&bytes)
        .map_err(|e| ArchguardError::Analysis(format!("Invalid {UI_FLOW_FILE}: {e}")))?;
    if flow.definition != UI_FLOW_DEFINITION {
        return Err(ArchguardError::Analysis(format!(
            "{UI_FLOW_FILE}: definition {:?}, expected {UI_FLOW_DEFINITION:?}",
            flow.definition
        )));
    }
    let import = || UI_FLOW_IMPORT_SOURCE.to_string();
    flow.screens.iter_mut().for_each(|e| e.source = import());
    flow.transitions
        .iter_mut()
        .for_each(|e| e.source = import());
    flow.actions.iter_mut().for_each(|e| e.source = import());
    flow.endpoints.iter_mut().for_each(|e| e.source = import());
    flow.roles.iter_mut().for_each(|e| e.source = import());
    flow.policies.iter_mut().for_each(|e| e.source = import());
    Ok(Some(flow))
}

/// The flow below `root` with the root's configuration: every extractor
/// merged with the import file. An invalid import file is an error;
/// extractor problems stay diagnostics.
pub fn analyze(root: &Path) -> Result<UiFlow, ArchguardError> {
    with_root_config(root, analyze_with)
}

/// [`analyze`] for one run, e.g. with the configuration given by `--config`.
pub fn analyze_with(ctx: &ExtractContext) -> Result<UiFlow, ArchguardError> {
    let import = read_ui_flow(ctx.root)?.unwrap_or_default();
    let mut flow = merge_ui_flows(import, extract_all(ctx));
    for configured in &ctx.config.views.backends {
        add_backend(&mut flow, ctx, configured);
    }
    Ok(flow)
}

/// `[views] backends` (ADR 0021): runs the extractors on the backend
/// directory `configured` (relative to the root, or absolute; it may leave
/// the root because the user wrote it, nothing else may) and adds ONLY its
/// endpoints, so frontend actions join them like local ones. Application
/// and file are prefixed with `configured`, which marks them as external
/// and keeps equally named applications of both sides apart. A path that
/// is no directory or yields no endpoint is a gap, never silent.
///
/// Access rule: the backend's own `[views.access]` comes from ITS
/// `archguard.toml` when it has one, else from the frontend's configuration.
/// A backend's own `backends` are not followed.
fn add_backend(flow: &mut UiFlow, ctx: &ExtractContext, configured: &str) {
    let mut fail = |message: String| {
        gap(&mut flow.diagnostics, "archguard.toml", 0, message);
        flow.complete = false;
    };
    let dir = ctx.root.join(configured);
    if !dir.is_dir() {
        return fail(format!(
            "views.backends `{configured}`: not an existing directory; its endpoints are unknown."
        ));
    }
    let own = match found_root_config(&dir) {
        Ok(own) => own,
        Err(e) => {
            return fail(format!(
                "views.backends `{configured}`: configuration not read ({e}); \
                 the frontend's access rules apply."
            ));
        }
    };
    let backend = extract_all(&ExtractContext::new(
        &dir,
        own.as_ref().unwrap_or(ctx.config),
    ));
    if backend.endpoints.is_empty() {
        return fail(format!(
            "views.backends `{configured}`: no HTTP endpoints found."
        ));
    }
    let name = configured.trim_end_matches('/');
    for mut e in backend.endpoints {
        e.application = Some(match e.application {
            Some(app) => format!("{name}/{app}"),
            None => name.to_string(),
        });
        e.file = format!("{name}/{}", e.file);
        flow.endpoints.push(e);
    }
    for mut site in backend.handler_sites {
        site.endpoint_file = format!("{name}/{}", site.endpoint_file);
        flow.handler_sites.push(site);
    }
    flow.endpoints.sort();
    flow.endpoints.dedup();
    flow.handler_sites.sort();
    flow.handler_sites.dedup();
}

fn extract_all(ctx: &ExtractContext) -> UiFlow {
    let extractors = [
        crate::ui_flow_blazor::extract_with,
        crate::ui_flow_angular::extract_with,
        crate::ui_flow_backend::extract_with,
        crate::ui_flow_react::extract_with,
        crate::ui_flow_xaml::extract_with,
        crate::ui_flow_vue::extract_with,
        crate::ui_flow_slint::extract_with,
        crate::ui_flow_nestjs::extract_with,
        crate::ui_flow_svelte::extract_with,
        crate::ui_flow_django::extract_with,
        crate::ui_flow_fastapi::extract_with,
        crate::ui_flow_laravel::extract_with,
        crate::ui_flow_rust::extract_with,
        crate::ui_flow_roles::extract_with,
    ];
    (extractors.into_iter()).fold(UiFlow::default(), |flow, extract| {
        merge_ui_flows(flow, extract(ctx))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Call, Condition, Target};

    fn write(dir: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    const BLAZOR_HOME: &str = "@page \"/\"\n<a href=\"orders\">Orders</a>\n";
    const BLAZOR_ORDERS: &str = "@page \"/orders\"\n@attribute [Authorize]\n";
    const API: &str = r#"
[ApiController]
[Route("api/orders")]
[Authorize(Roles = "Clerk")]
public class OrdersController : ControllerBase
{
    [HttpGet]
    public IActionResult List() => Ok();
}
"#;

    #[test]
    fn analyze_merges_extractors_and_import() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                ("Web/Web.csproj", "<Project />"),
                ("Web/Pages/Home.razor", BLAZOR_HOME),
                ("Web/Pages/Orders.razor", BLAZOR_ORDERS),
                ("Api/Api.csproj", "<Project />"),
                ("Api/OrdersController.cs", API),
                (
                    UI_FLOW_FILE,
                    r#"{"definition":"ui-flow/v1","complete":true,
                    "actions":[{"screen":{"screen":"Web:Pages/Orders.razor"},"trigger":"refresh",
                      "call":{"http":{"method":"GET","path":"/api/orders"}},"file":"x.json","line":1,"source":"x"}]}"#,
                ),
            ],
        );
        let flow = analyze(dir.path()).unwrap();
        let sources: std::collections::BTreeSet<&str> = flow
            .screens
            .iter()
            .map(|s| s.source.as_str())
            .chain(flow.actions.iter().map(|a| a.source.as_str()))
            .chain(flow.endpoints.iter().map(|e| e.source.as_str()))
            .collect();
        assert!(
            sources.contains("blazor") && sources.contains("import"),
            "{sources:?}"
        );
        assert_eq!(flow.screens.len(), 2, "{:#?}", flow.screens);
        assert!(matches!(flow.transitions[0].target, Target::Screen(_)));
        assert_eq!(
            flow.endpoints[0].access,
            AccessState::Protected(Condition::Role("Clerk".into()))
        );
        assert!(matches!(flow.actions[0].call, Call::Http { .. }));
        assert!(flow.complete, "{:#?}", flow.diagnostics);
        // The imported action meets the extracted endpoint in the findings.
        let findings = archguard_core::ui_flow_findings(&flow);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(
            findings[0].kind,
            archguard_core::UiFlowFindingKind::ShownButRefused
        );
    }

    #[test]
    fn analyze_resolves_policies_from_the_role_catalog() {
        use archguard_core::{Assignment, ScreenReach, screen_reaches};
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                ("Web/Web.csproj", "<Project />"),
                (
                    "Web/Pages/Home.razor",
                    "@page \"/\"\n<a href=\"purge\">Purge</a>\n",
                ),
                (
                    "Web/Pages/Purge.razor",
                    "@page \"/purge\"\n@attribute [Authorize(Policy = \"CanPurge\")]\n",
                ),
                (
                    "Web/Program.cs",
                    "builder.Services.AddAuthorizationBuilder()\n    .AddPolicy(\"CanPurge\", p => p.RequireRole(\"Administrator\"));\n",
                ),
            ],
        );
        let flow = analyze(dir.path()).unwrap();
        assert_eq!(flow.policies.len(), 1, "{:#?}", flow.policies);
        let purge = |roles: &[&str]| {
            let a = Assignment {
                authenticated: true,
                roles: roles.iter().map(|r| r.to_string()).collect(),
                ..Assignment::default()
            };
            screen_reaches(&flow, &a)["Web:Pages/Purge.razor"]
        };
        assert_eq!(purge(&["Administrator"]), ScreenReach::Sure);
        assert_eq!(purge(&[]), ScreenReach::Denied);
    }

    #[test]
    fn analyze_fails_on_an_invalid_import_only() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[("A.razor", "@page \"/\"\n<a href=\"@X\">x</a>\n")],
        );
        let flow = analyze(dir.path()).unwrap();
        assert!(!flow.complete && !flow.diagnostics.is_empty());
        write(dir.path(), &[(UI_FLOW_FILE, "{not json")]);
        let err = analyze(dir.path()).unwrap_err();
        assert!(err.to_string().contains(UI_FLOW_FILE), "{err}");
    }

    const FRONT_CALL: &str = r#"{"definition":"ui-flow/v1","complete":true,
        "actions":[{"screen":{"screen":"Web:Pages/Orders.razor"},"call":{"http":{"method":"GET","path":"/api/orders"}},
        "file":"x.json","line":1,"source":"x"}]}"#;
    const PERMISSION_API: &str = "[ApiController]\n[Route(\"api/orders\")]\n\
        [ApiPermission(\"orders.read\")]\npublic class OrdersController : ControllerBase\n\
        {\n    [HttpGet]\n    public IActionResult List() => Ok();\n}\n";

    /// `front/` calls `/api/orders`; `back/` serves it; `front` lists `back`.
    fn two_repos(front_toml: &str, back_files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                ("front/Web/Web.csproj", "<Project />"),
                ("front/Web/Pages/Orders.razor", BLAZOR_ORDERS),
                ("front/archguard.toml", front_toml),
                ("front/.archguard/ui-flow.json", FRONT_CALL),
                ("back/Api/Api.csproj", "<Project />"),
            ],
        );
        write(&dir.path().join("back"), back_files);
        dir
    }

    fn joined(flow: &UiFlow) -> usize {
        archguard_core::join_actions_to_endpoints(flow)
            .iter()
            .filter(|(_, hits)| !hits.is_empty())
            .count()
    }

    #[test]
    fn backends_join_only_when_configured() {
        let back = [("Api/OrdersController.cs", API)];
        let without = two_repos("", &back);
        let flow = analyze(&without.path().join("front")).unwrap();
        assert_eq!(joined(&flow), 0);
        let with = two_repos("[views]\nbackends = [\"../back\"]\n", &back);
        let flow = analyze(&with.path().join("front")).unwrap();
        assert_eq!(joined(&flow), 1, "{:#?}", flow.endpoints);
        let e = &flow.endpoints[0];
        assert_eq!(e.application.as_deref(), Some("../back/Api"));
        assert_eq!(e.file, "../back/Api/OrdersController.cs");
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn a_missing_or_empty_backend_is_a_gap() {
        let dir = two_repos(
            "[views]\nbackends = [\"../nope\", \"../back\", \"web/Pages/Orders.razor\"]\n",
            &[("README.md", "x")],
        );
        let flow = analyze(&dir.path().join("front")).unwrap();
        let messages: Vec<&str> = flow
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(!flow.complete);
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`../nope`: not an existing")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`../back`: no HTTP endpoints")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("Orders.razor`: not an existing")),
            "{messages:?}"
        );
        assert!(flow.endpoints.is_empty());
    }

    #[test]
    fn same_named_applications_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                ("front/Api/Api.csproj", "<Project />"),
                ("front/Api/OrdersController.cs", API),
                (
                    "front/archguard.toml",
                    "[views]\nbackends = [\"../back\"]\n",
                ),
                ("back/Api/Api.csproj", "<Project />"),
                ("back/Api/OrdersController.cs", API),
            ],
        );
        let flow = analyze(&dir.path().join("front")).unwrap();
        let apps: Vec<_> = flow
            .endpoints
            .iter()
            .map(|e| e.application.as_deref())
            .collect();
        assert_eq!(
            apps,
            [Some("../back/Api"), Some("Api")],
            "{:#?}",
            flow.endpoints
        );
    }

    #[test]
    fn backend_access_comes_from_its_own_config_else_the_frontends() {
        let mapped = AccessState::Protected(Condition::Policy("orders.read".into()));
        let front = |meaning: &str| {
            format!(
                "[views]\nbackends = [\"../back\"]\n[views.access]\nApiPermission = \"{meaning}\"\n"
            )
        };
        // Its own configuration wins over the frontend's.
        let own = "[views.access]\nApiPermission = \"policy:$arg\"\n";
        let dir = two_repos(
            &front("open"),
            &[("Api/O.cs", PERMISSION_API), ("archguard.toml", own)],
        );
        let flow = analyze(&dir.path().join("front")).unwrap();
        assert_eq!(flow.endpoints[0].access, mapped, "{:#?}", flow.diagnostics);
        // Without one, the frontend's applies.
        let dir = two_repos(&front("policy:$arg"), &[("Api/O.cs", PERMISSION_API)]);
        let flow = analyze(&dir.path().join("front")).unwrap();
        assert_eq!(flow.endpoints[0].access, mapped, "{:#?}", flow.diagnostics);
    }

    fn project(json: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if let Some(json) = json {
            std::fs::create_dir(dir.path().join(".archguard")).unwrap();
            std::fs::write(dir.path().join(UI_FLOW_FILE), json).unwrap();
        }
        dir
    }

    #[test]
    fn valid_import_forces_source_import() {
        let dir = project(Some(
            r#"{"definition":"ui-flow/v1","complete":false,
            "screens":[{"id":"home","name":"Home","kind":"page","route":"/","file":"a.ts","line":1,"source":"angular"}],
            "transitions":[{"from":{"any_screen":{"application":"web"}},"target":{"unresolved":"url"},"file":"a.ts","line":2,"source":"x"}],
            "actions":[{"screen":{"screen":"home"},"call":{"http":{"method":"GET","path":"/api"}},"file":"a.ts","line":3,"source":"x"}],
            "endpoints":[{"method":"GET","path":"/api","access":"undeclared","file":"A.cs","line":4,"source":"x"}]}"#,
        ));
        let flow = read_ui_flow(dir.path()).unwrap().unwrap();
        assert!(!flow.complete);
        assert_eq!(flow.screens[0].source, "import");
        assert_eq!(flow.transitions[0].source, "import");
        assert_eq!(flow.actions[0].source, "import");
        assert_eq!(flow.endpoints[0].source, "import");
    }

    #[test]
    fn missing_file_is_none() {
        assert_eq!(read_ui_flow(project(None).path()).unwrap(), None);
    }

    #[test]
    fn invalid_json_or_definition_is_an_error() {
        for json in [
            "{not json",
            r#"{"definition":"ui-flow/v2","complete":true}"#,
        ] {
            let err = read_ui_flow(project(Some(json)).path()).unwrap_err();
            assert!(err.to_string().contains(UI_FLOW_FILE), "{err}");
        }
    }
}
