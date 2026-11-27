//! Maven modules that make up one application, for the Spring central rules.
//!
//! A `SecurityFilterChain` often lives in a starter module while the
//! controllers it protects live in a module that starter depends on. Only the
//! literal `<dependencies>` of each `pom.xml` are read (no test scope, no
//! imports, no Gradle): an application is a module nothing depends on, made of
//! everything it reaches through those dependencies.

use archguard_flow_common::extract_ctx::read_text;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) struct Modules {
    /// Project directory → the project directories it depends on.
    deps: BTreeMap<String, BTreeSet<String>>,
    /// Modules with `<packaging>pom</packaging>`: aggregators and bundles, no code.
    pom_only: BTreeSet<String>,
}

/// `(artifactId, dependency artifactIds)` of a `pom.xml` text.
fn read_pom(text: &str) -> Option<(String, Vec<String>, bool)> {
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
        .filter(|d| !matches!(child(*d, "scope").as_deref(), Some("test" | "provided")))
        .filter(|d| child(*d, "optional").as_deref() != Some("true"))
        .filter_map(|d| child(d, "artifactId"))
        .collect();
    let pom = child(project, "packaging").as_deref() == Some("pom");
    Some((artifact, deps, pom))
}

impl Modules {
    /// The dependencies between the build-file directories `dirs` of `root`;
    /// an artifactId that two modules declare resolves to nothing.
    pub(crate) fn read<'a>(
        root: &Path,
        dirs: impl Iterator<Item = &'a String>,
        unread: &mut dyn FnMut(&str, String),
    ) -> Self {
        let mut poms = Vec::new();
        for dir in dirs {
            let rel = if dir.is_empty() {
                "pom.xml".to_string()
            } else {
                format!("{dir}/pom.xml")
            };
            let text = match read_text(root, &rel, MAX_BYTES) {
                Ok(text) => text,
                // Gradle-only directories have no pom.xml.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    unread(
                        &rel,
                        format!("pom.xml not read: {e}; its module dependencies are unknown."),
                    );
                    continue;
                }
            };
            if let Some((artifact, deps, pom)) = read_pom(&text) {
                poms.push((dir.clone(), artifact, deps, pom));
            }
        }
        let mut dir_of: BTreeMap<&str, Option<&String>> = BTreeMap::new();
        for (dir, artifact, ..) in &poms {
            let slot = dir_of.entry(artifact).or_insert(Some(dir));
            if slot.is_some_and(|d| d != dir) {
                *slot = None;
            }
        }
        let deps = (poms.iter())
            .map(|(dir, _, deps, _)| {
                let found = deps
                    .iter()
                    .filter_map(|a| (*dir_of.get(a.as_str())?).cloned());
                (dir.clone(), found.filter(|d| d != dir).collect())
            })
            .collect();
        let pom_only = (poms.iter().filter(|p| p.3)).map(|p| p.0.clone()).collect();
        Modules { deps, pom_only }
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
        let code = self
            .deps
            .iter()
            .filter(|(d, _)| !self.pom_only.contains(*d));
        let depended: BTreeSet<&String> = code.flat_map(|(_, v)| v).collect();
        (self.deps.keys())
            .filter(|r| !depended.contains(r) && !self.pom_only.contains(*r))
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
                (
                    "report".into(),
                    BTreeSet::from(["app".to_string(), "web".to_string()]),
                ),
            ]),
            pom_only: BTreeSet::from(["report".to_string()]),
        };
        let apps = m.applications("web");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0], ["app", "sec", "web"].map(String::from).into());
        assert_eq!(m.applications("app"), apps);
        assert!(m.applications("lone").is_empty());
    }

    #[test]
    fn a_pom_packaging_is_flagged() {
        let pom = "<project><artifactId>r</artifactId><packaging>pom</packaging></project>";
        assert_eq!(read_pom(pom), Some(("r".into(), vec![], true)));
    }

    #[test]
    fn a_pom_gives_its_artifact_and_non_test_dependencies() {
        let pom = "<project><artifactId>a</artifactId><dependencies>\
            <dependency><artifactId>b</artifactId></dependency>\
            <dependency><artifactId>t</artifactId><scope>test</scope></dependency>\
            <dependency><artifactId>o</artifactId><optional>true</optional></dependency>\
            <dependency><artifactId>p</artifactId><scope>provided</scope></dependency>\
            </dependencies></project>";
        assert_eq!(
            read_pom(pom),
            Some(("a".into(), vec!["b".to_string()], false))
        );
    }
}
