//! The files of one recording: `traces-<nnnn>.jsonl` in the run directory,
//! numbered from `0001`, a new file per process and after `max_bytes`
//! (probes/CONVENTIONS.md, "Files").

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Default size at which a file is rotated.
pub(crate) const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Writes complete lines; opens its first file only when the first line
/// arrives, so a run without spans leaves no empty file.
pub(crate) struct Sink {
    /// With `project_root`, the run name below `.archguard/runtime`.
    dir: PathBuf,
    /// Set when the directory lies below this analysed root, whose
    /// components are repository content and must not be followed.
    project_root: Option<PathBuf>,
    /// A refused project directory turns the probe off after one error.
    disabled: bool,
    max_bytes: u64,
    file: Option<(File, u64)>,
}

impl Sink {
    pub(crate) fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Sink {
            dir,
            project_root: None,
            disabled: false,
            max_bytes: max_bytes.max(1),
            file: None,
        }
    }

    /// The default `<root>/.archguard/runtime/<run>`, created and opened
    /// without following a symlink below `root`.
    pub(crate) fn in_project(root: &Path, run: &str, max_bytes: u64) -> Self {
        Sink {
            project_root: Some(root.to_path_buf()),
            ..Sink::new(PathBuf::from(run), max_bytes)
        }
    }

    /// Write one line (ending in `\n`) with a single `write_all`.
    pub(crate) fn write_line(&mut self, line: &str) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let full = self
            .file
            .as_ref()
            .is_some_and(|(_, written)| written + line.len() as u64 > self.max_bytes);
        if full || self.file.is_none() {
            let created = match &self.project_root {
                Some(root) => create_next_in_project(root, &self.dir),
                None => create_next(&self.dir),
            };
            self.disabled = created.is_err() && self.project_root.is_some();
            self.file = Some((created?, 0));
        }
        let (file, written) = self.file.as_mut().expect("opened above");
        file.write_all(line.as_bytes())?;
        *written += line.len() as u64;
        Ok(())
    }
}

/// Create the next free `traces-<nnnn>.jsonl`; never reuses a file another
/// process wrote, even when two processes start at once.
fn create_next(dir: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    let mut number = highest_number(dir)? + 1;
    loop {
        let path = dir.join(format!("traces-{number:04}.jsonl"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => number += 1,
            other => return other,
        }
    }
}

/// [`create_next`] in `<root>/.archguard/runtime/<run>` through held
/// directory descriptors: a symlinked component is refused (ELOOP or
/// ENOTDIR), never followed, even if it is swapped in mid-run.
#[cfg(unix)]
fn create_next_in_project(root: &Path, run: &Path) -> io::Result<File> {
    use crate::project_files::safe::{open_project_directory, open_subdirectory};
    use rustix::fs::{Dir, Mode, OFlags, openat};
    use rustix::io::Errno;

    let mut dir = open_project_directory(root)?;
    for name in [Path::new("runtime"), run] {
        dir = open_subdirectory(&dir, name)?;
    }
    let mut highest = 0;
    for entry in Dir::read_from(&dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        highest = highest.max(trace_number(&name).unwrap_or(0));
    }
    let mut number = highest + 1;
    loop {
        match openat(
            &dir,
            format!("traces-{number:04}.jsonl"),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_bits_retain(0o644),
        ) {
            Err(Errno::EXIST) => number += 1,
            other => return Ok(File::from(other?)),
        }
    }
}

/// Without no-follow syscalls: create each component and refuse it unless
/// it is a real directory, as `project_files::write_project_file` does.
#[cfg(not(unix))]
fn create_next_in_project(root: &Path, run: &Path) -> io::Result<File> {
    let mut dir = root.to_path_buf();
    let project = Path::new(crate::project_files::PROJECT_DIR);
    for name in [project, Path::new("runtime"), run] {
        dir.push(name);
        match fs::create_dir(&dir) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
        let metadata = fs::symlink_metadata(&dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::other("probe directory is not a real directory"));
        }
    }
    create_next(&dir)
}

fn highest_number(dir: &Path) -> io::Result<u32> {
    let mut highest = 0;
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        highest = highest.max(name.to_str().and_then(trace_number).unwrap_or(0));
    }
    Ok(highest)
}

fn trace_number(name: &str) -> Option<u32> {
    name.strip_prefix("traces-")?
        .strip_suffix(".jsonl")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn no_line_no_file_and_numbering_starts_at_0001() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("run");
        let mut sink = Sink::new(dir.clone(), DEFAULT_MAX_BYTES);
        assert!(!dir.exists());
        sink.write_line("{}\n").unwrap();
        sink.write_line("{}\n").unwrap();
        assert_eq!(names(&dir), ["traces-0001.jsonl"]);
        assert_eq!(
            fs::read_to_string(dir.join("traces-0001.jsonl")).unwrap(),
            "{}\n{}\n"
        );
    }

    #[test]
    fn a_restart_takes_the_next_number_and_never_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        fs::write(dir.join("traces-0007.jsonl"), "old\n").unwrap();
        Sink::new(dir.clone(), DEFAULT_MAX_BYTES)
            .write_line("a\n")
            .unwrap();
        Sink::new(dir.clone(), DEFAULT_MAX_BYTES)
            .write_line("b\n")
            .unwrap();
        assert_eq!(
            names(&dir),
            [
                "traces-0007.jsonl",
                "traces-0008.jsonl",
                "traces-0009.jsonl"
            ]
        );
        assert_eq!(
            fs::read_to_string(dir.join("traces-0007.jsonl")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn a_full_file_rotates_to_the_next_number() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let mut sink = Sink::new(dir.clone(), 10);
        sink.write_line("123456\n").unwrap();
        sink.write_line("123456\n").unwrap();
        assert_eq!(names(&dir), ["traces-0001.jsonl", "traces-0002.jsonl"]);
    }

    #[test]
    fn the_project_directory_numbers_like_any_other() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(".archguard/runtime/archguard");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("traces-0003.jsonl"), "old\n").unwrap();
        let mut sink = Sink::in_project(root.path(), "archguard", 10);
        sink.write_line("123456\n").unwrap();
        sink.write_line("123456\n").unwrap();
        assert_eq!(
            names(&dir),
            [
                "traces-0003.jsonl",
                "traces-0004.jsonl",
                "traces-0005.jsonl"
            ]
        );
    }

    /// A repository-committed symlink at `link` (below the root) must leave
    /// its target untouched and turn the probe off after one error.
    #[cfg(unix)]
    fn refuses_symlinked(link: &str) {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join(link);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let mut sink = Sink::in_project(root.path(), "archguard", DEFAULT_MAX_BYTES);
        assert!(sink.write_line("{}\n").is_err());
        assert!(sink.write_line("{}\n").is_ok(), "off after one refusal");
        assert!(names(outside.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_archguard_directory_is_not_written_through() {
        refuses_symlinked(".archguard");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_runtime_directory_is_not_written_through() {
        refuses_symlinked(".archguard/runtime");
    }
}
