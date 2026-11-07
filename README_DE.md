# ArchGuard

[English](README.md)

ArchGuard analysiert Software-Abhängigkeiten aus Quellcode und Projektkonfiguration. Das Werkzeug bietet eine Rust-CLI, einen lokalen Browser-Viewer und einen MCP-Server für Coding-Agenten. Der Einstieg funktioniert ohne Konfigurationsdatei; explizite Regeln machen Befunde bei Bedarf zu CI-Gates.

ArchGuard wird als nichtkommerzielles Open-Source-Projekt entwickelt. Die Software steht unter MIT OR Apache-2.0; beide Lizenzen erlauben auch ihre kommerzielle Nutzung.

## Funktionsumfang

- Abhängigkeitsgraphen für Pakete und Dateien, Zyklusgruppen, Schichtenprüfungen und Kopplungsmetriken.
- Eine World View mit Navigation von Diensten zu Paketen und Quellen, Baumgraphen, Baummatrix und kontextbezogenen Detail-Overlays.
- Compilerbasierte Typen, Methoden, Aufrufe und Quellmetriken, sofern ein Sprachadapter und dessen benötigte Werkzeuge verfügbar sind.
- Dienstverbindungen aus unterstützter Konfiguration und Quellmustern sowie separat importierte OpenTelemetry-Laufzeitbelege.
- Architekturdokumentation nach arc42, deklarierte Sollarchitektur, Git-Vergleiche und rein lesende Refactoring-Simulationen.
- Importe für Testabdeckung, Qualitätsbefunde, Testergebnisse, Duplikate, Mutationstests, API-Verträge und Laufzeitmessungen.

Der Viewer trennt statische Beobachtungen, deklarierte Absichten, importierte Messwerte und simulierte Änderungen. Die Oberfläche ist auf Deutsch und Englisch verfügbar.

## Installation

Das neueste Release-Binary installieren (macOS, Linux, Git Bash), nach `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.sh | sh
```

Windows (PowerShell), nach `%LOCALAPPDATA%\Programs\archguard`, wird dem Benutzer-`PATH` hinzugefügt:

```powershell
irm https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.ps1 | iex
```

Beide prüfen das Archiv gegen `SHA256SUMS`. `ARCHGUARD_VERSION` legt eine Version fest, `ARCHGUARD_INSTALL_DIR` ändert das Ziel. Später aktualisieren mit `archguard update`.

Oder aus dem Repository mit einer Rust-Toolchain bauen:

```sh
cargo install --path crates/archguard-cli --locked
```

Die optionale Graphdatenbank benötigt CMake und eine C++-Toolchain:

```sh
cargo install --path crates/archguard-cli --features db --locked
```

Ohne `db` bleiben Analyse und Viewer verfügbar; Datenbankbefehle und Cypher-Abfragen entfallen. Siehe [Release-Pakete und Prüfung](RELEASES.md).

## Mit einem Projekt starten

```sh
archguard serve /pfad/zum/projekt --port 8787
archguard check /pfad/zum/projekt
archguard graph /pfad/zum/projekt --format html
```

**http://127.0.0.1:8787/** öffnen. Bei einem erkannten Repository mit mehreren Projekten beginnt die Ansicht mit der Projekt- und Diensteübersicht. Ein Projekt führt zu seinem Quellumfang. **Link kopieren** erhält Projekt und Seite beim Neuladen oder teilt sie mit Kollegen, die denselben Checkout lokal starten.

In der World View zoomt das Mausrad; Ziehen verschiebt die Ansicht. Ziehen mit rechter Maustaste oder Umschalt-Taste dreht sie. Touch unterstützt Pinch-Zoom und Drehen mit zwei Fingern. Ein Element auswählen, für den Kontext Details oder Enter verwenden und das Overlay mit Esc schließen. Die Viewer-Hilfe erklärt weitere Bedienelemente.

Die Diensterkennung berücksichtigt `.gitignore` und `project.exclude`. Untergeordnete `examples/`, `fixtures/`, Agent-Worktrees sowie erzeugte Build- und Berichtsverzeichnisse bleiben außen vor; Beispiele lassen sich direkt als Analyseverzeichnis öffnen. Ein Rust-Binary ist ein Anwendungsprojekt und wird erst durch Deployment-Belege als Service eingeordnet.

Deployment-Belege stammen aus Compose-Dateien und einfachen Kubernetes-Manifesten. Bei Kubernetes liest ArchGuard Deployment-, StatefulSet-, DaemonSet-, Job-, CronJob- und Pod-Workloads, ordnet ihnen Services über den Selektor zu und verbindet feste Endpunkt-Variablen wie `ORDERS_URL` mit dem ausgewählten Workload. Werte aus `envFrom` und `configMapKeyRef` zählen, wenn die ConfigMap im selben Manifest-Verzeichnis und Namespace deklariert ist. Secrets werden nie gelesen, und es wird kein Cluster abgefragt; solche Verweise erscheinen als Analyselücken.

Helm-Charts und Kustomize-Overlays werden erst gerendert, wenn du `ARCHGUARD_RENDER_MANIFESTS=1` setzt, weil das Rendern externe Werkzeuge startet. Dieser Schalter ist getrennt von `ARCHGUARD_TRUST_PROJECT_COMPILER`: Jener erlaubt einen Compiler, den das Repository selbst benennt, das Rendern nutzt dagegen dein installiertes `helm` (oder `ARCHGUARD_HELM`) und `kustomize`, ohne `kustomize` auch `kubectl kustomize` (oder `ARCHGUARD_KUSTOMIZE`). Ohne Schalter oder Werkzeug erscheint jedes Chart und jedes Overlay als Analyselücke mit diesem Hinweis (Subcharts und Basen mit dem Chart oder Overlay, das sie nutzt). Die Werkzeuge laufen mit leerer Umgebung, temporären Home- und Cache-Verzeichnissen und ohne kubeconfig, begrenzt auf 120 Sekunden und 64 MiB Ausgabe; darüber wird nichts aus dem Rendern verwendet, und die Topologie meldet `complete: false`. Helm läuft als `helm template <Chart-Name> <Chart> --namespace default --skip-tests` nur mit der `values.yaml` des Charts; weitere Values-Dateien, `--set` und `lookup` (ohne Cluster leer) bleiben unbekannt. ArchGuard führt nie `helm dependency build` oder `--dependency-update` aus: Abhängigkeiten, die in `charts/` fehlen, werden gemeldet, und das Chart wird nicht gerendert. Ein Chart mit einem Symlink, der aus dem analysierten Verzeichnis führt, wird nicht gerendert. Kustomize läuft als `kustomize build` ohne Plugins, Exec-Funktionen und Helm; ein Overlay wird nicht ausgeführt, wenn eine referenzierte Ressource, Basis, Komponente, Patch- oder Generator-Datei eine URL, eine Git-Referenz, nicht vorhanden oder außerhalb des analysierten Verzeichnisses ist oder wenn es `helmCharts` deklariert. Gerenderte Secrets werden ungelesen verworfen. Gerenderte Belege zeigen auf die Helm-Template-Datei oder die `kustomization.yaml` und sagen „Rendered by helm template from …“ bzw. „Rendered by kustomize build from …“; das Chart- oder Overlay-Verzeichnis ist ihr Deployment-Bereich. Dateien, die ein erfolgreiches Rendern verbraucht (das ganze Chart-Verzeichnis; das Overlay-Verzeichnis und die Dateien und Basen, auf die es verweist), werden nur aus der gerenderten Ausgabe gelesen und nicht noch einmal als einfache Manifeste; eine Basis, die zwei Overlays teilen, erscheint daher einmal je Overlay und nie allein. Scheitert ein Rendern, werden seine Dateien wie bisher als einfache Manifeste gelesen.

Die Diensttopologie listet auch Datenspeicher. Compose-Services und Kubernetes-Workloads mit bekannten Datenbank-Images (PostgreSQL, MySQL, MariaDB, MongoDB, Redis, SQL Server, Oracle, Cassandra, Elasticsearch und weitere) werden zu Datenspeicher-Knoten. Verbindungs-URLs (`postgres://`, `mysql://`, `mongodb://`, `redis://`, `jdbc:…`), Host- und Datenbank-Einstellungen wie `DB_HOST` mit `DB_NAME` oder `POSTGRES_DB` sowie Spring `spring.datasource.url` verbinden einen Dienst mit einem solchen Knoten oder einem konfigurierten externen Host. Erhalten bleiben nur Engine, Host, Port und Datenbankname; Zugangsdaten und URL-Optionen erscheinen nie. Zwei Dienste mit demselben Datenspeicher und demselben Datenbanknamen gelten als **gemeinsame Datenbank**; derselbe Server mit unterschiedlichen oder unbekannten Datenbanknamen nur als **gemeinsamer Server**. Tabellen werden nicht analysiert, und ein konfigurierter Datenspeicher belegt keine Nutzung zur Laufzeit. Werte aus Secrets bleiben unbekannt und erscheinen als Analyselücken.

## Sprachen und Quellmetriken

| Sprache | Importgraph | Zusätzliche Quellanalyse | Benötigte externe Werkzeuge |
| --- | --- | --- | --- |
| Java | Ja | Typen, Methoden, Aufrufe und unterstützte Metriken | JDK 17+ und passende Projektreferenzen |
| C# | Ja | Typen, Methoden, Aufrufe und unterstützte Metriken | .NET SDK 8+, Ziel-Referenzpakete und vorhandene Abhängigkeitsmetadaten |
| TypeScript / JavaScript | Ja | Typen, Aufrufe und unterstützte Metriken; JavaScript benötigt `allowJs` | Node.js und ein installierter TypeScript-Compiler; `ARCHGUARD_TRUST_PROJECT_COMPILER=1`, um einen über das Repository gefundenen Compiler auszuführen |
| Go | Ja | Pakete, Structs, Interfaces, Receiver-Methoden, Funktionsgruppen und Quellmetriken | Go SDK auf `PATH` oder `ARCHGUARD_GO` als Pfad zur ausführbaren Datei |
| Rust | Ja | Complexity, Cognitive und Halstead pro Funktion aus der Syntax; Aufrufziele über rust-analyzer nach Opt-in; Typen bleiben unaufgelöst | Keine; für Aufrufe rust-analyzer und `ARCHGUARD_TRUST_PROJECT_COMPILER=1` |
| Python | Ja | Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus der Syntax; Aufrufziele statisch aus Importen, Modulfunktionen, Klassen, `self`/`cls` und `super()` aufgelöst; übrige Aufrufe und Typen bleiben unaufgelöst | Python 3.9+ auf `PATH` oder `ARCHGUARD_PYTHON` mit dem Pfad dazu; Importe brauchen nichts |
| PHP | Ja | Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; Aufrufe und Typen bleiben unaufgelöst | Keine |
| Kotlin (`.kt`) | Ja, gemeinsam mit Java aufgelöst | Klassen, Objekte, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; Aufrufe und Typen bleiben unaufgelöst; Gradle-Skripte `.kts` werden nicht gescannt | Keine |

Abhängigkeiten, die als voll qualifizierter Name ohne Import geschrieben sind, zählen ebenfalls: PHP `\A\B\C`, Rust-Pfade mit `crate::`, `self::` und `super::` sowie Java-/Kotlin-/C#-Namen wie `com.a.B` oder `A.B.C`. Bei Java, Kotlin und C# verknüpft der Scanner einen solchen Namen nur mit einer Typdatei eines deklarierten Pakets oder Namespace; die Compiler-Helfer lösen zusätzlich Namen hinter Variablen auf.

Go nutzt den mitgelieferten `go/parser`- und `go/types`-Helfer, ohne Projektcode auszuführen oder Module herunterzuladen. Embedding ist Komposition: DIT/NOC sind nicht anwendbar. Dynamische Aufrufe, fehlende Referenzen und zusammengefasste Build-Varianten bleiben ausdrückliche Analyselücken.

Python nutzt einen mitgelieferten Helfer auf dem Standardmodul `ast`, isoliert gestartet (`-I -S`), sodass kein Projektcode importiert oder ausgeführt wird. Er parst mit der Grammatik des lokalen Interpreters: Eine Datei mit neuerer Syntax als dieses Python (etwa `match` unter 3.9) erscheint als Syntaxdiagnose und wird nicht gemessen. Eine Klasse gilt als abstrakt, wenn sie `ABC` oder `Protocol` als Basis nennt, `metaclass=ABCMeta` nutzt oder eine `@abstractmethod` deklariert. Ein Aufruf wird nur aufgelöst, wo der Quelltext sein Ziel bindet: über einen Import (auch relative Importe und Re-Exporte über `__init__.py`), eine Modulfunktion, eine Klasse, `self`/`cls` oder `super()` entlang der C3-Reihenfolge der Projektklassen. Aufrufe auf lokalen Variablen, Aufrufergebnissen, neu gebundenen Namen und Stern-Importen erscheinen als `unresolved`, Builtins und andere Bibliotheken als `external`. `archguard calls` meldet die Anzahlen; der Bericht bleibt `partial`, weil Typen nicht aufgelöst werden. Rust-Aufrufziele stammen aus `rust-analyzer scip`, sobald `ARCHGUARD_TRUST_PROJECT_COMPILER=1` außerhalb des Repositorys gesetzt ist. ArchGuard ersetzt den Build-Skript-Befehl, daher laufen weder `build.rs` noch Proc-Macros; cargo bleibt offline und ignoriert `RUSTC_WRAPPER`. Aufrufe in Code, den Build-Skripte oder Proc-Macros erzeugen, bleiben deshalb unaufgelöst.

PHP und Kotlin misst der eigene Token-Scanner von ArchGuard ohne PHP-Installation oder Kotlin-Compiler; Interfaces und `abstract`-Klassen gelten als abstrakt, bei Kotlin auch `sealed`-Klassen. Property-Accessoren, Konstruktoren und `init`-Blöcke in Kotlin werden nicht gemessen. Quality-Gates mit Methodengrenzen bleiben für Python, PHP und Kotlin wie für Rust unvollständig, weil Syntax allein keine vollständige Compileranalyse ist.

**Duplikate.** `archguard duplicates . --report cpd.xml` importiert PMD-CPD-XML. Ohne PMD durchsucht `archguard duplicates . --scan` die gefundenen Quelldateien mit dem eingebauten Token-Scanner von ArchGuard (`.gitignore` und `project.exclude` gelten). Er meldet identische Tokenfolgen ab `--min-tokens` (Standard 100; jedes Satzzeichen ist ein Token, die Zählung ist also feiner als bei CPD). `--normalize` findet zusätzlich Kopien mit anderen Bezeichnern und Literalen. Gruppen sind Token-Übereinstimmungen, keine semantische Gleichheit. Dateien über 1 MiB, ein Budget von 5 Mio. Tokens und ein Zeitlimit von 120 s machen das Ergebnis `partial` mit Diagnose. Ein Scan ersetzt einen importierten CPD-Bericht nur mit `--replace`; das Feld `producer` zeigt, welcher gespeichert ist.

Die **KPI-Hilfe** im Viewer enthält Einrichtung, unterstützte Messwerte und Importbeispiele pro Sprache. Testabdeckung und Laufzeitmessungen benötigen echte Berichte; fehlende Daten gelten nicht als Nullwerte.

## Häufige Arbeitsabläufe

| Aufgabe | Befehl |
| --- | --- |
| Interaktiv erkunden | `archguard serve .` |
| Architekturregeln prüfen | `archguard check .` |
| Zyklen untersuchen | `archguard cycles . --format json` |
| Compilerbelege untersuchen | `archguard calls . --format json` |
| Dienstverbindungen untersuchen | `archguard services . --format json` |
| arc42-Belegblöcke erzeugen oder aktualisieren | `archguard doc . --arc42` |
| Mit einer Git-Revision vergleichen | `archguard review . --base main --format html` |
| Wirksame Konfiguration lesen | `archguard config . --format json` |

`archguard --help` und `archguard <command> --help` liefern die vollständige Befehlsreferenz. Der Viewer enthält zusätzlich CLI-Hilfe und natürlichsprachliche MCP-Beispiele.

Die arc42-Erzeugung aktualisiert ausdrücklich markierte Belegblöcke. Eigene Texte außerhalb dieser Blöcke bleiben unverändert. Erzeugte Architekturbeobachtungen ersetzen keine Teamentscheidungen und belegen keine vollständige Laufzeitarchitektur.

## Konfiguration und Projektdaten

Die Konfiguration wird in dieser Reihenfolge gewählt: ausdrückliches `--config`, `.archguard/archguard.toml`, bisherige `archguard.toml`, danach abgeleitete Werte und Defaults. Relative Quellpfade beziehen sich auf das Projektverzeichnis. Eine fehlerhafte Konfiguration wird gemeldet und nicht stillschweigend ersetzt.

Für ein Projekt ohne vorhandene Konfiguration:

```sh
archguard init /pfad/zum/projekt
archguard config /pfad/zum/projekt --format json
```

Erzeugte Regeln vor der Nutzung als Gates prüfen. Abgeleitete Schichten sind informativ; ausdrücklich deklarierte Regeln können einen Check fehlschlagen lassen. `init` überschreibt keine vorhandene Konfiguration.

Projektlokale Daten liegen unter `.archguard/`: Konfiguration, importierte Belege, Berichte, Caches und aufgezeichneter Verlauf. Erzeugte Berichte und Caches lassen sich neu erstellen; gewollte Konfiguration, Baselines, Szenarien und aufgezeichnete Messungen erhalten. Details und Beispiele stehen in [ARCHITECTURE_DE.md](ARCHITECTURE_DE.md).

### Qualitätsziele

Ein Qualitätsziel kann das Merkmal nach ISO/IEC 25010 nennen, auf das es zielt, den Teil des Systems, den es abdeckt, und ein Qualitätsszenario. Alle drei Felder sind optional:

```toml
[[architecture.goals]]
name = "Changeable checkout"
priority = "first"                       # eigene Formulierung; wird nie gereiht
rationale = "Weekly price changes"
attribute = "maintainability"            # functional_suitability, performance_efficiency, compatibility,
                                         # usability, reliability, security, maintainability, portability
scope = ["checkout/**"]                  # Globs auf Knotennamen; leer = Gesamtsystem
scenario = "A price rule change ships within one day"
```

`architecture_context` (MCP und `archguard review`), `archguard doc` und die arc42-Kapitel 1 und 10 zeigen dann je Ziel die passenden Befund-IDs, Kennzahlen, die Belegquellen und einen Status: `unclassified`, `not_assessable`, `no_evidence`, `findings` oder `no_findings` (keine Befunde aus den verfügbaren Quellen, nicht „Ziel erreicht“). Die Zuordnung von Belegen zu Merkmalen ist fest: Strukturbefunde und Review-Kennzahlen zählen für Wartbarkeit, ein gespeicherter `archguard security`-Scan für Sicherheit, importierte Messungen, Testergebnisse, Abdeckungs- und Mutationsberichte für Leistungseffizienz und Zuverlässigkeit. Für die übrigen vier Merkmale hat ArchGuard keine Belege. Ziele ändern `[review] health_weights` nicht. Entscheidung: [ADR 0003](docs/adr/0003-quality-goal-assessment.md).

## MCP für Coding-Agenten

ArchGuard im Client als stdio-MCP-Server registrieren und einen absoluten Projektpfad verwenden:

```json
{
  "mcpServers": {
    "archguard": {
      "command": "archguard",
      "args": ["mcp", "--root", "/absoluter/pfad/zum/projekt"]
    }
  }
}
```

Beispielaufträge an den Agenten:

- „Zeige die Dienste und ihre Verbindungen und unterscheide Konfiguration von beobachteten Laufzeitbelegen.“
- „Erkläre diesen Paketzyklus anhand seiner Quellstellen und simuliere anschließend eine mögliche Trennung.“
- „Finde die komplexesten Methoden im Checkout-Dienst und zeige die zugehörigen Messwerte.“
- „Prüfe meine Änderungen gegen die Architekturregeln und erkläre verbleibende Analyselücken.“

Der Server bietet belegorientierte Werkzeuge und, soweit unterstützt, Pagination. Zurückgegebene Cursor und Abdeckungsdiagnosen beachten; eine gekürzte Antwort ist nicht das vollständige Ergebnis. Simulationen verändern keine Quelldateien.

## CI und Entwicklung

`archguard check .` dient als Architektur-Gate. Exit-Codes: `0` für Erfolg, `1` für Gate-Verletzungen, `2` für Aufruffehler und `3` für Analyse-, I/O- oder Konfigurationsfehler.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node scripts/viewer/test_viewer_world.js
```

Der Workspace enthält den nativen Desktop-Viewer `archguard-desktop` ([ADR 0002](docs/adr/0002-native-desktop-ui.md)). Unter Linux baut er gegen das System-fontconfig; vorher dessen Header installieren (Debian/Ubuntu: `libfontconfig1-dev`).

Das Verzeichnis [`scripts/`](scripts/README_DE.md) enthält Integrations- und Browsertests, Benchmarks sowie Release-Prüfungen. Das sind Entwicklungswerkzeuge; für die normale CLI- und Viewer-Nutzung müssen sie nicht manuell gestartet werden. Go-Integrationstests nutzen `ARCHGUARD_GO`; sprachspezifische Tests und Browsertests benötigen gegebenenfalls zusätzliche installierte Werkzeuge.

Die CI-Workflows prüfen außerdem optionale Features, Dependency-Advisories, Secrets, native Releases und Malware. Release-Bundles enthalten Prüfberichte und verfügbare Scan-Protokolle; ein unauffälliger Scan garantiert keine Unbedenklichkeit. Siehe [SECURITY.md](SECURITY.md) und [RELEASES.md](RELEASES.md).

### GitHub Action

Das Repository enthält eine Composite Action. Sie installiert ein festes oder das neueste Release (Prüfsummen werden geprüft), führt `archguard check` als SARIF aus und legt in Pull Requests genau einen Review-Kommentar an oder aktualisiert ihn. Solange die Action in keinem Release enthalten ist, `@main` oder einen Commit-SHA verwenden.

```yaml
name: ArchGuard
on: pull_request
permissions:
  contents: read
  pull-requests: write    # Review-Kommentar
  security-events: write  # SARIF-Upload
jobs:
  archguard:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - id: archguard
        uses: svenhartmann/ArchGuard@main
        with:
          version: latest
      - if: success() || failure()
        uses: github/codeql-action/upload-sarif@v4
        with:
          sarif_file: ${{ steps.archguard.outputs.sarif-file }}
          category: archguard
```

| Eingabe | Standard | Bedeutung |
| --- | --- | --- |
| `version` | `latest` | Zu installierendes Release, z. B. `0.2.1`. |
| `path` | `.` | Zu analysierendes Projekt. Für den SARIF-Upload das Repository-Wurzelverzeichnis lassen: Dateipfade sind relativ dazu. |
| `base` | Basis-Commit des PR | Ref für `review --base`. |
| `comment` | `true` | Einen PR-Kommentar anlegen oder aktualisieren. |
| `fail-on-violation` | `true` | Bei Gate-Verletzungen (Exit 1) fehlschlagen. Exit 2 und 3 schlagen immer fehl. |
| `sarif-file` | `$RUNNER_TEMP/archguard.sarif` | Pfad der SARIF-Datei. |
| `token` | `github.token` | Braucht `pull-requests: write` für den Kommentar. |

Ausgaben sind `sarif-file`, `exit-code` und `comment-file`; der Kommentar steht auch in der Job-Zusammenfassung. Fehlt der Basis-Commit des PR im Checkout, holt die Action ihn nach; klappt das nicht, mit `fetch-depth: 0` auschecken. Pull Requests aus Forks erhalten ein Token nur mit Leserechten, dort entfällt der Kommentar. Aktualisiert wird nur ein Kommentar von `github-actions[bot]`; mit einem anderen Token kommt bei jedem Lauf ein neuer hinzu. `check` prüft den gesamten aktuellen Stand; wer nur an neuen Befunden scheitern will, ruft `archguard check . --base <ref>` selbst auf. Unterstützt werden Linux- und macOS-Runner; unter Windows bricht die Action mit einer Fehlermeldung ab.

## Interpretation und Grenzen

- Import- und Quellgraphen sind keine vollständigen Laufzeit-Aufrufgraphen. Reflection, dynamische Aufrufe, Framework-Registrierung, generierter Code und fehlende Abhängigkeiten können Lücken hinterlassen.
- Ein Paketzyklus ist nicht zwingend ein Dateizyklus. Zyklusbeispiele und Trennvorschläge sind begrenzte Beispiele, keine vollständigen Beweise aller möglichen Zyklen.
- Deklarierte Dienst-Endpunkte belegen keine ausgeführten Aufrufe. Trace-Importe beschreiben nur den aufgezeichneten Ablauf und die zugeordneten Dienste.
- Unbekannte, teilweise, veraltete und nicht anwendbare Messwerte bleiben unterscheidbar. Metriken nur mit passenden Sprachdefinitionen, Revisionen und Umfängen vergleichen.
- Importierte Befunde, Testabdeckung und Laufzeitberichte behalten ihre Herkunft. Ein Bericht für eine andere Quellrevision ist kein aktueller Beleg.
- Refactoring-Simulationen ändern nur den dargestellten Graphen. Tatsächliche Codeänderungen mit den projektspezifischen Tests und einer erneuten Analyse prüfen.

## Weitere Dokumentation

- [Architektur- und Konfigurationsreferenz](ARCHITECTURE_DE.md)
- [Reproduzierbare Analysebeispiele](examples/README_DE.md)
- [Service-Shop-Demo](examples/service-shop/README_DE.md)
- [Kafka-Beispiel](examples/kafka_DE.md) und [Jellyfin-Beispiel](examples/jellyfin_DE.md)
- [Methodik des Kontext-Benchmarks](examples/context-token-benchmark_DE.md)
- [Release-Pakete](RELEASES.md) und [Sicherheitsrichtlinie](SECURITY.md)

Lizenziert unter [MIT](LICENSE-MIT) ODER [Apache-2.0](LICENSE-APACHE). Fremdkomponenten und Beispielprojekte behalten ihre eigenen Lizenzen.
