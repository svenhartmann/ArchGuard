//! Labels of the Matrix's top rows and columns: the path below the
//! segments all of them share, without generic source roots, unique
//! within the list. The same rule names the arc42 building blocks
//! (`architecture_ui`, branch `fix/arc42-block-names`); the two are meant
//! to share one implementation once both are merged.

use std::collections::{BTreeMap, HashMap};

use crate::tree::Tree;

/// Segments that name a source layout, not a component: `src` (Cargo,
/// Python, Maven/Gradle), `main` and `java` (`src/main/java`), `lib`
/// (Ruby, Dart, Elixir), `source`/`sources` (C/C++, SwiftPM). Compared
/// without case.
const SOURCE_ROOTS: [&str; 6] = ["src", "lib", "main", "java", "source", "sources"];

/// Label per node of `roots`, from their full names.
pub fn root_labels(tree: &Tree, roots: &[usize]) -> BTreeMap<usize, String> {
    let names: Vec<&str> = roots.iter().map(|&r| tree.nodes[r].name.as_str()).collect();
    roots.iter().copied().zip(short_names(&names)).collect()
}

/// One label per name, in input order; equal names get equal labels.
pub fn short_names(names: &[&str]) -> Vec<String> {
    let parts: Vec<Vec<&str>> = names
        .iter()
        .map(|n| n.split('/').filter(|s| !s.is_empty()).collect())
        .collect();
    let common = common_prefix(&parts);
    let tiers: Vec<[String; 3]> = names
        .iter()
        .zip(&parts)
        .map(|(n, p)| tiers(p, common, n))
        .collect();
    resolve(names, &tiers)
        .into_iter()
        .zip(&tiers)
        .map(|(k, t)| t[k].clone())
        .collect()
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

fn is_root(segment: &str) -> bool {
    SOURCE_ROOTS.contains(&segment.to_lowercase().as_str())
}

/// Ever longer candidates: below the common prefix without source roots,
/// with them, the full name.
fn tiers(parts: &[&str], common: usize, name: &str) -> [String; 3] {
    let rest = &parts[common..];
    let specific: Vec<&str> = rest.iter().copied().filter(|s| !is_root(s)).collect();
    let first = if specific.is_empty() {
        // Only source roots below the prefix: the innermost other segment.
        let named = parts.iter().rev().find(|s| !is_root(s));
        named.or(parts.last()).copied().unwrap_or(name).to_string()
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

/// The candidate per name: names whose label collides with a different
/// name's move to their next candidate until all are unique (full names
/// always are).
fn resolve(names: &[&str], tiers: &[[String; 3]]) -> Vec<usize> {
    let mut tier = vec![0; tiers.len()];
    loop {
        let mut owners: HashMap<&str, Vec<&str>> = HashMap::new();
        for ((t, &k), n) in tiers.iter().zip(&tier).zip(names) {
            let v = owners.entry(t[k].as_str()).or_default();
            if !v.contains(n) {
                v.push(n);
            }
        }
        let bump: Vec<usize> = (0..tiers.len())
            .filter(|&i| tier[i] < 2 && owners[tiers[i][tier[i]].as_str()].len() > 1)
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

    #[test]
    fn cargo_workspace_crates_are_named_after_the_crate() {
        let names = [
            "crates/archguard-analysis/src",
            "crates/archguard-cli/src",
            "crates/archguard-cli/build.rs",
            "crates/archguard-engine/src/semantic_calls",
        ];
        assert_eq!(
            short_names(&names),
            [
                "archguard-analysis",
                "archguard-cli",
                "archguard-cli/build.rs",
                "archguard-engine/semantic_calls",
            ]
        );
    }

    #[test]
    fn maven_layouts_drop_src_main_java() {
        let names = [
            "orders/src/main/java/com/shop/orders",
            "billing/src/main/java/com/shop/billing",
        ];
        assert_eq!(
            short_names(&names),
            ["orders/com/shop/orders", "billing/com/shop/billing"]
        );
    }

    #[test]
    fn a_single_source_root_keeps_its_parent() {
        assert_eq!(
            short_names(&["crates/archguard-store/src"]),
            ["archguard-store"]
        );
        assert_eq!(short_names(&["src"]), ["src"]);
    }

    #[test]
    fn colliding_labels_fall_back_to_longer_ones() {
        // Both are "app" without their source root.
        assert_eq!(short_names(&["app/src", "app/lib"]), ["src", "lib"]);
        assert_eq!(
            short_names(&["a/x", "a/x"]),
            ["x", "x"],
            "equal names agree"
        );
    }

    #[test]
    fn unrelated_names_stay_whole() {
        assert_eq!(
            short_names(&["services/catalog", "dev.js"]),
            ["services/catalog", "dev.js"]
        );
    }
}
