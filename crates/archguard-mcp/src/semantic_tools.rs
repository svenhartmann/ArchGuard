//! Bounded, paginated access to the shared compiler model.
use archguard_engine::core::CallAnalysis;
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SemanticModelParams {
    /// Project directory inside the server root.
    pub path: Option<String>,
    /// `classes`, `methods`, `calls` (default), `type_uses`, `fields`,
    /// `field_accesses`, `cohesion` or flattened `cohesion_groups`.
    pub kind: Option<String>,
    /// Source language, e.g. `java` or `csharp`.
    pub language: Option<String>,
    /// Slash-separated package/graph-node prefix; calls match either endpoint.
    pub within: Option<String>,
    /// First matching row. Follow next_offset from the response.
    pub offset: Option<usize>,
    /// Requested rows, 1..=200; response bytes may reduce the page.
    pub limit: Option<usize>,
}

fn language(value: &str) -> Result<&'static str, ErrorData> {
    match value {
        "java" => Ok("java"),
        "csharp" | "cs" | "c#" => Ok("csharp"),
        "typescript" | "ts" => Ok("typescript"),
        "javascript" | "js" => Ok("javascript"),
        "python" | "py" => Ok("python"),
        "rust" | "rs" => Ok("rust"),
        "php" => Ok("php"),
        "go" | "golang" => Ok("go"),
        "kotlin" | "kt" => Ok("kotlin"),
        _ => Err(ErrorData::invalid_params(
            "language must be java, csharp, typescript, javascript, python, rust, php, go or kotlin",
            None,
        )),
    }
}
pub fn validate(params: &SemanticModelParams) -> Result<(), ErrorData> {
    if !matches!(
        params.kind.as_deref().unwrap_or("calls"),
        "classes"
            | "methods"
            | "calls"
            | "type_uses"
            | "fields"
            | "field_accesses"
            | "cohesion"
            | "cohesion_groups"
    ) {
        return Err(ErrorData::invalid_params(
            "kind must be classes, methods, calls, type_uses, fields, field_accesses, cohesion or cohesion_groups",
            None,
        ));
    }
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(ErrorData::invalid_params("limit must be 1..=200", None));
    }
    if let Some(value) = &params.language {
        language(value)?;
    }
    if params
        .within
        .as_ref()
        .is_some_and(|scope| scope.len() > 512 || scope.contains('\0'))
    {
        return Err(ErrorData::invalid_params(
            "within must be a package or graph-node prefix of at most 512 bytes",
            None,
        ));
    }
    Ok(())
}
fn in_scope(value: &str, scope: &str) -> bool {
    scope.is_empty()
        || value == scope
        || value
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
fn shortened(value: &str, maximum: usize, truncated: &mut bool) -> String {
    let result: String = value.chars().take(maximum).collect();
    if result.len() != value.len() {
        *truncated = true;
        format!("{result}…")
    } else {
        result
    }
}
fn size(value: &Value) -> usize {
    serde_json::to_vec(value).expect("plain JSON").len()
}

pub fn semantic_value(
    model: &CallAnalysis,
    params: &SemanticModelParams,
) -> Result<Value, ErrorData> {
    validate(params)?;
    let kind = params.kind.as_deref().unwrap_or("calls");
    let selected_language = params.language.as_deref().map(language).transpose()?;
    let within = params.within.as_deref().unwrap_or("").trim_end_matches('/');
    let classes: HashMap<_, _> = model
        .classes
        .iter()
        .map(|class| (class.id.as_str(), class))
        .collect();
    let class_language = |id: &str| {
        classes
            .get(id)
            .and_then(|class| class.language.as_deref())
            .unwrap_or(if id.starts_with("csharp:") {
                "csharp"
            } else {
                "java"
            })
    };
    let class_scope = |id: &str| {
        within.is_empty()
            || classes.get(id).is_some_and(|class| {
                in_scope(&class.graph_node, within) || in_scope(&class.package, within)
            })
    };
    let matches_language =
        |id: &str| selected_language.is_none_or(|selected| selected == class_language(id));
    let mut rows: Vec<Value> = match kind {
        "classes" => model
            .classes
            .iter()
            .filter(|class| class_scope(&class.id) && matches_language(&class.id))
            .map(|class| serde_json::to_value(class).expect("class serializes"))
            .collect(),
        "methods" => model
            .methods
            .iter()
            .filter(|method| class_scope(&method.class_id) && matches_language(&method.class_id))
            .map(|method| {
                let mut row = serde_json::to_value(method).expect("method serializes");
                row["language"] = class_language(&method.class_id).into();
                row
            })
            .collect(),
        "type_uses"=>model.details.iter().flat_map(|d|&d.type_uses).filter(|u|matches_language(&u.from_class)&&(class_scope(&u.from_class)||u.to_class.as_deref().is_some_and(class_scope))).map(|u|serde_json::to_value(u).unwrap()).collect(),
        "fields"|"field_accesses"=>model.details.iter().flat_map(|d|&d.class_states).filter(|s|class_scope(&s.class_id)&&matches_language(&s.class_id)).flat_map(|s|{
            let rows:Vec<_>=if kind=="fields"{s.fields.iter().map(|f|serde_json::to_value(f).unwrap()).collect()}else{s.accesses.iter().map(|a|serde_json::to_value(a).unwrap()).collect()};
            rows.into_iter().map(|mut row|{row["class_id"]=s.class_id.clone().into();row["status"]=s.status.clone().into();row}).collect::<Vec<_>>()
        }).collect(),
        "cohesion"|"cohesion_groups"=>archguard_engine::class_cohesion(model).into_iter().filter(|m|class_scope(&m.class_id)&&matches_language(&m.class_id)).flat_map(|metric|{
            if kind=="cohesion_groups" {let class_id=&metric.class_id;metric.groups.iter().enumerate().flat_map(|(group,methods)|methods.iter().map(move |method|json!({"class_id":class_id,"group":group,"method":method}))).collect::<Vec<_>>()}
            else {let mut value=serde_json::to_value(&metric).unwrap();value["groups_total"]=metric.groups.len().into();value.as_object_mut().unwrap().remove("groups");vec![value]}
        }).collect(),
        _ => model
            .calls
            .iter()
            .filter(|call| {
                matches_language(&call.from_class)
                    && (class_scope(&call.from_class)
                        || call.to_class.as_deref().is_some_and(class_scope))
            })
            .map(|call| {
                let mut row = serde_json::to_value(call).expect("call serializes");
                row["language"] = class_language(&call.from_class).into();
                row
            })
            .collect(),
    };
    rows.sort_by(|a, b| {
        if kind == "calls" {
            (
                a["file"].as_str(),
                a["line"].as_u64(),
                a["column"].as_u64(),
                a["from_class"].as_str(),
                a["to_method"].as_str(),
            )
                .cmp(&(
                    b["file"].as_str(),
                    b["line"].as_u64(),
                    b["column"].as_u64(),
                    b["from_class"].as_str(),
                    b["to_method"].as_str(),
                ))
        } else {
            a["id"]
                .as_str()
                .cmp(&b["id"].as_str())
                .then_with(|| a.to_string().cmp(&b.to_string()))
        }
    });
    let total = rows.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(crate::server::DEFAULT_LIMIT);
    let mut metadata_truncated = model.analyzers.len() > 8 || model.diagnostics.len() > 3;
    let analyzers:Vec<_>=model.analyzers.iter().take(8).map(|analyzer|json!({
        "language":shortened(&analyzer.language,24,&mut metadata_truncated),
        "backend":shortened(&analyzer.backend,48,&mut metadata_truncated),
        "status":shortened(&analyzer.status,16,&mut metadata_truncated),"files":analyzer.files,
        "version":analyzer.version.as_ref().map(|value|shortened(value,64,&mut metadata_truncated)),
        "project":analyzer.project.as_ref().map(|value|shortened(value,96,&mut metadata_truncated)),
        "diagnostics_count":analyzer.diagnostics.len()
    })).collect();
    let diagnostics: Vec<_> = model
        .diagnostics
        .iter()
        .take(3)
        .map(|value| shortened(value, 180, &mut metadata_truncated))
        .collect();
    let mut reply = json!({
        "status":model.status,"coverage_scope":"whole_report","detail_definition":model.details.as_ref().map(|d|&d.definition),"kind":kind,"language":selected_language,
        "within":params.within,"files":model.files,"analyzers":analyzers,"analyzers_total":model.analyzers.len(),
        "diagnostics":diagnostics,"diagnostics_total":model.diagnostics.len(),"metadata_truncated":metadata_truncated,
        "totals":{"classes":model.classes.len(),"methods":model.methods.len(),"calls":model.calls.len()},
        "total":total,"offset":offset,"returned":0,"next_offset":null,"truncated":false,"items":[]
    });
    // Reserve room for actual data. Metadata omissions are counted explicitly;
    // never let generic truncation silently discard the provider coverage list.
    while size(&reply) > crate::tools::MAX_RESPONSE_BYTES / 2 {
        reply["metadata_truncated"] = true.into();
        if !reply["diagnostics"].as_array().unwrap().is_empty() {
            reply["diagnostics"].as_array_mut().unwrap().pop();
        } else if !reply["analyzers"].as_array().unwrap().is_empty() {
            reply["analyzers"].as_array_mut().unwrap().pop();
        } else {
            break;
        }
    }
    let mut returned = 0;
    for (index, row) in rows.into_iter().enumerate().skip(offset).take(limit) {
        reply["items"].as_array_mut().unwrap().push(row);
        reply["returned"] = (returned + 1).into();
        reply["next_offset"] = if index + 1 < total {
            (index + 1).into()
        } else {
            Value::Null
        };
        reply["truncated"] = (index + 1 < total || reply["metadata_truncated"] == true).into();
        if size(&reply) > crate::tools::MAX_RESPONSE_BYTES {
            reply["items"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            // A row larger than the whole budget cannot be returned by retrying
            // the same cursor. Make the omission explicit and advance once.
            reply["items"].as_array_mut().unwrap().push(json!({"index":index,"omitted":true,"reason":"row_exceeds_response_limit","hint":"Use archguard calls --format json for complete source evidence."}));
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
    reply["truncated"] = (next < total
        || reply["metadata_truncated"] == true
        || reply["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["omitted"] == true))
    .into();
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ArchguardTools;
    use crate::tools;
    use rmcp::handler::server::wrapper::Parameters;

    fn model() -> CallAnalysis {
        let mut value = serde_json::to_value(CallAnalysis::default()).unwrap();
        value["status"] = "partial".into();
        value["files"] = 4.into();
        value["analyzers"] = json!([
            {"language":"java","backend":"javac","status":"ready","files":2,"version":"23","project":"pom.xml","diagnostics":[]},
            {"language":"csharp","backend":"roslyn","status":"partial","files":2,"version":"8","project":"app.csproj","diagnostics":["Missing dependency"]}
        ]);
        let classes = [
            ("p.Api", "p.Api", "p", "p/Api.java", "java"),
            (
                "outside.B",
                "outside.B",
                "outside",
                "outside/B.java",
                "java",
            ),
            ("csharp:p.Api", "p.Api", "p", "p/Api.cs", "csharp"),
            (
                "csharp:outside.B",
                "outside.B",
                "outside",
                "outside/B.cs",
                "csharp",
            ),
        ];
        value["classes"] = classes.iter().map(|(id,name,package,file,language)| json!({"id":id,"name":name,"package":package,"file":file,"graph_node":file,"line":1,"kind":"class","language":language})).collect::<Vec<_>>().into();
        value["methods"] = classes.iter().map(|(id,_,_,file,_)| json!({"id":format!("{id}#run()"),"class_id":id,"name":"run","signature":"run()","file":file,"line":2,"is_static":false})).collect::<Vec<_>>().into();
        value["calls"] = [("p.Api","outside.B","p/Api.java"),("outside.B","p.Api","outside/B.java"),("csharp:p.Api","csharp:outside.B","p/Api.cs"),("csharp:outside.B","csharp:p.Api","outside/B.cs")].iter().map(|(from,to,file)|json!({"from_class":from,"from_method":format!("{from}#run()"),"to_class":to,"to_method":format!("{to}#run()"),"target_label":"run()","dispatch":"instance","resolution":"project","file":file,"line":3,"column":7,"code":"other.run()"})).collect::<Vec<_>>().into();
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn semantic_details_have_bounded_queryable_rows_and_unknown_legacy_metrics() {
        let mut model = model();
        model.details=Some(serde_json::from_value(json!({"definition":"source-types-state/v1","class_states":[],"type_uses":[{"from_class":"p.Api","from_method":null,"to_class":"outside.B","target_label":"outside.B","role":"field","resolution":"project","file":"p/Api.java","line":3,"code":"B field"}]})).unwrap());
        let reply = semantic_value(
            &model,
            &SemanticModelParams {
                kind: Some("type_uses".into()),
                language: Some("java".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(reply["items"][0]["role"], "field");
        assert!(reply.to_string().len() <= 4096);
        let reply = semantic_value(
            &model,
            &SemanticModelParams {
                kind: Some("cohesion".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(reply["items"][0]["status"], "unavailable");
        assert!(reply["items"][0]["lcom4"].is_null());
    }

    #[test]
    fn language_and_scope_filters_keep_incoming_and_outgoing_without_identity_collisions() {
        let params = SemanticModelParams {
            language: Some("csharp".into()),
            within: Some("p".into()),
            ..Default::default()
        };
        let reply = semantic_value(&model(), &params).unwrap();
        assert_eq!(reply["status"], "partial");
        assert_eq!(reply["total"], 2);
        let rows = reply["items"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["language"] == "csharp"));
        assert!(rows.iter().any(|row| row["from_class"] == "csharp:p.Api"));
        assert!(rows.iter().any(|row| row["to_class"] == "csharp:p.Api"));
        assert!(
            rows.iter()
                .all(|row| row["line"] == 3 && row["code"] == "other.run()")
        );
        assert_eq!(reply["totals"]["classes"], 4);
        let classes = semantic_value(
            &model(),
            &SemanticModelParams {
                kind: Some("classes".into()),
                ..params.clone()
            },
        )
        .unwrap();
        assert_eq!(classes["total"], 1);
        assert_eq!(classes["items"][0]["id"], "csharp:p.Api");
        let methods = semantic_value(
            &model(),
            &SemanticModelParams {
                kind: Some("methods".into()),
                ..params
            },
        )
        .unwrap();
        assert_eq!(methods["total"], 1);
        assert_eq!(methods["items"][0]["class_id"], "csharp:p.Api");
        let boundary = semantic_value(
            &model(),
            &SemanticModelParams {
                within: Some("p/A".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            boundary["total"], 0,
            "scope uses path boundaries, not arbitrary string prefixes"
        );
    }

    #[test]
    fn response_guard_advances_by_returned_rows_even_when_a_row_is_too_large() {
        let mut model = model();
        model.calls[0].code = "large source ".repeat(1000);
        let mut offset = 0;
        let mut seen = 0;
        let mut omitted = 0;
        loop {
            let reply = semantic_value(
                &model,
                &SemanticModelParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(reply.to_string().len() <= tools::MAX_RESPONSE_BYTES);
            assert_eq!(reply["total"], 4);
            let rows = reply["items"].as_array().unwrap();
            assert!(!rows.is_empty());
            seen += rows.len();
            omitted += rows.iter().filter(|row| row["omitted"] == true).count();
            match reply["next_offset"].as_u64() {
                Some(next) => {
                    assert_eq!(next as usize, offset + rows.len());
                    offset = next as usize;
                }
                None => break,
            }
        }
        assert_eq!(seen, 4);
        assert_eq!(omitted, 1);
        let beyond = semantic_value(
            &model,
            &SemanticModelParams {
                offset: Some(usize::MAX),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(beyond["items"], json!([]));
        assert!(beyond["next_offset"].is_null());
        assert!(
            validate(&SemanticModelParams {
                limit: Some(0),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate(&SemanticModelParams {
                kind: Some("unknown".into()),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn unavailable_coverage_and_large_metadata_are_explicit_and_bounded() {
        let mut model = model();
        model.classes.clear();
        model.methods.clear();
        model.calls.clear();
        model.status = "unavailable".into();
        model.analyzers[0].language = "python".into();
        model.analyzers[0].status = "unavailable".into();
        model.analyzers[0].backend = "none".into();
        model.analyzers[0].diagnostics = vec!["Unsupported provider ".repeat(1000)];
        model.diagnostics = model.analyzers[0].diagnostics.clone();
        model.analyzers.truncate(1);
        let reply = semantic_value(&model, &SemanticModelParams::default()).unwrap();
        assert_eq!(reply["status"], "unavailable");
        assert_eq!(reply["analyzers"][0]["language"], "python");
        assert_eq!(reply["analyzers"][0]["status"], "unavailable");
        assert_eq!(reply["total"], 0);
        assert_eq!(reply["metadata_truncated"], true);
        assert!(reply.to_string().len() <= tools::MAX_RESPONSE_BYTES);
    }

    #[tokio::test]
    async fn semantic_handler_rejects_escape_before_analyzing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let handler = ArchguardTools::new(root);
        let error = handler
            .semantic_model(Parameters(SemanticModelParams {
                path: Some("../outside".into()),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert!(error.message.contains("outside the server root"));
    }

    #[tokio::test]
    async fn semantic_handler_reports_unsupported_language_with_effective_source_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.py"),
            "def run():\n    print('hello')\n",
        )
        .unwrap();
        let handler = ArchguardTools::new(dir.path().into());
        let reply: Value = serde_json::from_str(
            &handler
                .semantic_model(Parameters(SemanticModelParams::default()))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(reply["status"], "unavailable");
        assert_eq!(reply["analyzers"][0]["language"], "python");
        assert_eq!(reply["total"], 0);
    }
}
