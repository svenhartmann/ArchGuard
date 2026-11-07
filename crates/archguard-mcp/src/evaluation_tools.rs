//! `evaluation`: agreement of findings with an expert assessment (ADR 0005),
//! read-only and paginated within 4 KiB.

use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use archguard_engine::Evaluation;
use archguard_engine::core::ArchguardError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct EvaluationParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Assessment file inside the server root; default
    /// <project>/.archguard/assessment.toml.
    pub assessment: Option<String>,
}

#[tool_router(router = evaluation_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Compare current architecture findings with a human expert assessment (.archguard/assessment.toml, ADR 0005): recall = matched expected problems / expected problems, precision = judged true positives / judged current findings; null when nothing was counted (not measured). Lists missed expected problems with reason, unjudged findings (judge them next by their stable id), stale judged ids and revision warnings. ArchGuard never writes the assessment. Read-only; flattened pages within 4 KiB, follow next_offset."
    )]
    pub(crate) async fn evaluation(
        &self,
        Parameters(params): Parameters<EvaluationParams>,
    ) -> Result<String, ErrorData> {
        crate::engineering_tools::validate(&params.page)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let assessment = params
            .assessment
            .as_deref()
            .map(|a| crate::tools::resolve_file_in_root(&self.root, Path::new(a), false))
            .transpose()?;
        let evaluation = archguard_engine::evaluate_project(&root, None, assessment.as_deref())
            .map_err(|err| match err {
                ArchguardError::Config(_) => crate::tools::invalid_params(err.to_string()),
                _ => ErrorData::internal_error(err.to_string(), None),
            })?;
        value(&evaluation, &params.page).map(|v| v.to_string())
    }
}

/// Metrics as metadata, every listed entry as one flattened record.
fn value(e: &Evaluation, params: &EngineeringParams) -> Result<Value, ErrorData> {
    let query = params.query.as_deref().unwrap_or("").to_lowercase();
    let mut rows: Vec<Value> = e
        .warnings
        .iter()
        .map(|w| json!({"kind":"warning","message":w}))
        .collect();
    rows.extend(
        e.missed
            .iter()
            .map(|m| json!({"kind":"missed","id":m.id,"expected_kind":m.kind,"reason":m.reason})),
    );
    rows.extend(
        e.matched.iter().map(
            |m| json!({"kind":"matched","id":m.id,"expected_kind":m.kind,"findings":m.findings}),
        ),
    );
    rows.extend(
        e.unjudged
            .iter()
            .map(|f| json!({"kind":"unjudged","finding":f})),
    );
    rows.extend(
        e.stale
            .iter()
            .map(|id| json!({"kind":"stale","finding":id})),
    );
    rows.retain(|r| query.is_empty() || r.to_string().to_lowercase().contains(&query));
    crate::engineering_tools::page(
        "complete",
        json!({
            "recall":e.recall,"precision":e.precision,"findings_total":e.findings_total,
            "expected_total":e.recall.denominator,"unjudged_total":e.unjudged.len(),"stale_total":e.stale.len(),
            "assessment":e.assessment,"head":e.head,
            "interpretation":"Agreement with one recorded expert assessment, not ground truth. null = not measured. Judge unjudged findings by their stable id in the assessment file; ArchGuard never writes verdicts."
        }),
        rows.iter(),
        params,
        |r| r.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::fs;

    const META: &str = "[assessment]\nassessor='E'\ndate='2026-11-06'\nrevision='abc'\n";

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.ts"), "import './b';\n").unwrap();
        fs::write(dir.path().join("b.ts"), "import './a';\n").unwrap();
        fs::write(
            dir.path().join("archguard.toml"),
            "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(".archguard")).unwrap();
        dir
    }

    async fn call(dir: &std::path::Path, params: Value) -> Result<Value, ErrorData> {
        let handler = ArchguardTools::new(dir.into());
        let params: EvaluationParams = serde_json::from_value(params).unwrap();
        let text = handler.evaluation(Parameters(params)).await?;
        assert!(text.len() <= crate::tools::MAX_RESPONSE_BYTES, "{text}");
        Ok(serde_json::from_str(&text).unwrap())
    }

    fn items<'a>(value: &'a Value, kind: &str) -> Vec<&'a Value> {
        value["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == kind)
            .collect()
    }

    #[tokio::test]
    async fn evaluation_reports_metrics_and_rows() {
        let dir = project();
        let file = dir.path().join(".archguard/assessment.toml");
        fs::write(
            &file,
            format!(
                "{META}[[expected]]\nid='E1'\nkind='cycle'\nnodes=['a.ts','b.ts']\n\
                 [[expected]]\nid='E2'\nkind='other'\n"
            ),
        )
        .unwrap();
        let first = call(dir.path(), json!({"limit": 200})).await.unwrap();
        let meta = &first["metadata"];
        assert_eq!(
            meta["recall"],
            json!({"numerator":1,"denominator":2,"value":0.5})
        );
        assert!(meta["precision"]["value"].is_null());
        assert_eq!(
            items(&first, "missed")[0]["reason"],
            "no ArchGuard finding kind"
        );
        assert_eq!(items(&first, "matched")[0]["id"], "E1");
        let cycle = items(&first, "unjudged")
            .into_iter()
            .find(|r| r["finding"]["kind"] == "cycle")
            .unwrap()["finding"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        fs::write(
            &file,
            format!(
                "{META}[[judged]]\nfinding='{cycle}'\nverdict='true_positive'\n\
                 [[judged]]\nfinding='gone'\nverdict='false_positive'\n"
            ),
        )
        .unwrap();
        let judged = call(dir.path(), json!({})).await.unwrap();
        assert_eq!(
            judged["metadata"]["precision"],
            json!({"numerator":1,"denominator":1,"value":1.0})
        );
        assert!(judged["metadata"]["recall"]["value"].is_null());
        assert_eq!(items(&judged, "stale")[0]["finding"], "gone");
        assert!(
            items(&judged, "warning")[0]["message"]
                .as_str()
                .unwrap()
                .contains("HEAD unknown")
        );
    }

    #[tokio::test]
    async fn many_rows_page_within_the_budget() {
        let dir = project();
        let body: String = (0..80)
            .map(|i| {
                format!(
                    "[[expected]]\nid='E{i:03}'\nkind='other'\nnote='{}'\n",
                    "n".repeat(40)
                )
            })
            .collect();
        fs::write(
            dir.path().join(".archguard/assessment.toml"),
            format!("{META}{body}"),
        )
        .unwrap();
        let (mut offset, mut missed) = (0, 0);
        loop {
            let page = call(dir.path(), json!({"offset": offset, "limit": 200}))
                .await
                .unwrap();
            missed += items(&page, "missed").len();
            match page["next_offset"].as_u64() {
                Some(next) => offset = next as usize,
                None => break,
            }
        }
        assert_eq!(missed, 80);
        assert!(offset > 0, "80 rows cannot fit one 4 KiB page");
    }

    #[tokio::test]
    async fn missing_or_outside_assessment_is_invalid_params() {
        let dir = project();
        let err = call(dir.path(), json!({})).await.unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let err = call(dir.path(), json!({"assessment": "../outside.toml"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }
}
