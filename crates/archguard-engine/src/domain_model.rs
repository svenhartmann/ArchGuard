//! Import of `.archguard/domain-model.json` (ADR 0022): the way in for any
//! language without an extractor.
use std::path::Path;

use crate::extract_ctx::{ExtractContext, with_root_config};
use archguard_core::ArchguardError;
use archguard_core::domain_model::{
    DOMAIN_MODEL_DEFINITION, DOMAIN_MODEL_IMPORT_SOURCE, DomainModel, merge_domain_models,
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
    (model.message_handlers.iter_mut()).for_each(|e| e.source = import());
    Ok(Some(model))
}

/// The domain model below `root` with the root's configuration: every
/// extractor merged with the import file. An invalid import file is an error.
pub fn analyze(root: &Path) -> Result<DomainModel, ArchguardError> {
    with_root_config(root, analyze_with)
}

/// [`analyze`] for one run, e.g. with the configuration given by `--config`.
pub fn analyze_with(ctx: &ExtractContext) -> Result<DomainModel, ArchguardError> {
    let import = read_domain_model(ctx.root)?.unwrap_or_default();
    let extractors = [
        crate::domain_csharp::extract_with,
        crate::domain_java::extract_with,
        crate::domain_typescript::extract_with,
    ];
    let model = (extractors.into_iter())
        .map(|extract| extract(ctx))
        .reduce(merge_domain_models)
        .unwrap_or_default();
    Ok(merge_domain_models(model, import))
}

/// Counts for the summary line of the domain view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DomainModelSummary {
    pub entities: usize,
    pub state_machines: usize,
    pub transitions: usize,
    pub use_cases: usize,
    pub events: usize,
    pub complete: bool,
    #[serde(default)]
    pub rules: usize,
    #[serde(default)]
    pub validations: usize,
    #[serde(default)]
    pub integration_events: usize,
    /// Controller DTOs listed in `api_models` instead of as entities; absent
    /// from the JSON when zero, so apps without any keep their output.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub api_models: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

pub fn summary(model: &DomainModel) -> DomainModelSummary {
    DomainModelSummary {
        entities: model.entities.len(),
        state_machines: model.state_machines.len(),
        transitions: (model.state_machines.iter())
            .map(|m| m.transitions.len())
            .sum(),
        use_cases: model.use_cases.len(),
        events: model.events.len(),
        complete: model.complete,
        rules: model.rules.len(),
        validations: model.validations.len(),
        integration_events: model.integration_events.len(),
        api_models: model.api_models.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_merges_csharp_and_import() {
        let dir = project(Some(
            r#"{"definition":"domain-model/v1","complete":true,
            "entities":[{"id":"Invoice","name":"Invoice","kind":"entity","fields":[],"file":"x.json","line":1,"source":"x"}]}"#,
        ));
        std::fs::write(
            dir.path().join("Order.cs"),
            "public interface IAggregateRoot { }\npublic class Order : IAggregateRoot { }\n",
        )
        .unwrap();
        let model = analyze(dir.path()).unwrap();
        let entities: Vec<(&str, &str)> = model
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.source.as_str()))
            .collect();
        assert_eq!(entities, [("Invoice", "import"), ("Order", "csharp")]);
        assert_eq!(
            summary(&model),
            DomainModelSummary {
                entities: 2,
                complete: model.complete,
                ..DomainModelSummary::default()
            }
        );
        std::fs::write(dir.path().join(DOMAIN_MODEL_FILE), "{not json").unwrap();
        let err = analyze(dir.path()).unwrap_err();
        assert!(err.to_string().contains(DOMAIN_MODEL_FILE), "{err}");
    }

    #[test]
    fn summary_counts_transitions_of_every_machine() {
        let model: DomainModel = serde_json::from_str(
            r#"{"definition":"domain-model/v1","complete":false,
            "state_machines":[{"entity":"A","field":"S","enum_type":"S","states":[],"transitions":[
              {"to":"x","method":"A.X","file":"a","line":1},{"to":"y","method":"A.Y","file":"a","line":2}]},
              {"entity":"B","field":"S","enum_type":"S","states":[],"transitions":[
              {"to":"z","method":"B.Z","file":"b","line":1}]}],
            "use_cases":[{"id":"U","name":"U","kind":"command","calls":[],"file":"u","line":1,"source":"x"}],
            "events":[{"type":"E","raised_by":[],"handled_by":[],"source":"x"}]}"#,
        )
        .unwrap();
        assert_eq!(
            summary(&model),
            DomainModelSummary {
                entities: 0,
                state_machines: 2,
                transitions: 3,
                use_cases: 1,
                events: 1,
                complete: false,
                ..DomainModelSummary::default()
            }
        );
    }

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
