# Apache Kafka: großer Testfall mit Zyklen

[English](kafka.md) | [Deutsch](kafka_DE.md)

Verwendet wird Apache Kafka **4.1.0**, Commit
[`13f70256db3c994c590e5d262a7cc50b9e973204`](https://github.com/apache/kafka/tree/13f70256db3c994c590e5d262a7cc50b9e973204).
Upstream-Lizenz: Apache-2.0; `LICENSE` und `NOTICE` bleiben im Checkout.
Der Quellcode liegt ausschließlich unter dem ignorierten `examples/checkouts/kafka`.

Die [Konfiguration](kafka.toml) erfasst 3.369 Java-Produktionsdateien in 27
Quellwurzeln. Tests, Testhilfen, Beispiele, Benchmarks und andere Sprachen sind
ausgeschlossen. Die Analyse führt keinen Kafka-Build und keinen Kafka-Code aus.

## Gemessener Stand

Auswertung vom 30.10.2026 mit ArchGuard auf dem obigen Commit:

| Messung | Ergebnis |
| --- | ---: |
| Java-Produktionsdateien | 3.369 |
| Gerichtete Abhängigkeiten | 16.108 |
| Gegen Quellzeilen geprüfte Importbelege | 15.430 |
| Zyklische Gruppen auf Dateiebene | 46 |
| Dateien in der größten zyklischen Gruppe | 172 |
| Einträge in der Landschaft | 5.222 |
| Davon Klassen/Typen | 5.154 |
| Einträge mit erfasster Methodenkomplexität | 4.479 |

Die 46 Gruppen wurden zusätzlich mit einem unabhängigen Algorithmus nachgerechnet.
Eine Gruppe enthält einen oder mehrere mögliche Rundwege; die Gruppenzahl ist
keine Zählung aller einzelnen Zyklen. Paketzyklen hängen von der gewählten Ebene ab.

Ein kleiner, direkter Dateizyklus ist in den unveränderten Produktionsquellen belegt:

- [`CommonClientConfigs.java:24`](https://github.com/apache/kafka/blob/13f70256db3c994c590e5d262a7cc50b9e973204/clients/src/main/java/org/apache/kafka/clients/CommonClientConfigs.java#L24)
  importiert `ClientTelemetryReporter`.
- [`ClientTelemetryReporter.java:19`](https://github.com/apache/kafka/blob/13f70256db3c994c590e5d262a7cc50b9e973204/clients/src/main/java/org/apache/kafka/common/telemetry/internals/ClientTelemetryReporter.java#L19)
  importiert `CommonClientConfigs`.

Die Java-Analyse ist **teilweise**: Build-Abhängigkeiten und generierte Typen fehlen,
und nicht alle Gradle-Ausdrücke werden ausgewertet. Der Lauf projiziert 2.333
Vererbungsbelege, aber keine aufgelösten Aufrufe. Unbekannte Werte bleiben unbekannt;
die Landschaft behauptet weder vollständige Aufrufdaten noch Testabdeckung.
Die strenge Beispielregel meldet bestehende Zyklen; sie bildet keine von Kafka
vorgegebene Architekturregel ab. Der Checkout ist shallow, Git-Historienmetriken
sind dadurch begrenzt.

## Wiederholen

Vom ArchGuard-Repository aus:

```bash
git clone --depth 1 --single-branch --branch 4.1.0 \
  https://github.com/apache/kafka.git examples/checkouts/kafka
mkdir -p examples/checkouts/kafka/.archguard .archguard/reports/kafka-world
cp -n examples/kafka.toml examples/checkouts/kafka/.archguard/archguard.toml

target/release/archguard scan examples/checkouts/kafka --level file --format json \
  --out .archguard/reports/kafka-world/graph.json \
  > .archguard/reports/kafka-world/scan.json
python3 scripts/integration/test_kafka_example.py

target/release/archguard graph examples/checkouts/kafka --format html \
  > .archguard/reports/kafka-world/kafka.html
python3 scripts/integration/test_kafka_mcp.py target/release/archguard
```

Den HTML-Snapshot im Browser öffnen. Der URL-Zusatz
`#world=org%2Fapache%2Fkafka` öffnet direkt die Pakete mit sichtbaren Zyklen;
„Zurück“ führt zur vollständigen Übersicht einschließlich des zweiten Wurzelpakets.
Für die Klassenmessungen wird ein JDK 17+
benötigt. Die zusätzliche JSON-Ausgabeprüfung läuft mit
`python3 scripts/integration/test_java_json_memory.py`: über 100 MB Ausgabe werden mit 64 MiB
Java-Heap gestreamt und vollständig per Hash geprüft. Der produktive Adapter
behält ein 512-MiB-Heaplimit, 45 Sekunden Laufzeit und eine feste Ausgabegrenze
von 256 MiB.

Die Kamera wird mit Umschalt + Ziehen, `Q`/`E` oder den Drehknöpfen gedreht.
Die Richtungsanzeige setzt nur die Ausrichtung zurück. Zoom, Paketnavigation
und Zykluspfeile beziehen sich auf dieselben Quelldaten; die Kamerabewegung
verändert keine Analysewerte.
