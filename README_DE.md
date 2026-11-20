# ArchGuard

[English](README.md)

ArchGuard analysiert Software-Abhängigkeiten aus Quellcode und Projektkonfiguration. Das Werkzeug bietet eine Rust-CLI, eine native Desktop-App und einen MCP-Server für Coding-Agenten. Der Einstieg funktioniert ohne Konfigurationsdatei; explizite Regeln machen Befunde bei Bedarf zu CI-Gates.

ArchGuard wird als nichtkommerzielles Open-Source-Projekt entwickelt. Die Software steht unter MIT OR Apache-2.0; beide Lizenzen erlauben auch ihre kommerzielle Nutzung.

## Funktionsumfang

- Abhängigkeitsgraphen für Pakete und Dateien, Zyklusgruppen, Schichtenprüfungen und Kopplungsmetriken.
- Eine World View mit Navigation von Diensten zu Paketen und Quellen, Baumgraphen, Baummatrix und kontextbezogenen Detail-Overlays.
- Compilerbasierte Typen, Methoden, Aufrufe und Quellmetriken, sofern ein Sprachadapter und dessen benötigte Werkzeuge verfügbar sind.
- Dienstverbindungen aus unterstützter Konfiguration und Quellmustern sowie separat importierte OpenTelemetry-Laufzeitbelege.
- Architekturdokumentation nach arc42, deklarierte Sollarchitektur, Git-Vergleiche und rein lesende Refactoring-Simulationen.
- Importe für Testabdeckung, Qualitätsbefunde, Testergebnisse, Duplikate, Mutationstests, API-Verträge und Laufzeitmessungen.

Die Desktop-App trennt statische Beobachtungen, deklarierte Absichten, importierte Messwerte und simulierte Änderungen. Die Oberfläche ist auf Deutsch und Englisch verfügbar.

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

Die Skripte installieren auch die Desktop-App aus demselben Release, geprüft gegen `SHA256SUMS-desktop`: `ArchGuard.app` in `~/Applications` unter macOS (`ARCHGUARD_APP_DIR`), `archguard-desktop` (AppImage, x86_64 und aarch64) unter Linux, `archguard-desktop.exe` unter Windows. `ARCHGUARD_DESKTOP=0` überspringt sie.

Oder aus dem Repository mit einer Rust-Toolchain bauen:

```sh
cargo install --path crates/archguard-cli --locked
```

Die optionale Graphdatenbank benötigt CMake und eine C++-Toolchain:

```sh
cargo install --path crates/archguard-cli --features db --locked
```

Ohne `db` bleiben Analyse und Desktop-App verfügbar; Datenbankbefehle und Cypher-Abfragen entfallen. Siehe [Release-Pakete und Prüfung](RELEASES.md).

## Mit einem Projekt starten

```sh
archguard-desktop /pfad/zum/projekt
archguard check /pfad/zum/projekt
archguard diagram /pfad/zum/projekt
```

Die Desktop-App öffnet das Projekt; bei einem erkannten Repository mit mehreren Projekten beginnt sie mit der Projekt- und Diensteübersicht. Ein Projekt führt zu seinem Quellumfang. `archguard diagram` listet die arc42-Diagramme und zeichnet eines im Terminal; die Analysebefehle geben mit `--format json` JSON aus.

In der World-Ansicht der Desktop-App zoomt das Mausrad; Ziehen verschiebt die Ansicht. Ziehen mit rechter Maustaste oder Umschalt-Taste dreht sie; Pinch zoomt, Drehen mit zwei Fingern dreht. Ein Element auswählen; Enter zeigt seine Details, Esc schließt sie. Die Hilfe der Ansicht erklärt weitere Bedienelemente.

Die Diensterkennung berücksichtigt `.gitignore` und `project.exclude`. Untergeordnete `examples/`, `fixtures/`, Agent-Worktrees sowie erzeugte Build- und Berichtsverzeichnisse bleiben außen vor; Beispiele lassen sich direkt als Analyseverzeichnis öffnen. Ein Rust-Binary ist ein Anwendungsprojekt und wird erst durch Deployment-Belege als Service eingeordnet.

Deployment-Belege stammen aus Compose-Dateien, einfachen Kubernetes-Manifesten und Terraform. Bei Kubernetes liest ArchGuard Deployment-, StatefulSet-, DaemonSet-, Job-, CronJob- und Pod-Workloads, ordnet ihnen Services über den Selektor zu und verbindet feste Endpunkt-Variablen wie `ORDERS_URL` mit dem ausgewählten Workload. Werte aus `envFrom` und `configMapKeyRef` zählen, wenn die ConfigMap im selben Manifest-Verzeichnis und Namespace deklariert ist. Secrets werden nie gelesen, und es wird kein Cluster abgefragt; solche Verweise erscheinen als Analyselücken.

Helm-Charts und Kustomize-Overlays werden erst gerendert, wenn du `ARCHGUARD_RENDER_MANIFESTS=1` setzt, weil das Rendern externe Werkzeuge startet. Dieser Schalter ist getrennt von `ARCHGUARD_TRUST_PROJECT_COMPILER`: Jener erlaubt einen Compiler, den das Repository selbst benennt, das Rendern nutzt dagegen dein installiertes `helm` (oder `ARCHGUARD_HELM`) und `kustomize`, ohne `kustomize` auch `kubectl kustomize` (oder `ARCHGUARD_KUSTOMIZE`). Ohne Schalter oder Werkzeug erscheint jedes Chart und jedes Overlay als Analyselücke mit diesem Hinweis (Subcharts und Basen mit dem Chart oder Overlay, das sie nutzt). Die Werkzeuge laufen mit leerer Umgebung, temporären Home- und Cache-Verzeichnissen und ohne kubeconfig, begrenzt auf 120 Sekunden und 64 MiB Ausgabe; darüber wird nichts aus dem Rendern verwendet, und die Topologie meldet `complete: false`. Helm läuft als `helm template <Chart-Name> <Chart> --namespace default --skip-tests` nur mit der `values.yaml` des Charts; weitere Values-Dateien, `--set` und `lookup` (ohne Cluster leer) bleiben unbekannt. ArchGuard führt nie `helm dependency build` oder `--dependency-update` aus: Abhängigkeiten, die in `charts/` fehlen, werden gemeldet, und das Chart wird nicht gerendert. Ein Chart mit einem Symlink, der aus dem analysierten Verzeichnis führt, wird nicht gerendert. Kustomize läuft als `kustomize build` ohne Plugins, Exec-Funktionen und Helm; ein Overlay wird nicht ausgeführt, wenn eine referenzierte Ressource, Basis, Komponente, Patch- oder Generator-Datei eine URL, eine Git-Referenz, nicht vorhanden oder außerhalb des analysierten Verzeichnisses ist oder wenn es `helmCharts` deklariert. Gerenderte Secrets werden ungelesen verworfen. Gerenderte Belege zeigen auf die Helm-Template-Datei oder die `kustomization.yaml` und sagen „Rendered by helm template from …“ bzw. „Rendered by kustomize build from …“; das Chart- oder Overlay-Verzeichnis ist ihr Deployment-Bereich. Dateien, die ein erfolgreiches Rendern verbraucht (das ganze Chart-Verzeichnis; das Overlay-Verzeichnis und die Dateien und Basen, auf die es verweist), werden nur aus der gerenderten Ausgabe gelesen und nicht noch einmal als einfache Manifeste; eine Basis, die zwei Overlays teilen, erscheint daher einmal je Overlay und nie allein. Scheitert ein Rendern, werden seine Dateien wie bisher als einfache Manifeste gelesen.

Terraform wird statisch aus `*.tf`-Dateien gelesen ([ADR 0015](docs/adr/0015-terraform-deployment-evidence.md)): ArchGuard führt nie `terraform` aus, lädt keine Provider oder Module und liest weder State noch `.tfvars` oder `.terraform`. Workloads sind `kubernetes_deployment` und `kubernetes_stateful_set` (über `kubernetes_service` ausgewählt wie oben), `aws_ecs_service` mit den `container_definitions` (aus `jsonencode(...)` oder festem JSON) seiner Task-Definition, `aws_lambda_function`, `google_cloud_run_v2_service`, `google_cloud_run_service`, `azurerm_container_app` und `azurerm_linux_web_app`/`azurerm_windows_web_app` (`app_settings`). Verwaltete Datenspeicher wie `aws_db_instance`, `aws_rds_cluster`, `aws_elasticache_*`, `google_sql_database_instance`, `azurerm_postgresql_flexible_server` und `azurerm_mssql_server` werden zu Datenspeicher-Knoten; gelesen wird nur ihre Engine-Einstellung. Feste Umgebungswerte zählen wie oben; ein Verweis wie `aws_db_instance.orders.address` verbindet den Workload mit diesem Datenspeicher, ein Verweis auf einen anderen Workload wird zu einer konfigurierten Referenz, beides innerhalb desselben Moduls. Lokale Module (`source = "./…"`) werden mit dem Adresspräfix `module.<name>.` verfolgt; entfernte Module, Variablen, Locals, Modul-Outputs und Data Sources bleiben unbekannt und erscheinen als Analyselücken mit Grund. Secret-Ressourcen wie `aws_secretsmanager_*`, sensible Variablen und Variablen-Defaults werden nie gelesen. Belege nennen Ressource und Position, z. B. „Declared in Terraform resource aws_ecs_task_definition.orders (main.tf:34)“. `for_each`, `count`, `dynamic`-Blöcke und `*.tf.json` werden nicht expandiert.

Die Diensttopologie listet auch Datenspeicher. Compose-Services und Kubernetes-Workloads mit bekannten Datenbank-Images sowie Terraform-Datenspeicher-Ressourcen (PostgreSQL, MySQL, MariaDB, MongoDB, Redis, SQL Server, Oracle, Cassandra, Elasticsearch und weitere) werden zu Datenspeicher-Knoten. Verbindungs-URLs (`postgres://`, `mysql://`, `mongodb://`, `redis://`, `jdbc:…`), Host- und Datenbank-Einstellungen wie `DB_HOST` mit `DB_NAME` oder `POSTGRES_DB` sowie Spring `spring.datasource.url` verbinden einen Dienst mit einem solchen Knoten oder einem konfigurierten externen Host. Erhalten bleiben nur Engine, Host, Port und Datenbankname; Zugangsdaten und URL-Optionen erscheinen nie. Zwei Dienste mit demselben Datenspeicher und demselben Datenbanknamen gelten als **gemeinsame Datenbank**; derselbe Server mit unterschiedlichen oder unbekannten Datenbanknamen nur als **gemeinsamer Server**. Tabellen stammen aus dem eigenen Quellcode jedes Dienstes ([ADR 0012](docs/adr/0012-datastore-tables.md)): EF Core (`DbSet<T>`, `[Table]`, `ToTable`, Migrationen mit `CreateTable`), JPA- und TypeORM-`@Entity`/`@Table`, SQLAlchemy `__tablename__` sowie `CREATE TABLE`/`ALTER TABLE` in SQL-Dateien, etwa Flyway- oder Liquibase-Migrationen. Nennen zwei Dienste auf derselben Datenbank dieselbe Tabelle, wird daraus eine **gemeinsame Tabelle** mit den betroffenen Tabellen; sind die Tabellen aller Dienste bekannt und keine kommt doppelt vor, gilt die gemeinsame Datenbank als **mit getrennten Tabellen**. Tabellennamen aus Konstanten, Variablen oder Platzhaltern bleiben unbekannt und erscheinen als Analyselücken; Abfragen werden nicht analysiert, und ein konfigurierter Datenspeicher belegt keine Nutzung zur Laufzeit. Werte aus Secrets bleiben unbekannt und erscheinen als Analyselücken.

GraphQL-Schemas gelten ohne Import als API-Verträge ([ADR 0014](docs/adr/0014-graphql-contracts.md)). Dateien `*.graphql`, `*.graphqls` und `*.gql` sowie `gql`/`graphql`-Tagged-Templates in TS/JS gehören dem Dienst, dessen Build-Kontext oder Projektverzeichnis sie enthält. ArchGuard liest Typen, Felder, Argumente, Enum-Werte und Union-Mitglieder; Direktiven wie das Federation-`@key` bleiben Text und werden nicht ausgewertet. Eine Operation (`query`, `mutation`, `subscription`) in einer solchen Datei oder einem Template verbindet ihren Dienst mit dem einen Dienst, dessen Schema alle ihre Root-Felder deklariert; die Verbindung erscheint auch als HTTP-Verbindung der Topologie. Passt kein Schema oder passen mehrere, oder liegt das Dokument in keinem Dienst, bleibt die Operation mit Begründung unaufgelöst. Ein konfigurierter Endpunkt, dessen Variablenname oder URL `graphql` enthält, verbindet mit dem Zieldienst, wenn dieser ein Schema hat. `archguard contracts . --graphql-baseline` speichert die aktuellen Schemas in `.archguard/graphql-contracts.json`; spätere Läufe melden entfernte Typen und Felder, entfernte Argumente, neue Pflichtargumente und Argumente, die non-null wurden, als breaking, zusammen mit den Diensten, deren verbundene Operationen das geänderte Root-Feld nutzen. Verschachtelte Selektionen werden nicht gegen das Schema geprüft.

## Sprachen und Quellmetriken

| Sprache | Importgraph | Zusätzliche Quellanalyse | Benötigte externe Werkzeuge |
| --- | --- | --- | --- |
| Java | Ja | Typen, Methoden, Aufrufe und unterstützte Metriken | JDK 17+ und passende Projektreferenzen |
| C# | Ja | Typen, Methoden, Aufrufe und unterstützte Metriken | .NET SDK 8+, Ziel-Referenzpakete und vorhandene Abhängigkeitsmetadaten |
| TypeScript / JavaScript | Ja | Typen, Aufrufe und unterstützte Metriken; JavaScript benötigt `allowJs` | Node.js und ein installierter TypeScript-Compiler; `ARCHGUARD_TRUST_PROJECT_COMPILER=1`, um einen über das Repository gefundenen Compiler auszuführen |
| Go | Ja | Pakete, Structs, Interfaces, Receiver-Methoden, Funktionsgruppen und Quellmetriken | Go SDK auf `PATH` oder `ARCHGUARD_GO` als Pfad zur ausführbaren Datei |
| Rust | Ja | Complexity, Cognitive und Halstead pro Funktion, Abstraktheit, implementierte Traits und Supertraits aus der Syntax; Aufrufziele, Trait-Ziele und Typreferenzen über rust-analyzer nach Opt-in | Keine; für Aufrufe und Typreferenzen rust-analyzer und `ARCHGUARD_TRUST_PROJECT_COMPILER=1` |
| Python | Ja | Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus der Syntax; Aufrufziele statisch aus Importen, Modulfunktionen, Klassen, `self`/`cls` und `super()` aufgelöst; übrige Aufrufe und Typen bleiben unaufgelöst | Python 3.9+ auf `PATH` oder `ARCHGUARD_PYTHON` mit dem Pfad dazu; Importe brauchen nichts |
| PHP | Ja | Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; Aufrufziele statisch aufgelöst über Namespaces, `use`-Importe, Projektklassen und Traits, `$this`, `self`, `static` und `parent`; übrige Aufrufe und Typen bleiben unaufgelöst | Keine |
| Kotlin (`.kt`) | Ja, gemeinsam mit Java aufgelöst | Klassen, Objekte, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; Aufrufziele statisch aufgelöst über Package, Importe, Kotlin-Klassen, Objekte, Companion Objects, Top-Level-Funktionen, `this`, `super` und die Klassenhierarchie des Projekts; mit JDK auch in Java-Klassen des Projekts; andere Aufrufe und Typen bleiben unaufgelöst; Gradle-Skripte im Kotlin DSL werden nur für Abhängigkeiten zwischen Teilprojekten gelesen | Keine |
| C / C++ (`.c`, `.h`, `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hh`, `.hxx`) | Ja, `#include` gegen das Verzeichnis der einbindenden Datei und die Include-Verzeichnisse aus `compile_commands.json` und literalen CMake-`include_directories` | Klassen, Structs, Unions, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; abstrakt, wenn eine Klasse eine rein virtuelle Funktion deklariert; Aufrufe und Typen bleiben unaufgelöst | Keine |
| Swift (`.swift`) | Ja, zum Target des Swift Package Manager | Typen, Extensions, Funktionen, Complexity, Cognitive und Halstead pro Funktion aus dem Token-Scanner von ArchGuard; `Typ(…)` und `Typ.statischeMethode(…)` eines eindeutigen Projekttyps aufgelöst, andere Aufrufe und Typen bleiben unaufgelöst; Xcode-Projekte werden nicht gelesen | Keine |
| Ruby (`.rb`) | Ja: `require_relative`, literales `require` gegen `lib/`, Zeitwerk-Konstanten in Rails-Apps | Klassen, Module, Methoden, Complexity, Cognitive und Halstead pro Methode aus dem Token-Scanner von ArchGuard; Aufrufziele statisch aufgelöst für `Const.methode`/`Const.new` auf eindeutigen Projektklassen, `self.methode`, Methodenaufrufe ohne Empfänger und `super` entlang eingebundener Module und Oberklassen; andere Empfänger bleiben unaufgelöst, `send`/`define_method`/`method_missing` als dynamisch | Keine |
| Scala (`.scala`) | Ja, gemeinsam mit Java und Kotlin aufgelöst | Klassen, Case Classes, Traits, Objekte, Enums, Givens und defs in Scala-2- und Scala-3-Syntax, Complexity, Cognitive und Halstead pro def aus dem Token-Scanner von ArchGuard; Aufrufziele statisch aufgelöst über Package, Importe, Scala-Klassen, Objekte und ihr `apply`, Top-Level-defs, `this`, `super` und die Klassenhierarchie des Projekts; Implicits, Givens, Extension-Methoden, Aufrufe auf Werten und Aufrufe in Java- oder Kotlin-Klassen bleiben unaufgelöst; `.sc`-Skripte werden nicht gescannt ([ADR 0020](docs/adr/0020-scala-support.md)) | Keine |

Abhängigkeiten, die als voll qualifizierter Name ohne Import geschrieben sind, zählen ebenfalls: PHP `\A\B\C`, Rust-Pfade mit `crate::`, `self::` und `super::` sowie Java-/Kotlin-/C#-Namen wie `com.a.B` oder `A.B.C`. Bei Java, Kotlin und C# verknüpft der Scanner einen solchen Namen nur mit einer Typdatei eines deklarierten Pakets oder Namespace; die Compiler-Helfer lösen zusätzlich Namen hinter Variablen auf.

Go nutzt den mitgelieferten `go/parser`- und `go/types`-Helfer, ohne Projektcode auszuführen oder Module herunterzuladen. Embedding ist Komposition: DIT/NOC sind nicht anwendbar. Dynamische Aufrufe, fehlende Referenzen und zusammengefasste Build-Varianten bleiben ausdrückliche Analyselücken.

Python nutzt einen mitgelieferten Helfer auf dem Standardmodul `ast`, isoliert gestartet (`-I -S`), sodass kein Projektcode importiert oder ausgeführt wird. Er parst mit der Grammatik des lokalen Interpreters: Eine Datei mit neuerer Syntax als dieses Python (etwa `match` unter 3.9) erscheint als Syntaxdiagnose und wird nicht gemessen. Eine Klasse gilt als abstrakt, wenn sie `ABC` oder `Protocol` als Basis nennt, `metaclass=ABCMeta` nutzt oder eine `@abstractmethod` deklariert. Ein Aufruf wird nur aufgelöst, wo der Quelltext sein Ziel bindet: über einen Import (auch relative Importe und Re-Exporte über `__init__.py`), eine Modulfunktion, eine Klasse, `self`/`cls` oder `super()` entlang der C3-Reihenfolge der Projektklassen. Aufrufe auf lokalen Variablen, Aufrufergebnissen, neu gebundenen Namen und Stern-Importen erscheinen als `unresolved`, Builtins und andere Bibliotheken als `external`. `archguard calls` meldet die Anzahlen; der Bericht bleibt `partial`, weil Typen nicht aufgelöst werden.

Rust-Deklarationen stammen ohne Opt-in aus `syn`. Traits gelten als abstrakt, Structs, Enums und Unions als konkret; damit sind Martins A/I-Zonen für Rust-Module messbar. `impl Trait for Type` und Supertraits (`trait A: B`) werden zu Obertypen, aufgelöst aus der Syntax über `crate::`-, `self::`- und `super::`-Pfade, `use`-Bindungen, Re-Exporte und Globs lokaler Module. Traits unter `std`, `core` und `alloc` sind extern; Prelude-Namen wie `Iterator`, Traits anderer Crates (ein Workspace-Mitglied ist von einer Registry-Crate nicht zu unterscheiden) und Blanket-Impls (`impl<T> Trait for T`) bleiben unaufgelöst. Eine Datei mit einem Item-Makroaufruf (etwa `lazy_static!`) oder einem Syntaxfehler behält unbekannte Typen, daher bleibt die Abstraktheit ihres Moduls ungemessen; Typen, die Derive- oder Attribut-Makros erzeugen, zählen nicht. Rust kennt keine Implementierungsvererbung: DIT/NOC sind nicht anwendbar. Aufrufziele, Trait-Ziele und die Projekttypen, die jeder Typ in Feldern, Signaturen und Impl-Rümpfen referenziert, stammen aus `rust-analyzer scip`, sobald `ARCHGUARD_TRUST_PROJECT_COMPILER=1` außerhalb des Repositorys gesetzt ist; ein Indexlauf bedient Aufrufe und Typen. ArchGuard ersetzt den Build-Skript-Befehl, daher laufen weder `build.rs` noch Proc-Macros; cargo bleibt offline und ignoriert `RUSTC_WRAPPER`. Aufrufe und Typen in Code, den Build-Skripte oder Proc-Macros erzeugen, bleiben deshalb unaufgelöst, und welche Implementierung hinter einer generischen Schranke oder einem Trait-Objekt steht, wird nie aufgelöst. CBO und RFC bleiben partiell.

PHP und Kotlin misst der eigene Token-Scanner von ArchGuard ohne PHP-Installation oder Kotlin-Compiler; Interfaces und `abstract`-Klassen gelten als abstrakt, bei Kotlin auch `sealed`-Klassen. PHP-Aufrufziele werden so aufgelöst, wie PHP Namen beim Kompilieren auflöst: über `namespace`, `use`-Importe (auch Aliase, Group-Use und `use function`) und voll qualifizierte Namen `\A\B`. `new C`, `C::m()`, `$this->m()`, `self::m()` und `parent::m()` werden in den deklarierten Projektklassen gesucht: eigene Methoden, dann Methoden genutzter Projekt-Traits, dann die `extends`-Kette; `static::` gilt als `self::`, Late Static Binding auf eine Unterklasse wird also nicht verfolgt. Ein unqualifizierter Funktionsaufruf versucht zuerst den aktuellen Namespace, dann die globale Projektfunktion. Aufrufe auf anderen Variablen, dynamische Namen (`$f()`, `$o->$m()`), `__call`, `$this` in Closures (die `Closure::bind` neu binden kann), abstrakte Methoden, unbekannte oder externe Basisklassen, `insteadof`/`as`-Regeln von Traits und doppelt deklarierte Namen erscheinen als `unresolved`; ein an `call_user_func` übergebenes Callable wird nicht verfolgt. Klassen und Funktionen, die nirgends im Projekt deklariert sind, gelten als `external`, aber nur, wenn jede PHP-Datei gescannt werden konnte. Aufrufe in Strings, verschachtelten Funktionen und anonymen Klassen werden nicht gezählt. `archguard calls` meldet die Anzahlen; der Bericht bleibt `partial`, weil Typen nicht aufgelöst werden.

Kotlin-Aufrufziele werden in der Namensreihenfolge von Kotlin aufgelöst: explizite Importe (auch `import a.B as C`), dann das Package der Datei, dann Stern-Importe, die einen Namen nur auflösen, wenn genau ein importiertes Package ihn deklariert. `C(…)`, `C.m()`, `C.Nested(…)`, `C.Companion.m()`, `Obj.m()` und voll qualifizierte `a.b.C(…)` werden in den gescannten Kotlin-Klassen, Objekten und Companion Objects gesucht; `this.m()`, ein unqualifiziertes `m()` in einer Klasse und `super.m()`/`super<I>.m()` in den eigenen Funktionen der Klasse, ihrem Companion, verschachtelten und äußeren Klassen und ihren Projekt-Obertypen, auch Default-Methoden von Interfaces; Top-Level-Funktionen über Package und Importe. Aufrufe auf Variablen, Parametern, Properties und Aufrufergebnissen, Safe Calls, Extension-Funktionen, Scope-Funktionen (`let`, `run`, `apply` …), `this@L` mit Label, abstrakte oder überladene Funktionen, das generierte `copy`/`componentN` von Data Classes, doppelt deklarierte Namen, `invoke` auf Objekten und jeder Aufruf in Lambdas, anonymen, lokalen und Extension-Funktionen (wo ein impliziter Empfänger unbekannten Typs gelten kann) erscheinen als `unresolved`. Eine Klasse mit einem Obertyp außerhalb des gescannten Kotlin- und Java-Codes (eine Bibliotheksklasse, eine Java-Klasse ohne javac-Deklarationen, und jede Enum-Klasse) löst nur ihre eigenen Funktionen auf; alles von dort Geerbte und Namen, die dessen unbekannte verschachtelte Klassen verdecken könnten, bleibt `unresolved`. Überladungen werden nicht nach Argumenttypen unterschieden: eine einmal in der Klasse selbst deklarierte Funktion gewinnt. Mit JDK sind Java-Klassen des Projekts aus den Deklarationen des javac-Helfers bekannt: `J(…)` (nicht bei abstrakten Klassen, Interfaces und Enums), `J.statischeMethode()`, `J.Nested(…)`, eine mit `import a.J.m` importierte statische Methode und Funktionen, die eine Kotlin-Klasse von einer Java-Oberklasse erbt, werden zur Java-Klasse oder -Methode aufgelöst, mit der Java-Identität. Annotation Processors wie Lombok können Member erzeugen, die javac ohne Processors nicht meldet, und jede Java-Klasse erbt von `java.lang.Object`; daher bleibt ein Name, der weder in der Java-Klasse noch in ihren Projekt-Obertypen deklariert ist, `unresolved`, ebenso Instanzmethoden, die auf dem Klassennamen aufgerufen werden, abstrakte Java-Methoden und Java-Überladungen: ein Aufruf verweist auf eine Methodensignatur, und Überladungen werden nicht nach Argumenttypen unterschieden. Ohne JDK und für eine Java-Datei, zu der javac keine Klasse meldet, sind Java-Klassen nur über Package und Dateinamen bekannt; Aufrufe dorthin bleiben `unresolved`, nie `external`. Namen, die nirgends im Projekt (Kotlin oder Java) deklariert sind, etwa `println` oder `listOf`, gelten als `external`, aber nur, wenn jede Kotlin- und Java-Datei gelesen werden konnte. Infix- und Operator-Aufrufe zählen nicht als Aufrufstellen, ebenso wenig Aufrufe in String-Templates, Object Expressions, lokalen Klassen, Property-Initialisierern, Accessoren, Konstruktoren und `init`-Blöcken, die auch nicht gemessen werden. Quality-Gates mit Methodengrenzen bleiben für Python, PHP und Kotlin wie für Rust unvollständig, weil Syntax allein keine vollständige Compileranalyse ist.

Gradle-Skripte im Kotlin DSL werden weder gemessen noch ausgeführt. `archguard workspace` liest `<configuration>(project(":a:b"))` in einer `build.gradle.kts` als Abhängigkeit auf das Teilprojekt, dessen Build-Skript in `a/b` unterhalb der nächsten `settings.gradle.kts` liegt, wenn ein literales `include(…)` dort das Projekt nennt. `test…`-Konfigurationen erhalten den Scope `test`, `runtimeOnly` `runtime`, `compileOnly` `provided`, `kapt`, `ksp` und `annotationProcessor` `build`, alle anderen `compile`. Berechnete und relative Pfade (`project(name)`, `project("a")`), Projekte, deren Verzeichnis die Settings verschieben, und eingebundene Projekte ohne Build-Skript erscheinen als unaufgelöst; typsichere `projects.a`-Accessoren und Groovy-`build.gradle`/`settings.gradle` werden nicht gelesen.

C und C++ teilen sich eine Sprache (`cpp` in `project.languages`), daher muss ArchGuard bei einem `.h`-Header nicht raten ([ADR 0017](docs/adr/0017-c-cpp-support.md)). Ein `#include "x.h"` in Anführungszeichen wird zuerst gegen das Verzeichnis der einbindenden Datei aufgelöst, dann gegen die Include-Verzeichnisse (`-I`, `-iquote`, `-isystem` aus `compile_commands.json` im Wurzelverzeichnis oder in `build/` sowie Argumente von `include_directories`/`target_include_directories` ohne andere Variablen als `${CMAKE_CURRENT_SOURCE_DIR}` in den `CMakeLists.txt` oberhalb der Quellen); ein nicht aufgelöstes Include in Anführungszeichen erscheint als nicht aufgelöster Import. Ein `#include <x.h>` in spitzen Klammern wird nur gegen die Include-Verzeichnisse aufgelöst; sonst ist es ein System- oder Bibliotheksheader und bleibt ohne Warnung extern. Die Include-Verzeichnisse gelten gemeinsam für das ganze Repository; ein Header, der in zweien davon liegt, bleibt unaufgelöst. Das Paket ist das Verzeichnis. Es laufen weder Compiler noch Präprozessor: Direktivzeilen werden übersprungen und der Code jedes `#if`/`#else`-Zweigs wird so gemessen, wie er dasteht, beide Zweige zugleich; eine Datei, deren Zweige Klammern unausgeglichen lassen, wird gemeldet und nicht gemessen. Definitionen außerhalb der Klasse `A::f` gehören zu der Klasse `A`, die das Projekt genau einmal deklariert (auch über `using namespace`), sonst zum Modul der Datei. Aufrufe und Typen werden nicht aufgelöst, daher bleibt der Bericht `partial`.

Swift misst derselbe Typ Token-Scanner ohne Swift-Toolchain ([ADR 0018](docs/adr/0018-swift-support.md)). Ein Swift-Modul ist ein Target des Swift Package Manager: jedes `.target`, `.executableTarget`, `.testTarget` und `.macro` mit literalem `name:` in einer gefundenen `Package.swift` (Verzeichnis aus literalem `path:`, sonst `Sources/<name>` oder `Tests/<name>`); das Manifest wird als Text gelesen, nie ausgeführt. `import X` verbindet eine Datei mit dem Verzeichnis des Targets `X`; Plattformmodule wie `Foundation` werden übergangen, andere Importe als extern gemeldet. Xcode-Projekte werden nicht gelesen: Ihre Dateien bilden ein Modul, und eine Diagnose nennt ihre Zahl. Gemessen werden Klassen, Structs, Enums, Actors und Protokolle (Protokolle gelten als abstrakt); `extension T` übergibt ihre Funktionen und Konformitäten dem einzigen Projekttyp `T` dieses Namens, sonst bleibt sie eine Klasse der Art `extension`. `func`, `init`, `deinit` und `subscript` mit Rumpf werden gemessen; berechnete Properties, Accessoren, Property-Initialisierer und Top-Level-Anweisungen nicht. Complexity zählt `if`, `guard`, Schleifen, `catch`, jedes `case` eines `switch`, `&&`, `||`, `??` und den ternären `?`; `if let`/`guard let` zählen wie `if`/`guard`, die Kommas einer Bedingungsliste und `?.` nicht, und das `else` eines `guard` erhöht Cognitive nicht. Aufgelöst werden nur `Typ(…)`, `Typ.init(…)`, `Typ.Verschachtelt(…)` und `Typ.statischeMethode(…)` eines eindeutigen Top-Level-Projekttyps aus dem aufrufenden oder einem importierten Target; Aufrufe auf Werten, `self`, `super`, freie Funktionen, Trailing Closures, generische `Typ<T>(…)` und Überladungen bleiben `unresolved`, und nichts wird `external` genannt. Regex-Literale werden nicht erkannt.

Ruby misst der eigene Token-Scanner von ArchGuard ohne Ruby-Installation ([ADR 0019](docs/adr/0019-ruby-support.md)); Heredocs, `%w`/`%i` und andere Prozent-Literale, Regexe, Symbole und `#{…}`-Interpolation bringen ihn nicht aus dem Tritt. Das Paket einer Datei ist ihr Verzeichnis. `require_relative` wird gegen die anfordernde Datei aufgelöst, ein literales `require 'a/b'` gegen jedes `lib/`-Verzeichnis des Projekts; Gems, die Standardbibliothek und berechnete Pfade bleiben extern. Konstantenreferenzen wie `Billing::Invoice` werden nur in einer Rails-App (mit `config/application.rb`) oder einem Gem, das `Zeitwerk::Loader` referenziert, zu Import-Kanten: Die Zeitwerk-Benennung bildet sie auf `billing/invoice.rb` unter `app/*`, `app/*/concerns` oder dem Verzeichnis des Loaders ab, innerster lexikalischer Gültigkeitsbereich zuerst; sonst bleiben sie unaufgelöst. Ruby kennt keine abstrakten Klassen, daher sind Abstraktheit und die Martin-Zonen nicht anwendbar (nicht „unbekannt“). `include`, `prepend` und `extend` zählen als Obertypen. Aufrufe in Klassenrümpfen und im Code auf oberster Ebene werden nicht gezählt, als lokale Variablen gelten alle irgendwo in der Methode zugewiesenen Namen, und Aufrufe in `#{…}` werden nicht gezählt.

**Duplikate.** `archguard duplicates . --report cpd.xml` importiert PMD-CPD-XML. Ohne PMD durchsucht `archguard duplicates . --scan` die gefundenen Quelldateien mit dem eingebauten Token-Scanner von ArchGuard (`.gitignore` und `project.exclude` gelten). Er meldet identische Tokenfolgen ab `--min-tokens` (Standard 100; jedes Satzzeichen ist ein Token, die Zählung ist also feiner als bei CPD). `--normalize` findet zusätzlich Kopien mit anderen Bezeichnern und Literalen. Gruppen sind Token-Übereinstimmungen, keine semantische Gleichheit. Dateien über 1 MiB, ein Budget von 5 Mio. Tokens und ein Zeitlimit von 120 s machen das Ergebnis `partial` mit Diagnose. Ein Scan ersetzt einen importierten CPD-Bericht nur mit `--replace`; das Feld `producer` zeigt, welcher gespeichert ist.

Testabdeckung und Laufzeitmessungen benötigen echte Berichte; fehlende Daten gelten nicht als Nullwerte.

Martins A/I-Zonen brauchen Abstraktheit aus Compilerbelegen (Java, C#) oder Rust-Deklarationen, deshalb melden `archguard scan` und `archguard check` sie standardmäßig als nicht gemessen. Mit `--measure-zones` (MCP: `review_kpis` mit `measure_zones=true`) läuft die langsamere Aufrufanalyse und füllt die Zonen-Zeile; siehe [Review-Kennzahlen und ihre Grenzen](ARCHITECTURE_DE.md#review-kennzahlen-und-ihre-grenzen).

## Häufige Arbeitsabläufe

| Aufgabe | Befehl |
| --- | --- |
| Interaktiv erkunden | `archguard-desktop .` |
| Architekturregeln prüfen | `archguard check .` |
| Zyklen untersuchen | `archguard cycles . --format json` |
| Compilerbelege untersuchen | `archguard calls . --format json` |
| Dienstverbindungen untersuchen | `archguard services . --format json` |
| UI-Screens, Navigationen und Berechtigungen untersuchen ([ADR 0021](docs/adr/0021-ui-flow-and-permission-simulator.md)); eine Rolle simulieren | `archguard ui-flow .` oder `archguard ui-flow . --role Admin` |
| Entitäten, Zustandsautomaten, Use Cases und Domain-Events untersuchen ([ADR 0022](docs/adr/0022-domain-view.md)) | `archguard domain . [--entity Order] [--chain \| --state Paid] [--role Admin \| --anonymous]` |
| API-Verträge untersuchen (importiertes OpenAPI, GraphQL aus den Quellen) | `archguard contracts . --format json` |
| Aufgezeichnete Laufzeit untersuchen (Requests, DB-Calls pro Request, langsame und wiederholte Queries, Profiler-Samples pro Request; [ADR 0004](docs/adr/0004-runtime-probes.md)) | `archguard runtime . --format json` |
| arc42-Belegblöcke erzeugen oder aktualisieren | `archguard doc . --arc42` |
| arc42 als AsciiDoc oder Word exportieren (für einen Confluence-Import) | `archguard doc . --arc42 --format asciidoc` oder `--format docx` |
| arc42-Diagramme im Terminal auflisten oder zeichnen | `archguard diagram .` oder `archguard diagram . building-blocks` |
| Das arc42-Dokument und seinen Diagrammkatalog als JSON | `archguard doc . --arc42 --format json` |
| Diensttopologie als Structurizr-Workspace (C4) exportieren | `archguard graph . --format structurizr [--service orders]` |
| Mit einer Git-Revision vergleichen | `archguard review . --base main` oder `archguard diff . --to main` |
| Wirksame Konfiguration lesen | `archguard config . --format json` |
| SBOM der Fremdabhängigkeiten (CycloneDX 1.6, offline) | `archguard sbom . --format cyclonedx` |
| Teilbarer Bericht in einer Datei (Markdown mit Mermaid; Desktop: Datei → Exportieren → Bericht …) | `archguard report . --out report.md` |

`archguard --help` und `archguard <command> --help` liefern die vollständige Befehlsreferenz.

Die arc42-Erzeugung aktualisiert ausdrücklich markierte Belegblöcke. Eigene Texte außerhalb dieser Blöcke bleiben unverändert. Erzeugte Architekturbeobachtungen ersetzen keine Teamentscheidungen und belegen keine vollständige Laufzeitarchitektur.

Ohne `--out` nimmt `doc --arc42` den arc42-Text, den die Desktop-App bearbeitet: die am wenigsten tief liegende `arc42.md` oder den Ordner `arc42/`, bis vier Ordnerebenen tief (nicht in versteckten oder Build-Ordnern wie `target`, `node_modules`). Ein Ordner enthält je Kapitel eine Datei, benannt nach seiner Nummer (`05-bausteinsicht.md`, `arc42-05.md`); jede wird für sich geprüft und aktualisiert. Nur wenn das Projekt beides nicht hat, legt `doc --arc42` `arc42.md` im Projektordner an. Diagrammblöcke, die „In Kapitel einfügen“ der Desktop-App in ein Kapitel mit eigenem Text gesetzt hat (`<!-- archguard:diagram:ID:start/end -->`), werden wie die Belegblöcke aufgefrischt, der Block eines ausgeblendeten fällt weg. Ein Kapitel mit eigenem Text bekommt nur die Diagramme, die Sie dafür gewählt haben: Ein Diagramm, das in `[arc42] shown_diagrams` steht (der Schalter „In der Dokumentation“ der Desktop-App schreibt es) und in so einem Kapitel fehlt, kommt an dessen Ende dazu; die übrigen bietet nur der Reiter „Diagramme“ der Desktop-App an. Kapitel mit erzeugten Blöcken enthalten jedes Diagramm, das nicht in `[arc42] hidden_diagrams` steht. `--check` nennt Kapitel, die diese Regeln ändern würden, als veraltet.

`--format asciidoc` schreibt `arc42.adoc`, `--format docx` schreibt `arc42.docx` (oder `--out`). Beide wandeln den arc42-Text um, den `doc --arc42` findet (Datei oder Kapitelordner), mit aktuellen Beleg- und Diagrammblöcken, und lesen ihn nur; ohne ihn wandeln sie das erzeugte Dokument um. `--chapter N` exportiert dieses Kapitel. Diagramme stehen in AsciiDoc als `[mermaid]`/`[plantuml]`-Blöcke für Asciidoctor Diagram. Word-Überschriften nutzen die Formatvorlagen Überschrift 1–3, daher baut Confluences „Word-Dokument importieren" daraus das Inhaltsverzeichnis; Tabellen sind Word-Tabellen, Diagramme bleiben Mermaid- oder PlantUML-Quelltext, weil das Dokument keine gerenderten Bilder hat. `graph --format structurizr` schreibt `.archguard/reports/graph.dsl`: ein Softwaresystem, die Deployment-Dienste als Container, Datenspeicher außerhalb der Konfiguration als externe Systeme, Beziehungen mit Protokoll und Konfigurationsbeleg und mit `--service` die Pakete dieses Dienstes als Komponenten. Abbildung und Grenzen: [ADR 0013](docs/adr/0013-structurizr-and-asciidoc-export.md). Die Desktop-App bietet dieselben arc42-Dateien unter Datei → Exportieren an, das MCP-Werkzeug `architecture_document` liefert AsciiDoc (`format=asciidoc`) und den Workspace (`template=structurizr`).

### Diagramme im Terminal

`archguard diagram [PFAD]` listet den arc42-Diagrammkatalog: jedes Diagramm, das ArchGuard ableitet, mit Kapitel, Art, ob das Dokument es zeigt (`IN DOC`), ob der arc42-Text des Projekts es enthält (`IN TEXT`, nach der Regel der Desktop-App), Titel (bei 60 Zeichen gekürzt) und Id, zuletzt und ungekürzt, damit sie sich kopieren lässt. `archguard diagram [PFAD] ID` zeichnet ein Diagramm; ein einzelnes Argument, das kein Ordner ist, gilt als Id. `--level PAKET` zeichnet eine Ebene der Paketkarte (`building-blocks@PAKET`, oder `cycles --level PAKET`), dieselbe Ebene, die die Karte der Desktop-App öffnet, auch wenn `[arc42] shown_diagrams` sie nicht nennt. Kastengraphen (Kontext, Bausteine, Paketkarten, Deployment, Dienste, Zyklen, Schichten) stehen von oben nach unten, von der nutzenden zur genutzten Seite; ein Zyklus wird an einer Kante gebrochen, die nach oben zeigt, und `!` markiert Kästen und Kanten im Zyklus. Sequenzen zeigen Teilnehmer mit Lebenslinien, nummerierte Nachrichten, gestrichelte Rückgaben, Aufrufe an sich selbst als Schleife, Ereignisse mit Doppelspitze (`>>`) und Fehler mit `✗`; die Szenarien haben keine alt/loop-Rahmen, daher werden keine gezeichnet. Die Ausgabe nutzt Unicode-Rahmenzeichen, `--ascii` reines ASCII; Farbe (Zyklen rot, HTTP blau) nur im Terminal ohne `NO_COLOR`; die Breite folgt `COLUMNS` (Standard 100), breite Reihen brechen um. `--format json` gibt stattdessen die Daten des Diagramms (Elemente und Beziehungen) oder den ganzen Katalog aus. Beispiel aus `fixtures/py-layers`:

```text
$ archguard diagram fixtures/py-layers
CHAPTER  KIND      IN DOC  IN TEXT  TITLE                                            ID
5        packages  yes     yes      Package overview                                 building-blocks
6        packages  yes     yes      Static calls between building blocks             runtime-calls
6        sequence  yes     yes      Scenario · handle · api (static)                 runtime-scenario-python-api-http-py-module-handle-4
...
11       packages  yes     yes      Cyclic package relations                         cycles

$ archguard diagram fixtures/py-layers building-blocks
Package overview [building-blocks, chapter 5, packages]

┌─────────┐
│ api     │
│ package │
└────┬────┘
     │
     ▼ 2
┌─────────┐
│ ! app   │
│ package │
└──┬───┬──┘
   │   ▲ !1
!3 ▼   │
┌──────┴───┐
│ ! domain │
│ package  │
└──────────┘

counts: source connections

$ archguard diagram fixtures/py-layers runtime-scenario-python-api-http-py-module-handle-4
Scenario · handle · api (static) [runtime-scenario-python-api-http-py-module-handle-4, chapter 6, sequence]

      ┌─────┐           ┌─────┐         ┌────────┐
      │ api │           │ app │         │ domain │
      └──┬──┘           └──┬──┘         └────┬───┘
         │ 1. estimate     │                 │
         ├────────────────>│                 │
         │                 │ 2. price_cents  │
         │                 ├────────────────>│
         │                 │<╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
         │<╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤                 │
         │                 │                 │
```

`archguard doc --arc42 --format json [--chapter N]` gibt das arc42-Dokument als Daten aus und schreibt nichts: wo der Text liegt (`target`: Datei oder Ordner, vorhanden), jedes Kapitel mit Nummer, Titel, der Datei, die es enthält (`null`: nur erzeugt), und seinem Markdown, wie die Desktop-App es liest, und den Katalog wie oben, jedes Diagramm mit `shown`, `chosen` (in `[arc42] shown_diagrams` genannt), `in_text`, Elementen und Beziehungen. Das MCP-Werkzeug `architecture_document` liefert denselben Katalog mit `template=arc42, format=catalogue`.

### UI-Fluss und Domänensicht

`archguard ui-flow .` listet die Screens jeder Frontend-Anwendung (Route, Name, Zugriffsbedingung, Datei:Zeile), die Befunde zu Abweichungen zwischen Frontend und Backend und die Lücken, die die Extraktoren nicht auflösen konnten (die ersten 10; `-v` zeigt alle). `--role ROLLE` (wiederholbar) oder `--anonymous` simuliert diesen Nutzer: Jeder Screen wird von den Start-Screens seiner Anwendung aus (Route `/` oder vom Extraktor markiert: die erste Shell-Seite in MAUI, `StartupUri` oder das beim Start erzeugte Fenster in WPF, WinUI und Avalonia) als sicher oder möglicherweise erreicht markiert oder nennt den Grund, warum nicht: verweigert (mit der nötigen Berechtigung), kein bekannter Pfad (mit der Zahl der Links der Anwendung, die die Regeln nicht auflösen konnten) oder kein Start-Screen, jeder Backend-Endpunkt als erlaubt, verweigert oder unbekannt. `archguard domain . [--entity NAME]` zeigt Entitäten mit ihren Beziehungen, jeden Zustandsautomaten als Übergangstabelle (der Guard ist die Bedingung, unter der der Schritt erlaubt ist; ein Guard mit throw oder return erscheint als `!(Quellbedingung)`), Use Cases und Domain-Events (ausgelöst von → behandelt von); `--entity` beschränkt die Ausgabe auf eine Entität. `--chain` zeigt für jeden Backend-Endpunkt und jeden Message-Handler (`IIntegrationEventHandler<T>`, MassTransit `IConsumer<T>`, NServiceBus `IHandleMessages<T>`, `@KafkaListener`, `@RabbitListener`, `@JmsListener`, `@StreamListener`) die Kette in die Domäne: die Screen-Aktion, die ihn aufruft, seinen Handler und die Request-Typen, die er sendet (`mediator.Send`, Axon `commandGateway.send`, ein aufgerufenes `*UseCase`-Feld), die Use Cases, die sie behandeln, deren Zustandsübergänge, Events und Event-Handler, z. B. `POST /api/orders → CreateOrderCommandHandler`, `Order: any → Submitted`, `→ OrderStartedDomainEvent → …Handler`. `--entity Order --state Paid` beantwortet die Gegenfrage: Welche Endpunkte, Screens und Message-Handler können diesen Zustand auslösen, z. B. `message OrderPaymentSucceededIntegrationEvent → SetPaidOrderStatusCommandHandler`. Mit `--role ROLLE` oder `--anonymous` bekommt jede Kette ein Recht: erlaubt, verweigert oder unbekannt mit den Gründen (Endpunkt-Zugriff, Erreichbarkeit des Screens; ein Endpunkt ohne Deklaration ist unbekannt), oder kein Nutzerpfad bei einem Message-Handler. Jobs sind keine Kettenstarts. Beide kennen `--format json` und enden mit Exit-Code 0. Nur statische Analyse: Was erkannt wird, steht in [ADR 0021](docs/adr/0021-ui-flow-and-permission-simulator.md) und [ADR 0022](docs/adr/0022-domain-view.md).

`archguard ui-flow open SCREEN . [--param id=4711] [--print]` gibt die lokale URL eines Screens aus (Id, Name oder Route; mehrere Treffer werden aufgelistet, Exit-Code 2) und öffnet sie mit `open`/`xdg-open`/`start`. `archguard ui-flow replay SCREEN . [--role ROLLE | --anonymous] [--out DATEI] [--run]` schreibt einen Playwright-Test, der sich als Testnutzer der Rolle anmeldet, den Start-Screen öffnet und dem kürzesten aufgelösten Klickpfad folgt; nach jedem Klick wartet er auf die URL des nächsten Screens. Ein Schritt ohne bekannten Auslöser navigiert direkt und ist mit `// no trigger read: direct navigation` markiert (schwächerer Beleg). Das Skript landet im temporären Verzeichnis des Systems, außer `--out` nennt eine Datei. Nur `--run` führt es aus (`npx playwright test`, `@playwright/test` muss vom Ordner des Skripts aus auflösbar sein) und gibt jeden Schritt aus; ein fehlgeschlagener Schritt ist der Befund „view and application disagree at step n“. Die Einstellungen stehen in der Konfiguration; ein wörtliches `password` ist ein Konfigurationsfehler:

```toml
[views.run.WebApp]                      # Anwendungsname aus dem Flow
base_url = "https://localhost:7298"
login_path = "/user/login"              # optional; zeigt das Anmeldeformular oder leitet dorthin
[views.run.WebApp.users.Admin]          # Testnutzer je Rolle
login = "admin@example.org"
password_env = "ARCHGUARD_PW_ADMIN"     # oder password_keychain = "archguard-webapp-admin" (macOS)
```

Konventionen halten beide Sichten vollständig, wenn ein Agent den Code schreibt und `archguard check` nach jedem seiner Schritte läuft. Sie sind optional; ohne `[views]` wird nichts geprüft:

```toml
[views]
conventions = [
  "navigation_literal",            # jedes Navigationsziel ist eine literale Route oder Routen-Konstante
  "endpoint_access_declared",      # jeder Endpunkt deklariert [Authorize]/[AllowAnonymous] oder @PreAuthorize/@PermitAll
  "http_url_literal",              # jeder HTTP-Aufruf eines Screens hat eine literale URL
  "frontend_backend_agree",        # keine Abweichung shown_but_refused / hidden_but_unprotected
  "state_changes_in_entity",       # C#: `x.Feld = Enum.Wert` auf ein Zustandsfeld nur in seiner Entität, immer literal
  "events_through_raise_function", # C#: ein `*DomainEvent`/`INotification`-Typ entsteht nur für AddDomainEvent(…)
]
```

Jeder Verstoß ist ein `convention`-Befund mit Schwere error (check endet mit Exit-Code 1; Text, JSON und SARIF mit Regel-Id `archguard/convention/<regel>`) mit Datei:Zeile und einem Hinweis zur Behebung, z. B. ``[views] navigation_literal: Navigation target `returnUrl` is not a literal, … Fix: write the target as a literal route or a route constant.`` Die Id `view-<regel>:<datei>:<gegenstand>` enthält keine Zeile und bleibt gleich, wenn sich Code darüber verschiebt. Ein unbekannter Name in der Liste ist ein Konfigurationsfehler.

Um Konventionen in einem bestehenden Projekt einzuschalten, ohne zuerst alles zu beheben, schreibt `archguard baseline write` die aktuellen Konventions-Ids in die Baseline-Datei (`conventions`, neben den Zyklus-Hashes). `archguard check --baseline` schlägt dann nur bei Befunden fehl, die dort fehlen, und listet die akzeptierten nicht; eine Id aus der Baseline, die nicht mehr auftritt, erscheint als Info-Hinweis "remove it from the baseline" und lässt check nicht fehlschlagen. Ein nicht lesbarer Sicht-Import wird nie aufgenommen. Baseline-Dateien ohne `conventions` werden weiter gelesen.

Notizen für den Coding-Agenten, erfasst in der Screens-Sicht der Desktop-App oder mit `archguard review-decision record --finding <id> --decision fix|accept-risk|defer --note "…"`, erscheinen in `check`, solange Konventionen eingeschaltet sind. Ein Befund mit Notiz bekommt in der Textausgabe die Zeile `note (fix|intentional|later, by <Autor>): <Text>`, in JSON und im MCP-Tool `check` `note {decision, text, author, ts}` und `accepted_by_note`, in SARIF `properties.note`. Eine Notiz "intentional" (accept-risk) akzeptiert den Befund wie ein Baseline-Eintrag: Er bleibt in der Ausgabe mit `accepted: intentional note`, lässt `check` nicht fehlschlagen und ist in SARIF ein Ergebnis mit `suppressions: [{kind: "external", justification: <Notiz>}]`; ein vergangenes `expires_on` beendet das. Notizen zu Lücken ohne Konvention (`view-gap:<Datei>:<Meldung>`) stehen unter "Noted gaps" (`noted_gaps` in JSON, MCP und den SARIF-Run-Properties).

### Befunde mit einer Expertenbewertung vergleichen

`archguard evaluate --init .` schreibt `.archguard/assessment.toml` mit jedem aktuellen Befund als unbewertetem Eintrag; eine vorhandene Datei wird nie überschrieben. Eine Expertin trägt die Probleme ein, die sie sieht (`[[expected]]`, mit Art und Knoten-Globs), und bewertet Befunde (`verdict = "true_positive"` oder `"false_positive"`; leer heißt unbewertet). `archguard evaluate . [--assessment DATEI] [--format json]` meldet dann Recall (gefundene erwartete Probleme / erwartete Probleme), Precision (echte Treffer / bewertete aktuelle Befunde), unbewertete Befunde, nicht gefundene erwartete Probleme mit Grund und veraltete Bewertungen und warnt, wenn die Revision der Bewertung nicht `HEAD` ist. Eine Kennzahl ohne gezählte Fälle ist „nicht gemessen", nie 0. ArchGuard schreibt selbst keine Bewertungen, und `evaluate` ist kein Gate. Das MCP-Tool `evaluation` liefert dasselbe Ergebnis. Format und Zuordnungsregeln: [ADR 0005](docs/adr/0005-evaluation-against-expert-assessment.md).

### SBOM und Lizenzrichtlinie

`archguard sbom .` listet die Fremdabhängigkeiten, die die Lock- und Manifestdateien im Repository nennen: `Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `packages.lock.json` (sonst `*.csproj` `PackageReference`), `pom.xml`, `gradle.lockfile`, `go.mod`, `poetry.lock`, `uv.lock` (sonst `requirements*.txt` / `pyproject.toml`) und `composer.lock` (sonst `composer.json`). Es fragt keine Registry und braucht kein Netz. `--format cyclonedx` schreibt CycloneDX-1.6-JSON mit purl, Version, Scope und den Manifesten (Projekte/Dienste) jeder Komponente; `--format json` und Text liefern eine Zusammenfassung. Lizenzen stammen nur aus Dateien, die sie nennen (`package-lock.json`, `composer.lock`); jede andere Lizenz ist `unknown` und wird nie nachgeladen oder geraten. Workspace-Mitglieder und Pfad-/Editable-/Projektverweise sind eigene Pakete und fehlen in der Liste; `fixtures/` sind Testdaten und werden übersprungen.

Eine Lizenzrichtlinie in `.archguard/archguard.toml` lässt `archguard check` scheitern (Exit 1, ein `license`-Befund je Komponente, auch in SARIF):

```toml
[licenses]
allow = ["MIT", "Apache-2.0", "BSD-3-Clause"]  # SPDX-IDs; nicht leer = alles andere verstößt
deny = ["GPL-3.0-only"]                         # verstößt immer
unknown = "allow"                               # "deny": eine Abhängigkeit ohne genannte Lizenz verstößt
```

SPDX-Ausdrücke werden ausgewertet (`OR`: ein erlaubter Zweig genügt, `AND`: alle Zweige müssen bestehen). `unknown` ist ein eigener Zustand und nur mit `unknown = "deny"` ein Verstoß. Das MCP-Tool `sbom` liefert dieselben Daten seitenweise. Entscheidung: [ADR 0016](docs/adr/0016-sbom-and-licence-policy.md).

## Konfiguration und Projektdaten

Die Konfiguration wird in dieser Reihenfolge gewählt: ausdrückliches `--config`, `.archguard/archguard.toml`, bisherige `archguard.toml`, danach abgeleitete Werte und Defaults. Relative Quellpfade beziehen sich auf das Projektverzeichnis. Eine fehlerhafte Konfiguration wird gemeldet und nicht stillschweigend ersetzt.

`project.exclude` enthält Globs über Pfade relativ zum Projektverzeichnis, verankert an dessen Wurzel: `vendor` meint `./vendor`, nicht `a/vendor`; `**/vendor` trifft jede Tiefe. Wie in `.gitignore` schließt ein Muster, das ein Verzeichnis trifft, alles darunter aus, und das Verzeichnis wird nicht betreten: `vendor`, `vendor/`, `vendor/**` und `**/generated` schließen ganze Verzeichnisse aus. Ein abschließendes `/` trifft nur Verzeichnisse, nie eine gleichnamige Datei. Negation (`!`) gibt es nicht. Dieselbe Regel gilt für den Quell-Walk, die Diensterkennung und die Abhängigkeitssuche.

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
```

Der Workspace enthält die native Desktop-App `archguard-desktop` ([ADR 0002](docs/adr/0002-native-desktop-ui.md)). Unter Linux baut sie gegen das System-fontconfig; vorher dessen Header installieren (Debian/Ubuntu: `libfontconfig1-dev`).

Das Verzeichnis [`scripts/`](scripts/README_DE.md) enthält Integrationstests, Benchmarks sowie Release-Prüfungen. Das sind Entwicklungswerkzeuge; für die normale CLI- und Desktop-Nutzung müssen sie nicht manuell gestartet werden. Go-Integrationstests nutzen `ARCHGUARD_GO`; sprachspezifische Tests benötigen gegebenenfalls zusätzliche installierte Werkzeuge.

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
