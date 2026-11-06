//! A target model can contain many nested source mappings. Preserve exact
//! coverage totals while marking every detail list shortened for MCP transport.
//! Goal assessments keep every goal; only their finding id lists shrink.
use archguard_engine::core::ArchitectureContext;
use serde_json::{Value, json};

fn bytes(value: &Value) -> usize {
    serde_json::to_vec(value).expect("JSON value").len()
}
fn largest_array(value: &Value, path: &str, best: &mut Option<(String, usize)>) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            let size = bytes(value);
            if best.as_ref().is_none_or(|(_, largest)| size > *largest) {
                *best = Some((path.into(), size));
            }
            // A containing array is always larger than its child lists.
        }
        Value::Object(map) => {
            for (key, value) in map {
                // Goal statuses are never cut to a prefix of the goals; only
                // their id lists shrink (shorten_goal_findings).
                if key == "omitted_arrays" || (path.is_empty() && key == "goal_assessment") {
                    continue;
                }
                let escaped = key.replace('~', "~0").replace('/', "~1");
                largest_array(value, &format!("{path}/{escaped}"), best);
            }
        }
        _ => {}
    }
}
/// Finding ids kept per goal once the context exceeds the budget; every
/// goal's status survives, its shortened id list is counted.
const GOAL_FINDING_IDS: usize = 10;
fn shorten_goal_findings(value: &mut Value) {
    let mut omitted = vec![];
    for (index, goal) in value["goal_assessment"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(ids) = goal["findings"].as_array_mut()
            && ids.len() > GOAL_FINDING_IDS
        {
            omitted.push(json!({"path":format!("/goal_assessment/{index}/findings"),"total":ids.len(),"returned":GOAL_FINDING_IDS}));
            ids.truncate(GOAL_FINDING_IDS);
        }
    }
    value["omitted_arrays"]
        .as_array_mut()
        .unwrap()
        .extend(omitted);
}
pub(crate) fn context_value(context: &ArchitectureContext) -> Value {
    let mut value = serde_json::to_value(context).expect("architecture context serializes");
    if bytes(&value) <= crate::tools::MAX_RESPONSE_BYTES {
        return value;
    }
    value["truncated"] = true.into();
    value["omitted_arrays"] = json!([]);
    shorten_goal_findings(&mut value);
    while bytes(&value) > crate::tools::MAX_RESPONSE_BYTES {
        let mut best = None;
        largest_array(&value, "", &mut best);
        let Some((path, _)) = best else {
            break;
        };
        let full = std::mem::take(value.pointer_mut(&path).unwrap().as_array_mut().unwrap());
        let index = value["omitted_arrays"].as_array().unwrap().len();
        value["omitted_arrays"]
            .as_array_mut()
            .unwrap()
            .push(json!({"path":path,"total":full.len(),"returned":0}));
        if bytes(&value) <= crate::tools::MAX_RESPONSE_BYTES {
            let (mut low, mut high) = (0, full.len());
            while low < high {
                let middle = (low + high).div_ceil(2);
                *value.pointer_mut(&path).unwrap() = json!(full[..middle]);
                value["omitted_arrays"][index]["returned"] = middle.into();
                if bytes(&value) <= crate::tools::MAX_RESPONSE_BYTES {
                    low = middle;
                } else {
                    high = middle - 1;
                }
            }
            *value.pointer_mut(&path).unwrap() = json!(full[..low]);
            value["omitted_arrays"][index]["returned"] = low.into();
            return value;
        }
    }
    if bytes(&value) <= crate::tools::MAX_RESPONSE_BYTES {
        return value;
    }
    // Huge scalar intent/identifiers cannot be safely shortened without changing
    // their identity. Return measured numeric metadata rather than forged names.
    let target = context.observed.target_model.as_ref().map(|target| json!({"status":target.status,"source_nodes":target.source_nodes,"mapped_nodes":target.mapped_nodes,"unmapped_nodes":target.unmapped_nodes,"ambiguous_nodes":target.ambiguous_nodes,"violating_dependencies":target.violating_dependencies,"components_total":target.components.len(),"dependencies_total":target.dependencies.len(),"unclassified_edges":target.unclassified_edges}));
    let mut goal_statuses = std::collections::BTreeMap::<&str, usize>::new();
    for goal in &context.goal_assessment {
        *goal_statuses.entry(goal.status.as_str()).or_default() += 1;
    }
    json!({"schema_version":context.schema_version,"truncated":true,
        "omitted":"context_exceeds_response_limit",
        "hint":"Use archguard architecture --format json and archguard config for complete model and intent.",
        "observed":{"summary":context.observed.summary,"source_files":context.observed.source_files,"findings_total":context.observed.findings_total,"target_model":target},
        "goal_status_counts":goal_statuses,
        "intent_totals":{"goals":context.intent.goals.len(),"constraints":context.intent.constraints.len(),"decisions":context.intent.decisions.len()}})
}
