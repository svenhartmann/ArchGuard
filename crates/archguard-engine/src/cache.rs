//! Optional graph cache (T23): a fingerprint over the walked file set
//! lets a second run skip the expensive extract+build half of the
//! pipeline. The cached [`Graph`] and the fingerprint it was built for
//! form a single JSON document `.archguard/cache.json` under the
//! analysed root — one entry committed by atomic replacement, so an
//! interrupted or concurrent write can never pair an old fingerprint
//! with a new graph (F03).
//!
//! Every component of the cache path below the root is repository
//! state, so access is descriptor-relative and no-follow (F03): on
//! unix, `mkdirat`/`openat`/`renameat` via `rustix` create and replace
//! `.archguard`, the entry and the temporary files through opened
//! directory descriptors with `O_NOFOLLOW`, never through pathnames a
//! concurrent actor could swap for symlinks mid-flight. A symlinked
//! `.archguard` (or a non-regular entry) is refused, not followed:
//! saving returns `Err`, which [`analyze_cached`](crate::effective::analyze_cached) downgrades to
//! a warning — analysis stays correct without a cache, and a load of
//! anything non-regular is a cache miss, never trusted content. On
//! non-unix platforms there is no no-follow syscall; there the entry
//! still goes through a unique temporary file plus one atomic rename.
//!
//! Superseded T23 layout: `graph.json` + `fingerprint`. Those names
//! are ignored on load and never deleted on save (the CLI `scan`
//! writes its default output as `.archguard/graph.json`, a user
//! artifact), so a stale two-file cache is simply rebuilt once.
//!
//! The analyses themselves (SCCs, cycles, cuts, layers, metrics) are
//! cheap and always rerun on a hit, so a changed `archguard.toml` never
//! requires cache invalidation.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use archguard_core::{ArchguardError, Config, Graph};
use archguard_extract::SourceFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::pipeline;

/// Cache directory under the analysed root (hidden, hence never walked).
/// Also the anchor of the T41 snapshot history (see [`crate::snapshot`]).
pub(crate) const CACHE_DIR: &str = crate::project_files::PROJECT_DIR;
/// The single cache entry inside [`CACHE_DIR`]: [`CacheEntry`] as JSON.
pub(crate) const CACHE_FILE: &str = "cache.json";
// Increment when extractor semantics or the cache contract change.
const CACHE_SCHEMA: u32 = 7;

/// On-disk shape of [`CACHE_FILE`]: graph and fingerprint are
/// serialised as one unit and committed via one atomic replacement,
/// so no observer ever sees a generation half written.
#[derive(Serialize)]
struct CacheEntryRef<'a> {
    schema_version: u32,
    fingerprint: &'a str,
    graph: &'a Graph,
    warnings: &'a [String],
}

/// Owned [`CacheEntryRef`] for deserialisation.
#[derive(Deserialize)]
pub(crate) struct CacheEntry {
    schema_version: u32,
    pub(crate) fingerprint: String,
    graph: Graph,
    warnings: Vec<String>,
}

/// SHA-256 over the canonical byte stream of `(rel_path, mtime, len)`
/// per file, files sorted by `rel_path` — the same list order [`archguard_extract::walk`]
/// produces, sorted again so callers cannot perturb the result. A file
/// whose metadata cannot be read (e.g. vanished between walk and
/// hashing) is hashed as a distinct sentinel state, never as a panic.
pub fn fingerprint(files: &[SourceFile]) -> String {
    let mut entries: Vec<(&str, FileStamp)> = files
        .iter()
        .map(|file| (file.rel_path.as_str(), file_stamp(&file.path)))
        .collect();
    entries.sort_unstable_by_key(|(rel_path, _)| *rel_path);

    let mut hasher = Sha256::new();
    // Content identity prevents same-size edits with restored timestamps
    // from reusing stale dependencies. Sort independently of walker order.
    let mut sources: Vec<_> = files.iter().collect();
    sources.sort_by_key(|file| &file.rel_path);
    for file in sources {
        match fs::read(&file.path) {
            Ok(bytes) => {
                hasher.update([1]);
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(bytes);
            }
            Err(_) => hasher.update([0]),
        }
    }
    for (rel_path, stamp) in &entries {
        hasher.update(rel_path.as_bytes());
        hasher.update(b"\0");
        match stamp {
            FileStamp::Missing => hasher.update(b"-"),
            FileStamp::Stamped { secs, nanos, len } => {
                hasher.update(secs.to_le_bytes());
                hasher.update(nanos.to_le_bytes());
                hasher.update(len.to_le_bytes());
            }
        }
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Content identity of the configured source set, without extraction or analysis.
pub fn source_fingerprint(root: &Path, config: &Config) -> Result<String, ArchguardError> {
    let files = pipeline::walk_files(root, config)?;
    let mut hash = Sha256::new();
    hash.update(fingerprint(&files));
    pipeline::hash_manifests(
        root,
        files
            .iter()
            .filter(|file| file.lang == archguard_extract::Lang::Rust)
            .map(|file| file.rel_path.as_str()),
        &mut hash,
    );
    Ok(format!("{:x}", hash.finalize()))
}

/// The hashed state of one file, or a sentinel when it cannot be stat'ed.
enum FileStamp {
    Missing,
    Stamped { secs: i64, nanos: u32, len: u64 },
}

/// [`FileStamp`] of one path: modification time since the Unix epoch
/// plus length; [`FileStamp::Missing`] on any metadata failure
/// (including an unrepresentable pre-epoch mtime).
fn file_stamp(path: &Path) -> FileStamp {
    let Ok(metadata) = fs::metadata(path) else {
        return FileStamp::Missing;
    };
    let Ok(modified) = metadata.modified() else {
        return FileStamp::Missing;
    };
    let Ok(since_epoch) = modified.duration_since(UNIX_EPOCH) else {
        return FileStamp::Missing;
    };
    FileStamp::Stamped {
        secs: since_epoch.as_secs() as i64,
        nanos: since_epoch.subsec_nanos(),
        len: metadata.len(),
    }
}

/// Commit `graph` plus `fp` as one entry at `<dir>/.archguard/cache.json`,
/// creating the cache directory as a real directory. Any
/// repository-controlled symlink on the way (`O_NOFOLLOW` violations,
/// non-directory `.archguard`) returns `Err` instead of following it;
/// [`crate::effective::analyze_cached`] turns that into a warning, so analysis
/// never depends on the cache being writable.
pub fn save_graph(dir: &Path, graph: &Graph, fp: &str) -> Result<(), ArchguardError> {
    save_entry(dir, graph, fp, &[])
}

fn save_entry(
    dir: &Path,
    graph: &Graph,
    fp: &str,
    warnings: &[String],
) -> Result<(), ArchguardError> {
    let json = serde_json::to_vec(&CacheEntryRef {
        schema_version: CACHE_SCHEMA,
        fingerprint: fp,
        graph,
        warnings,
    })
    .map_err(|e| ArchguardError::Analysis(e.to_string()))?;
    safe::save(dir, &json).map_err(ArchguardError::Io)
}

/// Load the cached graph iff the entry at `<dir>/.archguard/cache.json`
/// is a regular file that parses and carries `fp`; `None` on any
/// mismatch, missing file, symlink, non-file or parse error.
pub fn load_graph_if_fresh(dir: &Path, fp: &str) -> Option<Graph> {
    load_entry(dir, fp).map(|entry| entry.graph)
}

fn load_entry(dir: &Path, fp: &str) -> Option<CacheEntry> {
    let raw = safe::load(dir)?;
    let entry: CacheEntry = serde_json::from_str(&raw).ok()?;
    (entry.schema_version == CACHE_SCHEMA && entry.fingerprint == fp && valid_graph(&entry.graph))
        .then_some(entry)
}

/// Only canonical hierarchical extraction graphs are trusted as cache input.
fn valid_graph(graph: &Graph) -> bool {
    use archguard_core::NodeKind;
    use std::collections::BTreeSet;
    if graph.schema_version != 1 {
        return false;
    }
    let mut names = BTreeSet::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        if node.id.0 as usize != index || !names.insert(&node.name) {
            return false;
        }
        let mut seen = BTreeSet::from([node.id]);
        let mut parent = node.parent;
        while let Some(id) = parent {
            let Some(ancestor) = graph.nodes.get(id.0 as usize) else {
                return false;
            };
            if !seen.insert(id) || ancestor.kind != NodeKind::Module {
                return false;
            }
            parent = ancestor.parent;
        }
    }
    let mut pairs = BTreeSet::new();
    graph.edges.iter().all(|edge| {
        edge.from != edge.to
            && pairs.insert((edge.from, edge.to))
            && [edge.from, edge.to].iter().all(|id| {
                graph
                    .nodes
                    .get(id.0 as usize)
                    .is_some_and(|node| node.kind == NodeKind::File)
            })
            && edge
                .count
                .is_some_and(|count| count >= edge.refs.len() as u64)
    })
}

pub(crate) fn extraction_fingerprint(
    root: &Path,
    files: &[SourceFile],
    config: &Config,
) -> Result<String, ArchguardError> {
    let mut hash = Sha256::new();
    hash.update(CACHE_SCHEMA.to_le_bytes());
    hash.update(fingerprint(files));
    hash.update(crate::go_sources::fingerprint(root, files));
    hash.update(serde_json::to_vec(&config.project).expect("project config serializes"));
    // Cargo manifests decide which `use` statements link workspace crates.
    crate::pipeline::hash_manifests(
        root,
        files
            .iter()
            .filter(|file| file.lang == archguard_extract::Lang::Rust)
            .map(|file| file.rel_path.as_str()),
        &mut hash,
    );
    // PSR-4 mappings participate in resolution even for declared source roots.
    match fs::read(root.join("composer.json")) {
        Ok(bytes) => {
            hash.update([1]);
            hash.update(bytes);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
        Err(error) => return Err(error.into()),
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Platform-safe cache storage: descriptor-relative no-follow access
/// on unix, at minimum unique-temp-file + atomic rename elsewhere.
#[cfg(unix)]
pub(crate) mod safe {
    use std::fs::File;
    use std::io::Read;
    use std::path::Path;

    use rustix::fs::{Mode, OFlags, openat};

    use super::{CACHE_DIR, CACHE_FILE};
    use crate::project_files::safe::{dir_flags, open_project_directory, write_atomic};

    pub fn save(dir: &Path, json: &[u8]) -> std::io::Result<()> {
        let cache = open_project_directory(dir)?;
        write_atomic(&cache, Path::new(CACHE_FILE), json, true)
    }

    pub fn load(dir: &Path) -> Option<String> {
        let root = openat(rustix::fs::CWD, dir, dir_flags(true), Mode::empty()).ok()?;
        let cache = openat(&root, CACHE_DIR, dir_flags(false), Mode::empty()).ok()?;
        let entry = openat(
            &cache,
            CACHE_FILE,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .ok()?;
        let mut raw = String::new();
        File::from(entry).read_to_string(&mut raw).ok()?;
        Some(raw)
    }
}

#[cfg(not(unix))]
mod safe {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use super::{CACHE_DIR, CACHE_FILE};

    /// Single unique temporary file plus one atomic rename: without a
    /// no-follow syscall (Windows has none) this is the strongest
    /// available pairing guarantee — and rename still replaces a
    /// symlinked target itself rather than writing through it.
    pub fn save(dir: &Path, json: &[u8]) -> std::io::Result<()> {
        let cache = dir.join(CACHE_DIR);
        fs::create_dir_all(&cache)?;
        let tmp = cache.join(format!(
            "{CACHE_FILE}.tmp.{}.{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // create_new (O_CREAT|O_EXCL): a pre-planted symlink at the
        // temporary name is refused instead of written through, the
        // closest std analogue of the unix guarantee.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(json)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, cache.join(CACHE_FILE))?;
        Ok(())
    }

    pub fn load(dir: &Path) -> Option<String> {
        fs::read_to_string(dir.join(CACHE_DIR).join(CACHE_FILE)).ok()
    }
}

pub(crate) fn extract_cached(
    root: &Path,
    files: &[SourceFile],
    config: &Config,
) -> Result<(Graph, Vec<String>, bool), ArchguardError> {
    let fp = extraction_fingerprint(root, files, config)?;
    if let Some(entry) = load_entry(root, &fp) {
        return Ok((entry.graph, entry.warnings, true));
    }
    let (full, mut warnings) = pipeline::build_graph(root, files, config)?;
    // A source changing during extraction must not be saved under an old key.
    if extraction_fingerprint(root, files, config)? != fp {
        warnings.insert(
            0,
            "sources changed during extraction; cache was not written".into(),
        );
        return Ok((full, warnings, false));
    }
    if let Err(e) = save_entry(root, &full, &fp, &warnings) {
        warnings.insert(
            0,
            format!(
                "graph cache {} not written ({e}); analysis is unaffected",
                root.join(CACHE_DIR).display()
            ),
        );
    }
    Ok((full, warnings, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_extract::Lang;
    use std::fs::File;
    use std::time::SystemTime;

    /// Write `files` (path, content) into a fresh temp dir, mirroring
    /// the T22 integration fixture: a → b → c → a cycle under `src`.
    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for (rel, content) in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
            fs::write(path, content).expect("write file");
        }
        dir
    }

    #[test]
    fn schema_two_csharp_cache_is_rebuilt_with_block_namespaces() {
        let dir = fixture(&[(
            "src/Elsewhere/Widget.cs",
            "namespace Acme.Domain\n{\n    public class Widget {}\n}\n",
        )]);
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        config.project.languages = vec!["cs".into()];
        let files = pipeline::walk_files(dir.path(), &config).unwrap();

        // Schema 2 used the same source/config fingerprint but missed block
        // namespaces whose opening brace was on the following line. Recreate
        // that valid old cache without modifying any source bytes or stamps.
        let mut old_hash = Sha256::new();
        old_hash.update(2_u32.to_le_bytes());
        old_hash.update(fingerprint(&files));
        old_hash.update(serde_json::to_vec(&config.project).unwrap());
        old_hash.update([0]); // no composer.json in this C# fixture
        let old_fingerprint = format!("{:x}", old_hash.finalize());
        let mut builder = archguard_core::GraphBuilder::new();
        builder.ensure_path("Elsewhere/Widget.cs", Some("cs"));
        let old_graph = builder.build(dir.path().to_string_lossy().into_owned());
        assert!(valid_graph(&old_graph));
        fs::create_dir(dir.path().join(CACHE_DIR)).unwrap();
        fs::write(
            dir.path().join(CACHE_DIR).join(CACHE_FILE),
            serde_json::to_vec(&CacheEntryRef {
                schema_version: 2,
                fingerprint: &old_fingerprint,
                graph: &old_graph,
                warnings: &[],
            })
            .unwrap(),
        )
        .unwrap();

        let (graph, _, hit) = extract_cached(dir.path(), &files, &config).unwrap();
        assert!(
            !hit,
            "old extractor results must not survive a semantic change"
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.name == "Acme/Domain/Widget.cs")
        );
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| node.name == "Elsewhere/Widget.cs")
        );
        let (warm, _, hit) = extract_cached(dir.path(), &files, &config).unwrap();
        assert!(hit, "the rebuilt current-version cache is reusable");
        assert_eq!(warm, graph);
    }

    #[test]
    fn schema_five_php_cache_is_rebuilt_with_qualified_names() {
        let dir = fixture(&[
            (
                "src/A/User.php",
                "<?php\nnamespace A;\nclass User { function f() { return new \\B\\Repo(); } }\n",
            ),
            ("src/B/Repo.php", "<?php\nnamespace B;\nclass Repo {}\n"),
        ]);
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        config.project.languages = vec!["php".into()];
        let files = pipeline::walk_files(dir.path(), &config).unwrap();

        // Schema 5 ignored fully-qualified names in code: the same sources
        // produced no edge. Its entry must not be served after the fix.
        let mut builder = archguard_core::GraphBuilder::new();
        builder.ensure_path("A/User.php", Some("php"));
        builder.ensure_path("B/Repo.php", Some("php"));
        let old_graph = builder.build(dir.path().to_string_lossy().into_owned());
        fs::create_dir(dir.path().join(CACHE_DIR)).unwrap();
        fs::write(
            dir.path().join(CACHE_DIR).join(CACHE_FILE),
            serde_json::to_vec(&CacheEntryRef {
                schema_version: 5,
                fingerprint: &extraction_fingerprint(dir.path(), &files, &config).unwrap(),
                graph: &old_graph,
                warnings: &[],
            })
            .unwrap(),
        )
        .unwrap();

        let (graph, _, hit) = extract_cached(dir.path(), &files, &config).unwrap();
        assert!(!hit, "schema 5 results lack qualified-name edges");
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn schema_six_java_cache_is_rebuilt_with_qualified_names() {
        let dir = fixture(&[
            (
                "src/a/User.java",
                "package a;\nclass User { b.Repo repo = new b.Repo(); }\n",
            ),
            ("src/b/Repo.java", "package b;\npublic class Repo {}\n"),
        ]);
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        config.project.languages = vec!["java".into()];
        let files = pipeline::walk_files(dir.path(), &config).unwrap();

        // Schema 6 ignored qualified Java/C# names in code: the same sources
        // produced no edge. Its entry must not be served after the fix.
        let mut builder = archguard_core::GraphBuilder::new();
        builder.ensure_path("a/User.java", Some("java"));
        builder.ensure_path("b/Repo.java", Some("java"));
        let old_graph = builder.build(dir.path().to_string_lossy().into_owned());
        fs::create_dir(dir.path().join(CACHE_DIR)).unwrap();
        fs::write(
            dir.path().join(CACHE_DIR).join(CACHE_FILE),
            serde_json::to_vec(&CacheEntryRef {
                schema_version: 6,
                fingerprint: &extraction_fingerprint(dir.path(), &files, &config).unwrap(),
                graph: &old_graph,
                warnings: &[],
            })
            .unwrap(),
        )
        .unwrap();

        let (graph, _, hit) = extract_cached(dir.path(), &files, &config).unwrap();
        assert!(!hit, "schema 6 results lack Java qualified-name edges");
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn fingerprint_is_order_independent_and_sensitive_to_len_and_mtime() {
        let dir = fixture(&[
            ("x.ts", "export const x = 1;\n"),
            ("y.ts", "export const y = 2;\n"),
        ]);
        let source = |rel: &str| SourceFile {
            path: dir.path().join(rel),
            rel_path: rel.to_string(),
            lang: Lang::Ts,
        };
        let forward = vec![source("x.ts"), source("y.ts")];
        let backward = vec![source("y.ts"), source("x.ts")];
        assert_eq!(
            fingerprint(&forward),
            fingerprint(&backward),
            "order must not matter"
        );
        assert_eq!(fingerprint(&[]), fingerprint(&[]));

        // Same rel_path and len, changed mtime → different fingerprint.
        let before = fingerprint(&forward);
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        File::options()
            .write(true)
            .open(dir.path().join("x.ts"))
            .expect("open")
            .set_modified(time)
            .expect("set mtime");
        assert_ne!(before, fingerprint(&forward), "mtime change must matter");

        // Changed length → different fingerprint.
        let before = fingerprint(&forward);
        fs::write(dir.path().join("y.ts"), "export const y = 222;\n").expect("grow");
        assert_ne!(before, fingerprint(&forward), "length change must matter");

        // A vanished file cannot panic and yields yet another value.
        fs::remove_file(dir.path().join("x.ts")).expect("remove");
        assert_ne!(before, fingerprint(&forward), "missing file must differ");
    }

    #[test]
    fn load_graph_if_fresh_is_none_without_a_valid_pair() {
        let dir = fixture(&[("x.ts", "export const x = 1;\n")]);
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "proj".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        // Nothing cached yet.
        assert!(load_graph_if_fresh(dir.path(), "abc").is_none());

        save_graph(dir.path(), &graph, "abc").expect("save");
        assert_eq!(
            load_graph_if_fresh(dir.path(), "abc").as_ref(),
            Some(&graph)
        );
        assert!(
            load_graph_if_fresh(dir.path(), "abd").is_none(),
            "fingerprint mismatch"
        );

        // Corrupt graph: no panic, just no graph.
        fs::write(dir.path().join(".archguard").join(CACHE_FILE), "{ not json").expect("corrupt");
        assert!(
            load_graph_if_fresh(dir.path(), "abc").is_none(),
            "parse error tolerated"
        );
    }
}
