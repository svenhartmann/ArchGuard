//! A `tracing_subscriber::Layer` that turns ArchGuard's own closed spans
//! into OTLP spans (probes/CONVENTIONS.md): nanosecond timestamps as
//! strings, hex ids, `kind` 1 (INTERNAL), `intValue` as string.
//!
//! Only spans whose target starts with `archguard` are recorded, so
//! dependencies that use `tracing` (rmcp) stay out. Of a span's fields only
//! `archguard.*` become attributes and `otel.name` renames the span; other
//! fields are dropped, so a path passed by mistake is never written.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Closed spans waiting for the next flush.
pub(crate) type Pending = Arc<Mutex<Vec<Value>>>;

pub(crate) struct OtlpLayer {
    pending: Pending,
    clock: Clock,
    ids: Ids,
}

impl OtlpLayer {
    pub(crate) fn new(pending: Pending) -> Self {
        OtlpLayer {
            pending,
            clock: Clock::new(),
            ids: Ids::new(),
        }
    }
}

/// Wall-clock nanoseconds from one reading plus a monotonic offset, so
/// durations keep the monotonic clock's resolution.
struct Clock {
    unix_nanos: u128,
    base: Instant,
}

impl Clock {
    fn new() -> Self {
        let unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        Clock {
            unix_nanos,
            base: Instant::now(),
        }
    }

    fn now(&self) -> u128 {
        self.unix_nanos + self.base.elapsed().as_nanos()
    }
}

/// Non-zero pseudo-random ids (splitmix64 over a counter): unique within a
/// process, and seeded per process so runs do not collide.
struct Ids {
    seed: u64,
    counter: AtomicU64,
}

impl Ids {
    fn new() -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Ids {
            seed: time ^ (u64::from(std::process::id()) << 32),
            counter: AtomicU64::new(1),
        }
    }

    fn next(&self) -> u64 {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut z = self
            .seed
            .wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)).max(1)
    }
}

/// What a span carries between start and close.
struct Open {
    trace: String,
    id: u64,
    parent: Option<u64>,
    start: u128,
    fields: Fields,
}

#[derive(Default)]
struct Fields {
    name: Option<String>,
    attributes: Vec<(String, Value)>,
}

impl Fields {
    fn set(&mut self, field: &Field, value: Value) {
        let key = field.name();
        if key == "otel.name" {
            self.name = value
                .get("stringValue")
                .and_then(Value::as_str)
                .map(str::to_owned);
        } else if key.starts_with("archguard.") {
            self.attributes.retain(|(k, _)| k != key);
            self.attributes.push((key.to_owned(), value));
        }
    }

    fn entry(&self) -> bool {
        self.attributes
            .iter()
            .any(|(k, v)| k == "archguard.entry" && v["boolValue"] == true)
    }
}

impl Visit for Fields {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.set(field, json!({ "boolValue": value }));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.set(field, json!({ "intValue": value.to_string() }));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.set(field, json!({ "intValue": value.to_string() }));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.set(field, json!({ "stringValue": value }));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.set(field, json!({ "stringValue": format!("{value:?}") }));
    }
}

/// ArchGuard's own spans; events are not recorded.
fn wanted(metadata: &Metadata<'_>) -> bool {
    metadata.is_span() && metadata.target().starts_with("archguard")
}

impl<S> Layer<S> for OtlpLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn register_callsite(
        &self,
        metadata: &'static Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        if wanted(metadata) {
            tracing::subscriber::Interest::always()
        } else {
            tracing::subscriber::Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>, _: Context<'_, S>) -> bool {
        wanted(metadata)
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let parent = span.parent().and_then(|p| {
            let ext = p.extensions();
            ext.get::<Open>().map(|o| (o.trace.clone(), o.id))
        });
        let (trace, parent) = match parent {
            Some((trace, id)) => (trace, Some(id)),
            None => (
                format!("{:016x}{:016x}", self.ids.next(), self.ids.next()),
                None,
            ),
        };
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        span.extensions_mut().insert(Open {
            trace,
            id: self.ids.next(),
            parent,
            start: self.clock.now(),
            fields,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(open) = span.extensions_mut().get_mut::<Open>()
        {
            values.record(&mut open.fields);
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(open) = span.extensions_mut().remove::<Open>() else {
            return;
        };
        let value = otlp_span(open, span.metadata(), self.clock.now());
        if let Ok(mut pending) = self.pending.lock() {
            pending.push(value);
        }
    }
}

fn otlp_span(open: Open, metadata: &Metadata<'_>, end: u128) -> Value {
    let module = metadata.module_path().unwrap_or_else(|| metadata.target());
    let short = module.rsplit("::").next().unwrap_or(module);
    let entry = open.fields.entry();
    let name = open
        .fields
        .name
        .clone()
        .unwrap_or_else(|| format!("{short}.{}", metadata.name()));
    let mut attributes: Vec<Value> = open
        .fields
        .attributes
        .into_iter()
        .map(|(key, value)| json!({ "key": key, "value": value }))
        .collect();
    if !entry {
        method_attributes(&mut attributes, module, metadata);
    }
    let mut span = json!({
        "traceId": open.trace,
        "spanId": format!("{:016x}", open.id),
        "name": name,
        "kind": 1,
        "startTimeUnixNano": open.start.to_string(),
        "endTimeUnixNano": end.max(open.start).to_string(),
        "attributes": attributes,
    });
    if let Some(parent) = open.parent {
        span["parentSpanId"] = json!(format!("{parent:016x}"));
    }
    span
}

/// The method-span attributes (CONVENTIONS.md, "Method spans"): section
/// kind `method` unless the span set one, the function from module path
/// and span name, the location of ArchGuard's own source.
fn method_attributes(attributes: &mut Vec<Value>, module: &str, metadata: &Metadata<'_>) {
    let has = |key: &str| attributes.iter().any(|a| a["key"] == key);
    if !has("archguard.section.kind") {
        attributes.push(string("archguard.section.kind", "method"));
    }
    let function = format!("{}.{}", module.replace("::", "."), metadata.name());
    attributes.push(string("code.function.name", &function));
    if let Some(file) = metadata.file() {
        attributes.push(string("code.file.path", &source_path(file)));
    }
    if let Some(line) = metadata.line() {
        attributes
            .push(json!({ "key": "code.line.number", "value": { "intValue": line.to_string() } }));
    }
}

fn string(key: &str, value: &str) -> Value {
    json!({ "key": key, "value": { "stringValue": value } })
}

/// ArchGuard's source file relative to the workspace: an absolute build
/// path is cut below `crates/`, else to the file name, so no user or host
/// name is written.
pub(crate) fn source_path(file: &str) -> String {
    let file = file.replace('\\', "/");
    let absolute = file.starts_with('/') || file.get(1..2) == Some(":");
    if !absolute {
        return file;
    }
    match file.rfind("/crates/") {
        Some(at) => file[at + 1..].to_owned(),
        None => file.rsplit('/').next().unwrap_or_default().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_carry_no_user_directories() {
        assert_eq!(source_path("crates/a/src/x.rs"), "crates/a/src/x.rs");
        assert_eq!(
            source_path("/Users/me/p/crates/a/src/x.rs"),
            "crates/a/src/x.rs"
        );
        assert_eq!(source_path("/home/me/.cargo/registry/y.rs"), "y.rs");
        assert_eq!(
            source_path("C:\\Users\\me\\crates\\a\\x.rs"),
            "crates/a/x.rs"
        );
    }

    #[test]
    fn ids_are_non_zero_and_distinct() {
        let ids = Ids::new();
        let a = ids.next();
        let b = ids.next();
        assert!(a != 0 && b != 0 && a != b);
    }
}
