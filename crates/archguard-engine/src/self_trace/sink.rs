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
    dir: PathBuf,
    max_bytes: u64,
    file: Option<(File, u64)>,
}

impl Sink {
    pub(crate) fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Sink {
            dir,
            max_bytes: max_bytes.max(1),
            file: None,
        }
    }

    /// Write one line (ending in `\n`) with a single `write_all`.
    pub(crate) fn write_line(&mut self, line: &str) -> io::Result<()> {
        let full = self
            .file
            .as_ref()
            .is_some_and(|(_, written)| written + line.len() as u64 > self.max_bytes);
        if full || self.file.is_none() {
            self.file = Some((create_next(&self.dir)?, 0));
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

fn highest_number(dir: &Path) -> io::Result<u32> {
    let mut highest = 0;
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        let number = name.to_str().and_then(|n| {
            n.strip_prefix("traces-")?
                .strip_suffix(".jsonl")?
                .parse()
                .ok()
        });
        highest = highest.max(number.unwrap_or(0));
    }
    Ok(highest)
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
}
