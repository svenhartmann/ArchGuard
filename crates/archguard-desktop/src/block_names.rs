//! Building-block names: the path below the segments all shown blocks
//! share, without generic source roots, unique within the list. The arc42
//! views (`architecture_ui`) and the Matrix's top rows and columns
//! (`matrix_ui`) use it. The same rule as `arc42ShortNames`
//! (viewer_architecture.js); both test the cases in
//! `scripts/viewer/arc42_block_names.json`.

use std::collections::HashMap;

/// Segments that name a source layout, not a block: `src` (Cargo, Python,
/// Maven/Gradle), `main` and `java` (Maven/Gradle `src/main/java`), `lib`
/// (Ruby, Dart, Elixir), `source`/`sources` (C/C++, SwiftPM). Compared
/// without case.
const SOURCE_ROOTS: [&str; 6] = ["src", "lib", "main", "java", "source", "sources"];

/// One label per name, in input order; equal names get equal labels.
pub fn short_names(names: &[&str]) -> Vec<String> {
    let mut unique: Vec<&str> = Vec::new();
    for n in names {
        if !unique.contains(n) {
            unique.push(n);
        }
    }
    let parts: Vec<Vec<&str>> = unique
        .iter()
        .map(|n| n.split('/').filter(|s| !s.is_empty()).collect())
        .collect();
    let common = common_prefix(&parts);
    let tiers: Vec<[String; 3]> = unique
        .iter()
        .zip(&parts)
        .map(|(n, p)| tiers(p, common, n))
        .collect();
    let tier = resolve(&tiers);
    let label: HashMap<&str, &str> = unique
        .iter()
        .zip(&tiers)
        .zip(&tier)
        .map(|((n, t), &k)| (*n, t[k].as_str()))
        .collect();
    names.iter().map(|n| label[n].to_string()).collect()
}

/// Leading segments every name shares, leaving each at least one.
fn common_prefix(parts: &[Vec<&str>]) -> usize {
    let Some(first) = parts.first() else {
        return 0;
    };
    let keep = parts
        .iter()
        .map(Vec::len)
        .min()
        .unwrap_or(0)
        .saturating_sub(1);
    (0..keep)
        .take_while(|&k| parts.iter().all(|p| p[k] == first[k]))
        .count()
}

/// Ever longer candidates: without source roots, with them, the full path.
fn tiers(parts: &[&str], common: usize, name: &str) -> [String; 3] {
    let root = |s: &&str| SOURCE_ROOTS.contains(&s.to_lowercase().as_str());
    let rest = &parts[common..];
    let specific: Vec<&str> = rest.iter().copied().filter(|s| !root(s)).collect();
    let fallback = parts
        .iter()
        .rev()
        .find(|s| !root(s))
        .or(parts.last())
        .copied()
        .unwrap_or(name);
    let first = if specific.is_empty() {
        fallback.to_string()
    } else {
        specific.join("/")
    };
    let all = if rest.is_empty() {
        name.to_string()
    } else {
        rest.join("/")
    };
    [first, all, name.to_string()]
}

/// The candidate per name: colliding names move to their next candidate
/// until every label is unique (full paths always are).
fn resolve(tiers: &[[String; 3]]) -> Vec<usize> {
    let mut tier = vec![0; tiers.len()];
    loop {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (t, &k) in tiers.iter().zip(&tier) {
            *seen.entry(t[k].as_str()).or_default() += 1;
        }
        let bump: Vec<usize> = (0..tiers.len())
            .filter(|&i| tier[i] < 2 && seen[tiers[i][tier[i]].as_str()] > 1)
            .collect();
        if bump.is_empty() {
            return tier;
        }
        for i in bump {
            tier[i] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &serde_json::Value) -> Vec<&str> {
        v.as_array()
            .expect("list")
            .iter()
            .map(|s| s.as_str().expect("string"))
            .collect()
    }

    /// The cases the web viewer's `test_viewer_arc42_names.js` checks too.
    #[test]
    fn shared_cases_match_the_web_viewer() {
        let text = include_str!("../../../scripts/viewer/arc42_block_names.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(text).expect("cases");
        assert!(cases.len() > 5);
        for c in &cases {
            let names = strings(&c["names"]);
            assert_eq!(short_names(&names), strings(&c["labels"]), "{}", c["case"]);
        }
    }

    #[test]
    fn equal_names_share_a_label_and_empty_lists_work() {
        assert_eq!(
            short_names(&["crates/a/src", "crates/a/src", "crates/b/src"]),
            vec!["a", "a", "b"]
        );
        assert!(short_names(&[]).is_empty());
    }
}
