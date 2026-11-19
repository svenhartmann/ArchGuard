# OpenTelemetry Demo: mehrsprachiges Service-Beispiel

[English](opentelemetry-demo.md) | [Deutsch](opentelemetry-demo_DE.md)

Die [OpenTelemetry Demo](https://github.com/open-telemetry/opentelemetry-demo) ist
ein Onlineshop, dessen Dienste in Go, Java, C#, TypeScript, Rust, PHP, Ruby, C++,
Kotlin und weiteren Sprachen geschrieben sind und über eine Docker-Compose-Datei
zusammenspielen. ArchGuard liest die Compose-Datei, die Collector-Konfiguration
und die Quellen der Dienste. Es baut und startet keinen Container.

Verwendet wird Commit
[`33bd823107b1b38eaa88dd4ef625cc16d60475d4`](https://github.com/open-telemetry/opentelemetry-demo/tree/33bd823107b1b38eaa88dd4ef625cc16d60475d4)
vom 29.10.2026. Upstream-Lizenz: Apache-2.0; `LICENSE` bleibt im Checkout.
Der Quellcode liegt ausschließlich unter dem ignorierten `examples/checkouts/opentelemetry-demo`.

## Start

Im ArchGuard-Repository ausführen:

```bash
git clone https://github.com/open-telemetry/opentelemetry-demo.git \
  examples/checkouts/opentelemetry-demo
git -C examples/checkouts/opentelemetry-demo checkout 33bd823107b1b38eaa88dd4ef625cc16d60475d4

archguard-desktop examples/checkouts/opentelemetry-demo
```

**Services** wählen. Eine Konfigurationsdatei
ist nicht nötig.

## Gemessener Stand

Auswertung vom 10.11.2026 mit ArchGuard 0.3.1 auf dem obigen Commit:

| Messung | Ergebnis |
| --- | ---: |
| Manifest-Projekte / Anwendungen | 20 / 6 |
| Compose-Container (`compose.yaml`) | 20 |
| Service-Beziehungen | 105 |
| Davon Startreihenfolge (`depends_on`) | 46 |
| Konfigurationsverweise | 34 |
| Konfigurierte HTTP-Endpunkte | 19 |
| gRPC-Client-Aufrufe / HTTP-Client-Aufrufe | 4 / 2 |
| Zyklische Gruppen (Konfigurationsverweise) | 1, mit 5 Diensten und 9 Verbindungen |

## 1. Den Bestellweg im Quellcode verfolgen

**Relation → gRPC client** wählen. Vier Verbindungen gehen von `checkout` aus.
`checkout → product-catalog` fokussieren und Enter drücken: Der Beleg zeigt auf
[`src/checkout/main.go:546`](https://github.com/open-telemetry/opentelemetry-demo/blob/33bd823107b1b38eaa88dd4ef625cc16d60475d4/src/checkout/main.go#L546),
den Aufruf `GetProduct`, und auf die Endpunktbindung in `compose.yaml`. Die Aufrufe
zu `currency` (Zeile 563) und `payment` (Zeile 582) folgen demselben Muster;
**HTTP client** zeigt `checkout → email` in Zeile 604.

## 2. Den Konfigurationszyklus prüfen

**Relation → Configuration reference**, dann **Cycles only** wählen. Eine Gruppe
bleibt: `frontend-proxy`, `frontend`, `image-provider`, `telemetry-docs` und
`otel-collector`. Alle senden Telemetrie an den Collector. Der Collector verweist
zurück, weil seine Konfiguration den Proxy per Health-Check prüft und den
nginx-Status des Image-Providers liest
([`otelcol-config.yml:26-30`](https://github.com/open-telemetry/opentelemetry-demo/blob/33bd823107b1b38eaa88dd4ef625cc16d60475d4/src/otel-collector/otelcol-config.yml#L26-L30)).
`otel-collector → frontend-proxy` zeigt die Quelle:
`/services/otel-collector/environment/FRONTEND_PROXY_ADDR` in `compose.yaml`.

Das ist ein Monitoring-Rückverweis, kein fachlicher Aufrufzyklus. Genau deshalb
hält ArchGuard Konfigurationsverweise, API-Aufrufe und Startreihenfolge getrennt.

```bash
archguard services examples/checkouts/opentelemetry-demo --format json
```

## Grenzen der Aussage

- Die Abdeckung wird als **partiell** gemeldet. Go-Aufrufe nutzen begrenzten
  lokalen Wertfluss; Python- und Gradle-Clients, dynamische Endpunkte und
  Laufzeit-Overrides werden nicht vollständig aufgelöst. Fehlende Kanten beweisen
  keine Isolation.
- Analysiert wird nur `compose.yaml`. `compose.full.yaml`, `compose.observability.yaml`,
  `compose.extras.yaml` und `compose.agent.yaml` werden als nicht zusammengeführte
  Varianten gemeldet.
- Konfigurationsverweise und Endpunktbindungen sind statische Kandidaten, kein
  beobachteter Verkehr. Umgebungswerte und Zugangsdaten werden nicht gelesen.
