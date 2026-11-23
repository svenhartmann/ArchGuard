//! Maven modules that make up one application, for the Spring central rules.
//!
//! A `SecurityFilterChain` often lives in a starter module while the
//! controllers it protects live in a module that starter depends on. Only the
//! literal `<dependencies>` of each `pom.xml` are read (no test scope, no
//! imports, no Gradle): an application is a module nothing depends on, made of
//! everything it reaches through those dependencies.

use crate::extract_ctx::read_text;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) struct Modules {
    /// Project directory → the project directories it depends on.
    deps: BTreeMap<String, BTreeSet<String>>,
}

/// `(artifactId, dependency artifactIds)` of a `pom.xml` text.
fn read_pom(text: &str) -> Option<(String, Vec<String>)> {
    let doc = roxmltree::Document::parse(text).ok()?;
    let project = doc.root_element();
    let child = |n: roxmltree::Node<'_, '_>, tag: &str| {
        (n.children().find(|c| c.has_tag_name(tag)))
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
    };
    let artifact = child(project, "artifactId")?;
    let deps = (project.children().find(|c| c.has_tag_name("dependencies")))
        .into_iter()
        .flat_map(|d| d.children().filter(|c| c.has_tag_name("dependency")))
        .filter(|d| child(*d, "scope").as_deref() != Some("test"))
        .filter_map(|d| child(d, "artifactId"))
        .collect();
    Some((artifact, deps))
}

impl Modules {
    /// The dependencies between the build-file directories `dirs` of `root`;
    /// an artifactId that two modules declare resolves to nothing.
    pub(crate) fn read<'a>(root: &Path, dirs: impl Iterator<Item = &'a String>) -> Self {
        let mut poms = Vec::new();
        for dir in dirs {
            let rel = if dir.is_empty() {
                "pom.xml".to_string()
            } else {
                format!("{dir}/pom.xml")
            };
            if let Some((artifact, deps)) = read_text(root, &rel, MAX_BYTES)
                .ok()
                .and_then(|t| read_pom(&t))
            {
                poms.push((dir.clone(), artifact, deps));
            }
        }
        let mut dir_of: BTreeMap<&str, Option<&String>> = BTreeMap::new();
        for (dir, artifact, _) in &poms {
            let slot = dir_of.entry(artifact).or_insert(Some(dir));
            if slot.is_some_and(|d| d != dir) {
                *slot = None;
            }
        }
        let deps = (poms.iter())
            .map(|(dir, _, deps)| {
                let found = deps
                    .iter()
                    .filter_map(|a| (*dir_of.get(a.as_str())?).cloned());
                (dir.clone(), found.filter(|d| d != dir).collect())
            })
            .collect();
        Modules { deps }
    }

    /// Every module reachable from `dir` through dependencies, `dir` included.
    fn closure(&self, dir: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::from([dir.to_string()]);
        let mut todo = vec![dir.to_string()];
        while let Some(d) = todo.pop() {
            for n in self.deps.get(&d).into_iter().flatten() {
                if seen.insert(n.clone()) {
                    todo.push(n.clone());
                }
            }
        }
        seen
    }

    /// The applications that include `dir`: for each module that nothing
    /// depends on and that reaches `dir` (or is `dir`), all its modules. Empty
    /// when `dir` stands alone.
    pub(crate) fn applications(&self, dir: &str) -> Vec<BTreeSet<String>> {
        let depended: BTreeSet<&String> = self.deps.values().flatten().collect();
        (self.deps.keys())
            .filter(|r| !depended.contains(r))
            .map(|r| self.closure(r))
            .filter(|c| c.contains(dir) && c.len() > 1)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_nothing_depends_on_is_an_application_of_its_dependencies() {
        let m = Modules {
            deps: BTreeMap::from([
                ("web".into(), BTreeSet::new()),
                ("sec".into(), BTreeSet::from(["web".to_string()])),
                ("app".into(), BTreeSet::from(["sec".to_string()])),
                ("lone".into(), BTreeSet::new()),
            ]),
        };
        let apps = m.applications("web");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0], ["app", "sec", "web"].map(String::from).into());
        assert_eq!(m.applications("app"), apps);
        assert!(m.applications("lone").is_empty());
    }

    #[test]
    fn a_pom_gives_its_artifact_and_non_test_dependencies() {
        let pom = "<project><artifactId>a</artifactId><dependencies>\
            <dependency><artifactId>b</artifactId></dependency>\
            <dependency><artifactId>t</artifactId><scope>test</scope></dependency>\
            </dependencies></project>";
        assert_eq!(read_pom(pom), Some(("a".into(), vec!["b".to_string()])));
    }
}
