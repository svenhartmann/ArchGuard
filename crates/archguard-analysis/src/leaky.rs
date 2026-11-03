//! Leaky encapsulation (R17).
//!
//! A file references a symbol from another file that does not export it
//! (the symbol is only in the target's internal set). The visibility
//! input is produced by the language scanners in `archguard-extract`.
//!
//! Evidence-based rule: a reference exists when the import site (`Ref::raw`)
//! mentions the symbol as a whole identifier. Files without visibility data
//! make no claim at all — silence means "not measured", never "clean".

use std::collections::BTreeMap;

use archguard_core::{Graph, Leak, Visibility};

/// References that reach past a file's exported surface.
///
/// Deterministic: results sorted by `(from_file, in_file, symbol)`,
/// evidence sorted and deduplicated. Self-references (a file reaching
/// into its own internals) are not leaks.
pub fn leaky_encapsulation(graph: &Graph, visibility: &[Visibility]) -> Vec<Leak> {
    let by_file: BTreeMap<&str, &Visibility> = visibility
        .iter()
        .map(|entry| (entry.file.as_str(), entry))
        .collect();
    let mut leaks: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for edge in &graph.edges {
        let (Some(from), Some(to)) = (graph.node_name(edge.from), graph.node_name(edge.to)) else {
            continue;
        };
        if from == to {
            continue; // own internals are none of the analyzer's business
        }
        let Some(target) = by_file.get(to) else {
            continue; // no visibility data for the target: no claim
        };
        for symbol in &target.internal {
            if target.exported.contains(symbol) {
                continue; // exported everywhere, nothing leaked
            }
            let mut evidence: Vec<String> = Vec::new();
            for reference in &edge.refs {
                if mentions_identifier(&reference.raw, symbol) {
                    evidence.push(format!("{}:{}", reference.file, reference.line));
                }
            }
            if !evidence.is_empty() {
                leaks
                    .entry((from.to_string(), to.to_string(), symbol.clone()))
                    .or_default()
                    .extend(evidence);
            }
        }
    }
    leaks
        .into_iter()
        .map(|((from_file, in_file, symbol), mut evidence)| {
            evidence.sort();
            evidence.dedup();
            Leak {
                from_file,
                symbol,
                in_file,
                evidence,
            }
        })
        .collect()
}

/// True when `text` contains `symbol` as a whole identifier.
fn mentions_identifier(text: &str, symbol: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = symbol.as_bytes();
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(offset) = text.as_bytes()[from..]
        .windows(needle.len())
        .position(|w| w == needle)
    {
        let start = from + offset;
        let end = start + needle.len();
        let boundary_before = start == 0 || !is_ident_byte(bytes[start - 1]);
        let boundary_after = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if boundary_before && boundary_after {
            return true;
        }
        from = end;
    }
    false
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;

    fn visibility(file: &str, exported: &[&str], internal: &[&str]) -> Visibility {
        Visibility {
            file: file.to_string(),
            exported: exported.iter().map(|s| s.to_string()).collect(),
            internal: internal.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn access_to_an_internal_symbol_is_a_leak_with_evidence() {
        let mut g = graph(&["src/a.ts", "src/b.ts"], &[("src/a.ts", "src/b.ts")]);
        g.edges[0].refs.push(archguard_core::Ref {
            file: "src/a.ts".to_string(),
            line: 3,
            raw: "import { Thing, Secret } from './b';".to_string(),
        });
        let visibility = vec![visibility("src/b.ts", &["Thing"], &["Thing", "Secret"])];
        let leaks = leaky_encapsulation(&g, &visibility);
        assert_eq!(leaks.len(), 1, "only the non-exported symbol leaks");
        assert_eq!(leaks[0].from_file, "src/a.ts");
        assert_eq!(leaks[0].in_file, "src/b.ts");
        assert_eq!(leaks[0].symbol, "Secret");
        assert_eq!(leaks[0].evidence, ["src/a.ts:3"]);
    }

    #[test]
    fn exported_access_is_clean() {
        let mut g = graph(&["src/a.ts", "src/b.ts"], &[("src/a.ts", "src/b.ts")]);
        g.edges[0].refs.push(archguard_core::Ref {
            file: "src/a.ts".to_string(),
            line: 1,
            raw: "import { Thing } from './b';".to_string(),
        });
        let visibility = vec![visibility("src/b.ts", &["Thing"], &["Thing"])];
        assert!(leaky_encapsulation(&g, &visibility).is_empty());
    }

    #[test]
    fn underscore_prefixed_python_helpers_leak_when_reached() {
        let mut g = graph(&["a.py", "b.py"], &[("a.py", "b.py")]);
        g.edges[0].refs.push(archguard_core::Ref {
            file: "a.py".to_string(),
            line: 2,
            raw: "from b import _helper".to_string(),
        });
        let visibility = vec![visibility("b.py", &[], &["_helper"])];
        let leaks = leaky_encapsulation(&g, &visibility);
        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].symbol, "_helper");
    }

    #[test]
    fn identifier_boundaries_prevent_substring_false_positives() {
        let mut g = graph(&["a.ts", "b.ts"], &[("a.ts", "b.ts")]);
        g.edges[0].refs.push(archguard_core::Ref {
            file: "a.ts".to_string(),
            line: 1,
            raw: "import { Secrets } from './b'; // SecretVault too".to_string(),
        });
        let visibility = vec![visibility("b.ts", &[], &["Secret"])];
        assert!(
            leaky_encapsulation(&g, &visibility).is_empty(),
            "`Secrets`/`SecretVault` are not `Secret`"
        );
    }

    #[test]
    fn self_reference_and_unknown_visibility_make_no_claim() {
        // a → a touching its own internals is fine.
        let mut g = graph(&["a.ts"], &[("a.ts", "a.ts")]);
        g.edges[0].refs.push(archguard_core::Ref {
            file: "a.ts".to_string(),
            line: 1,
            raw: "const x = _internal;".to_string(),
        });
        let own = visibility("a.ts", &[], &["_internal"]);
        assert!(leaky_encapsulation(&g, std::slice::from_ref(&own)).is_empty());
        // A target without any visibility entry: not measured, not clean.
        let mut g2 = graph(&["a.ts", "b.ts"], &[("a.ts", "b.ts")]);
        g2.edges[0].refs.push(archguard_core::Ref {
            file: "a.ts".to_string(),
            line: 1,
            raw: "import { _x } from './b';".to_string(),
        });
        assert!(leaky_encapsulation(&g2, &[own]).is_empty());
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&Graph, &[Visibility]) -> Vec<Leak> = leaky_encapsulation;
        let _ = f;
    }
}
