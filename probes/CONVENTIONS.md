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

Probes write `.archguard/runtime/<run>/traces-<nnnn>.jsonl`:

- `<run>` names one recording (for example a start time or a name the user
  chose); it may contain letters, digits, `-`, `_` and `.`.
- `<nnnn>` is a four-digit, zero-padded number starting at `0001`, so path
  order is write order. A file is rotated at a configurable size; the next
  file takes the next number.
- A process that starts again with the same `<run>` writes a new file with
  the next free number. It never appends to a file another process wrote.
- A line is complete when written (one `write` per line, ending in `\n`);
  probes flush within about one second so live views see new requests.

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
| `archguard.db.statement_hash` | Grouping key, preferred: set by ArchGuard probes, see below |
| `db.query.summary` | Grouping key when no hash exists |
| `db.query.text` / `db.statement` | Shown as the statement, normalized (below). Grouping key when neither of the above exists |
| `archguard.db.read_ms` | Time spent reading rows after execution (double, milliseconds) |

The span duration is taken as execution time as seen by the client, which
includes network and pool wait.

**Normalization** (importer and probes alike): remove `-- …` line comments
and `/* … */` block comments; replace each string literal `'…'` (with `''`
as an escaped quote) by `?`; replace each number literal that does not
continue a word (not preceded by a letter, digit, `_`, `@`, `$`, `:`, `.`,
`"` or `` ` ``) by `?`; collapse all whitespace to single spaces and trim.
Case is kept. Placeholders such as `@p0`, `$1`, `:name` and `?` stay.

A number literal starts with a digit and runs over the following ASCII
letters, digits and `.` (so `1.5`, `1e3` and `0x1F` each become one `?`).
Quoted identifiers are not parsed: a number after a space inside `"col 2"`
or `[col 2]` is replaced, and whitespace inside them is collapsed. A
positional placeholder `?1` becomes `??`. These cases follow the importer
(`normalize_sql`) and are part of the contract, so producers and importer
hash the same text.

**`archguard.db.statement_hash`**: the first 16 lowercase hexadecimal
characters of SHA-256 over the UTF-8 bytes of the normalized text. Probes
send the normalized text as `db.query.text` (or `db.statement`) next to it,
so the statement can be shown. They do not enable `db.query.summary`; if an
instrumentation sets it anyway, the hash still wins, so grouping does not
change with instrumentation settings.

Test vector for producers:

| | |
| --- | --- |
| Input | `SELECT *  FROM orders /* by id */ WHERE id = 42 AND status = 'it''s' AND tag = @p0 -- tag` |
| Normalized | `SELECT * FROM orders WHERE id = ? AND status = ? AND tag = @p0` |
| `archguard.db.statement_hash` | `5acbd1b5486068b4` |

### Call site (on the database span or on a database span wrapping it)

| Attribute | Use |
| --- | --- |
| `code.function.name` (old: `code.namespace` + `code.function`) | The declared method, see below |
| `code.file.path` (old: `code.filepath`) | Source file, see below |
| `code.line.number` (old: `code.lineno`) | Line of the call |
| `archguard.callsite.source` | How the probe found the site: `stack`, `tag` or `none` |

**`code.function.name`** is `<namespace or package>.<type>.<method>`, `.`
between all parts; nested types as the runtime names them (`Outer+Inner` in
.NET, `Outer$Inner` in Java). Compiler-generated code is mapped back to the
declared method it belongs to:

- .NET async and iterator state machines: `Shop.Orders.OrderService+<Load>d__3.MoveNext`
  becomes `Shop.Orders.OrderService.Load`; local functions and lambdas
  (`<Load>g__Inner|3_0`, `<Load>b__3_0`) become the enclosing method.
- Java lambdas (`lambda$find$0`) and synthetic accessors become the enclosing
  method; proxies (`$$SpringCGLIB$$…`, `$Proxy12`) are skipped as frames.

**`code.file.path`** identifies the file as precisely as the runtime knows
it. Matching it to the checkout by path suffix is planned (ADR 0004, step 5);
today the importer shows it as recorded.

- .NET: the path from the PDB. Prefer deterministic builds or `PathMap`
  (`/_/src/...`). An absolute path is cut to the part below the project's
  root directory before it is written, so it carries no user name.
- Java: package directory and file name, `com/example/shop/OrderRepository.java`
  (stack frames carry only the file name; the package comes from the class).

### Outgoing calls

A CLIENT span named only by its HTTP method is shown as method and target:
host, port and path from `url.full` (`http.url`), never its query string or
fragment, else `server.address`.

### Span kind

`kind` is the OTLP enum as a number (`1` internal … `5` consumer), as in the
OTLP JSON mapping. The names (`SPAN_KIND_SERVER`, …) are accepted too.

## Privacy

Producers must not write:

- query parameter values, and literals in query texts (send the normalized
  text);
- query strings and fragments of URLs (`url.full`, `http.url`,
  `url.query`): write `url.full` without them;
- the command line and the executable path (`process.command_args`,
  `process.command_line`, `process.executable.path`), host and user names
  (`host.name`, `process.owner`, `db.user`), connection strings
  (`db.connection_string`) and any credentials;
- client identity: `user_agent.original`, `client.address`,
  `client.port`, `network.peer.address`, and HTTP header attributes
  (`http.request.header.*`, `http.response.header.*`);
- exception text: of `exception.*` only `exception.type` is written, never
  `exception.message` or `exception.stacktrace` (a SQL error can quote
  values); the span status description is dropped too.

Paths (`url.path`, `http.route`) are kept: they are needed to tell
requests apart, and routes keep IDs out where the framework knows them.

Files under `.archguard/runtime/` may be committed or shared by mistake, so
this holds even though the importer keeps no other attribute values than
those listed here. It additionally replaces literals in query texts.

## Changes

A change that makes older input read differently needs a new
`archguard.probe.schema` value, and the importer keeps reading the old one.

- 2026-11-09: file numbering, hash algorithm, normalization, method and
  file formats, outgoing call names, span kind and the privacy list fixed
  after the first Java probe. The hash now wins over `db.query.summary`; no
  producer sent both before, so schema 1 stays.
- 2026-11-09: after the first .NET probe: number literals and quoted
  identifiers in the normalization, absolute .NET paths cut below the
  project root, client identity, headers and exception text added to the
  privacy list.
