# Runtime input conventions (schema 1)

This is the contract between programs that record runtime data and the
ArchGuard importer (`crates/archguard-engine/src/runtime_traces*`). The
decision behind it is [ADR 0004](../docs/adr/0004-runtime-probes.md).
Any OpenTelemetry setup can produce valid input; the ArchGuard probes add
the optional `archguard.*` attributes.

## Files

| Path | Format |
| --- | --- |
| `.archguard/runtime/**/*.jsonl` (up to 4 directory levels) | OTLP JSON Lines: one `ExportTraceServiceRequest` (`{"resourceSpans":[...]}`) per line, as in the [OTLP file exporter spec](https://opentelemetry.io/docs/specs/otel/protocol/file-exporter/) |
| `.archguard/otel-traces.json` (legacy) | One OTLP JSON document, at most 16 MiB |

Files are read in path order. Symlinks are refused. Lines over 16 MiB, files
over 1 GiB and spans beyond 500,000 are skipped with a diagnostic; the
profile is then marked incomplete. A span repeated with the same trace and
span id counts once.

Recommended layout for one analysis run: `.archguard/runtime/<run>/traces-<n>.jsonl`.

## Attributes the importer reads

Old attribute names are accepted next to the stable ones.

### Resource

| Attribute | Use |
| --- | --- |
| `service.name` | Service of all spans in the resource; `unknown_service` when absent |
| `archguard.probe.schema` | Set by ArchGuard probes, currently `1` |
| `archguard.sampling` | `always_on` or the sampling ratio; reported with every profile |

### Request spans (kind SERVER or CONSUMER)

Each such span is one request. Its endpoint name is `http.request.method` (or
`http.method`) plus `http.route`, else `rpc.service/rpc.method`, else the span
name. Status code ERROR counts as an error.

### Database spans

A span with `db.system.name` / `db.system` or `db.query.text` / `db.statement`
is a database call. It belongs to the nearest request span of the same
service above it; without one, it counts as "outside a request". When a
database span has a database span as child (an ORM span around the driver
span), only the innermost one is counted.

| Attribute | Use |
| --- | --- |
| `db.system.name` / `db.system` | Database system, e.g. `postgresql` |
| `db.query.summary` | Grouping key, preferred |
| `archguard.db.statement_hash` | Grouping key when no summary exists |
| `db.query.text` / `db.statement` | Normalized text: comments removed, string and number literals replaced by `?`, whitespace collapsed. Grouping key when neither of the above exists |
| `archguard.db.read_ms` | Time spent reading rows after execution (double, milliseconds) |

The span duration is taken as execution time as seen by the client, which
includes network and pool wait.

### Call site (on the database span or on a database span wrapping it)

| Attribute | Use |
| --- | --- |
| `code.function.name` (old: `code.namespace` + `code.function`) | Fully qualified method |
| `code.file.path` (old: `code.filepath`) | Source file; build paths such as `/_/src/...` are allowed |
| `code.line.number` (old: `code.lineno`) | Line of the call |
| `archguard.callsite.source` | How the probe found the site: `stack`, `tag` or `none` |

## Privacy

Producers must not write query parameter values. The importer additionally
replaces literals in query texts and keeps no other attribute values than
those listed here.

## Changes

A change that makes older input read differently needs a new
`archguard.probe.schema` value, and the importer keeps reading the old one.
