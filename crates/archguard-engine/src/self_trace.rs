//! ArchGuard records its own runtime (ADR 0004): with `ARCHGUARD_PROBE=1`
//! its `tracing` spans are written as OTLP JSON Lines to
//! `<project>/.archguard/runtime/archguard/traces-<nnnn>.jsonl`
//! (`ARCHGUARD_PROBE_DIR` replaces that directory), so the Runtime tab and
//! `archguard runtime` show ArchGuard's own requests and phases.
//!
//! Without the variable no subscriber is installed; a span then costs one
//! cached "disabled" check at its call site.
//!
//! Requests are root spans from [`entry_span`]: one per CLI command, MCP
//! tool call or desktop action. Method spans come from
//! `#[tracing::instrument(skip_all)]` at the pipeline's phases.

mod layer;
mod sink;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt;

use layer::{OtlpLayer, Pending};
use sink::{DEFAULT_MAX_BYTES, Sink};

/// Re-exported so crates without their own `tracing` dependency (the
/// desktop app) can open spans.
pub use tracing;

/// `1` turns the probe on.
pub const PROBE_ENV: &str = "ARCHGUARD_PROBE";
/// Output directory instead of `<project>/.archguard/runtime/archguard`.
pub const PROBE_DIR_ENV: &str = "ARCHGUARD_PROBE_DIR";
/// Size in bytes at which a trace file is rotated (default 64 MiB).
pub const PROBE_MAX_BYTES_ENV: &str = "ARCHGUARD_PROBE_MAX_BYTES";

/// The `<run>` directory name of ArchGuard's own recordings. One name for
/// all processes: each process writes the next numbered file, so the
/// Runtime tab lists every CLI run of a project together.
pub const RUN_NAME: &str = "archguard";

/// How often closed spans are written; the desktop's live view re-reads
/// once a second.
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

/// A root span that counts as one request (`archguard.entry = true`),
/// named `name`: `archguard check`, an MCP tool name, `desktop: show …`.
pub fn entry_span(name: &str) -> tracing::Span {
    tracing::info_span!(parent: None, "entry", otel.name = name, archguard.entry = true)
}

/// Run `f` on this thread without recording spans: for ArchGuard reading
/// its own recordings (the Runtime tab's live reload), which would otherwise
/// record itself, change the file and reload again every second.
pub fn untraced<R>(f: impl FnOnce() -> R) -> R {
    tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), f)
}

/// Install the probe when `ARCHGUARD_PROBE=1`. Keep the returned value
/// until the process ends: dropping it writes the remaining spans.
/// `None` when the probe is off or another subscriber is installed.
pub fn start(project_root: &Path, service: &str) -> Option<Probe> {
    if std::env::var_os(PROBE_ENV).is_none_or(|v| v != "1") {
        return None;
    }
    let dir = std::env::var_os(PROBE_DIR_ENV).map_or_else(
        || project_root.join(".archguard/runtime").join(RUN_NAME),
        PathBuf::from,
    );
    let max_bytes = std::env::var(PROBE_MAX_BYTES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_BYTES);
    let (mut probe, subscriber) = Probe::recorder(dir, max_bytes, service);
    tracing::subscriber::set_global_default(subscriber).ok()?;
    probe.spawn_flusher();
    Some(probe)
}

/// The installed probe; writes pending spans at least every
/// [`FLUSH_INTERVAL`] and once more when dropped.
pub struct Probe {
    writer: Arc<Writer>,
    flusher: Option<(mpsc::Sender<()>, JoinHandle<()>)>,
}

struct Writer {
    pending: Pending,
    sink: Mutex<Sink>,
    resource: Value,
}

impl Probe {
    /// The probe and its subscriber, not yet installed (tests install it
    /// per thread).
    pub(crate) fn recorder(
        dir: PathBuf,
        max_bytes: u64,
        service: &str,
    ) -> (Probe, impl tracing::Subscriber + Send + Sync) {
        let pending = Pending::default();
        let writer = Arc::new(Writer {
            pending: pending.clone(),
            sink: Mutex::new(Sink::new(dir, max_bytes)),
            resource: resource(service),
        });
        let subscriber = tracing_subscriber::registry().with(OtlpLayer::new(pending));
        let probe = Probe {
            writer,
            flusher: None,
        };
        (probe, subscriber)
    }

    fn spawn_flusher(&mut self) {
        let (stop, stopped) = mpsc::channel::<()>();
        let writer = self.writer.clone();
        let thread = std::thread::spawn(move || {
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(FLUSH_INTERVAL) {
                writer.flush();
            }
        });
        self.flusher = Some((stop, thread));
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if let Some((stop, thread)) = self.flusher.take() {
            drop(stop);
            let _ = thread.join();
        }
        self.writer.flush();
    }
}

impl Writer {
    fn flush(&self) {
        let spans = match self.pending.lock() {
            Ok(mut pending) if !pending.is_empty() => std::mem::take(&mut *pending),
            _ => return,
        };
        let line = export_line(&self.resource, spans);
        let written = self.sink.lock().map(|mut sink| sink.write_line(&line));
        if let Ok(Err(error)) = written {
            eprintln!("warning: ARCHGUARD_PROBE could not write its trace file: {error}");
        }
    }
}

/// The resource of every line: service, probe schema and sampling, nothing
/// about host, user or command line.
fn resource(service: &str) -> Value {
    let attribute =
        |key: &str, value: &str| json!({ "key": key, "value": { "stringValue": value } });
    json!({
        "attributes": [
            attribute("service.name", service),
            attribute("archguard.probe.schema", "1"),
            attribute("archguard.sampling", "always_on"),
        ]
    })
}

/// One `ExportTraceServiceRequest` as one line.
fn export_line(resource: &Value, spans: Vec<Value>) -> String {
    let request = json!({
        "resourceSpans": [{
            "resource": resource,
            "scopeSpans": [{
                "scope": { "name": "archguard", "version": env!("CARGO_PKG_VERSION") },
                "spans": spans,
            }],
        }]
    });
    format!("{request}\n")
}

#[cfg(test)]
mod tests {

    #[test]
    fn untraced_work_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (probe, subscriber) =
            Probe::recorder(dir.path().to_path_buf(), DEFAULT_MAX_BYTES, "test");
        tracing::subscriber::with_default(subscriber, || {
            assert!(!tracing::info_span!("recorded").is_disabled());
            untraced(|| assert!(tracing::info_span!("hidden").is_disabled()));
        });
        drop(probe);
    }
    use super::*;

    /// Run `f` with a recorder installed on this thread; the written lines.
    fn record(dir: &Path, f: impl FnOnce()) -> Vec<Value> {
        let (probe, subscriber) =
            Probe::recorder(dir.to_path_buf(), DEFAULT_MAX_BYTES, "archguard");
        tracing::subscriber::with_default(subscriber, f);
        drop(probe);
        let text = std::fs::read_to_string(dir.join("traces-0001.jsonl")).unwrap();
        assert!(text.ends_with('\n'));
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn spans(lines: &[Value]) -> Vec<Value> {
        let spans = |l: &Value| l["resourceSpans"][0]["scopeSpans"][0]["spans"].clone();
        lines
            .iter()
            .flat_map(|l| spans(l).as_array().unwrap().clone())
            .collect()
    }

    fn attribute<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
        let attributes = span["attributes"].as_array()?;
        attributes
            .iter()
            .find(|a| a["key"] == key)
            .map(|a| &a["value"])
    }

    #[tracing::instrument(skip_all)]
    fn phase(_secret: &Path) {
        let _inner =
            tracing::info_span!("inner", archguard.language = "rs", path = "/Users/me/p").entered();
    }

    fn check_run() {
        let _entry = entry_span("archguard check").entered();
        phase(Path::new("/Users/me/project"));
    }

    #[test]
    fn an_entry_span_with_method_spans_is_one_otlp_trace() {
        let tmp = tempfile::tempdir().unwrap();
        let lines = record(tmp.path(), check_run);
        let resource = &lines[0]["resourceSpans"][0]["resource"];
        let text = |key| attribute(resource, key).unwrap()["stringValue"].clone();
        assert_eq!(text("service.name"), "archguard");
        assert_eq!(text("archguard.probe.schema"), "1");
        assert_eq!(text("archguard.sampling"), "always_on");

        let spans = spans(&lines);
        assert_eq!(spans.len(), 3, "{spans:#?}");
        let by_name = |n: &str| spans.iter().find(|s| s["name"] == n).unwrap();
        let entry = by_name("archguard check");
        let method = by_name("tests.phase");
        let inner = by_name("tests.inner");
        assert!(entry.get("parentSpanId").is_none());
        assert_eq!(
            attribute(entry, "archguard.entry").unwrap()["boolValue"],
            true
        );
        assert!(attribute(entry, "code.function.name").is_none());
        assert_eq!(method["parentSpanId"], entry["spanId"]);
        assert_eq!(inner["parentSpanId"], method["spanId"]);
        for span in &spans {
            assert_eq!(span["traceId"], entry["traceId"]);
            assert_eq!(span["kind"], 1);
            assert_eq!(span["traceId"].as_str().unwrap().len(), 32);
            assert_eq!(span["spanId"].as_str().unwrap().len(), 16);
            let nanos = |key: &str| span[key].as_str().unwrap().parse::<u128>().unwrap();
            let (start, end) = (nanos("startTimeUnixNano"), nanos("endTimeUnixNano"));
            assert!(start > 1_600_000_000_000_000_000 && end >= start);
        }
        let text = |span, key| attribute(span, key).unwrap()["stringValue"].clone();
        assert_eq!(
            text(method, "code.function.name"),
            "archguard_engine.self_trace.tests.phase"
        );
        assert_eq!(
            text(method, "code.file.path"),
            "crates/archguard-engine/src/self_trace.rs"
        );
        assert_eq!(text(method, "archguard.section.kind"), "method");
        assert_eq!(text(inner, "archguard.language"), "rs");
        let line = attribute(method, "code.line.number").unwrap()["intValue"].clone();
        assert!(line.as_str().unwrap().parse::<u32>().unwrap() > 0);
    }

    #[test]
    fn no_paths_users_or_hosts_are_written() {
        let tmp = tempfile::tempdir().unwrap();
        let text = serde_json::to_string(&record(tmp.path(), check_run)).unwrap();
        for forbidden in ["/Users", "me/p", "host.", "process.", "user", "\"path\""] {
            assert!(!text.contains(forbidden), "{forbidden} in {text}");
        }
    }

    #[test]
    fn foreign_spans_are_not_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let lines = record(tmp.path(), || {
            let _own = entry_span("x").entered();
            let _foreign = tracing::info_span!(target: "rmcp::service", "serve").entered();
        });
        assert_eq!(spans(&lines).len(), 1);
    }

    #[test]
    fn written_spans_read_back_as_a_request() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".archguard/runtime").join(RUN_NAME);
        record(&dir, check_run);
        let profile = crate::runtime_traces::runtime_profile(tmp.path(), None).expect("profile");
        assert_eq!(profile.requests.len(), 1, "{:?}", profile.diagnostics);
        let request = &profile.requests[0];
        assert_eq!(request.name, "archguard check");
        assert_eq!(request.service, "archguard");
        let names: Vec<_> = request
            .spans
            .iter()
            .map(|s| (s.depth, s.name.as_str()))
            .collect();
        assert_eq!(names, [(1, "tests.phase"), (2, "tests.inner")]);
        let file = request.spans[0].file.as_deref();
        assert_eq!(file, Some("crates/archguard-engine/src/self_trace.rs"));
    }
}
