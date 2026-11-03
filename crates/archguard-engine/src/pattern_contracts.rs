//! Bounded, local AsyncAPI projection. No server/URL contact or cross-document joins.
use super::model::{PatternAssessment, add, assessment, evidence};
use archguard_core::Config;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
const MAX_BYTES: u64 = 2 * 1024 * 1024;

fn paths(config: &Config) -> Vec<String> {
    let mut paths = vec![
        "asyncapi.json".into(),
        "asyncapi.yaml".into(),
        "asyncapi.yml".into(),
    ];
    paths.extend(config.architecture.asyncapi.iter().cloned());
    paths.sort();
    paths.dedup();
    paths
}
fn read(root: &Path, path: &str) -> Result<Vec<u8>, String> {
    let normalized = crate::evidence_import::relative(path).map_err(|e| e.to_string())?;
    crate::security::read_repo_file(root, Path::new(&normalized), MAX_BYTES)
        .map_err(|e| e.to_string())
}
pub fn pattern_inputs_fingerprint(root: &Path, config: &Config) -> String {
    let records: Vec<_> = paths(config)
        .into_iter()
        .take(32)
        .map(|path| {
            let value = read(root, &path)
                .map(|b| crate::evidence_import::hash(&b))
                .unwrap_or_else(|e| format!("unavailable:{e}"));
            (path, value)
        })
        .collect();
    crate::evidence_import::hash(serde_json::to_string(&json!({"definition":"asyncapi-patterns/v1","patterns":super::PATTERN_DEFINITION,"configured":config.architecture.asyncapi,"records":records})).unwrap().as_bytes())
}
fn resolve<'a>(document: &'a Value, mut value: &'a Value) -> Result<&'a Value, String> {
    let mut visited = BTreeSet::new();
    for _ in 0..16 {
        let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
            return Ok(value);
        };
        let pointer=reference.strip_prefix("#/").map(|s|format!("/{s}")).ok_or("Only local JSON-pointer references are supported; remote/file references are not fetched")?;
        if !visited.insert(pointer.clone()) {
            return Err("Cyclic AsyncAPI reference".into());
        }
        value = document
            .pointer(&pointer)
            .ok_or("Unresolved AsyncAPI reference")?;
    }
    Err("AsyncAPI reference depth exceeds 16".into())
}
fn pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
pub(super) fn assess(root: &Path, config: &Config) -> (PatternAssessment, String, Vec<String>) {
    let mut p = assessment(
        "event_driven",
        ".",
        "messaging_contract",
        &[
            "AsyncAPI describes messaging operations, which may carry an event, command, request or response. Event semantics and runtime behavior remain unverified.",
            "No producer/consumer or service connection is inferred across documents, even when channel addresses match. Broker identity, implementation mapping and delivery behavior are unknown.",
        ],
    );
    let mut diagnostics = vec![];
    let mut send = 0;
    let mut receive = 0;
    let mut documents = 0;
    let mut unknown = 0;
    let mut examined = 0;
    let selected = paths(config);
    if selected.len() > 32 {
        diagnostics
            .push("AsyncAPI document limit: only the first 32 sorted paths are read.".into());
    }
    for path in selected.into_iter().take(32) {
        // Missing automatic candidates are normal. Explicitly configured paths
        // must be reported even if missing. Never follow symlinks to decide this.
        if !config.architecture.asyncapi.contains(&path)
            && root.join(&path).symlink_metadata().is_err()
        {
            continue;
        }
        let document: Value = match read(root, &path)
            .and_then(|bytes| serde_yaml_ng::from_slice(&bytes).map_err(|e| e.to_string()))
        {
            Ok(d) => d,
            Err(error) => {
                diagnostics.push(format!("{path}: {error}"));
                continue;
            }
        };
        let version = document["asyncapi"].as_str().unwrap_or("");
        if !version.starts_with("3.0.") && !version.starts_with("2.") {
            diagnostics.push(format!(
                "{path}: unsupported AsyncAPI version; supported projections are 2.x and 3.0.x."
            ));
            continue;
        }
        documents += 1;
        let mut operations = vec![];
        if version.starts_with("3.") {
            if let Some(items) = document["operations"].as_object() {
                for (id, raw) in items {
                    let result = resolve(&document, raw).and_then(|op| {
                        let action = op["action"]
                            .as_str()
                            .filter(|s| matches!(*s, "send" | "receive"))
                            .ok_or("Missing or invalid operation action")?;
                        let channel_ref = op["channel"]["$ref"]
                            .as_str()
                            .filter(|r| r.starts_with("#/channels/"))
                            .ok_or("Operation channel needs a local #/channels reference")?;
                        let channel = resolve(&document, &op["channel"])?;
                        Ok((
                            action.to_owned(),
                            channel["address"].as_str().map(str::to_owned),
                            format!("/operations/{}", pointer(id)),
                            channel_ref.to_owned(),
                        ))
                    });
                    match result {
                        Ok(op) => operations.push(op),
                        Err(error) => {
                            diagnostics.push(format!("{path} /operations/{}: {error}", pointer(id)))
                        }
                    }
                }
            }
        } else if let Some(channels) = document["channels"].as_object() {
            for (address, raw) in channels {
                let channel = match resolve(&document, raw) {
                    Ok(c) => c,
                    Err(error) => {
                        diagnostics.push(format!("{path}: {error}"));
                        continue;
                    }
                };
                // AsyncAPI 2 uses the peer's perspective: publish -> receive,
                // subscribe -> send. AsyncAPI 3 names application actions.
                for (key, action) in [("publish", "receive"), ("subscribe", "send")] {
                    if let Some(raw) = channel.get(key) {
                        match resolve(&document, raw) {
                            Ok(op) if op.is_object() => operations.push((
                                action.into(),
                                Some(address.clone()),
                                format!("/channels/{}/{key}", pointer(address)),
                                format!("#/channels/{}", pointer(address)),
                            )),
                            _ => diagnostics.push(format!(
                                "{path}: invalid or unresolved {key} operation reference"
                            )),
                        }
                    }
                }
            }
        }
        for (action, address, location, channel) in operations.into_iter().take(2001) {
            examined += 1;
            if examined > 2000 {
                diagnostics.push(
                    "AsyncAPI operation limit: only the first 2000 operations are projected."
                        .into(),
                );
                break;
            }
            if action == "send" {
                send += 1;
            } else {
                receive += 1;
            }
            if address.is_none() {
                unknown += 1;
            }
            let mut item = evidence(
                "message_operation",
                None,
                None,
                Some(&path),
                None,
                &format!(
                    "Application action: {action}; channel: {}; address: {}. Declared contract, not runtime traffic.",
                    channel,
                    address.as_deref().unwrap_or("unknown")
                ),
            );
            item.pointer = Some(location);
            add(&mut p, item, false);
        }
        if examined > 2000 {
            break;
        }
    }
    if send + receive > 0 {
        p.status = "candidate".into();
    }
    if unknown > 0 {
        p.missing_evidence.push(format!("{unknown} operation channels have unknown addresses; channel keys are not substituted for addresses."));
    }
    if documents == 0 {
        p.missing_evidence.push("No supported AsyncAPI document read. Add root asyncapi.yaml/yml/json or architecture.asyncapi paths to inspect messaging contracts.".into());
    }
    let total = diagnostics.len();
    diagnostics.truncate(100);
    if total > 100 {
        diagnostics.push(format!(
            "{} additional contract diagnostics omitted",
            total - 100
        ));
    }
    p.metrics = json!({"documents":documents,"send_operations":send,"receive_operations":receive,"unknown_addresses":unknown,"diagnostics_total":total});
    (
        p,
        if !diagnostics.is_empty() {
            "partial"
        } else if documents > 0 {
            "declared_contracts"
        } else {
            "not_found"
        }
        .into(),
        diagnostics,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_pattern_rules_invalidate_legacy_review_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::default();
        let records: Vec<_> = paths(&config)
            .into_iter()
            .map(|path| {
                std::fs::write(root.path().join(&path), "{}").unwrap();
                (path, crate::evidence_import::hash(b"{}"))
            })
            .collect();
        let legacy = crate::evidence_import::hash(
            serde_json::to_string(&json!({
                "definition":"asyncapi-patterns/v1", "configured":[], "records":records
            }))
            .unwrap()
            .as_bytes(),
        );
        assert_ne!(
            pattern_inputs_fingerprint(root.path(), &config),
            legacy,
            "saved v1 reviews must not reuse a fingerprint that omits the new detector rules"
        );
    }
}
