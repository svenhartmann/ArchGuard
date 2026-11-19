# Apache Kafka: a large cycle-analysis example

[English](kafka.md) | [Deutsch](kafka_DE.md)

Uses Apache Kafka **4.1.0**, commit
[`13f70256db3c994c590e5d262a7cc50b9e973204`](https://github.com/apache/kafka/tree/13f70256db3c994c590e5d262a7cc50b9e973204).
Upstream license: Apache-2.0; `LICENSE` and `NOTICE` remain in the checkout.
Source code lives only in the ignored `examples/checkouts/kafka` directory.

The [configuration](kafka.toml) covers 3,369 production Java files across 27 source
roots. Tests, test utilities, examples, benchmarks and other languages are
excluded. Analysis neither builds Kafka nor executes its code.

## Recorded measurements

Analysis on 2026-11-10 with ArchGuard 0.3.1 and the commit above:

| Measurement | Result |
| --- | ---: |
| Production Java files | 3,369 |
| Directed dependencies | 16,321 |
| Import references checked against source lines | 16,164 |
| Cyclic groups at file level | 48 |
| Files in the largest cyclic group | 175 |
| Landscape entries | 5,222 |
| Of these, classes/types | 5,154 |
| Entries with measured method complexity | 4,479 |

The 48 groups were also verified with an independent algorithm. A group contains
one or more possible cycles; the group count does not count every individual
cycle. Package cycles depend on the selected level.

A small direct file cycle is evidenced in the unmodified production sources:

- [`CommonClientConfigs.java:24`](https://github.com/apache/kafka/blob/13f70256db3c994c590e5d262a7cc50b9e973204/clients/src/main/java/org/apache/kafka/clients/CommonClientConfigs.java#L24) imports `ClientTelemetryReporter`.
- [`ClientTelemetryReporter.java:19`](https://github.com/apache/kafka/blob/13f70256db3c994c590e5d262a7cc50b9e973204/clients/src/main/java/org/apache/kafka/common/telemetry/internals/ClientTelemetryReporter.java#L19) imports `CommonClientConfigs`.

Java analysis is **partial**: build dependencies and generated types are missing,
and not all Gradle expressions are evaluated. This run projects 2,333 inheritance
references but no resolved calls. Unknown values remain unknown; the landscape
does not claim complete call data or test coverage. The strict example rule
reports existing cycles; it does not represent Kafka's own architecture policy.
The checkout is shallow, limiting Git history metrics.

## Reproduce

From the ArchGuard repository:

```bash
git clone --depth 1 --single-branch --branch 4.1.0 \
  https://github.com/apache/kafka.git examples/checkouts/kafka
mkdir -p examples/checkouts/kafka/.archguard .archguard/reports/kafka-world
cp -n examples/kafka.toml examples/checkouts/kafka/.archguard/archguard.toml

target/release/archguard scan examples/checkouts/kafka --level file --format json \
  --out .archguard/reports/kafka-world/graph.json \
  > .archguard/reports/kafka-world/scan.json
python3 scripts/integration/test_kafka_example.py

python3 scripts/integration/test_kafka_mcp.py target/release/archguard
```

Open the checkout in the desktop app (`archguard-desktop examples/checkouts/kafka`);
its World view starts at the overview, including the second root package.
Class measurements require JDK 17+. The additional JSON output check,
`python3 scripts/integration/test_java_json_memory.py`, streams over 100 MB of output with a
64-MiB Java heap and verifies the complete output by hash. The production adapter
retains a 512-MiB heap limit, 45-second timeout and fixed 256-MiB output limit.

Rotate the camera with Shift-drag, `Q`/`E` or the rotation controls. The direction
indicator resets orientation only. Zoom, package navigation and cycle arrows use
the same source data; moving the camera does not change analysis values.
