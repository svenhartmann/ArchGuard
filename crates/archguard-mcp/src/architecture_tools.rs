//! A target model can contain many nested source mappings. Preserve exact
//! coverage totals while marking every detail list shortened for MCP transport.
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
                if key == "omitted_arrays" {
                    continue;
                }
                let escaped = key.replace('~', "~0").replace('/', "~1");
                largest_array(value, &format!("{path}/{escaped}"), best);
            }
        }
        _ => {}
    }
}
pub(crate) fn context_value(context: &ArchitectureContext) -> Value {
    if context.observed.target_model.is_none() {
        return crate::tools::guarded(context);
    }
    let mut value = serde_json::to_value(context).expect("architecture context serializes");
    if bytes(&value) <= crate::tools::MAX_RESPONSE_BYTES {
        return value;
    }
    value["truncated"] = true.into();
    value["omitted_arrays"] = json!([]);
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
    let target = context.observed.target_model.as_ref().unwrap();
    json!({"schema_version":context.schema_version,"truncated":true,
        "omitted":"context_exceeds_response_limit",
        "hint":"Use archguard architecture --format json and archguard config for complete model and intent.",
        "observed":{"summary":context.observed.summary,"source_files":context.observed.source_files,"findings_total":context.observed.findings_total,
            "target_model":{"status":target.status,"source_nodes":target.source_nodes,"mapped_nodes":target.mapped_nodes,"unmapped_nodes":target.unmapped_nodes,"ambiguous_nodes":target.ambiguous_nodes,"violating_dependencies":target.violating_dependencies,"components_total":target.components.len(),"dependencies_total":target.dependencies.len(),"unclassified_edges":target.unclassified_edges}},
        "intent_totals":{"goals":context.intent.goals.len(),"constraints":context.intent.constraints.len(),"decisions":context.intent.decisions.len()}})
}
