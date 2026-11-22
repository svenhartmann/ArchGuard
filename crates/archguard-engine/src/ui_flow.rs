//! Import of `.archguard/ui-flow.json` (ADR 0021): the way in for frameworks
//! without an extractor.
use std::path::Path;

use crate::extract_ctx::{
    ExtractContext, config_file_in, found_root_config, gap, with_root_config,
};

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
    let own = absolute(ctx.root, ".");
    let mut seen = std::collections::BTreeSet::new();
    for configured in &ctx.config.views.backends {
        let name = normalise(configured);
        let same = absolute(ctx.root, &name);
        if seen.insert(same.clone()) {
            add_backend(&mut flow, ctx, &name, same == own);
        }
    }
    Ok(flow)
}

/// `[views] backends` (ADR 0021): runs the extractors on the backend
/// directory `name` (relative to the root, or absolute; it may leave the root
/// because the user wrote it, nothing else may) and adds ONLY its endpoints,
/// so frontend actions join them like local ones. Application and file are
/// prefixed with the configured path, which marks them as external and keeps
/// equally named applications of both sides apart. A path that is no
/// directory or yields no endpoint is a gap, never silent; the backend's own
/// gaps and incompleteness are carried over (files prefixed).
///
/// Access rule: the backend's own `[views.access]` comes from ITS
/// `archguard.toml` when it has one, else from the frontend's configuration;
/// a backend configuration that does not parse is a gap and the frontend's
/// rules apply. A backend's own `backends` are not followed.
///
/// Paths are normalised lexically (see [`normalise`]); ones that name the same
/// directory (compared as normalised absolute forms, so an absolute and a
/// relative spelling too) are read once, and one naming the project itself is a
/// gap. A backend that lies inside the project also yields endpoints the local
/// extractors found; they stay (prefixed, so ids differ) and are not skipped.
/// With `ctx.allowed_root` set (the MCP server root) a backend outside it, or
/// reached through a symlink, is not read at all, with one message that does
/// not depend on the target.
///
/// IDs, notes and baselines built from these elements contain the configured
/// path: relative paths keep them portable between machines, absolute paths
/// make them machine-specific. Foreign endpoints are part of the flow, so they
/// can make `endpoint_access_declared` of the frontend's `check` fail; that is
/// intended.
fn add_backend(flow: &mut UiFlow, ctx: &ExtractContext, name: &str, itself: bool) {
    let config_file =
        (ctx.config_file.clone()).unwrap_or_else(|| config_file_in(ctx.root).to_string());
    let mut fail = |message: String| {
        gap(&mut flow.diagnostics, &config_file, 0, message);
        flow.complete = false;
    };
    let dir = ctx.root.join(name);
    if let Some(why) = not_read(ctx, &dir, itself) {
        return fail(format!("views.backends `{name}`: {why}"));
    }
    let prefixed = |file: &str| prefix(name, file);
    let own = match found_root_config(&dir) {
        Ok(own) => own,
        Err((rel, e)) => {
            let message = format!(
                "views.backends `{name}`: configuration not read ({e}); \
                 the frontend's access rules apply."
            );
            gap(&mut flow.diagnostics, &prefixed(&rel), 0, message);
            flow.complete = false;
            None
        }
    };
    let backend = extract_all(&ExtractContext::new(
        &dir,
        own.as_ref().unwrap_or(ctx.config),
    ));
    flow.complete &= backend.complete;
    for mut d in backend.diagnostics {
        d.file = prefixed(&d.file);
        flow.diagnostics.push(d);
    }
    if backend.endpoints.is_empty() {
        gap(
            &mut flow.diagnostics,
            &config_file,
            0,
            format!("views.backends `{name}`: no HTTP endpoints found."),
        );
        flow.complete = false;
    }
    for mut e in backend.endpoints {
        e.application = Some(match e.application {
            Some(app) => format!("{name}/{app}"),
            None => name.to_string(),
        });
        e.file = prefixed(&e.file);
        flow.endpoints.push(e);
    }
    for mut site in backend.handler_sites {
        site.endpoint_file = prefixed(&site.endpoint_file);
        site.file = prefixed(&site.file);
        flow.handler_sites.push(site);
    }
    flow.endpoints.sort();
    flow.endpoints.dedup();
    flow.handler_sites.sort();
    flow.handler_sites.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
}

/// `file` below the backend `name`; the backend itself for no file.
fn prefix(name: &str, file: &str) -> String {
    match file {
        "" => name.to_string(),
        file => format!("{name}/{file}"),
    }
}

/// Why the backend `dir` is not read at all, if so (see [`add_backend`]).
fn not_read(ctx: &ExtractContext, dir: &Path, itself: bool) -> Option<&'static str> {
    if itself {
        return Some("a backend must not be the project itself.");
    }
    match ctx.allowed_root.and_then(|a| refusal(dir, a, ctx.root)) {
        Some(Refusal::Outside) => Some("a backend outside the server root is not read."),
        Some(Refusal::Link) => Some("a backend path through a symlink is not read."),
        None if !dir.is_dir() => Some("not an existing directory; its endpoints are unknown."),
        None => None,
    }
}

/// `path` written lexically: `.` dropped, `x/..` collapsed, no trailing `/`;
/// a leading `..` stays. Links are not looked at.
fn normalise(path: &str) -> String {
    use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
    let mut parts: Vec<String> = Vec::new();
    let mut rooted = false;
    for c in Path::new(path).components() {
        match c {
            CurDir => {}
            ParentDir => match parts.last() {
                Some(last) if last != ".." => {
                    parts.pop();
                }
                _ if rooted => {}
                _ => parts.push("..".into()),
            },
            RootDir => rooted = true,
            Prefix(p) => parts.push(p.as_os_str().to_string_lossy().into_owned()),
            Normal(n) => parts.push(n.to_string_lossy().into_owned()),
        }
    }
    let joined = parts.join("/");
    match (rooted, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    }
}

/// Why a confined run does not read a backend.
enum Refusal {
    /// Lexically outside the allowed root; decided without the file system.
    Outside,
    /// A component below the allowed root is a symlink: refused whatever it
    /// points to (never followed), so nothing outside tells existence.
    Link,
}

/// The canonical form of the root joined with a backend path, written
/// lexically (an absolute `name` replaces the root).
fn absolute(root: &Path, name: &str) -> String {
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    normalise(&base.join(name).to_string_lossy())
}

/// [`Refusal`] of `dir` (the root joined with a backend path) against the
/// canonical `allowed` root; `None` when it may be read.
fn refusal(dir: &Path, allowed: &Path, root: &Path) -> Option<Refusal> {
    let name = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy();
    let written = absolute(root, &name);
    let Ok(below) = Path::new(&written).strip_prefix(allowed) else {
        return Some(Refusal::Outside);
    };
    let mut current = allowed.to_path_buf();
    for component in below.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Some(Refusal::Link),
            Ok(_) => {}
            Err(_) => break, // absent: reported as not a directory
        }
    }
    None
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
        crate::ui_flow_express::extract_with,
        crate::ui_flow_svelte::extract_with,
        crate::ui_flow_django::extract_with,
        crate::ui_flow_fastapi::extract_with,
        crate::ui_flow_flask::extract_with,
        crate::ui_flow_laravel::extract_with,
        crate::ui_flow_rust::extract_with,
        crate::ui_flow_go::extract_with,
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
        assert_eq!(purge(&["Administrator"]), ScreenReach::Undeclared);
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

    fn frontend_flow(front_toml: &str, back: &[(&str, &str)]) -> UiFlow {
        let dir = two_repos(front_toml, back);
        let flow = analyze(&dir.path().join("front")).unwrap();
        drop(dir);
        flow
    }

    const BACK_ENDPOINT: (&str, &str) = ("Api/OrdersController.cs", API);

    #[test]
    fn backend_gaps_and_incompleteness_are_carried_over() {
        let flow = frontend_flow(
            "[views]\nbackends = [\"../back\"]\n",
            &[
                BACK_ENDPOINT,
                ("W/A.razor", "@page \"/\"\n<a href=\"@X\">x</a>\n"),
            ],
        );
        assert!(!flow.complete, "{:#?}", flow.diagnostics);
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.file == "../back/W/A.razor"),
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(flow.endpoints.len(), 1);
    }

    #[test]
    fn handler_sites_of_a_backend_are_prefixed_in_both_files() {
        let flow = frontend_flow("[views]\nbackends = [\"../back\"]\n", &[BACK_ENDPOINT]);
        assert!(!flow.handler_sites.is_empty(), "{:#?}", flow.endpoints);
        for s in &flow.handler_sites {
            assert!(s.file.starts_with("../back/"), "{s:?}");
            assert!(s.endpoint_file.starts_with("../back/"), "{s:?}");
        }
    }

    #[test]
    fn an_unparsable_backend_config_is_a_gap_and_the_frontends_rules_apply() {
        let flow = frontend_flow(
            "[views]\nbackends = [\"../back\"]\n",
            &[BACK_ENDPOINT, ("archguard.toml", "[future]\nx = 1\n")],
        );
        assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.diagnostics);
        assert!(!flow.complete);
        let gap = flow
            .diagnostics
            .iter()
            .find(|d| d.message.contains("configuration not read"));
        assert_eq!(
            gap.map(|d| d.file.as_str()),
            Some("../back/archguard.toml"),
            "{:#?}",
            flow.diagnostics
        );
    }

    #[test]
    fn gaps_about_the_setting_name_the_config_file_in_use() {
        let dir = two_repos("", &[]);
        let front = dir.path().join("front");
        std::fs::remove_file(front.join("archguard.toml")).unwrap();
        write(
            &front,
            &[(
                ".archguard/archguard.toml",
                "[views]\nbackends = [\"../nope\"]\n",
            )],
        );
        let flow = analyze(&front).unwrap();
        assert_eq!(
            flow.diagnostics[0].file, ".archguard/archguard.toml",
            "{:#?}",
            flow.diagnostics
        );
        let config: archguard_core::Config =
            toml::from_str("[views]\nbackends = [\"../nope\"]\n").unwrap();
        let ctx = ExtractContext::new(&front, &config).with_config_file("other.toml");
        let flow = analyze_with(&ctx).unwrap();
        assert_eq!(flow.diagnostics[0].file, "other.toml");
    }

    #[test]
    fn equal_backends_written_differently_join_once() {
        let flow = frontend_flow(
            "[views]\nbackends = [\"../back/\", \"./../back\", \"../back/x/..\"]\n",
            &[BACK_ENDPOINT],
        );
        assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
        assert_eq!(
            flow.endpoints[0].application.as_deref(),
            Some("../back/Api")
        );
    }

    #[test]
    fn a_confined_run_does_not_read_a_backend_outside_its_root() {
        let dir = two_repos(
            "[views]\nbackends = [\"../back\", \"../back/Api/..\"]\n",
            &[BACK_ENDPOINT],
        );
        let front = dir.path().join("front");
        let config: archguard_core::Config =
            toml::from_str("[views]\nbackends = [\"../back\", \"/\", \"../nope\"]\n").unwrap();
        let allowed = front.canonicalize().unwrap();
        let ctx = ExtractContext::new(&front, &config).confined_to(&allowed);
        let flow = analyze_with(&ctx).unwrap();
        assert!(flow.endpoints.is_empty(), "{:#?}", flow.endpoints);
        let outside = flow
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("outside the server root"))
            .count();
        assert_eq!(outside, 3, "{:#?}", flow.diagnostics);
        // Inside the allowed root a backend is read.
        let allowed = dir.path().canonicalize().unwrap();
        let config: archguard_core::Config =
            toml::from_str("[views]\nbackends = [\"../back\"]\n").unwrap();
        let flow =
            analyze_with(&ExtractContext::new(&front, &config).confined_to(&allowed)).unwrap();
        assert_eq!(flow.endpoints.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn confined_backends_behind_a_symlink_give_one_message_whatever_the_target() {
        let dir = two_repos("", &[BACK_ENDPOINT]);
        let front = dir.path().join("front");
        std::os::unix::fs::symlink(dir.path().join("back"), front.join("valid_link")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("nothing"), front.join("dangling_link"))
            .unwrap();
        let config: archguard_core::Config = toml::from_str(
            "[views]\nbackends = [\"valid_link\", \"dangling_link\", \"valid_link/Api\"]\n",
        )
        .unwrap();
        let allowed = front.canonicalize().unwrap();
        let flow =
            analyze_with(&ExtractContext::new(&front, &config).confined_to(&allowed)).unwrap();
        assert!(flow.endpoints.is_empty(), "{:#?}", flow.endpoints);
        let messages: Vec<String> = flow
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .filter(|m| m.contains("views.backends"))
            .collect();
        assert_eq!(messages.len(), 3, "{messages:?}");
        let generic = |m: &String| m.split_once(": ").map(|(_, rest)| rest.to_string());
        let rests: std::collections::BTreeSet<_> = messages.iter().filter_map(generic).collect();
        assert_eq!(rests.len(), 1, "{messages:?}");
        assert!(
            rests.iter().next().unwrap().contains("symlink"),
            "{messages:?}"
        );
    }

    #[test]
    fn a_backend_must_not_be_the_project_itself_and_equal_directories_read_once() {
        let dir = two_repos("", &[BACK_ENDPOINT]);
        let front = dir.path().join("front");
        write(&front, &[BACK_ENDPOINT]);
        let back = dir.path().join("back").canonicalize().unwrap();
        let own = front.canonicalize().unwrap();
        let toml = format!(
            "[views]\nbackends = [\".\", \"\", \"./\", {:?}, \"../back\", {:?}]\n",
            own.to_string_lossy(),
            back.to_string_lossy()
        );
        let config: archguard_core::Config = toml::from_str(&toml).unwrap();
        let flow = analyze_with(&ExtractContext::new(&front, &config)).unwrap();
        let itself = flow
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("must not be the project itself"))
            .count();
        assert_eq!(itself, 1, "{:#?}", flow.diagnostics);
        // The local controller once, the backend's once: no double read.
        assert_eq!(flow.endpoints.len(), 2, "{:#?}", flow.endpoints);
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
