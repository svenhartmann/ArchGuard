# OpenTelemetry Demo: a polyglot service example

[English](opentelemetry-demo.md) | [Deutsch](opentelemetry-demo_DE.md)

The [OpenTelemetry Demo](https://github.com/open-telemetry/opentelemetry-demo) is
an online shop whose services are written in Go, Java, C#, TypeScript, Rust, PHP,
Ruby, C++, Kotlin and other languages, wired together by one Docker Compose file.
ArchGuard reads the Compose file, the collector configuration and the service
sources. It neither builds nor starts a container.

Uses commit
[`33bd823107b1b38eaa88dd4ef625cc16d60475d4`](https://github.com/open-telemetry/opentelemetry-demo/tree/33bd823107b1b38eaa88dd4ef625cc16d60475d4)
of 29 October 2026. Upstream license: Apache-2.0; `LICENSE` stays in the checkout.
Source code lives only in the ignored `examples/checkouts/opentelemetry-demo` directory.

## Start

Run from the ArchGuard repository:

```bash
git clone https://github.com/open-telemetry/opentelemetry-demo.git \
  examples/checkouts/opentelemetry-demo
git -C examples/checkouts/opentelemetry-demo checkout 33bd823107b1b38eaa88dd4ef625cc16d60475d4

archguard-desktop examples/checkouts/opentelemetry-demo
```

Choose **Services**. No configuration file is
needed.

## Recorded scope

Analysis on 2026-11-10 with ArchGuard 0.3.1 and the commit above:

| Measurement | Result |
| --- | ---: |
| Manifest projects / applications | 20 / 6 |
| Compose containers (`compose.yaml`) | 20 |
| Service relationships | 105 |
| Of these: startup (`depends_on`) | 46 |
| Configuration references | 34 |
| Configured HTTP endpoints | 19 |
| gRPC client calls / HTTP client calls | 4 / 2 |
| Cyclic groups (configuration references) | 1, with 5 services and 9 connections |

## 1. Follow the order path in source

Choose **Relation → gRPC client**. Four connections leave `checkout`. Focus
`checkout → product-catalog` and press Enter: the evidence points to
[`src/checkout/main.go:546`](https://github.com/open-telemetry/opentelemetry-demo/blob/33bd823107b1b38eaa88dd4ef625cc16d60475d4/src/checkout/main.go#L546),
the `GetProduct` call, and to the endpoint binding in `compose.yaml`. The calls to
`currency` (line 563) and `payment` (line 582) follow the same pattern;
**HTTP client** shows `checkout → email` at line 604.

## 2. Inspect the configuration cycle

Choose **Relation → Configuration reference**, then **Cycles only**. One group
remains: `frontend-proxy`, `frontend`, `image-provider`, `telemetry-docs` and
`otel-collector`. All of them send telemetry to the collector. The collector
points back because its configuration health-checks the proxy and reads the
nginx status of the image provider
([`otelcol-config.yml:26-30`](https://github.com/open-telemetry/opentelemetry-demo/blob/33bd823107b1b38eaa88dd4ef625cc16d60475d4/src/otel-collector/otelcol-config.yml#L26-L30)).
`otel-collector → frontend-proxy` shows the source:
`/services/otel-collector/environment/FRONTEND_PROXY_ADDR` in `compose.yaml`.

This is a monitoring back-reference, not a business call cycle. It shows why
ArchGuard keeps configuration references, API calls and startup order apart.

```bash
archguard services examples/checkouts/opentelemetry-demo --format json
```

## Interpretation limits

- Coverage is reported as **partial**. Go calls use bounded local value flow;
  Python and Gradle source clients, dynamic endpoints and runtime overrides are
  not fully resolved. Missing edges do not prove isolation.
- Only `compose.yaml` is analyzed. `compose.full.yaml`, `compose.observability.yaml`,
  `compose.extras.yaml` and `compose.agent.yaml` are reported as unmerged variants.
- Configuration references and endpoint bindings are static candidates, not
  observed traffic. Environment values and credentials are not read.
