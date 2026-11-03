# Shop-Monorepo: API-Zyklen mit fachlichem Hintergrund

[English](README.md) | [Deutsch](README_DE.md)

Dieses absichtlich gekoppelte Demo-System zeigt **14 Dienste in drei getrennten
Compose-Stacks**, darunter zehn lokale APIs, zwei Datenbanken, Redis und RabbitMQ.
ArchGuard erkennt 15 konfigurierte HTTP-Verbindungen, zwei zyklische Gruppen
mit jeweils drei Diensten und sieben separate Startabhängigkeiten.

Die API-Handler führen Produktabfragen und Bestellungen aus. Daten bleiben im
Speicher, Zahlungen sind Mock-Autorisierungen und Benachrichtigungen werden nur
lokal gesammelt. Datenbank-, Cache- und Broker-Adapter sind nicht implementiert;
deren Compose-Einträge zeigen die vorgesehene Deployment-Umgebung. Sie sind
über **Startup**, nicht als angeblich beobachtete API-Aufrufe sichtbar.

## Im ArchGuard-Viewer öffnen

Alle Befehle vom Repository-Root aus:

```bash
cargo run -p archguard-cli --locked -- serve examples/service-shop --port 8789
```

Unter <http://127.0.0.1:8789/> den Tab **Services** öffnen. Laufende Container sind
für die Analyse nicht nötig. Drei Compose-Dateien bilden drei Baumgruppen:

| Stack | Lokale APIs | Infrastruktur |
| --- | --- | --- |
| Storefront | gateway, catalog, pricing, promotions | catalog-db, redis |
| Checkout | orders, payments, risk | orders-db |
| Fulfillment | inventory, shipping, notifications | rabbitmq |

Ein gemeinsames, ausdrücklich benanntes Netzwerk verbindet die Stacks.
Die Daten-Netzwerke bleiben je Stack getrennt. Jeder lokale Dienst hat einen
eigenen Build-Kontext und eine eigene Handler-Datei unter `services/`.

Zum Erkunden:

1. **Configured HTTP → All** zeigt 15 gerichtete API-Beziehungen. **Cycles only**
   reduziert auf die sechs Kanten der beiden zyklischen Gruppen.
2. `orders` auswählen: **Incoming** zeigt `gateway` und `risk`; **Outgoing**
   zeigt `payments`, `inventory`, `shipping` und `notifications`.
3. Eine Kante auswählen: Der Inspektor zeigt Compose-Datei und Environment-Key.
   Beispielsweise führt `risk → orders` zu `ORDERS_API` im Checkout-Stack.
4. **Startup** zeigt sieben Startabhängigkeiten zur Infrastruktur und keine
   Startzyklen. HTTP-Zyklen werden nicht in diese Ansicht übertragen.

Der Paketgraph analysiert die JavaScript-Imports separat. Er kann deshalb keine
dieser HTTP-Beziehungen ersetzen; seine KPIs beschreiben die Quellstruktur.

## Warum die zwei Zyklen entstehen

**Katalog → Preise → Aktionen → Katalog:** Der Katalog braucht berechnete Preise.
Der Preisservice fragt Rabattregeln ab; der Aktionsservice liest dafür die
Produktkategorie wieder aus dem Katalog. Die Rückfrage endet bei `/categories`,
das keine Preise berechnet. Es entsteht ein Architekturzyklus, keine endlose
Rekursion innerhalb einer Anfrage.

Eine mögliche Auflösung: Der Katalog gibt die Kategorie zusammen mit dem
Preisantrag weiter; Preise reicht sie an Aktionen durch. Dann entfällt die
Kante `promotions → catalog`. Der Preis dafür ist ein umfangreicherer Vertrag
und die Entscheidung, wer die Gültigkeit dieser Produktdaten verantwortet.
Ein eigenes Produkt-Lesemodell wäre eine Alternative mit zusätzlicher
Synchronisation und möglichen Verzögerungen.

**Bestellungen → Zahlungen → Risiko → Bestellungen:** Vor einer Mock-Autorisierung
fragt Zahlungen die Risikoprüfung ab. Diese benötigt die bisherige Bestellhistorie.
Die Rückfrage endet beim unabhängigen Lese-Endpunkt `/customer-history`.

Eine mögliche Auflösung: Bestellungen gibt den benötigten Kundenkontext mit der
Autorisierung weiter, oder Risiko pflegt ein eigenes Lesemodell. Das entfernt
`risk → orders`, verlangt aber eine Entscheidung über Datenvertrauen, Aktualität
und zusätzliche Infrastruktur. Diese Optionen sind hier erklärt; die Demo
behält ihre Zyklen bewusst als Analysefall.

`gateway`, `inventory`, `shipping` und `notifications` liefern Kontrast:
mehrere Eingänge, Verzweigungen und Verbindungen zwischen Stacks ohne eigenen
API-Zyklus. Eine hohe Zahl von Beziehungen bedeutet also nicht automatisch Zyklus.

## APIs lokal ausprobieren, ohne Docker

Node 20+ und die gebaute ArchGuard-Binary genügen; keine npm-Pakete erforderlich:

```bash
cargo build -p archguard-cli --locked
node examples/service-shop/dev.js
```

Der Starter liest die Compose-Topologie über ArchGuard und verdrahtet genau deren
HTTP-Verbindungen auf lokale Ports. Das Gateway liegt auf `127.0.0.1:8890`, die
anderen neun APIs auf freien Loopback-Ports. `PORT` ändert den Gateway-Port;
`ARCHGUARD_BIN` kann eine andere gebaute Binary angeben. `Ctrl-C` beendet alle APIs.

```bash
curl http://127.0.0.1:8890/products
curl 'http://127.0.0.1:8890/availability?sku=coffee'
curl -X POST http://127.0.0.1:8890/checkout \
  -H 'Content-Type: application/json' \
  -d '{"customerId":"customer-1","sku":"coffee","quantity":1}'
```

Kaffee kostet nach dem Demo-Rabatt 1.800 Cent. Eine erfolgreiche Bestellung
autorisiert eine Mock-Zahlung, reserviert Bestand, plant eine Demo-Sendung und
sammelt Benachrichtigungen. Neustarten setzt alle Daten zurück. Die Demo hat
keine Authentifizierung, dauerhafte Speicherung, Idempotenz oder Kompensation
verteilter Teilschritte: Ein Fehler nach der Reservierung kann reservierten
Bestand ohne abgeschlossene Bestellung hinterlassen.

## Optional mit Docker Compose

Für das gemeinsame Dockerfile wird `additional_contexts` genutzt; dafür werden
Compose 2.17+ und ein unterstützender Builder benötigt
([Docker-Dokumentation](https://docs.docker.com/reference/compose-file/build/#additional_contexts)).
Die drei Stacks werden getrennt gestartet, damit ihre relativen Pfade und privaten
Netzwerke erhalten bleiben:

```bash
docker network create archguard-service-shop-demo
docker compose -p shop-fulfillment -f examples/service-shop/stacks/fulfillment/compose.yaml up --build -d
docker compose -p shop-checkout -f examples/service-shop/stacks/checkout/compose.yaml up --build -d
docker compose -p shop-storefront -f examples/service-shop/stacks/storefront/compose.yaml up --build -d
```

Nur das Gateway veröffentlicht einen Host-Port, gebunden an Loopback. Der
Compose- und der Node-Starter nutzen denselben Gateway-Port und werden daher
alternativ gestartet. Die Infrastruktur-Container ersetzen die fehlenden
Adapter der Demo nicht.

Beenden:

```bash
docker compose -p shop-storefront -f examples/service-shop/stacks/storefront/compose.yaml down
docker compose -p shop-checkout -f examples/service-shop/stacks/checkout/compose.yaml down
docker compose -p shop-fulfillment -f examples/service-shop/stacks/fulfillment/compose.yaml down
docker network rm archguard-service-shop-demo
```

## Automatisiert geprüft

```bash
cargo test -p archguard-engine --locked --test service_shop_example
node --test examples/service-shop/test/*.test.js
node scripts/viewer/test_service_shop.js
```

Der Rust-Test prüft die echte Compose-Topologie samt Quellenbelegen, getrennten
Zyklen und lokalen Implementierungen. Die Node-Tests prüfen Geschäftsabläufe
und Fehlerfälle. Der Integrationstest startet zehn lokale HTTP-Listener,
prüft Preis, Bestellung und Bestand und prüft dieselbe Topologie im Viewer.
Alle drei Compose-Dateien wurden mit `docker compose config --quiet` validiert;
Container-Build und Container-Laufzeit wurden nicht ausgeführt.
