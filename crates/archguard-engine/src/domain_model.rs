//! Import of `.archguard/domain-model.json` (ADR 0022): the way in for any
//! language without an extractor.
use std::path::Path;

use archguard_core::ArchguardError;
use archguard_core::domain_model::{
    DOMAIN_MODEL_DEFINITION, DOMAIN_MODEL_IMPORT_SOURCE, DomainModel,
};

pub const DOMAIN_MODEL_FILE: &str = ".archguard/domain-model.json";
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The imported model with every entry's source set to `import`; `Ok(None)`
/// without the file. An unreadable, oversized or invalid file is an error.
pub fn read_domain_model(root: &Path) -> Result<Option<DomainModel>, ArchguardError> {
    let bytes =
        match crate::project_files::read_repo_file(root, Path::new(DOMAIN_MODEL_FILE), MAX_BYTES) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(ArchguardError::Analysis(format!(
                    "Cannot read {DOMAIN_MODEL_FILE}: {e}"
                )));
            }
        };
    let mut model: DomainModel = serde_json::from_slice(&bytes)
        .map_err(|e| ArchguardError::Analysis(format!("Invalid {DOMAIN_MODEL_FILE}: {e}")))?;
    if model.definition != DOMAIN_MODEL_DEFINITION {
        return Err(ArchguardError::Analysis(format!(
            "{DOMAIN_MODEL_FILE}: definition {:?}, expected {DOMAIN_MODEL_DEFINITION:?}",
            model.definition
        )));
    }
    let import = || DOMAIN_MODEL_IMPORT_SOURCE.to_string();
    model.entities.iter_mut().for_each(|e| e.source = import());
    model.relations.iter_mut().for_each(|e| e.source = import());
    model.use_cases.iter_mut().for_each(|e| e.source = import());
    model.events.iter_mut().for_each(|e| e.source = import());
    Ok(Some(model))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(json: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if let Some(json) = json {
            std::fs::create_dir(dir.path().join(".archguard")).unwrap();
            std::fs::write(dir.path().join(DOMAIN_MODEL_FILE), json).unwrap();
        }
        dir
    }

    #[test]
    fn valid_import_forces_source_import() {
        let dir = project(Some(
            r#"{"definition":"domain-model/v1","complete":false,
            "entities":[{"id":"Order","name":"Order","kind":"aggregate_root",
              "fields":[{"name":"OrderItems","type":"OrderItem","collection":true}],
              "file":"Order.cs","line":1,"source":"csharp"}],
            "relations":[{"from":"Order","to":"OrderItem","kind":"has_many","via":"OrderItems","source":"x"}],
            "state_machines":[{"entity":"Order","field":"OrderStatus","enum_type":"OrderStatus",
              "states":["StockConfirmed","Paid"],"initial":"StockConfirmed",
              "transitions":[{"from":["StockConfirmed"],"to":"Paid","method":"Order.SetPaidStatus",
                "guard":"OrderStatus == OrderStatus.StockConfirmed",
                "events":["OrderStatusChangedToPaidDomainEvent"],"file":"Order.cs","line":30}]}],
            "use_cases":[{"id":"SetPaid","name":"SetPaid","kind":"command","request":"SetPaidOrderStatusCommand",
              "calls":["Order.SetPaidStatus"],"file":"H.cs","line":3,"source":"x"}],
            "events":[{"type":"OrderStatusChangedToPaidDomainEvent","raised_by":["Order.SetPaidStatus"],
              "handled_by":["OrderStatusChangedToPaidDomainEventHandler"],"source":"x"}]}"#,
        ));
        let model = read_domain_model(dir.path()).unwrap().unwrap();
        assert!(!model.complete);
        assert_eq!(model.entities[0].source, "import");
        assert_eq!(model.relations[0].source, "import");
        assert_eq!(model.use_cases[0].source, "import");
        assert_eq!(model.events[0].source, "import");
        assert_eq!(model.state_machines[0].transitions[0].to, "Paid");
    }

    #[test]
    fn missing_file_is_none() {
        assert_eq!(read_domain_model(project(None).path()).unwrap(), None);
    }

    #[test]
    fn invalid_json_or_definition_is_an_error() {
        for json in [
            "{not json",
            r#"{"definition":"domain-model/v2","complete":true}"#,
            r#"{"definition":"ui-flow/v1","complete":true}"#,
        ] {
            let err = read_domain_model(project(Some(json)).path()).unwrap_err();
            assert!(err.to_string().contains(DOMAIN_MODEL_FILE), "{err}");
        }
    }
}
