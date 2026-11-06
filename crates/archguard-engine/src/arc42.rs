//! Gather existing evidence; documentation never invokes builds or scanners.
use archguard_core::{
    ArchguardError, CallAnalysis, Config, Cut, Finding, Graph, Report, ServiceTopology, aggregate,
};
use archguard_report::{Arc42Blocks, Arc42Document, Arc42Inputs};
use serde_json::{Value, json};
use std::path::Path;

pub fn arc42_from_evidence(
    graph: &Graph,
    config: &Config,
    findings: &[Finding],
    calls: &CallAnalysis,
    services: &ServiceTopology,
    insights: &Value,
) -> Result<Arc42Document, ArchguardError> {
    let view = aggregate(graph, &Cut::at_depth(graph, config.modules.depth as usize));
    let report = Report {
        schema_version: 1,
        semantic: graph.semantic.clone(),
        review_kpis: graph.review_kpis.clone(),
        summary: archguard_analysis::summarize(
            &view,
            &archguard_analysis::find_sccs(&view),
            findings,
        ),
        config: Default::default(),
        findings: findings.to_vec(),
        metrics: Default::default(),
    };
    let root = Path::new(&graph.root);
    let paths = crate::reference_paths::source_reference_paths(root, config)?;
    // Only declared layers name a building block's place; derived ones are observations.
    let mut layers = archguard_analysis::layer_assignment(graph, &config.layers)?;
    layers.retain(|_, i| config.layers[*i].origin == archguard_core::LayerOrigin::Declared);
    let blocks = Arc42Blocks {
        layers,
        descriptions: services
            .projects
            .iter()
            .filter_map(|p| Some((p.id.clone(), manifest_description(root, &p.manifest)?)))
            .collect(),
    };
    Ok(archguard_report::render_arc42(Arc42Inputs {
        source_graph: graph,
        view: &view,
        report: &report,
        config,
        calls,
        services,
        insights,
        source_paths: &paths,
        blocks: &blocks,
    }))
}

/// The responsibility a project declares about itself. Values a manifest
/// inherits from elsewhere (`description.workspace = true`) are not resolved.
fn manifest_description(root: &Path, manifest: &str) -> Option<String> {
    let text = std::fs::read_to_string(root.join(manifest)).ok()?;
    let file = Path::new(manifest)
        .file_name()?
        .to_str()?
        .to_ascii_lowercase();
    let raw = raw_description(&file, &text)?;
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

/// The description field of the manifest kinds that have one.
fn raw_description(file: &str, text: &str) -> Option<String> {
    match file {
        "cargo.toml" => toml_text(text, &[&["package", "description"]]),
        "pyproject.toml" => toml_text(
            text,
            &[
                &["project", "description"],
                &["tool", "poetry", "description"],
            ],
        ),
        "package.json" => serde_json::from_str::<Value>(text)
            .ok()?
            .get("description")?
            .as_str()
            .map(str::to_owned),
        "pom.xml" => xml_text(text, "project", "description"),
        _ if [".csproj", ".fsproj", ".vbproj"]
            .iter()
            .any(|ext| file.ends_with(ext)) =>
        {
            xml_text(text, "PropertyGroup", "Description")
        }
        _ => None,
    }
}

/// The first string found at one of the key paths.
fn toml_text(text: &str, keys: &[&[&str]]) -> Option<String> {
    let table: toml::Table = toml::from_str(text).ok()?;
    keys.iter().find_map(|path| {
        let mut value = table.get(path[0])?;
        for key in &path[1..] {
            value = value.get(key)?;
        }
        value.as_str().map(str::to_owned)
    })
}

/// The text of the first `name` element directly below a `parent` element.
fn xml_text(text: &str, parent: &str, name: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(text).ok()?;
    doc.descendants()
        .find(|n| {
            n.tag_name().name() == name
                && n.parent_element()
                    .is_some_and(|p| p.tag_name().name() == parent)
        })
        .and_then(|n| n.text())
        .map(str::to_owned)
}

pub fn arc42_document(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report: &Report,
) -> Result<Arc42Document, ArchguardError> {
    let calls = crate::semantic_calls::analyze_calls(root, config);
    let services = crate::services::analyze_services(root)?;
    let insights = json!({
        "patterns":crate::patterns::analyze_architecture_patterns(root,config,graph,&calls,&services),
        "code_metrics":crate::code_metrics(&calls),
        "test_coverage":crate::test_coverage::read_test_coverage(root,config,graph),
        "quality":crate::quality_findings::read_quality_findings(root,config,graph),
        "security":crate::security::read_security(root,config),
        "contracts":crate::api_contracts::read_api_contracts(root,&services),
        "measurement_runs":crate::measurement_runs::read_measurement_runs(root),
        "mutations":crate::mutation_report::read_mutations(root,config,graph),
        "duplicates":crate::duplicates::read_duplicates(root,config,graph)
    });
    arc42_from_evidence(
        graph,
        config,
        &report.findings,
        &calls,
        &services,
        &insights,
    )
}

#[cfg(test)]
mod tests {
    use super::manifest_description;
    use std::fs;

    #[test]
    fn manifest_descriptions_are_read_for_each_ecosystem() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "rust/Cargo.toml",
            "[package]\nname = \"a\"\ndescription = \"\"\"L2: pipeline\n  orchestration\"\"\"\n",
        );
        write(
            "ws/Cargo.toml",
            "[package]\nname = \"w\"\ndescription.workspace = true\n",
        );
        write(
            "web/package.json",
            r#"{"name":"web","description":"Checkout UI"}"#,
        );
        write(
            "py/pyproject.toml",
            "[project]\nname = \"p\"\ndescription = \"Pricing rules\"\n",
        );
        write(
            "poetry/pyproject.toml",
            "[tool.poetry]\nname = \"q\"\ndescription = \"Risk scoring\"\n",
        );
        write(
            "java/pom.xml",
            "<project xmlns=\"http://maven.apache.org/POM/4.0.0\"><parent><description>parent</description></parent><description>Order service</description></project>",
        );
        write(
            "net/Api.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><Description>Payment API</Description></PropertyGroup></Project>",
        );
        write("go/go.mod", "module example.com/x\n");
        let read = |manifest: &str| manifest_description(dir.path(), manifest);
        assert_eq!(
            read("rust/Cargo.toml").as_deref(),
            Some("L2: pipeline orchestration")
        );
        assert_eq!(
            read("ws/Cargo.toml"),
            None,
            "inherited values are not guessed"
        );
        assert_eq!(read("web/package.json").as_deref(), Some("Checkout UI"));
        assert_eq!(read("py/pyproject.toml").as_deref(), Some("Pricing rules"));
        assert_eq!(
            read("poetry/pyproject.toml").as_deref(),
            Some("Risk scoring")
        );
        assert_eq!(
            read("java/pom.xml").as_deref(),
            Some("Order service"),
            "the project's own description, not the parent's"
        );
        assert_eq!(read("net/Api.csproj").as_deref(), Some("Payment API"));
        assert_eq!(read("go/go.mod"), None);
        assert_eq!(read("missing/Cargo.toml"), None);
    }
}
