# ADR 0004: Runtime probes for .NET and Java

- **Status:** proposed
- **Date:** 2026-11-06

## Context

ArchGuard analyzes source code. It cannot say how often code runs, how long
it takes, or how many database calls a request makes. For performance
reviews we want these numbers, and we want them on the same graph as the
static findings: "this module is in a cycle *and* carries 40 % of the
request time" is a statement no APM tool and no static tool makes alone.

What exists today:

- `archguard-engine/src/service_traces.rs` reads one OTLP/JSON document from
  `.archguard/otel-traces.json` (at most 16 MiB, 100,000 spans). It derives
  service edges from matched CLIENT/SERVER and producer/consumer spans with
  call count, errors, mean and max duration. It ignores database spans,
  per-request counts and code locations. It never starts a collector or an
  application.
- `archguard-core/src/calls.rs` holds a compiler-backed call model for Java
  (javac) and C# (Roslyn): `CallClass` (with `graph_node` and `file`) and
  `CallMethod` (with `file`, `line`, `end_line`). A runtime code location
  (file and line, or qualified method name) can be joined to a method, and
  through its class to a node of the static graph.

What the platforms offer (checked 2026-11-06):

- OpenTelemetry defines a file format: OTLP JSON as JSON Lines, one signal
  type per file, `.jsonl` recommended. The spec is in Development status.
  The Collector's `fileexporter` writes this format.
  <https://opentelemetry.io/docs/specs/otel/protocol/file-exporter/>
- The database span conventions (`db.system.name`, `db.query.text`,
  `db.query.summary`, `db.operation.name`) are stable; instrumentations move
  from the old names (`db.system`, `db.statement`) through an opt-in.
  <https://opentelemetry.io/docs/specs/semconv/database/database-spans/>
- The code conventions (`code.function.name`, `code.file.path`,
  `code.line.number`) are stable since semconv 1.33; `code.namespace`,
  `code.function`, `code.filepath`, `code.lineno` are the old names.
  <https://opentelemetry.io/docs/specs/semconv/non-normative/code-attrs-migration/>
- .NET: `OpenTelemetry.Instrumentation.SqlClient` is marked stable and emits
  `db.query.text` and `db.query.summary`; parameter values are a separate
  opt-in. Combining it with the EF Core instrumentation produced broken
  parent links in 2023 (contrib issue 1764); current behavior is unknown.
  We found no official OTLP file exporter in opentelemetry-dotnet.
- .NET EF Core: `IDbCommandInterceptor` has paired before/after calls;
  `ReaderExecuted` fires after the query was sent, and disposing the data
  reader is a separate interception point, so execution time and row-reading
  time can be measured apart.
  <https://learn.microsoft.com/en-us/ef/core/logging-events-diagnostics/interceptors>
  `TagWithCallSite()` writes the file and line of a LINQ query into the SQL
  as a comment. <https://learn.microsoft.com/en-us/ef/core/querying/tags>
- Java: the OpenTelemetry Java agent instruments JDBC, HTTP and messaging
  without code changes. Extensions register a `SpanProcessor` and
  configuration through `AutoConfigurationCustomizerProvider` and load with
  `-Dotel.javaagent.extensions=<jar>`. A `SpanProcessor` runs synchronously
  on span start and end. `otel.instrumentation.methods.include` creates spans
  for listed methods.
  <https://opentelemetry.io/docs/zero-code/java/agent/extensions/>

Known gaps of plain auto-instrumentation, which motivate own code:

1. Row-reading time is not part of the database span.
2. Spans name the database call, not the line of application code that
   issued it. Without that line, ArchGuard cannot place runtime cost on the
   static graph, and N+1 patterns cannot be told apart from legitimate
   repeated queries.
3. Default sampling drops traces, so counts are samples.
4. Work outside the request context (fire-and-forget tasks, own threads,
   job schedulers) produces orphan spans that no request owns.
5. Client-side duration mixes network, pool wait and server execution.
6. ORM-internal cost (query compilation, materialization, change tracking)
   is invisible.

## Decision

### 1. The contract is OTLP JSON Lines plus a documented attribute set

ArchGuard does not define its own file format. Probes write OTLP JSON Lines
as the OpenTelemetry file exporter spec describes. The same files open in
Jaeger or Tempo, and any language with OpenTelemetry can produce input
without an ArchGuard package.

The contract between applications and ArchGuard is a versioned document,
`probes/CONVENTIONS.md`, listing the attributes the importer evaluates:

| Attribute | Where | Source |
| --- | --- | --- |
| `db.system.name` / `db.system`, `db.query.text` / `db.statement`, `db.query.summary`, `db.operation.name` | DB CLIENT spans | OTel semconv (new and old names accepted) |
| `code.function.name`, `code.file.path`, `code.line.number` (and old names) | DB spans, method spans | OTel semconv |
| `http.route`, `messaging.*`, `rpc.*` | root spans | OTel semconv |
| `archguard.probe.schema` | resource | probe; absent means plain OTel input |
| `archguard.db.read_ms` | DB spans | probe (.NET EF, later Java) |
| `archguard.db.statement_hash` | DB spans | probe: hash of the normalized statement |
| `archguard.callsite.source` | DB spans | probe: `stack`, `tag` or `none` |
| `archguard.sampling` | resource | probe: `always_on` or the configured ratio |

Parameter values are never written. Query text keeps placeholders only.

### 2. One probe architecture, two implementations

Both probes have the same four layers. Each layer reuses the official
OpenTelemetry component where one exists and adds code only for a gap above.

```text
 application
     │  official instrumentation (HTTP, gRPC, messaging, SqlClient/Npgsql, JDBC)
     ▼
 ┌────────────────────────────────────────────────────────────┐
 │ 1 Capture    own hooks only for gaps: EF interceptor        │
 │              (read time, statement hash)                    │
 │ 2 Enrich     SpanProcessor.OnStart on DB spans:             │
 │              call site from stack / EF tag → code.* attrs   │
 │ 3 Configure  AlwaysOn sampler, source list, app prefixes,   │
 │              context propagation for job frameworks         │
 │ 4 Export     OTLP JSON Lines writer, size-rotated files in  │
 │              .archguard/runtime/<run>/traces-<n>.jsonl      │
 └────────────────────────────────────────────────────────────┘
```

**Call-site detection** (layer 2) walks the stack on span start and keeps
the first frame whose type matches the configured application prefixes
(`AppNamespaces` / `app.packages`); framework frames are skipped. The probe
records how it found the site in `archguard.callsite.source`, so the
importer can report how much of the runtime cost it could place.

- Java: JDBC calls are blocking, so the issuing frame is on the stack when
  the span starts. Reactive drivers (R2DBC) are out of scope for version 1.
- .NET: for async EF queries the command may start in a continuation after
  an `await`, where the synchronous stack no longer contains the caller. A
  spike must measure how often this happens. Fallbacks, in order: the
  `TagWithCallSite()` comment if present (`tag`), then the stack (`stack`),
  else only the request (`none`).

Stack walking costs CPU. The probes are analysis tools for test and staging
runs, not for production; capture can be limited to a ratio of DB spans.

**Activation:** the probe does nothing unless `ARCHGUARD_PROBE=1` is set,
so a stray reference in a release build has no effect.

### 3. Packaging

| | .NET | Java |
| --- | --- | --- |
| Form | NuGet package `ArchGuard.Probe` (capture of ASP.NET, HttpClient, SqlClient/Npgsql; enrich; export) and `ArchGuard.Probe.EntityFrameworkCore` (interceptor), so non-EF apps take no EF dependency | One extension JAR for the official OTel Java agent |
| Wiring | `builder.Services.AddArchGuardProbe(o => o.AppNamespaces = ["Shop"])`, one line | No code change: `-javaagent:opentelemetry-javaagent.jar -Dotel.javaagent.extensions=archguard-probe.jar` |
| Built against | OpenTelemetry .NET SDK and instrumentation packages | SDK autoconfigure SPI as `compileOnly`; the agent supplies it |
| Exporter | own OTLP JSON Lines writer (no official one found) | own writer, unless an official OTLP JSON span exporter fits (to check) |

Both probes also allow the standard OTLP exporter to a local Collector with
`fileexporter`, for teams that already run one. The file layout is the same.

The probes live in their own private repositories next to this one,
`../hse-perf-net` and `../hse-perf-java` (changed 2026-11-09, see History).
The contract stays here: `probes/CONVENTIONS.md` and the golden fixtures
under `fixtures/runtime/`. Each probe repository's CI runs its sample app
and checks the output with `archguard runtime --format json`, so producers
and importer cannot drift unnoticed.

**The probes are not published publicly** (no nuget.org, no Maven
Central). The .NET probe is built with `dotnet pack` into a local folder and
consumed through a local package source or a private feed, or as a project
reference; the Java extension is a JAR from a private build.

### 4. Integration into ArchGuard

**Import (archguard-engine).** A new `runtime_traces` module reads
`.archguard/runtime/**/*.jsonl` line by line instead of loading one document;
`.archguard/otel-traces.json` stays supported. Version 1 keeps a compact
record per span and caps the total at 500,000 spans; grouping per trace with
a bound on open traces follows when real runs exceed that. Limits (bytes per
line and file, total spans) produce diagnostics and `complete = false`, as
today. `service_traces.rs` keeps deriving service edges. Observed database
use (`db.system.name`, `server.address`, `db.namespace`) is added to the
datastore model (`Datastore`, `DatastoreUsage`) as a second kind of
evidence, "observed", next to "configured". A datastore seen only at runtime
becomes a new `Datastore` node; one that is configured but never observed
stays as it is, because a sample cannot prove that it is unused.

**Model (archguard-core).** A new `RuntimeProfile`, separate from
`ServiceTopology`:

- run data: inputs, span and trace counts, window, sampling, diagnostics,
  completeness
- `endpoints`: route or root span name; request count; duration mean, p50,
  p95, max; DB calls per request (same statistics); summed DB time
- `statements`: summary, hash or normalized text; calls; execution time
  statistics; summed read time; most calls within one request; call sites
- `call_sites`: code location → `CallMethod.id` → `graph_node`, with the
  match kind `line_range`, `name` or `unmapped`
- `nodes`: per graph node, executions, DB calls and DB time, summed from
  mapped call sites

**Join to the static graph.** Primary key is file and line: `code.file.path`
is matched by path suffix (build paths, deterministic `/_/` paths and
container paths differ from the checkout), then `code.line.number` must fall
inside `CallMethod.line..=end_line`. Java stack frames carry only the file
name, so for Java the qualified class name selects the `CallClass` first.
Fallback is `code.function.name` against method identities. Everything else
stays `unmapped` and is counted, never guessed.

**Findings.** Computed with the profile (`runtime_traces/findings.rs`), with
evidence pointers into the `.jsonl` files (file, line, trace and span id) and
thresholds in a `[runtime]` section of `archguard.toml`. A finding fires when
the value reaches the threshold. The defaults in parentheses are starting
values, not calibrated on measured projects:

- `n_plus_one`: the same statement from the same call site at least
  `n_plus_one_min_calls` (5) times within one request; without recorded call
  sites, per statement
- `chatty_endpoint`: DB calls per request at p95 of at least
  `chatty_endpoint_db_calls` (20)
- `slow_statement`: execution time at p95 of at least `slow_statement_ms`
  (100); read time is reported but not yet a finding
- `runtime_hotspot`: a node with a high runtime share that also has a
  static finding (cycle membership, high complexity, hotspot)

**Surfaces.** MCP tool `runtime_profile`; CLI `archguard runtime`; desktop:
a runtime overlay (heat by DB time or executions) on the Graph and Calls
tabs, database nodes in the Services tab; report section. The `performance`
tool keeps its static meaning; its description points to `runtime_profile`.

### 6. Single requests inside one application (2026-11-09)

The main use is local: run one application with a probe, send a REST
request, and see at once what that request did. Aggregates per endpoint are
not enough for that. Decided with the user on 2026-11-09:

- **Request list and timeline.** The profile keeps individual requests
  (entry spans with their span tree), newest first, bounded in number. A
  request shows total time, its database and HTTP calls with call site and
  duration, the time of the application's own code, and the findings that
  concern it.
- **Own code by sampling profiler.** CPU samples taken while the request
  runs are attributed to it and shown as the methods that used the time.
  Candidates, to be decided by spike 0b:
  1. Pyroscope span profiles (Grafana; `Pyroscope.OpenTelemetry` for .NET,
     the OTel Java agent extension for Java): samples tagged with the span
     id. Needs a Pyroscope server that ArchGuard reads, or a local ingest
     endpoint in the probe; licence and macOS support to check. In .NET
     only with manual OTel instrumentation, which the probe uses anyway.
  2. OpenTelemetry Profiles signal: alpha since March 2026, still alpha in
     September 2026; its eBPF profiler targets Linux. Not yet.
  3. Own sampler in the probe (.NET EventPipe, Java JFR `ExecutionSample`),
     attributing samples by thread to the active span. Full control; async
     work hopping threads is the hard part.
  Samples become a convention in `probes/CONVENTIONS.md` (stack per span,
  sample interval, sampled thread time), never raw profiler files.
- **Live.** The desktop app watches `.archguard/runtime/` and shows a new
  request about one second after the probe wrote it. The view is a tab of
  its own, "Runtime", not part of the Services tab: it is about one
  application, not about connections between services. The aggregate view
  built first in the Services tab moves there.

**Honesty rules**, as in the existing trace import: every output names the
sample window and sampling, the share of DB time that could be mapped to
code, and that a sample proves neither complete traffic nor absence of a
pattern.

### 5. Tests and the contract between languages

- Each probe has its own test suite and a small sample app (ASP.NET + EF
  Core + SQLite; Spring Boot + JDBC + H2) with a known N+1 and a known
  chatty endpoint. A test run writes `.jsonl` output.
- That output is checked in as golden fixtures under `fixtures/runtime/`.
  The Rust importer tests read only these fixtures, so Rust CI needs neither
  `dotnet` nor a JDK. Separate CI jobs build the probes and fail when their
  output no longer matches the fixtures.

## Plan

Each step ends with tests; estimates are for one developer, not measurements.

| Step | Content | Estimate |
| --- | --- | --- |
| 0 | Spikes: .NET async call-site rate with EF; current EF+SqlClient double-span behavior; existence of an official OTLP JSON span exporter for .NET and Java | 2–3 days |
| 1 | `probes/CONVENTIONS.md`; hand-written fixture; Rust import of JSON Lines and DB spans; service → database edges | 3–4 days |
| 2 | `RuntimeProfile` with endpoints and statements; `n_plus_one`, `chatty_endpoint`, `slow_statement`; MCP and CLI | 1 week |
| 3 | .NET probe with EF package and sample app; golden fixtures | 1–2 weeks |
| 4 | Java agent extension with sample app; golden fixtures | 1 week |
| 5 | Join to the call model; `runtime_hotspot`; desktop overlay | 1–2 weeks |
| 0b | Spike: sampling profiler attributed to requests, options 1 and 3 above, on .NET and Java, macOS and Linux | 3–5 days |
| 2b | Requests in the profile (span tree per request, bounded); desktop tab "Runtime" with request list, timeline and live file watching; aggregate view moves there | 1–2 weeks |
| 3b / 4b | Probe repositories `../hse-perf-net`, `../hse-perf-java` with the profiler chosen in 0b | with 3 / 4 |
| 6 | Comparing two runs (before/after a refactoring), building on baseline diff | later |

## Spike results (step 0, 2026-11-09)

Setup: .NET 8, EF Core 8.0.11, Npgsql 8.0.9, OpenTelemetry 1.20.1,
OpenTelemetry.Instrumentation.EntityFrameworkCore 1.20.2-beta.1, PostgreSQL 16
in a local container. Nine query patterns (sync and async, cold and warm
connection pool, N+1 loop, query after explicit open, query after nested
awaits, `TagWithCallSite`), 19 query executions. "Found" means the first frame of
application code was on the stack with method, file and line.

| Observation point | Found |
| --- | --- |
| Stack in `ReaderExecuting(Async)`, `CommandCreating`, or OTel `OnStart` of the EF or Npgsql span | sync 2/2; async 13/17. All 4 misses are async queries that had to open a physical connection (cold pool); they lose the caller at every one of these points |
| Stack in `DbConnectionInterceptor.ConnectionOpening(Async)` | 19/19: EF calls it before the first real await |
| Driver span (`Npgsql`) `OnStart`, reading the site the EF interceptor put into an `AsyncLocal` just before execution (from the stack, else from the site remembered at connection opening for the same `DbContext`) | 19/19 |

Further results:

- EF instrumentation plus Npgsql tracing gives two spans per query, the
  Npgsql span a correct child of the EF span; the broken parent links of
  contrib issue 1764 did not occur with Npgsql. SqlClient was not tested
  (no SQL Server on arm64 here).
- With the EF instrumentation, every synchronous query leaves its finished
  EF span as `Activity.Current`: the EF span ends before its Npgsql child,
  whose end restores the finished parent. Later spans of the same flow
  become children of a finished span. Without the EF instrumentation,
  `Activity.Current` stays clean.
- Both instrumentations write the old attribute names (`db.system`,
  `db.statement`, `db.name`), not `db.system.name` / `db.query.text`.
- EF 8 `TagWithCallSite()` writes `-- File: <path>:<line>` (capital F) as
  the first line of the command text.
- Exporters: no official OTLP file exporter for .NET (one third-party
  package, 0.1.0). Java `io.opentelemetry:opentelemetry-exporter-logging-otlp`
  1.68.0 registers the span exporters `logging-otlp` (OTLP JSON through
  `java.util.logging`) and `experimental-otlp/stdout` (OTLP JSON lines to a
  stream); both live in `internal` packages and are experimental. Whether the
  Java agent bundles them was not checked.

Consequences for the design:

1. The .NET probe does not enable the OTel EF instrumentation. Its EF
   interceptor finds the call site (stack, else the site remembered in
   `ConnectionOpening` per `DbContext`), hands it to the driver span through
   an `AsyncLocal` that is cleared after execution, and the probe's
   processor writes it as `code.*` attributes onto the driver span. One
   span per query; read time from the interceptor goes onto the same span.
2. Plain ADO.NET or Dapper without EF has no `ConnectionOpening` hook; the
   stack alone loses async callers on a cold pool. Version 1 records
   `archguard.callsite.source = none` there; the importer reports the share.
3. The importer treats a database span as wrapping another only when the
   child starts within the parent's time window, so a finished span left
   as `Activity.Current` is not mistaken for a wrapper.
4. The .NET probe writes OTLP JSON Lines itself. The Java extension may use
   `experimental-otlp/stdout` if the agent bundles it, else its own writer;
   decided in step 4.

## Alternatives

- **Own file format.** Simpler to parse, but every language would need an
  ArchGuard package, and the files would not open in existing trace tools.
  Rejected.
- **Plain OpenTelemetry without probes.** Works for service edges and DB
  counts, but leaves gaps 1–3 open; without call sites there is no join to
  the static graph. Kept as a supported input with reduced findings.
- **Commercial APM (Datadog, Dynatrace, New Relic, Sentry).** Strong runtime
  views, some with N+1 detection, but data leaves the machine, export formats
  are vendor-specific, and there is no join to source-level structure.
  Rejected as a dependency; their OTLP export remains usable input.
- **Sampling profilers on their own (dotnet-trace, async-profiler).** Answer
  where CPU time goes, but not per request. Since 2026-11-09 their samples
  are attributed to requests (Section 6) instead of standing alone.
- **Probes in this repository (`probes/dotnet`, `probes/java`).** The
  first choice, replaced on 2026-11-09: the repository has a public remote,
  and the probes must stay private; it also kept .NET and JDK out of the
  Rust build.
- **MiniProfiler (.NET).** Good per-request SQL counts in the browser, but
  .NET only and no standard export. Rejected.

## Consequences

- The repository gains two non-Rust build chains (.NET SDK, JDK with
  Gradle or Maven) with their own CI jobs. The .NET package is a local or
  private artifact only; the Java JAR's distribution is open.
- Runtime numbers are samples from analysis runs. Every surface must say so;
  thresholds must not be read as production SLOs.
- The importer's limits change from "one document" to "streamed lines with
  bounded open traces"; the existing `.archguard/otel-traces.json` path keeps
  working.
- `CONVENTIONS.md` becomes the contract between producers and importer. Breaking changes need a new
  `archguard.probe.schema` value and importer support for the old one.

## History

- **2026-11-09:** The .NET probe is not published publicly; it is built
  and consumed locally or from a private feed (Section 3).
- **2026-11-09:** Step 1 partly implemented on branch `feat/runtime-db-spans`:
  `probes/CONVENTIONS.md`, fixture `fixtures/runtime/shop.jsonl`,
  `archguard_engine::runtime_profile` with endpoints and statements (moved
  forward from step 2). Open from step 1: observed use in the datastore model.
- **2026-11-09:** CLI `archguard runtime` and MCP tool `runtime_profile`
  (step 2 surfaces) implemented; findings (`n_plus_one`, ...) still open.
- **2026-11-09:** Findings `n_plus_one`, `chatty_endpoint`, `slow_statement`
  with `[runtime]` thresholds implemented in the engine instead of
  archguard-analysis, because they read only the profile. `runtime_hotspot`
  waits for the join to the call model (step 5).
- **2026-11-09:** Step 0 spikes done for .NET and the exporter question
  (see "Spike results"); the .NET probe drops the OTel EF instrumentation
  and hands the call site to the driver span. SqlClient and the Java call
  site remain untested. Consequence 3 (time-window rule for wrapping DB
  spans) is implemented in the importer.
- **2026-11-09:** Probes move to private repositories `../hse-perf-net` and
  `../hse-perf-java`; the contract and fixtures stay here (Section 3).
- **2026-11-09:** Decided with the user: single requests inside one
  application with a timeline, own-code time from a sampling profiler
  attributed to requests, live display in a desktop tab "Runtime"
  (Section 6, steps 0b and 2b). The Services-tab view of branch
  `feat/desktop-runtime-view` is the first part of that tab.
