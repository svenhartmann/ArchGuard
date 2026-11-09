//! One OTLP span reduced to what the runtime profile needs. Attribute names
//! follow the OpenTelemetry semantic conventions; old and new names are both
//! accepted (`db.system`/`db.system.name`, `code.filepath`/`code.file.path`, ...).
use serde_json::Value;

const MAX_TEXT: usize = 2_000;
const MAX_NAME: usize = 512;

pub(super) struct Span {
    pub trace: String,
    pub id: String,
    pub parent: Option<String>,
    pub service: String,
    /// OTLP span kind (0 unspecified … 5 consumer).
    pub kind: u8,
    pub name: String,
    /// Endpoint name for SERVER and CONSUMER spans.
    pub entry: Option<String>,
    pub start: u64,
    pub end: u64,
    pub error: bool,
    pub db: Option<Db>,
    pub site: Option<Site>,
    /// Input index and 1-based line, for evidence.
    pub input: usize,
    pub line: u32,
}

pub(super) struct Db {
    pub system: Option<String>,
    pub key: String,
    pub text: Option<String>,
    pub read_ms: Option<f64>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Site {
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub source: Option<String>,
}

pub(super) fn attribute<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .get("attributes")?
        .as_array()?
        .iter()
        .find(|item| item.get("key").and_then(Value::as_str) == Some(key))?
        .get("value")
}

pub(super) fn text(value: &Value, key: &str, max: usize) -> Option<String> {
    let s = attribute(value, key)?.get("stringValue")?.as_str()?.trim();
    (!s.is_empty()).then(|| bounded(s, max))
}

fn integer(value: &Value, key: &str) -> Option<u64> {
    let v = attribute(value, key)?.get("intValue")?;
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

fn double(value: &Value, key: &str) -> Option<f64> {
    let v = attribute(value, key)?;
    let n = v.get("doubleValue").and_then(Value::as_f64);
    n.or_else(|| integer(value, key).map(|i| i as f64))
        .filter(|n| n.is_finite() && *n >= 0.0)
}

fn bounded(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn identifier(value: Option<&Value>, length: usize) -> Option<String> {
    let s = value?.as_str()?;
    (s.len() == length && s.bytes().all(|b| b.is_ascii_hexdigit()) && s.bytes().any(|b| b != b'0'))
        .then(|| s.to_ascii_lowercase())
}

fn nanos(value: Option<&Value>) -> Option<u64> {
    let v = value?;
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

/// Why a span was skipped; counted per reason.
pub(super) const INVALID: &str = "spans with invalid IDs or timestamps";

impl Span {
    pub fn parse(
        value: &Value,
        service: &str,
        input: usize,
        line: u32,
    ) -> Result<Self, &'static str> {
        let trace = identifier(value.get("traceId"), 32).ok_or(INVALID)?;
        let id = identifier(value.get("spanId"), 16).ok_or(INVALID)?;
        let parent = match value.get("parentSpanId").and_then(Value::as_str) {
            None | Some("") => None,
            Some(_) => Some(identifier(value.get("parentSpanId"), 16).ok_or(INVALID)?),
        };
        let start = nanos(value.get("startTimeUnixNano"))
            .filter(|s| *s > 0)
            .ok_or(INVALID)?;
        let end = nanos(value.get("endTimeUnixNano"))
            .filter(|e| *e >= start)
            .ok_or(INVALID)?;
        let status = value.get("status").and_then(|s| s.get("code"));
        Ok(Span {
            trace,
            id,
            parent,
            service: service.to_owned(),
            kind: kind(value).unwrap_or(0),
            name: name(value),
            entry: entry(value),
            start,
            end,
            error: status
                .is_some_and(|c| c.as_u64() == Some(2) || c.as_str() == Some("STATUS_CODE_ERROR")),
            db: db(value),
            site: site(value),
            input,
            line,
        })
    }
}

/// The span's name; an outgoing HTTP call named only by its method gets its
/// target: host, port and path from `url.full` (never the query string), else
/// `server.address`.
fn name(value: &Value) -> String {
    let raw = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let method = text(value, "http.request.method", 16).or_else(|| text(value, "http.method", 16));
    let target = text(value, "url.full", usize::MAX)
        .or_else(|| text(value, "http.url", usize::MAX))
        .map(|url| {
            let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
            rest.split(['?', '#']).next().unwrap_or_default().to_owned()
        })
        .or_else(|| text(value, "server.address", MAX_NAME));
    match (kind(value), method, target) {
        (Some(3), Some(m), Some(t)) if raw.is_empty() || raw == m => {
            bounded(&format!("{m} {t}"), MAX_NAME)
        }
        _ => bounded(raw, MAX_NAME),
    }
}

fn kind(value: &Value) -> Option<u8> {
    let k = value.get("kind")?;
    match (k.as_u64(), k.as_str()) {
        (Some(n), _) if n <= 5 => Some(n as u8),
        (_, Some("SPAN_KIND_INTERNAL")) => Some(1),
        (_, Some("SPAN_KIND_SERVER")) => Some(2),
        (_, Some("SPAN_KIND_CLIENT")) => Some(3),
        (_, Some("SPAN_KIND_PRODUCER")) => Some(4),
        (_, Some("SPAN_KIND_CONSUMER")) => Some(5),
        _ => None,
    }
}

fn entry(value: &Value) -> Option<String> {
    if !matches!(kind(value), Some(2 | 5)) {
        return None;
    }
    crate::trace_sequences::operation(|key| text(value, key, MAX_NAME)).or_else(|| {
        value
            .get("name")?
            .as_str()
            .filter(|n| !n.is_empty())
            .map(|n| bounded(n, MAX_NAME))
    })
}

fn db(value: &Value) -> Option<Db> {
    let system =
        text(value, "db.system.name", MAX_NAME).or_else(|| text(value, "db.system", MAX_NAME));
    let raw = text(value, "db.query.text", usize::MAX)
        .or_else(|| text(value, "db.statement", usize::MAX));
    if system.is_none() && raw.is_none() {
        return None;
    }
    let normalized = raw.map(|q| bounded(&normalize_sql(&q), MAX_TEXT));
    let key = text(value, "db.query.summary", MAX_NAME)
        .or_else(|| text(value, "archguard.db.statement_hash", MAX_NAME))
        .or_else(|| normalized.clone())
        .or_else(|| value.get("name")?.as_str().map(|n| bounded(n, MAX_NAME)))
        .unwrap_or_default();
    Some(Db {
        system,
        key,
        text: normalized,
        read_ms: double(value, "archguard.db.read_ms"),
    })
}

fn site(value: &Value) -> Option<Site> {
    let function = text(value, "code.function.name", MAX_NAME).or_else(|| {
        let function = text(value, "code.function", MAX_NAME)?;
        Some(match text(value, "code.namespace", MAX_NAME) {
            Some(ns) => format!("{ns}.{function}"),
            None => function,
        })
    });
    let file =
        text(value, "code.file.path", MAX_NAME).or_else(|| text(value, "code.filepath", MAX_NAME));
    let line = integer(value, "code.line.number")
        .or_else(|| integer(value, "code.lineno"))
        .and_then(|l| u32::try_from(l).ok());
    let source = text(value, "archguard.callsite.source", 32);
    (function.is_some() || file.is_some()).then_some(Site {
        function,
        file,
        line,
        source,
    })
}

/// Query text with comments removed, string and number literals replaced by `?`
/// and whitespace collapsed. Placeholders (`@p0`, `$1`, `:name`) stay.
pub(super) fn normalize_sql(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push(' ');
        } else if c == '/' && next == Some('*') {
            i = skip_block_comment(&chars, i + 2);
            out.push(' ');
        } else if c == '\'' {
            i = skip_string(&chars, i + 1);
            out.push('?');
        } else if c.is_ascii_digit() && !continues_word(out.chars().last()) {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            out.push('?');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn continues_word(prev: Option<char>) -> bool {
    prev.is_some_and(|p| {
        p.is_alphanumeric() || matches!(p, '_' | '@' | '$' | ':' | '.' | '"' | '`')
    })
}

/// Index after the closing `'`; a doubled `''` is an escaped quote.
fn skip_string(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() {
        if chars[i] == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    i
}

fn skip_block_comment(chars: &[char], mut i: usize) -> usize {
    while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
        i += 1;
    }
    (i + 2).min(chars.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn literals_and_comments_are_removed_placeholders_stay() {
        let sql = "-- file: OrderService.cs:42\nSELECT *  FROM t1 /* x */ WHERE a = 'it''s' AND b = 42 AND c = @p0 AND d = $1 AND e = 1.5e3";
        assert_eq!(
            normalize_sql(sql),
            "SELECT * FROM t1 WHERE a = ? AND b = ? AND c = @p0 AND d = $1 AND e = ?"
        );
    }

    #[test]
    fn summary_wins_over_hash_and_text() {
        let span = json!({"attributes":[
            {"key":"db.query.text","value":{"stringValue":"SELECT 1"}},
            {"key":"archguard.db.statement_hash","value":{"stringValue":"h1"}},
            {"key":"db.query.summary","value":{"stringValue":"SELECT orders"}}]});
        assert_eq!(db(&span).unwrap().key, "SELECT orders");
        let span = json!({"attributes":[
            {"key":"db.query.text","value":{"stringValue":"SELECT 1"}},
            {"key":"archguard.db.statement_hash","value":{"stringValue":"h1"}}]});
        assert_eq!(db(&span).unwrap().key, "h1");
    }

    #[test]
    fn long_text_is_bounded_on_a_char_boundary() {
        let s = "ä".repeat(10);
        assert_eq!(bounded(&s, 5), "ää…");
    }
}
