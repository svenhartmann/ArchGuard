# Fallstudien- und Tutorial-Entwürfe — Deutsch

Arbeitsfassungen für die Wochen 4–9 und 11, keine veröffentlichten Messergebnisse. Beispiele frisch klonen, Lizenz und Notices aufbewahren sowie `git rev-parse HEAD`, `archguard --version`, Konfiguration und Werkzeugversionen festhalten. Die lokal installierte Version 0.3.2 kennt SBOM und neuere Exportformate aus dem aktuellen README noch nicht; vor einer Aufnahme die verwendete Version prüfen.

## Kafka

### Die Zykluskarte ist der Beginn eines Reviews

Eine große Abhängigkeitskarte verleitet dazu, Größe mit einer Diagnose zu verwechseln. Nützlicher ist die Frage, welche Quellabhängigkeit einen Zyklus schließt und welche Verantwortung sie ausdrückt. Bei Apache Kafka beginnen wir auf Paketebene, wählen eine Zyklusgruppe und verfolgen eine Kante bis zur Quelle. Danach betrachten wir denselben Bereich in der Matrix.

```sh
archguard cycles examples/checkouts/kafka --format json
archguard metrics examples/checkouts/kafka --format json
archguard hotspots examples/checkouts/kafka --format json
```

Der [vorhandene Rundgang](../../site/examples/kafka.html) nennt Revision und Konfiguration. Für neue Aufnahmen frisch klonen. Ein flacher Klon genügt für das statische Beispiel, aber nicht für belastbare Aussagen über langjährige Änderungsaktivität. Für Hotspots und Co-Change den benötigten Historienzeitraum laden und nennen.

Der Artikel endet mit einer begrenzten Aussage: Welche Abhängigkeit verdient ein Review, welche Quellstellen stützen das und welchen Kontext brauchen Kafka-Maintainer noch? Ein großer Zyklus beweist keine schlechte Architektur.

**Redaktionell offen:** Zyklus- und Architekturmetrik-Ansichten aus dem aktuellen Lauf aufnehmen. Zahlen und Scope aus JSON belegen. „3.369 Dateien“ aus dem Marketingplan ohne passenden neuen Lauf nicht übernehmen. Ein kurzer Clip führt von der Gruppe zu einer Quellkante.

## Shared tables

### Gleicher Server, gleiche Datenbank, gleiche Tabelle?

Zwei Services können denselben Server und trotzdem getrennte Datenbanken nutzen. Selbst eine gemeinsame Datenbank kann bekannte, disjunkte Tabellen enthalten. Jede Beobachtung erlaubt eine andere Review-Frage.

```sh
archguard services examples/checkouts/eshop --format json
```

ArchGuard liest Tabellenbelege aus unterstützten Quell- und Migrationsmustern; nicht jede Laufzeitabfrage wird daraus sichtbar. Eine gemeldete Nutzung führt im Artikel zunächst zur Konfiguration und dann zur Tabellendeklaration. Enthält der gepinnte eShop-Stand keine gemeinsam genutzte Tabelle, steht genau das im Ergebnis. Sock Shop erst nach Prüfung von aktuellem Inhalt und Lizenz verwenden. Keinen Befund für die gewünschte Überschrift erfinden.

**Redaktionell offen:** Datastore-Identität, Datenbankname und Tabellenbeleg sichern; echte Detailansicht aufnehmen. Keine Zugangsdaten veröffentlichen. Fehlende Namen als unbekannt beschreiben. Grundlage: [README](../../README.md#start-with-a-project).

## MCP

### Erst nach der Kante fragen, dann nach der Reparatur

Ein Coding-Agent kann einen plausiblen Umbau vorschlagen, bevor er die zu entfernende Abhängigkeit verstanden hat. Die erste Frage ist kleiner: Welche Quellstelle belegt eine Zykluskante, und wo endet die Analyse?

```sh
archguard mcp --root /absolute/path/to/commons-collections
```

Die stdio-Konfiguration des Clients verwenden, siehe [MCP-Anleitung](../../site/docs/mcp.html). Der Befehl startet einen Server, der auf seinen Client wartet; er erzeugt keinen einmaligen Report.

Aufnahmeablauf:

1. „Finde einen Paketzyklus. Zeige den Quellbeleg jeder Kante und den genauen Projekt-Commit.“
2. „Warum hängt `map` von `bag` ab? Falls diese Kante in diesem Stand fehlt, sage das.“
3. „Simuliere das Entfernen der belegten Kante. Vergleiche die Zyklusgruppen und erläutere die Grenzen. Ändere keinen Quellcode.“
4. Ergebnis prüfen. Erst anschließend Umsetzung und Tests in einem separaten Branch planen.

**Redaktionell offen:** Tatsächliche Prompts, Tool-Namen, Argumente und Antworten aufzeichnen; unaufgelöste Ziele und gekürzte Ergebnisse sichtbar halten. Keine erfundene Agentenantwort als Dialog ausgeben. Eine wirklich vorhandene Kante wählen und Simulation nicht als erfolgreiche Reparatur bezeichnen.

## Documentation

### Evidenz in arc42; Entscheidungen aus dem Team

Kontext und Bausteine lassen sich aus unterstützten Belegen ableiten. Gründe für Grenzen, akzeptierte Risiken und organisatorische Randbedingungen brauchen weiterhin Autor:innen. Dokumentation hilft, wenn diese Aufgabenteilung erkennbar ist.

Im eShop-Checkout, mit einer Version, die diese Formate anbietet:

```sh
archguard doc . --arc42 --stdout
archguard doc . --arc42 --format docx --out arc42-review.docx
archguard graph . --format structurizr
```

Der erste Befehl zeigt Markdown, der zweite schreibt Word und der letzte `.archguard/reports/graph.dsl`. Word enthält Überschriften und Tabellen; Diagramme bleiben Mermaid-/PlantUML-Quelltext. Der Import nach Confluence ist ein eigener Schritt, keine automatische Synchronisierung.

**Redaktionell offen:** Export und tatsächlichen Import aufnehmen, Überschriftennavigation prüfen und eine menschlich verfasste Entscheidung neben generierter Evidenz zeigen. CLI-Unterstützung vorher prüfen. Grundlage: [README](../../README.md) und [ADR 0013](../adr/0013-structurizr-and-asciidoc-export.md).

## Deployment

### Das Deployment lesen, bevor man den Cluster braucht

Schon ein Repository beschreibt Teile der beabsichtigten Service-Verbindungen. Kubernetes-Selektoren, Compose-Endpunkte und unterstützte Terraform-Ressourcen liefern unterschiedliche Ausschnitte. Keiner beweist den aktuellen Betriebszustand.

```sh
archguard services examples/checkouts/online-boutique --format json
ARCHGUARD_RENDER_MANIFESTS=1 archguard services examples/checkouts/online-boutique --format json
```

Der zweite Lauf aktiviert bewusst installierte Helm-/Kustomize-Werkzeuge. Ihre Versionen und die veränderten Diagnosen gehören zum Beispiel. Entfernte Terraform-Module, variablenabhängige Werte und nicht unterstützte Konstrukte können unbekannt bleiben.

**Redaktionell offen:** Online Boutique frisch klonen, Commit und Lizenz nennen, die erwarteten K8s-/Helm-/Kustomize-/Terraform-Dateien prüfen und eine Konfigurationskante mit Quellstelle aufnehmen. Vollständige Erkennung von elf Services oder allen Deployment-Arten nur behaupten, wenn der Lauf sie belegt.

## CI

### Eine Grenze wird zum überprüfbaren Gate

Ein Architekturcheck hilft, wenn das Team weiß, was er ablehnen soll. Eine Grenze in einem temporären Spring-PetClinic-Branch auswählen, den Verstoß lokal reproduzieren und dieselbe Prüfung in CI ausführen.

```sh
archguard check . --format sarif > archguard.sarif
archguard review . --base main --format pr-comment
```

Bestehende Zyklen lassen sich bewusst mit `archguard baseline write .` übernehmen. Baseline einmal prüfen und committen, danach `archguard check . --baseline` ausführen. Ein Neuschreiben bei jedem CI-Lauf würde neue Zyklen akzeptieren.

Die [Action-Definition](../../action.yml) bestimmt die verfügbaren Eingaben. Sie erzeugt SARIF und kann einen bestehenden PR-Kommentar aktualisieren; der Vergleichs-Commit muss geladen sein. Bei Fork-PRs mit nur lesendem Token entfällt der Kommentar. Im endgültigen Tutorial die Action auf einen geprüften Commit pinnen.

**Redaktionell offen:** Echten roten und grünen Lauf samt PR-Kommentar und KPI-Deltas aufnehmen. Vor „in fünf Minuten“ die Dauer messen. Den Fehlerstatus beim Speichern des Reports erhalten. Für eine Aufnahme keinen absichtlich defekten öffentlichen PR ohne entsprechende Absicht des Maintainers anlegen.

## Supply chain

### Ein Offline-Inventar nennt auch seine Lücken

Ein Lockfile kann Paket und Version belegen, ohne eine Lizenz zu nennen. Ein Offline-Werkzeug sollte diese Lücke zeigen. ArchGuard liest unterstützte Manifeste und Lockfiles; fehlende Lizenzfelder ergänzt es nicht aus Paketregistries.

```sh
archguard sbom . --format cyclonedx > sbom.cdx.json
archguard check .
```

Eine konfigurierte Lizenzregel kann bestimmte Lizenzen oder unbekannte Werte ablehnen. Das Ergebnis beschreibt die Projektregel und ist kein rechtliches Urteil über die Anwendung. Die Lizenz des Repositorys ist nicht automatisch die Lizenz einer Abhängigkeit.

**Redaktionell offen:** Einen Mastodon-Stand erst auswählen, nachdem die unterstützten JavaScript-Lockfiles geprüft wurden. **Ruby/Gemfile.lock gehört laut README nicht zu den unterstützten SBOM-Eingaben**; kein vollständiges Gems-/npm-Inventar versprechen. Auch Yarn-Einträge können unbekannte Lizenzen haben. Für eine Lizenzregel-Demo gegebenenfalls ein tatsächlich unterstütztes Lockfile mit expliziten Lizenzangaben wählen. Ergebnis aufnehmen, ausgelassene Ökosysteme nennen und `archguard sbom --help` mit der Aufnahmeversion prüfen.
