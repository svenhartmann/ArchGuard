//! Import of `.archguard/ui-flow.json` (ADR 0021): the way in for frameworks
//! without an extractor.
use std::path::Path;

use archguard_core::{ArchguardError, UI_FLOW_DEFINITION, UI_FLOW_IMPORT_SOURCE, UiFlow};

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

#[cfg(test)]
mod tests {
    use super::*;

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
            "transitions":[{"from":"home","target":{"unresolved":"url"},"file":"a.ts","line":2,"source":"x"}],
            "actions":[{"screen":"home","call":{"http":{"method":"GET","path":"/api"}},"file":"a.ts","line":3,"source":"x"}],
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
