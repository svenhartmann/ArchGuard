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
//! saving returns `Err`, which [`analyze_cached_inner`] downgrades to
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

use archguard_core::{ArchguardError, Config, Graph, Report};
use archguard_extract::SourceFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::pipeline::{self, Level};

/// Cache directory under the analysed root (hidden, hence never walked).
/// Also the anchor of the T41 snapshot history (see [`crate::snapshot`]).
pub(crate) const CACHE_DIR: &str = ".archguard";
/// The single cache entry inside [`CACHE_DIR`]: [`CacheEntry`] as JSON.
const CACHE_FILE: &str = "cache.json";
// Increment when extractor semantics or the cache contract change.
const CACHE_SCHEMA: u32 = 5;

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
struct CacheEntry {
    schema_version: u32,
    fingerprint: String,
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
/// [`analyze_cached_inner`] turns that into a warning, so analysis
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

fn extraction_fingerprint(
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
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{AtFlags, Mode, OFlags, fsync, linkat, mkdirat, openat, renameat, unlinkat};
    use rustix::io::Errno;

    use super::{CACHE_DIR, CACHE_FILE};

    /// Disambiguates temporary names from the same process.
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// `openat` flags for a directory we traverse *into*; `follow` is
    /// only ever true for the caller-chosen root itself — everything
    /// below it is repository state and must not be a symlink.
    fn dir_flags(follow: bool) -> OFlags {
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY;
        if !follow {
            flags |= OFlags::NOFOLLOW;
        }
        flags
    }

    /// Atomically commit bytes in an already opened directory. Create-only
    /// publication uses a hard link so an existing entry can never be replaced.
    pub(crate) fn write_atomic(
        dir_fd: &impl AsFd,
        name: &Path,
        bytes: &[u8],
        overwrite: bool,
    ) -> std::io::Result<()> {
        let tmp = format!(
            ".archguard-{}-{}.tmp",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        // O_EXCL | O_NOFOLLOW: the fresh name can never be a pre-planted
        // symlink, and renameat below replaces the *target entry* — a
        // symlinked cache.json is swapped for the new regular file
        // instead of being written through.
        let fd = openat(
            dir_fd,
            &tmp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_bits_retain(0o600),
        )?;
        let result = (|| {
            let mut file = File::from(fd);
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            if overwrite {
                // POSIX rename replaces the target entry itself; a symlinked
                // cache.json is swapped, never written through one.
                renameat(dir_fd, &tmp, dir_fd, name)?;
            } else {
                linkat(dir_fd, &tmp, dir_fd, name, AtFlags::empty())?;
            }
            Ok::<_, std::io::Error>(())
        })();
        // Rename consumed the name on success; create-only publication and
        // failures both need to remove it explicitly.
        let _ = unlinkat(dir_fd, &tmp, AtFlags::empty());
        result?;
        // Durability of the rename itself; the visible state is already
        // consistent without it (rename is atomic for readers).
        let _ = fsync(dir_fd);
        Ok(())
    }

    pub(crate) fn open_subdirectory(parent: &impl AsFd, name: &Path) -> std::io::Result<OwnedFd> {
        match mkdirat(parent, name, Mode::from_bits_retain(0o755)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(openat(parent, name, dir_flags(false), Mode::empty())?)
    }

    pub(crate) fn open_project_directory(dir: &Path) -> std::io::Result<OwnedFd> {
        // The root is chosen by the caller (CLI/MCP), so following
        // symlinks on the way there is fine — e.g. /tmp-style symlinks
        // on macOS. Only components *below* the root are untrusted.
        let root = openat(rustix::fs::CWD, dir, dir_flags(true), Mode::empty())?;
        // ELOOP on a symlink, ENOTDIR on a file: a repository-controlled
        // redirect is refused here, before anything is opened or created.
        open_subdirectory(&root, Path::new(CACHE_DIR))
    }

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

/// [`analyze`](crate::pipeline::analyze) with the graph cache: walk, fingerprint,
/// and on a fingerprint hit load `.archguard/cache.json` instead of
/// extracting and building — the analyses always rerun. On a miss the
/// full pipeline runs and the fresh graph is saved for the next run;
/// an unwritable or symlinked (unsafe) cache only warns, it never
/// fails the analysis.
pub fn analyze_cached(
    root: &Path,
    config: &Config,
    level: Level,
) -> Result<(Graph, Report), ArchguardError> {
    let (graph, report, _cache_hit) = analyze_cached_inner(root, config, level)?;
    Ok((graph, report))
}

/// [`analyze_cached`] additionally reporting whether the graph came
/// from the cache — the observable seam its tests assert on.
pub(crate) fn analyze_cached_inner(
    root: &Path,
    config: &Config,
    level: Level,
) -> Result<(Graph, Report, bool), ArchguardError> {
    let loaded = crate::config::LoadedConfig {
        config: config.clone(),
        sources: crate::config::all_default_sources(),
    };
    let run = crate::effective::analyze_run(root, &loaded, level)?;
    Ok((run.graph, run.report, run.cache_hit))
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
    use archguard_core::Status;
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

    fn cycle_fixture() -> tempfile::TempDir {
        fixture(&[
            (
                "src/a.ts",
                "import { b } from \"./b\";\nexport const a = b + 1;\n",
            ),
            (
                "src/b.ts",
                "import { c } from \"./c\";\nexport const b = c + 1;\n",
            ),
            (
                "src/c.ts",
                "import { a } from \"./a\";\nexport const c = a + 1;\n",
            ),
        ])
    }

    #[test]
    fn second_run_is_served_from_the_cache() {
        let dir = cycle_fixture();
        let (graph1, report1, hit1) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit1, "first run must build, not hit");

        // The single cache entry exists and carries a fingerprint that
        // matches a freshly computed one.
        let cache = dir.path().join(".archguard");
        assert!(cache.join(CACHE_FILE).is_file(), "cache entry missing");
        let raw = fs::read_to_string(cache.join(CACHE_FILE)).expect("cache entry file");
        let entry: CacheEntry = serde_json::from_str(&raw).expect("cache entry parses");
        assert_eq!(
            entry.fingerprint,
            current_fingerprint(dir.path()),
            "stored fingerprint mismatch"
        );

        let (graph2, report2, hit2) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(hit2, "second run must be served from the cache");
        assert_eq!(graph1, graph2);
        assert_eq!(report1, report2);
        assert_eq!(
            report2.summary.status,
            Status::Fail,
            "the cycle is still found"
        );
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
    fn changed_mtime_and_length_miss_the_cache() {
        let dir = cycle_fixture();
        let (_, _, hit1) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit1);

        // Modify one fixture file: different content (→ different len)
        // and an explicitly pinned mtime (→ different mtime), so the
        // fingerprint changes no matter the filesystem's time granularity.
        let path = dir.path().join("src/b.ts");
        fs::write(
            &path,
            "import { c } from \"./c\";\nexport const b = c + 1000;\n",
        )
        .expect("rewrite");
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(time)
            .expect("set mtime");

        let (graph, report, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(!hit, "changed mtime+length must invalidate the cache");
        assert_eq!(graph.edges.len(), 3, "graph rebuilt correctly");
        assert_eq!(report.summary.sccs, 1);
        assert_eq!(report.summary.status, Status::Fail);

        // A third, unmodified run hits again.
        let (_, _, hit3) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 3");
        assert!(hit3, "fresh cache after rebuild");
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

    /// The freshly computed fingerprint of everything currently under
    /// `dir` with the default config.
    fn current_fingerprint(dir: &Path) -> String {
        let files = pipeline::walk_files(dir, &Config::default()).expect("walk");
        let mut config = Config::default();
        (config.project.source_roots, config.project.languages) =
            crate::effective::derive_project_hints(dir).unwrap();
        extraction_fingerprint(dir, &files, &config).unwrap()
    }

    /// A graph claiming "no cycle" — anything a poisoned cache could
    /// serve to flip the analysis verdict.
    fn foreign_graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "foreign".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    // F03 regression: the cache lives under the repository-controlled
    // root, so every component of its path is attacker-controlled and
    // may be swapped for a symlink at any moment.

    #[cfg(unix)]
    #[test]
    fn a_symlinked_cache_directory_cannot_redirect_writes_outside_the_root() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().expect("outside dir");
        let sentinel = outside.path().join("sentinel");
        fs::write(&sentinel, b"do not touch").expect("sentinel");

        let dir = cycle_fixture();
        symlink(outside.path(), dir.path().join(".archguard")).expect("symlink cache dir");

        // The cache is an optional optimisation: analysis must stay
        // correct even though the cache cannot be used.
        let (graph, report, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module)
                .expect("analysis survives an unsafe cache directory");
        assert!(!hit, "an unsafe cache directory is never served");
        assert_eq!(graph.edges.len(), 3, "graph built from the sources");
        assert_eq!(report.summary.status, Status::Fail);

        // Nothing was created or altered through the symlink.
        assert_eq!(
            fs::read(&sentinel).expect("sentinel readable"),
            b"do not touch",
            "external sentinel untouched"
        );
        for name in ["graph.json", "fingerprint", "cache.json"] {
            assert!(
                !outside.path().join(name).exists(),
                "{name} must not appear outside the root"
            );
        }

        let (_, _, hit2) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(!hit2, "an unsafe cache directory stays untrusted");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cache_files_cannot_overwrite_external_sentinels() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().expect("outside dir");
        let victims: Vec<std::path::PathBuf> = [
            "victim-graph.json",
            "victim-fingerprint",
            "victim-cache.json",
        ]
        .iter()
        .map(|name| {
            let path = outside.path().join(name);
            fs::write(&path, b"ORIGINAL").expect("write victim");
            path
        })
        .collect();

        let dir = cycle_fixture();
        let cache = dir.path().join(".archguard");
        fs::create_dir_all(&cache).expect("cache dir");
        symlink(&victims[0], cache.join("graph.json")).expect("symlink graph");
        symlink(&victims[1], cache.join("fingerprint")).expect("symlink fingerprint");
        symlink(&victims[2], cache.join("cache.json")).expect("symlink cache entry");

        let (graph, _, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit, "symlinked cache files are never served");
        assert_eq!(graph.edges.len(), 3);

        for victim in &victims {
            assert_eq!(
                fs::read(victim).expect("victim readable"),
                b"ORIGINAL",
                "{} untouched",
                victim.display()
            );
        }

        // The cache works again once the entry is a real regular file
        // (replaced as a symlink, never written through one).
        let entry = fs::symlink_metadata(cache.join("cache.json")).expect("cache entry");
        assert!(
            entry.is_file(),
            "cache entry is a regular file, not a symlink"
        );
        let (_, _, hit2) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(hit2, "the regular cache entry is served");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_cache_entry_serves_no_foreign_graph() {
        use std::os::unix::fs::symlink;

        let dir = cycle_fixture();
        analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("warm");
        let fp = current_fingerprint(dir.path());

        // A valid-looking cache document (right fingerprint, cycle-free
        // graph) placed outside the root, reached through a symlink.
        let outside = tempfile::tempdir().expect("outside dir");
        let poison = outside.path().join("poison.json");
        let poison_bytes = format!(
            "{{\"fingerprint\":\"{fp}\",\"graph\":{}}}",
            serde_json::to_string(&foreign_graph()).expect("json")
        );
        fs::write(&poison, &poison_bytes).expect("write poison");
        let _ = fs::remove_file(dir.path().join(".archguard").join("cache.json"));
        symlink(&poison, dir.path().join(".archguard").join("cache.json")).expect("symlink");

        let (graph, report, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run");
        assert!(!hit, "a symlinked cache entry is not trusted");
        assert_eq!(graph.edges.len(), 3, "graph rebuilt, not poisoned");
        assert_eq!(report.summary.status, Status::Fail, "no verdict flip");
        assert_eq!(
            fs::read(&poison).expect("poison readable"),
            poison_bytes.as_bytes(),
            "poison file untouched"
        );
    }

    #[test]
    fn a_stale_or_hand_written_pair_is_never_trusted_as_a_mixed_generation() {
        let dir = cycle_fixture();
        let (graph_a, _, hit1) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit1);
        let fp_a = current_fingerprint(dir.path());
        let files = pipeline::walk_files(dir.path(), &Config::default()).unwrap();
        let (imports_a, _) = pipeline::build_graph(dir.path(), &files, &Config::default()).unwrap();
        assert!(
            imports_a.semantic.is_none(),
            "disk cache contains import extraction only"
        );
        assert!(
            graph_a.semantic.is_some(),
            "analysis attaches fresh compiler coverage"
        );
        assert_eq!(graph_a.nodes, imports_a.nodes);
        assert_eq!(
            graph_a.edges, imports_a.edges,
            "TS fixture has import evidence only"
        );
        assert_eq!(
            load_graph_if_fresh(dir.path(), &fp_a).as_ref(),
            Some(&imports_a),
            "the saved generation is served for its own fingerprint"
        );

        // Simulate a crash between the (formerly two) cache writes: the
        // graph file replaced while the fingerprint is still the old
        // one. The store must never serve the foreign graph under the
        // old fingerprint.
        let foreign = foreign_graph();
        fs::write(
            dir.path().join(".archguard").join("graph.json"),
            serde_json::to_vec(&foreign).expect("json"),
        )
        .expect("plant foreign graph file");
        let served = load_graph_if_fresh(dir.path(), &fp_a);
        assert_ne!(
            served.as_ref(),
            Some(&foreign),
            "an old fingerprint must never pair with a foreign graph"
        );

        // A save for another fingerprint is visible all-or-nothing.
        save_graph(dir.path(), &foreign, "fp-B").expect("save B");
        assert_eq!(
            load_graph_if_fresh(dir.path(), "fp-B").as_ref(),
            Some(&foreign)
        );
        assert!(
            load_graph_if_fresh(dir.path(), &fp_a).is_none(),
            "generation A is fully replaced"
        );

        // A torn entry (interrupted write, corruption) is a miss, never
        // a panic, and recovers by rebuilding.
        fs::write(
            dir.path().join(".archguard").join("cache.json"),
            b"{\"fingerprint\":\"fp-B\",\"graph\":{\"nod",
        )
        .expect("torn entry");
        assert!(
            load_graph_if_fresh(dir.path(), "fp-B").is_none(),
            "torn cache is a miss"
        );
        let (graph_c, _, hit_c) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("rebuild");
        assert!(!hit_c);
        assert_eq!(graph_c, graph_a, "rebuilt from sources, verdict intact");
    }
}
