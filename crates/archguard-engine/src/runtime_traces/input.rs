//! Finding and reading runtime input: `.archguard/runtime/**/*.jsonl` (OTLP JSON
//! Lines, one export request per line) and the legacy single document
//! `.archguard/otel-traces.json`. Symlinks are never followed; oversized lines
//! and files are skipped with a diagnostic instead of being truncated.
use super::span::Span;
use archguard_core::RuntimeDiagnostic;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

const LEGACY: &str = ".archguard/otel-traces.json";
const RUNTIME: &str = ".archguard/runtime";
const MAX_DEPTH: usize = 4;
const MAX_LINE: u64 = 16 * 1024 * 1024;
const MAX_FILE: u64 = 1024 * 1024 * 1024;
pub(super) const MAX_SPANS: usize = 500_000;

pub(super) struct Input {
    pub rel: String,
    path: Option<PathBuf>,
    refused: Option<&'static str>,
}

/// What reading produced: spans, sampling values and diagnostics.
#[derive(Default)]
pub(super) struct Reader {
    pub spans: Vec<Span>,
    pub sampling: Vec<String>,
    pub diagnostics: Vec<RuntimeDiagnostic>,
    pub complete: bool,
    skipped: BTreeMap<(usize, &'static str), usize>,
    full: bool,
}

/// Input files in read order; `None` when the project has no runtime input at all.
pub(super) fn discover(root: &Path) -> Option<Vec<Input>> {
    let mut inputs = Vec::new();
    let legacy = root.join(LEGACY);
    if fs::symlink_metadata(&legacy).is_ok() {
        inputs.push(entry(root, legacy));
    }
    let dir = root.join(RUNTIME);
    if let Ok(meta) = fs::symlink_metadata(&dir) {
        if meta.is_dir() && no_symlink(root, &dir) {
            walk(root, &dir, 0, &mut inputs);
        } else {
            inputs.push(refused(
                RUNTIME.into(),
                "runtime input directory must be a real directory, not a symlink",
            ));
        }
    }
    (!inputs.is_empty()).then_some(inputs)
}

fn no_symlink(root: &Path, dir: &Path) -> bool {
    fs::symlink_metadata(root.join(".archguard")).is_ok_and(|m| m.is_dir())
        && fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir())
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Input>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() && depth + 1 < MAX_DEPTH {
            walk(root, &path, depth + 1, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(entry(root, path));
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn refused(rel: String, reason: &'static str) -> Input {
    Input {
        rel,
        path: None,
        refused: Some(reason),
    }
}

fn entry(root: &Path, path: PathBuf) -> Input {
    let rel = relative(root, &path);
    match fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() => refused(rel, "symlinked input refused"),
        Ok(m) if !m.is_file() => refused(rel, "input is not a regular file"),
        Ok(m) if m.len() > MAX_FILE => refused(rel, "input larger than 1 GiB refused"),
        Ok(_) => Input {
            rel,
            path: Some(path),
            refused: None,
        },
        Err(_) => refused(rel, "input not readable"),
    }
}

impl Reader {
    pub fn new() -> Self {
        Self {
            complete: true,
            ..Self::default()
        }
    }

    fn diagnose(&mut self, file: &str, line: Option<u32>, message: impl Into<String>) {
        self.complete = false;
        self.diagnostics.push(RuntimeDiagnostic {
            file: file.into(),
            line,
            message: message.into(),
        });
    }

    pub fn read(&mut self, index: usize, input: &Input) {
        if let Some(reason) = input.refused {
            return self.diagnose(&input.rel, None, reason);
        }
        let Some(file) = input.path.as_ref().and_then(|p| open(p).ok()) else {
            return self.diagnose(&input.rel, None, "input not readable");
        };
        if input.rel == LEGACY {
            self.read_document(index, input, file);
        } else {
            self.read_lines(index, input, file);
        }
        self.flush_skipped(index, input);
    }

    fn read_document(&mut self, index: usize, input: &Input, file: fs::File) {
        let mut bytes = Vec::new();
        if file.take(MAX_LINE + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_LINE
        {
            return self.diagnose(
                &input.rel,
                None,
                "legacy trace document unreadable or larger than 16 MiB; skipped",
            );
        }
        self.document(index, input, &bytes, 1);
    }

    fn read_lines(&mut self, index: usize, input: &Input, file: fs::File) {
        let mut reader = BufReader::new(file);
        let mut buf = Vec::new();
        for line in 1u32.. {
            buf.clear();
            match (&mut reader).take(MAX_LINE + 1).read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) if buf.len() as u64 > MAX_LINE && buf.last() != Some(&b'\n') => {
                    self.diagnose(&input.rel, Some(line), "line larger than 16 MiB; skipped");
                    if skip_to_newline(&mut reader).is_err() {
                        break;
                    }
                }
                Ok(_) => self.document(index, input, &buf, line),
                Err(_) => {
                    return self.diagnose(
                        &input.rel,
                        Some(line),
                        "read error; rest of file skipped",
                    );
                }
            }
            if self.full {
                break;
            }
        }
    }

    fn document(&mut self, index: usize, input: &Input, bytes: &[u8], line: u32) {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let resources = serde_json::from_slice::<Value>(bytes)
            .ok()
            .and_then(|v| v.get("resourceSpans").and_then(Value::as_array).cloned());
        let Some(resources) = resources else {
            return self.diagnose(
                &input.rel,
                Some(line),
                "line is not an OTLP JSON trace export (resourceSpans); skipped",
            );
        };
        for resource in &resources {
            self.resource(index, resource, line);
        }
    }

    fn resource(&mut self, index: usize, resource: &Value, line: u32) {
        let attrs = resource.get("resource").unwrap_or(&Value::Null);
        let service = super::span::text(attrs, "service.name", 128)
            .unwrap_or_else(|| "unknown_service".into());
        if let Some(s) = super::span::text(attrs, "archguard.sampling", 64)
            && !self.sampling.contains(&s)
        {
            self.sampling.push(s);
        }
        let scopes = resource.get("scopeSpans").and_then(Value::as_array);
        for span in scopes
            .into_iter()
            .flatten()
            .filter_map(|s| s.get("spans")?.as_array())
            .flatten()
        {
            if self.spans.len() >= MAX_SPANS {
                self.full = true;
                return;
            }
            match Span::parse(span, &service, index, line) {
                Ok(span) => self.spans.push(span),
                Err(reason) => *self.skipped.entry((index, reason)).or_default() += 1,
            }
        }
    }

    fn flush_skipped(&mut self, index: usize, input: &Input) {
        let skipped: Vec<_> = self
            .skipped
            .iter()
            .filter(|((i, _), _)| *i == index)
            .map(|((_, r), n)| (*r, *n))
            .collect();
        for (reason, count) in skipped {
            self.diagnose(&input.rel, None, format!("{count} {reason} skipped"));
        }
        if self.full {
            self.diagnose(
                &input.rel,
                None,
                format!("span limit of {MAX_SPANS} reached; remaining input not read"),
            );
        }
    }

    pub fn is_full(&self) -> bool {
        self.full
    }
}

fn skip_to_newline(reader: &mut impl BufRead) -> io::Result<()> {
    loop {
        let (done, used) = {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                return Ok(());
            }
            match buf.iter().position(|b| *b == b'\n') {
                Some(p) => (true, p + 1),
                None => (false, buf.len()),
            }
        };
        reader.consume(used);
        if done {
            return Ok(());
        }
    }
}

#[cfg(unix)]
fn open(path: &Path) -> io::Result<fs::File> {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    let file = fs::File::from(fd);
    if file.metadata()?.is_file() {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ))
    }
}

#[cfg(not(unix))]
fn open(path: &Path) -> io::Result<fs::File> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "symlink"));
    }
    fs::File::open(path)
}
