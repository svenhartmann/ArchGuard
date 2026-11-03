//! Snapshots and trends (T41, Kap. 9): `write_snapshot` appends one
//! metrics snapshot per run into `.archguard/history/`, `read_trend`
//! reads them back as a time series.
//!
//! The file name is `<unix_ts>-<git_sha_or_notag>.json` under the
//! history directory (the git SHA comes from `git rev-parse --short
//! HEAD` run inside the analysed root; when git is unavailable, the
//! directory is no repository or the command fails, the literal
//! `notag` takes its place — never an error). Optional `--tag` values
//! are stored *inside* the JSON, so the naming rule stays stable. When
//! the target file already exists (two snapshots in the same second at
//! the same SHA), `-1`, `-2`, … are appended before `.json` until the
//! name is free, so no snapshot is ever overwritten.
//!
//! A history file that does not parse as a [`Snapshot`] is skipped by
//! [`read_trend`] (same tolerance as the graph cache in [`crate::cache`]):
//! one corrupt file must not break the whole trend.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use archguard_core::{ArchguardError, FindingKind, Report};
use archguard_store::SnapshotRecord;
use serde::{Deserialize, Serialize};

use crate::cache::CACHE_DIR;
use archguard_analysis::cycle_hash;

/// Larger files in the history directory are not snapshots.
const SNAPSHOT_LIMIT: u64 = 64 * 1024 * 1024;

/// Subdirectory of [`CACHE_DIR`] holding the snapshot JSON files.
const HISTORY_DIR: &str = "history";
/// Filename fallback when the git SHA cannot be determined.
const NOTAG: &str = "notag";
/// Format marker of the snapshot files, kept for future migrations.
const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// One persisted metrics snapshot (the JSON document in
/// `.archguard/history/`): the summary numbers Kap. 9 lists for the
/// trend — cycle count, largest SCC, ACD/NCCD — plus node/edge counts
/// and the stable cycle hashes of the T36 baseline identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_scope: Option<String>,
    /// Format marker of the snapshot files.
    pub schema_version: u32,
    /// Unix seconds at write time (also the leading filename component).
    pub timestamp: u64,
    /// Short git SHA of the analysed root, or [`NOTAG`].
    pub git_sha: String,
    /// Optional `--tag` label, stored inside the file (not in the name).
    #[serde(default)]
    pub tag: Option<String>,
    /// Node count from [`Report::summary`].
    pub nodes: u32,
    /// Edge count from [`Report::summary`].
    pub edges: u32,
    /// Number of cyclic SCCs from [`Report::summary`].
    pub sccs: u32,
    /// Size of the largest SCC from [`Report::summary`].
    pub largest_scc: u32,
    /// Lakos ACD from [`Report::summary`].
    pub acd: f64,
    /// Lakos NCCD from [`Report::summary`].
    pub nccd: f64,
    /// [`cycle_hash`] per cycle finding, stably sorted.
    pub cycle_hashes: Vec<String>,
    /// Review KPIs at snapshot time (R28); absent in pre-R28 files, so
    /// the KPI delta column reads them as "no baseline".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_kpis: Option<archguard_core::ReviewKpis>,
}

/// One [`Snapshot`] reduced to the trend row the CLI table and the MCP
/// `trend` tool render (Kap. 9: date, sha/tag, cycles, largest SCC,
/// ACD, edges).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_scope: Option<String>,
    /// Unix seconds at write time.
    pub timestamp: u64,
    /// Short git SHA, or `notag`.
    pub git_sha: String,
    /// Optional `--tag` label.
    pub tag: Option<String>,
    /// Number of cyclic SCCs.
    pub cycles: u32,
    /// Size of the largest SCC.
    pub largest_scc: u32,
    /// Lakos ACD.
    pub acd: f64,
    /// Edge count.
    pub edges: u32,
}

impl From<&Snapshot> for SnapshotRow {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            analysis_scope: snapshot.analysis_scope.clone(),
            timestamp: snapshot.timestamp,
            git_sha: snapshot.git_sha.clone(),
            tag: snapshot.tag.clone(),
            cycles: snapshot.sccs,
            largest_scc: snapshot.largest_scc,
            acd: snapshot.acd,
            edges: snapshot.edges,
        }
    }
}

/// Append a snapshot of `report` to `<root>/.archguard/history/` and
/// return the written path. `tag` (from `--tag`) is stored inside the
/// file; the SHA comes from git and degrades to `notag` — the call
/// never fails because of git. I/O errors surface as
/// [`ArchguardError::Io`].
pub fn write_snapshot(
    root: &Path,
    report: &Report,
    tag: Option<&str>,
) -> Result<PathBuf, ArchguardError> {
    write_snapshot_with_scope(root, report, tag, None)
}

/// Metric scope identity prevents comparisons across changed roots/exclusions/zoom.
pub fn snapshot_scope(config: &archguard_core::Config) -> String {
    serde_json::to_string(&serde_json::json!({"metric":"imports/acd/v1","project":config.project,"depth":config.modules.depth})).expect("scope serializes")
}

pub fn write_snapshot_with_scope(
    root: &Path,
    report: &Report,
    tag: Option<&str>,
    scope: Option<&str>,
) -> Result<PathBuf, ArchguardError> {
    let timestamp = unix_now();
    let git_sha = git_sha(root).unwrap_or_else(|| NOTAG.to_string());
    let mut snapshot = build_snapshot(timestamp, &git_sha, tag, report);
    snapshot.analysis_scope = scope.map(str::to_owned);
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|err| ArchguardError::Analysis(format!("snapshot serialisation failed: {err}")))?;
    Ok(save_snapshot(
        root,
        timestamp,
        &sanitize_component(&git_sha),
        format!("{json}\n").as_bytes(),
    )?)
}

#[cfg(unix)]
fn save_snapshot(
    root: &Path,
    timestamp: u64,
    label: &str,
    content: &[u8],
) -> std::io::Result<PathBuf> {
    use rustix::fs::{Mode, OFlags, mkdirat, openat};
    use rustix::io::Errno;
    use std::io::Write;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let root_fd = openat(rustix::fs::CWD, root, flags, Mode::empty())?;
    let mut parent = root_fd;
    for name in [CACHE_DIR, HISTORY_DIR] {
        match mkdirat(&parent, name, Mode::RWXU) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        parent = openat(&parent, name, flags | OFlags::NOFOLLOW, Mode::empty())?;
    }
    let mut suffix = 0u32;
    loop {
        let name = snapshot_file_name(timestamp, label, suffix);
        match openat(
            &parent,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => {
                let mut file = fs::File::from(fd);
                file.write_all(content)?;
                return Ok(root.join(CACHE_DIR).join(HISTORY_DIR).join(name));
            }
            Err(Errno::EXIST) => {
                suffix = suffix
                    .checked_add(1)
                    .ok_or_else(|| std::io::Error::other("snapshot name space exhausted"))?
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(not(unix))]
fn save_snapshot(
    root: &Path,
    timestamp: u64,
    label: &str,
    content: &[u8],
) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let mut dir = root.to_path_buf();
    for part in [CACHE_DIR, HISTORY_DIR] {
        dir.push(part);
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(std::io::Error::other(
                "snapshot directory must not be a symlink",
            ));
        }
    }
    let mut suffix = 0u32;
    loop {
        let path = dir.join(snapshot_file_name(timestamp, label, suffix));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(content)?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
            Err(error) => return Err(error),
        }
    }
}

fn snapshot_file_name(timestamp: u64, label: &str, suffix: u32) -> String {
    if suffix == 0 {
        format!("{timestamp}-{label}.json")
    } else {
        format!("{timestamp}-{label}-{suffix}.json")
    }
}

/// Every parseable snapshot in `<root>/.archguard/history/`, oldest
/// first. Same-second ties are ordered by the numeric `-1`, `-2`, …
/// collision suffix of [`write_snapshot`] (the plain `…-<sha>.json`
/// counts as suffix `0`), which keeps same-second snapshots in write
/// order — a plain file-name sort would not, because `-1` sorts before
/// the extension separator. Unparseable files are skipped, and so is anything
/// reached through a symlink or not a bounded regular file: the history
/// directory is repository content.
pub fn read_snapshots(root: &Path) -> Vec<Snapshot> {
    let dir = root.join(CACHE_DIR).join(HISTORY_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut snapshots: Vec<(u64, String, Snapshot)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            let relative = Path::new(CACHE_DIR).join(HISTORY_DIR).join(&name);
            let bytes = crate::security::read_repo_file(root, &relative, SNAPSHOT_LIMIT).ok()?;
            let text = String::from_utf8(bytes).ok()?;
            // Same tolerance as the graph cache: a corrupt snapshot is
            // skipped, never fatal.
            let snapshot = serde_json::from_str::<Snapshot>(&text).ok()?;
            Some((snapshot.timestamp, name, snapshot))
        })
        .collect();
    snapshots.sort_by(|(left_ts, left_name, _), (right_ts, right_name, _)| {
        left_ts
            .cmp(right_ts)
            .then_with(|| {
                collision_suffix(*left_ts, left_name).cmp(&collision_suffix(*right_ts, right_name))
            })
            .then_with(|| left_name.cmp(right_name))
    });
    snapshots
        .into_iter()
        .map(|(_, _, snapshot)| snapshot)
        .collect()
}

/// The newest snapshot in the history, or `None` — the KPI delta
/// baseline of the `metrics` table (R28).
pub fn latest_snapshot(root: &Path) -> Option<Snapshot> {
    read_snapshots(root).pop()
}

/// The trend time series as [`SnapshotRow`]s; `last` limits the result
/// to the newest N rows; `last == 0` returns all.
pub fn read_trend(root: &Path, last: usize) -> Vec<SnapshotRow> {
    let rows = read_snapshots(root)
        .iter()
        .map(SnapshotRow::from)
        .collect::<Vec<_>>();
    if last == 0 || last >= rows.len() {
        rows
    } else {
        rows[rows.len() - last..].to_vec()
    }
}

/// A [`SnapshotRecord`] for the store's `snapshot` table (T41 db path):
/// same numbers as the JSON file, with the file-style id
/// `<ts>-<sha_or_notag>`. A record with the same id replaces the
/// previous one in the table (unlike the history files, which keep
/// every collision via their suffixes).
pub fn snapshot_record(root: &Path, report: &Report, tag: Option<&str>) -> SnapshotRecord {
    let timestamp = unix_now();
    let git_sha = git_sha(root).unwrap_or_else(|| NOTAG.to_string());
    let snapshot = build_snapshot(timestamp, &git_sha, tag, report);
    SnapshotRecord {
        id: format!("{}-{git_sha}", snapshot.timestamp),
        taken_at: snapshot.timestamp as i64,
        git_sha: snapshot.git_sha,
        tag: snapshot.tag,
        cycles: i64::from(snapshot.sccs),
        largest_scc: i64::from(snapshot.largest_scc),
        acd: snapshot.acd,
        edges: i64::from(snapshot.edges),
        cycle_hashes: snapshot.cycle_hashes.join("\n"),
    }
}

/// The shared field extraction of [`write_snapshot`] and
/// [`snapshot_record`].
fn build_snapshot(timestamp: u64, git_sha: &str, tag: Option<&str>, report: &Report) -> Snapshot {
    let cycle_hashes = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
        .map(|finding| cycle_hash(&finding.nodes))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Snapshot {
        analysis_scope: None,
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        timestamp,
        git_sha: git_sha.to_string(),
        tag: tag
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        nodes: report.summary.nodes,
        edges: report.summary.edges,
        sccs: report.summary.sccs,
        largest_scc: report.summary.largest_scc,
        acd: report.summary.acd,
        nccd: report.summary.nccd,
        cycle_hashes,
        review_kpis: report.review_kpis.clone(),
    }
}

/// `<dir>/<ts>-<label>.json`, falling back to `-1`, `-2`, … before the
/// extension when the name is taken (two snapshots in the same second
/// at the same SHA must not overwrite each other).
/// The collision counter of a history file name `{ts}-{label}[-{n}].json`
/// written by [`write_snapshot`]: `0` for the plain name, `n` for the
/// `-n` variant, `0` for names that do not follow the scheme (they then
/// fall back to the name comparison of the sort).
fn collision_suffix(timestamp: u64, name: &str) -> u32 {
    let stem = name.strip_suffix(".json").unwrap_or(name);
    let Some(rest) = stem.strip_prefix(&format!("{timestamp}-")) else {
        return 0;
    };
    match rest.rsplit_once('-') {
        Some((_, digits)) if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => {
            digits.parse().unwrap_or(0)
        }
        _ => 0,
    }
}

/// Short git SHA of `root` (`git rev-parse --short HEAD`), or `None`
/// whenever git cannot answer: missing binary, no repository, empty
/// HEAD, non-zero exit, non-UTF-8 output. Never an error (Kap. 9).
fn git_sha(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--short", "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// Unix seconds now; a clock before 1970 (adjustments aside) degrades
/// to `0` rather than failing the snapshot.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_secs())
        .unwrap_or(0)
}

/// Keep the filename safe: git SHAs are hex in practice, but a tag- or
/// SHA-derived label only ever keeps ASCII letters, digits, `.` and
/// `-`; everything else becomes `-`, an empty result falls back to
/// [`NOTAG`].
fn sanitize_component(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() {
        NOTAG.to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use archguard_core::{Finding, ModuleMetrics, Severity, Status, SuggestedCut, Summary};

    fn report(
        sccs: u32,
        largest_scc: u32,
        acd: f64,
        edges: u32,
        cycle_nodes: &[&[&str]],
    ) -> Report {
        let findings = cycle_nodes
            .iter()
            .enumerate()
            .map(|(index, nodes)| Finding {
                message: None,
                evidence: Vec::new(),
                kind: FindingKind::Cycle,
                id: format!("scc-{}", index + 1),
                severity: Severity::Error,
                nodes: nodes.iter().map(|name| (*name).to_string()).collect(),
                edge_count: 3,
                cycles_sampled: 1,
                sampling: None,
                suggested_cuts: vec![SuggestedCut {
                    from: "a".into(),
                    to: "b".into(),
                    score: 1.0,
                    reason: "test".into(),
                }],
            })
            .collect();
        Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: Summary {
                nodes: 5,
                edges,
                sccs,
                largest_scc,
                layer_violations: 0,
                status: Status::Fail,
                ccd: 10,
                acd,
                nccd: acd / 2.0,
            },
            config: BTreeMap::new(),
            findings,
            metrics: BTreeMap::<String, ModuleMetrics>::new(),
        }
    }

    fn write_raw(dir: &Path, name: &str, text: &str) {
        let history = dir.join(CACHE_DIR).join(HISTORY_DIR);
        fs::create_dir_all(&history).expect("create history");
        fs::write(history.join(name), text).expect("write snapshot file");
    }

    fn snapshot_json(timestamp: u64, sccs: u32, edges: u32) -> String {
        serde_json::to_string(&Snapshot {
            analysis_scope: None,
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            timestamp,
            git_sha: "abc1234".to_string(),
            tag: None,
            nodes: 5,
            edges,
            sccs,
            largest_scc: sccs,
            acd: 0.5,
            nccd: 0.25,
            cycle_hashes: vec![],
            review_kpis: None,
        })
        .expect("plain data")
    }

    #[test]
    fn write_snapshot_lands_in_history_with_notag_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The temp dir lives outside any git repository, so the SHA
        // degrades to the literal `notag` — and the call still succeeds.
        let path = write_snapshot(
            dir.path(),
            &report(1, 3, 2.0, 6, &[&["a", "b", "c"]]),
            Some("baseline"),
        )
        .expect("write snapshot");
        assert!(path.starts_with(dir.path().join(CACHE_DIR).join(HISTORY_DIR)));
        assert!(
            path.file_name()
                .expect("file name")
                .to_str()
                .expect("utf-8")
                .ends_with("-notag.json"),
            "unexpected file name: {}",
            path.display()
        );
        let text = fs::read_to_string(&path).expect("read back");
        let snapshot: Snapshot = serde_json::from_str(&text).expect("parse");
        assert_eq!(snapshot.git_sha, "notag");
        assert_eq!(snapshot.tag.as_deref(), Some("baseline"));
        assert_eq!(snapshot.sccs, 1);
        assert_eq!(snapshot.edges, 6);
        assert_eq!(snapshot.cycle_hashes.len(), 1);
        // The hash is the stable T36 identity.
        let nodes: Vec<String> = ["a", "b", "c"].iter().map(|n| (*n).to_string()).collect();
        assert_eq!(snapshot.cycle_hashes, vec![cycle_hash(&nodes)]);
    }

    #[test]
    fn write_snapshot_never_fails_on_empty_tag_and_keeps_two_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = write_snapshot(dir.path(), &report(0, 0, 0.0, 2, &[]), Some("   "))
            .expect("first snapshot");
        let second = write_snapshot(dir.path(), &report(1, 3, 2.0, 6, &[&["a", "b"]]), None)
            .expect("second snapshot");
        assert_ne!(first, second, "collision guard must keep both files");
        assert!(first.exists() && second.exists());
        let rows = read_trend(dir.path(), 0);
        assert_eq!(rows.len(), 2, "both snapshots show up");
        assert!(rows.iter().any(|row| row.tag.is_none()));
        assert!(
            rows.iter()
                .any(|row| row.cycles == 1 && row.largest_scc == 3),
            "second snapshot row missing"
        );
    }

    #[test]
    fn atomic_snapshot_creation_appends_numbered_suffixes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plain = save_snapshot(dir.path(), 100, "abc1234", b"first").unwrap();
        assert_eq!(plain.file_name().expect("name"), "100-abc1234.json");
        let first = save_snapshot(dir.path(), 100, "abc1234", b"second").unwrap();
        assert_eq!(first.file_name().expect("name"), "100-abc1234-1.json");
        let second = save_snapshot(dir.path(), 100, "abc1234", b"third").unwrap();
        assert_eq!(second.file_name().expect("name"), "100-abc1234-2.json");
        assert_eq!(fs::read(plain).unwrap(), b"first");
        assert_eq!(fs::read(first).unwrap(), b"second");
    }

    #[test]
    fn read_trend_sorts_ascending_and_skips_corrupt_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_raw(dir.path(), "300-abc.json", &snapshot_json(300, 3, 9));
        write_raw(dir.path(), "100-abc.json", &snapshot_json(100, 1, 5));
        write_raw(dir.path(), "200-abc.json", "{ not json");
        write_raw(dir.path(), "notes.txt", "ignored");
        let rows = read_trend(dir.path(), 0);
        assert_eq!(
            rows.iter().map(|row| row.timestamp).collect::<Vec<_>>(),
            vec![100, 300],
            "corrupt file skipped, ascending order"
        );
        assert_eq!(rows[0].cycles, 1);
        assert_eq!(rows[1].edges, 9);
    }

    #[test]
    fn read_trend_last_n_keeps_newest_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        for timestamp in [100_u64, 200, 300] {
            write_raw(
                dir.path(),
                &format!("{timestamp}-abc.json"),
                &snapshot_json(timestamp, u32::try_from(timestamp / 100).expect("small"), 6),
            );
        }
        assert_eq!(read_trend(dir.path(), 0).len(), 3, "last == 0 → all");
        assert_eq!(read_trend(dir.path(), 2).len(), 2);
        assert_eq!(
            read_trend(dir.path(), 9).len(),
            3,
            "last beyond the count → all"
        );
        let one = read_trend(dir.path(), 1);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].timestamp, 300, "the newest row survives");
        assert!(read_trend(&dir.path().join("missing"), 0).is_empty());
    }

    #[test]
    fn same_second_snapshots_keep_write_order_via_numeric_suffix() {
        // A plain file-name sort would put `5-abc-1.json` BEFORE the
        // un-suffixed `5-abc.json` (`-` sorts before `.`) and `…-10`
        // before `…-2` — the tie-break must parse the suffix numerically.
        let dir = tempfile::tempdir().expect("tempdir");
        write_raw(dir.path(), "5-abc.json", &snapshot_json(5, 1, 1));
        write_raw(dir.path(), "5-abc-10.json", &snapshot_json(5, 3, 3));
        write_raw(dir.path(), "5-abc-1.json", &snapshot_json(5, 2, 2));
        let rows = read_trend(dir.path(), 0);
        assert_eq!(
            rows.iter().map(|row| row.edges).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "suffix 0 first, then 1, then 10 — write order, not lex order"
        );
    }

    #[test]
    fn two_snapshots_yield_correct_delta_inputs() {
        // The CLI renders the `+/-` deltas; here the row pair that
        // drives them is pinned: improving metrics must read as a
        // decrease, growing edges as an increase.
        let dir = tempfile::tempdir().expect("tempdir");
        let before = report(3, 4, 3.0, 10, &[&["a", "b"], &["c", "d"], &["e", "f"]]);
        let after = report(1, 2, 1.5, 12, &[&["a", "b", "c", "d"]]);
        write_snapshot(dir.path(), &before, Some("before")).expect("first");
        write_snapshot(dir.path(), &after, None).expect("second");
        // The row pair below pins the deltas with explicit timestamps —
        // both runs share one clock second, so the on-disk order would
        // depend on the collision suffix alone.
        let rows_expected = {
            let mut rows = Vec::new();
            for (offset, rep) in [(100_u64, &before), (200, &after)] {
                rows.push(SnapshotRow::from(&build_snapshot(
                    offset, "notag", None, rep,
                )));
            }
            rows
        };
        assert_eq!(rows_expected[0].cycles - rows_expected[1].cycles, 2);
        assert!(rows_expected[1].acd < rows_expected[0].acd);
        assert!(rows_expected[1].edges > rows_expected[0].edges);
        // The two on-disk snapshots are readable regardless of the clock.
        assert_eq!(read_trend(dir.path(), 0).len(), 2);
    }

    #[test]
    fn snapshot_record_mirrors_the_file_and_joins_hashes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let record = snapshot_record(
            dir.path(),
            &report(2, 3, 2.5, 7, &[&["a", "b"], &["c", "d"]]),
            Some("db"),
        );
        assert!(record.id.ends_with("-notag"), "id: {}", record.id);
        assert!(record.taken_at > 0);
        assert_eq!(record.git_sha, "notag");
        assert_eq!(record.tag.as_deref(), Some("db"));
        assert_eq!(record.cycles, 2);
        assert_eq!(record.largest_scc, 3);
        assert_eq!(record.edges, 7);
        assert_eq!(record.acd, 2.5);
        assert_eq!(record.cycle_hashes.lines().count(), 2);
    }
}
