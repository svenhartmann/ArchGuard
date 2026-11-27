//! Every detected-but-unreadable input is a gap, never a silent skip (ticket 34).
//! Each `fixtures/es-classdiagram/*/unread/` directory is analyzed as its own root; every file in it
//! (nested paths allowed, for readers that need a directory shape) must yield a diagnostic naming it (an Elasticsearch gap, or the diagnostic of the shared parser, e.g. Terraform) and
//! no artifact. Files with `unreadable` in their path are served as unreadable by the reader callback.
//! Each top-level directory is registered as a service (it owns the files below it). A file `CONTEXT`
//! in the `unread/` directory lists, one path per line, support files (pom.xml, main.tf) that need no gap.
use archguard_core::{Service, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn unread_dirs() -> Vec<PathBuf> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram");
    let mut dirs: Vec<_> = std::fs::read_dir(base)
        .expect("fixtures dir")
        .map(|e| e.expect("entry").path().join("unread"))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let p = entry.expect("entry").path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn every_unread_form_yields_a_gap_naming_its_file_and_no_artifact() {
    let (mut total, mut failures) = (0, Vec::new());
    for root in unread_dirs() {
        let mut report = ServiceTopology::default();
        let mut owned: Vec<_> = std::fs::read_dir(&root)
            .expect("unread dir")
            .map(|e| e.expect("entry").path())
            .filter(|p| p.is_dir())
            .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
            .collect();
        owned.sort();
        for dir in owned {
            report.services.push(Service {
                id: dir.clone(),
                name: dir.clone(),
                compose_file: String::new(),
                build_context: Some(dir),
                networks: vec![],
                profiles: vec![],
                project: None,
            });
        }
        let support = std::fs::read_to_string(root.join("CONTEXT")).unwrap_or_default();
        let discovery = Discovery::new(&root, &mut report).expect("discovery");
        let base = root.clone();
        let read = move |_: &Path, p: &Path| {
            let unreadable = p
                .strip_prefix(&base)
                .is_ok_and(|r| r.to_string_lossy().contains("unreadable"));
            (!unreadable)
                .then(|| std::fs::read_to_string(p).ok())
                .flatten()
        };
        archguard_search::analyze(&root, &discovery, &read, &mut report);
        let mut all = Vec::new();
        files(&root, &mut all);
        for file in all {
            let rel = file
                .strip_prefix(&root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "CONTEXT" || support.lines().any(|l| l.trim() == rel) {
                continue;
            }
            let ctx = format!("{} ({})", file.display(), rel);
            if !report.diagnostics.iter().any(|d| d.file == rel) {
                failures.push(format!("silent exit: no gap naming {ctx}"));
            }
            if report
                .elasticsearch
                .artifacts
                .iter()
                .any(|a| a.evidence.iter().any(|e| e.file == rel))
            {
                failures.push(format!("artifact from an unread input: {ctx}"));
            }
            total += 1;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(total >= 5, "unread fixtures went missing ({total})");
}
