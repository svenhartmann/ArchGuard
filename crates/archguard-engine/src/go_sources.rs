//! Module identities for selected Go sources. No module downloads or execution.
use archguard_extract::SourceFile;
use sha2::{Digest, Sha256};
use std::path::Path;

pub(crate) fn package_path(root: &Path, relative: &str) -> String {
    let directory = Path::new(relative).parent().unwrap_or(Path::new(""));
    for ancestor in directory.ancestors() {
        let manifest = ancestor.join("go.mod");
        if let Ok(source) = crate::project_files::read_manifest(root, &manifest) {
            if let Some(module) = archguard_extract::go::module_path(&source) {
                let suffix = directory
                    .strip_prefix(ancestor)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                return if suffix.is_empty() {
                    module
                } else {
                    format!("{module}/{suffix}")
                };
            }
            break; // An invalid nested module must not inherit its parent's identity.
        }
    }
    format!(
        "archguard.local/{}",
        directory.to_string_lossy().replace('\\', "/")
    )
}

pub(crate) fn fingerprint(root: &Path, files: &[SourceFile]) -> [u8; 32] {
    let mut paths = std::collections::BTreeSet::new();
    for file in files
        .iter()
        .filter(|f| f.lang == archguard_extract::Lang::Go)
    {
        for ancestor in Path::new(&file.rel_path)
            .parent()
            .unwrap_or(Path::new(""))
            .ancestors()
        {
            paths.insert(ancestor.join("go.mod"));
        }
    }
    let mut hash = Sha256::new();
    for path in paths {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update([0]);
        match crate::project_files::read_manifest(root, &path) {
            Ok(source) => {
                hash.update([1]);
                hash.update(source.as_bytes());
            }
            Err(_) => hash.update([0]),
        }
        hash.update([0]);
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_module_identity_and_manifest_changes() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("nested/pkg")).unwrap();
        std::fs::write(d.path().join("go.mod"), "module example.org/root\n").unwrap();
        assert_eq!(
            package_path(d.path(), "nested/pkg/a.go"),
            "example.org/root/nested/pkg"
        );
        std::fs::write(d.path().join("nested/go.mod"), "module example.org/child\n").unwrap();
        assert_eq!(
            package_path(d.path(), "nested/pkg/a.go"),
            "example.org/child/pkg"
        );
        std::fs::write(d.path().join("nested/go.mod"), "invalid\n").unwrap();
        assert_eq!(
            package_path(d.path(), "nested/pkg/a.go"),
            "archguard.local/nested/pkg"
        );
    }
}
