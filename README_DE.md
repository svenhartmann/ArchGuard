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

## Sprachen und Quellmetriken

| Sprache | Importgraph | Zusätzliche Quellanalyse | Benötigte externe Werkzeuge |
| --- | --- | --- | --- |
| Java | Ja | Typen, Methoden, Aufrufe und unterstützte Metriken | JDK 17+ und passende Projektreferenzen |
| C# | Ja | Typen, Methoden, Aufrufe und unterstützte Metriken | .NET SDK 8+, Ziel-Referenzpakete und vorhandene Abhängigkeitsmetadaten |
| TypeScript / JavaScript | Ja | Typen, Aufrufe und unterstützte Metriken; JavaScript benötigt `allowJs` | Node.js und ein installierter TypeScript-Compiler; `ARCHGUARD_TRUST_PROJECT_COMPILER=1`, um einen über das Repository gefundenen Compiler auszuführen |
| Go | Ja | Pakete, Structs, Interfaces, Receiver-Methoden, Funktionsgruppen und Quellmetriken | Go SDK auf `PATH` oder `ARCHGUARD_GO` als Pfad zur ausführbaren Datei |
| Rust | Ja | Complexity, Cognitive und Halstead pro Funktion aus der Syntax; Aufrufziele über rust-analyzer nach Opt-in; Typen bleiben unaufgelöst | Keine; für Aufrufe rust-analyzer und `ARCHGUARD_TRUST_PROJECT_COMPILER=1` |
| Python | Ja | Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus der Syntax; Aufrufe und Typen bleiben unaufgelöst | Python 3.9+ auf `PATH` oder `ARCHGUARD_PYTHON` mit dem Pfad dazu; Importe brauchen nichts |
| PHP | Ja | Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; Aufrufe und Typen bleiben unaufgelöst | Keine |
| Kotlin (`.kt`) | Ja, gemeinsam mit Java aufgelöst | Importgraph-Analyse; keine Klassenmetriken; Gradle-Skripte `.kts` werden nicht gescannt | Keine |

Abhängigkeiten, die als voll qualifizierter Name ohne Import geschrieben sind, zählen ebenfalls: PHP `\A\B\C`, Rust-Pfade mit `crate::`, `self::` und `super::` sowie Java-/Kotlin-/C#-Namen wie `com.a.B` oder `A.B.C`. Bei Java, Kotlin und C# verknüpft der Scanner einen solchen Namen nur mit einer Typdatei eines deklarierten Pakets oder Namespace; die Compiler-Helfer lösen zusätzlich Namen hinter Variablen auf.

Go nutzt den mitgelieferten `go/parser`- und `go/types`-Helfer, ohne Projektcode auszuführen oder Module herunterzuladen. Embedding ist Komposition: DIT/NOC sind nicht anwendbar. Dynamische Aufrufe, fehlende Referenzen und zusammengefasste Build-Varianten bleiben ausdrückliche Analyselücken.

Python nutzt einen mitgelieferten Helfer auf dem Standardmodul `ast`, isoliert gestartet (`-I -S`), sodass kein Projektcode importiert oder ausgeführt wird. Er parst mit der Grammatik des lokalen Interpreters: Eine Datei mit neuerer Syntax als dieses Python (etwa `match` unter 3.9) erscheint als Syntaxdiagnose und wird nicht gemessen. Eine Klasse gilt als abstrakt, wenn sie `ABC` oder `Protocol` als Basis nennt, `metaclass=ABCMeta` nutzt oder eine `@abstractmethod` deklariert. Rust-Aufrufziele stammen aus `rust-analyzer scip`, sobald `ARCHGUARD_TRUST_PROJECT_COMPILER=1` außerhalb des Repositorys gesetzt ist. ArchGuard ersetzt den Build-Skript-Befehl, daher laufen weder `build.rs` noch Proc-Macros; cargo bleibt offline und ignoriert `RUSTC_WRAPPER`. Aufrufe in Code, den Build-Skripte oder Proc-Macros erzeugen, bleiben deshalb unaufgelöst.

PHP misst der eigene Token-Scanner von ArchGuard ohne PHP-Installation; Interfaces und `abstract`-Klassen gelten als abstrakt. Quality-Gates mit Methodengrenzen bleiben für Python und PHP wie für Rust unvollständig, weil Syntax allein keine vollständige Compileranalyse ist.

Die **KPI-Hilfe** im Viewer enthält Einrichtung, unterstützte Messwerte und Importbeispiele pro Sprache. Testabdeckung und Laufzeitmessungen benötigen echte Berichte; fehlende Daten gelten nicht als Nullwerte.

Martins A/I-Zonen brauchen Abstraktheit aus Compilerbelegen (Java und C#), deshalb melden `archguard scan` und `archguard check` sie standardmäßig als nicht gemessen. Mit `--measure-zones` (MCP: `review_kpis` mit `measure_zones=true`) läuft die langsamere Aufrufanalyse und füllt die Zonen-Zeile; siehe [Review-Kennzahlen und ihre Grenzen](ARCHITECTURE_DE.md#review-kennzahlen-und-ihre-grenzen).

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

Das Verzeichnis [`scripts/`](scripts/README_DE.md) enthält Integrations- und Browsertests, Benchmarks sowie Release-Prüfungen. Das sind Entwicklungswerkzeuge; für die normale CLI- und Viewer-Nutzung müssen sie nicht manuell gestartet werden. Go-Integrationstests nutzen `ARCHGUARD_GO`; sprachspezifische Tests und Browsertests benötigen gegebenenfalls zusätzliche installierte Werkzeuge.

Die CI-Workflows prüfen außerdem optionale Features, Dependency-Advisories, Secrets, native Releases und Malware. Release-Bundles enthalten Prüfberichte und verfügbare Scan-Protokolle; ein unauffälliger Scan garantiert keine Unbedenklichkeit. Siehe [SECURITY.md](SECURITY.md) und [RELEASES.md](RELEASES.md).

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
