//! The service of each file the Domain view shows: the folder of the
//! nearest build file (`*.csproj`, `pom.xml`, `build.gradle[.kts]`), the
//! rule the engine applies to publish sites and message handlers (ADR
//! 0022, 2026-11-20). Without one, the first path segment.
//!
//! shortcut: the engine's `domain_csharp::code::project_of` is crate
//! private, so this repeats its walk; use it once the engine exports it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_engine::core::UiFlow;
use archguard_engine::core::domain_model::DomainModel;

const BUILD: [&str; 4] = [".csproj", "pom.xml", "build.gradle", "build.gradle.kts"];

#[derive(Debug, Default, Clone)]
pub struct Services {
    by_file: BTreeMap<String, String>,
    /// Every service that owns something shown, sorted.
    pub names: Vec<String>,
}

impl Services {
    /// The service of `file`; `""` for a file not looked up.
    pub fn of(&self, file: &str) -> &str {
        self.by_file.get(file).map_or("", String::as_str)
    }

    /// The services of the model's and the flow's files, looked up under
    /// `root`; publishers and message handlers keep the engine's service.
    pub fn find(root: &Path, d: &DomainModel, flow: Option<&UiFlow>) -> Services {
        let mut files: BTreeSet<&str> = BTreeSet::new();
        files.extend(d.entities.iter().map(|e| e.file.as_str()));
        files.extend(d.use_cases.iter().map(|u| u.file.as_str()));
        files.extend(d.rules.iter().map(|r| r.file.as_str()));
        files.extend(d.validations.iter().map(|v| v.file.as_str()));
        files.extend(d.event_handler_sites.iter().map(|h| h.file.as_str()));
        let transitions = d.state_machines.iter().flat_map(|m| &m.transitions);
        files.extend(transitions.map(|t| t.file.as_str()));
        files.extend(
            flow.into_iter()
                .flat_map(|f| &f.endpoints)
                .map(|e| e.file.as_str()),
        );
        let mut cache = BTreeMap::new();
        let mut by_file: BTreeMap<String, String> = (files.into_iter())
            .map(|f| (f.to_string(), project_of(root, f, &mut cache)))
            .collect();
        let publishers = d.integration_events.iter().flat_map(|e| &e.published_by);
        for (file, service) in publishers
            .map(|p| (&p.file, &p.service))
            .chain(d.message_handlers.iter().map(|h| (&h.file, &h.service)))
            .filter(|(_, s)| !s.is_empty())
        {
            by_file.insert(file.clone(), service.clone());
        }
        let owners = (d.entities.iter().map(|e| &e.file))
            .chain(d.use_cases.iter().map(|u| &u.file))
            .chain(d.message_handlers.iter().map(|h| &h.file))
            .chain(publishers_files(d));
        let names: BTreeSet<String> = owners
            .filter_map(|f| by_file.get(f).filter(|s| !s.is_empty()).cloned())
            .collect();
        Services {
            by_file,
            names: names.into_iter().collect(),
        }
    }

    #[cfg(test)]
    pub fn of_pairs(pairs: &[(&str, &str)]) -> Services {
        let by_file: BTreeMap<String, String> = (pairs.iter())
            .map(|(f, s)| (f.to_string(), s.to_string()))
            .collect();
        let names: BTreeSet<String> = by_file.values().cloned().collect();
        Services {
            by_file,
            names: names.into_iter().collect(),
        }
    }
}

fn publishers_files(d: &DomainModel) -> impl Iterator<Item = &String> {
    (d.integration_events.iter()).flat_map(|e| e.published_by.iter().map(|p| &p.file))
}

/// The folder name of the nearest build file above `rel`.
fn project_of(root: &Path, rel: &str, cache: &mut BTreeMap<String, String>) -> String {
    let start = rel.rsplit_once('/').map_or("", |(d, _)| d);
    if let Some(known) = cache.get(start) {
        return known.clone();
    }
    let has_build = |dir: &str| {
        std::fs::read_dir(root.join(dir)).is_ok_and(|entries| {
            (entries.flatten()).any(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                BUILD.iter().any(|b| name.ends_with(b))
            })
        })
    };
    let mut dir = start;
    let found = loop {
        if !dir.is_empty() && has_build(dir) {
            break dir.rsplit('/').next().unwrap_or(dir).to_string();
        }
        match dir.rsplit_once('/') {
            Some((up, _)) => dir = up,
            None => {
                break rel
                    .split_once('/')
                    .map_or("", |(first, _)| first)
                    .to_string();
            }
        }
    };
    cache.insert(start.to_string(), found.clone());
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{Entity, EntityKind};

    #[test]
    fn a_file_belongs_to_the_folder_of_its_nearest_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let put = |rel: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).expect("dir");
            std::fs::write(p, "x").expect("file");
        };
        put("src/Ordering.Domain/Ordering.Domain.csproj");
        put("src/Ordering.Domain/Aggregates/Order.cs");
        put("loose/Thing.cs");
        let entity = |file: &str| Entity {
            id: file.into(),
            name: file.into(),
            kind: EntityKind::Entity,
            fields: vec![],
            file: file.into(),
            line: 1,
            source: "t".into(),
            module: String::new(),
        };
        let d = DomainModel {
            entities: vec![
                entity("src/Ordering.Domain/Aggregates/Order.cs"),
                entity("loose/Thing.cs"),
            ],
            ..DomainModel::default()
        };
        let s = Services::find(dir.path(), &d, None);
        assert_eq!(
            s.of("src/Ordering.Domain/Aggregates/Order.cs"),
            "Ordering.Domain"
        );
        assert_eq!(s.of("loose/Thing.cs"), "loose", "no project: first segment");
        assert_eq!(s.names, ["Ordering.Domain", "loose"]);
        assert_eq!(s.of("elsewhere.cs"), "");
    }
}
