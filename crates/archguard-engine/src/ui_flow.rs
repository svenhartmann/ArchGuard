//! Import of `.archguard/ui-flow.json` (ADR 0021): the way in for frameworks
//! without an extractor.
use std::path::Path;

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
    Ok(Some(flow))
}

/// The flow below `root`: every extractor merged with the import file. An
/// invalid import file is an error; extractor problems stay diagnostics.
pub fn analyze(root: &Path) -> Result<UiFlow, ArchguardError> {
    let import = read_ui_flow(root)?.unwrap_or_default();
    let extractors = [
        crate::ui_flow_blazor::extract,
        crate::ui_flow_angular::extract,
        crate::ui_flow_backend::extract,
        crate::ui_flow_react::extract,
        crate::ui_flow_xaml::extract,
        crate::ui_flow_vue::extract,
        crate::ui_flow_slint::extract,
    ];
    Ok(extractors
        .into_iter()
        .fold(import, |flow, extract| merge_ui_flows(flow, extract(root))))
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
