//! Shared boundaries for explicit test, mutation and duplication evidence imports.
use archguard_core::{ArchguardError, Config, Graph};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const MAX_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_RECORDS: usize = 100_000;
pub(crate) const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("test evidence: {message}"))
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// Content hash of a project file, or why it is unavailable.
pub(crate) fn file_fingerprint(root: &Path, name: &str, max_bytes: u64) -> String {
    match crate::project_files::read_repo_file(root, Path::new(name), max_bytes) {
        Ok(bytes) => hash(&bytes),
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}
pub(crate) fn valid_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn now() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|v| v.as_secs())
}
pub(crate) fn read_input(root: &Path, path: &Path) -> Result<Vec<u8>, ArchguardError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // Explicit report paths may point at special files; metadata is checked
    // after opening, without blocking indefinitely waiting on a FIFO writer.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = options.open(if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    })?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err(error("input must be a regular file of at most 32 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("input exceeds 32 MiB"));
    }
    Ok(bytes)
}
pub(crate) fn fingerprint(root: &Path, path: &str) -> String {
    match crate::project_files::read_repo_file(root, Path::new(path), MAX_BYTES) {
        Ok(bytes) => hash(&bytes),
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}
pub(crate) fn relative(value: &str) -> Result<String, ArchguardError> {
    let value = value.replace('\\', "/");
    if value.is_empty()
        || value.starts_with('/')
        || value.chars().any(char::is_control)
        || value.split('/').any(|p| p == ".." || p.contains(':'))
    {
        return Err(error(
            "source path must be project-relative without traversal, controls or URI scheme",
        ));
    }
    let value = value
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/");
    if value.is_empty() {
        return Err(error("source path must name a file"));
    }
    Ok(value)
}
pub(crate) struct SourceFile {
    pub node: String,
    pub hash: String,
    pub text: String,
    pub lines: u32,
}
pub(crate) struct Sources<'a> {
    root: &'a Path,
    prefix: Option<String>,
    paths: BTreeMap<String, String>,
    cache: BTreeMap<String, Option<SourceFile>>,
}
impl<'a> Sources<'a> {
    pub fn new(
        root: &'a Path,
        config: &Config,
        graph: &Graph,
        source_root: Option<&Path>,
    ) -> Result<Self, ArchguardError> {
        let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, &n.name)).collect();
        let mut paths = BTreeMap::<String, Vec<String>>::new();
        for source in crate::review_sources::collect_source_evidence(root, config, graph)? {
            if let (Some(path), Some(name)) = (source.path, nodes.get(&source.node_id)) {
                paths.entry(path).or_default().push((*name).clone());
            }
        }
        let paths = paths
            .into_iter()
            .filter_map(|(p, mut n)| {
                n.sort();
                n.dedup();
                (n.len() == 1).then(|| (p, n.remove(0)))
            })
            .collect();
        let prefix = if let Some(base) = source_root {
            let project = root.canonicalize()?;
            let canonical = if base.is_absolute() {
                base.to_owned()
            } else {
                root.join(base)
            }
            .canonicalize()?;
            let p = canonical
                .strip_prefix(project)
                .map_err(|_| error("source root must remain inside project"))?;
            if p.as_os_str().is_empty() {
                None
            } else {
                Some(relative(&p.to_string_lossy())?)
            }
        } else {
            None
        };
        Ok(Self {
            root,
            prefix,
            paths,
            cache: BTreeMap::new(),
        })
    }
    pub fn path(&self, value: &str) -> Result<String, ArchguardError> {
        let p = relative(value)?;
        Ok(if let Some(prefix) = &self.prefix {
            format!("{prefix}/{p}")
        } else {
            p
        })
    }
    /// The graph node a project-relative path maps to, without reading it.
    pub fn node(&self, path: &str) -> Option<String> {
        self.paths.get(path).cloned()
    }
    pub fn get(&mut self, path: &str) -> Option<&SourceFile> {
        self.cache
            .entry(path.into())
            .or_insert_with(|| {
                let node = self.paths.get(path)?.clone();
                let bytes = crate::project_files::read_repo_file(
                    self.root,
                    Path::new(path),
                    MAX_SOURCE_BYTES,
                )
                .ok()?;
                let hash = hash(&bytes);
                let text = String::from_utf8(bytes).ok()?;
                let lines = u32::try_from(text.lines().count()).ok()?;
                Some(SourceFile {
                    node,
                    hash,
                    text,
                    lines,
                })
            })
            .as_ref()
    }
}
pub(crate) fn save<T: serde::Serialize>(
    root: &Path,
    name: &str,
    report: &T,
) -> Result<(), ArchguardError> {
    let bytes = serde_json::to_vec_pretty(report).map_err(error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("normalized snapshot exceeds 32 MiB"));
    }
    crate::project_files::write_project_file(root, Path::new(name), &bytes, true)?;
    Ok(())
}
