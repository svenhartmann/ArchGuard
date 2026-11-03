# Jellyfin: reales C#-Analysebeispiel

[English](jellyfin.md) | [Deutsch](jellyfin_DE.md)

[Jellyfin](https://github.com/jellyfin/jellyfin) ist ein freier Medienserver mit
HTTP-API, Medienbibliothek, Metadaten-Anbietern, Streaming und Transcoding.
Das Beispiel analysiert seinen echten C#-Quellcode. Jellyfin selbst wird dabei
nicht gebaut oder gestartet. Die Importanalyse benötigt weder .NET noch Datenbanken
oder Medienbestände. Mit einem installierten .NET SDK 8+ ergänzt Roslyn den Datei-
und Paketgraphen um aufgelöste interne Aufrufe und Vererbungsbeziehungen. Fehlende
Compiler oder Referenzen werden als eingeschränkte Abdeckung ausgewiesen.

Verwendet wird **v10.11.11**, festgehalten auf Commit
[`1fbd8739292cce610231be93daf43368733edf63`](https://github.com/jellyfin/jellyfin/tree/1fbd8739292cce610231be93daf43368733edf63).
Die GPL-2.0-Lizenzdatei `LICENSE` bleibt im Upstream-Checkout. Der fremde Quellcode
liegt ausschließlich im ignorierten Verzeichnis `examples/checkouts/jellyfin`.

## Starten

Befehle im ArchGuard-Repository ausführen. Den Clone-Befehl einmal für eine
frische Arbeitskopie ausführen.

```bash
git clone --depth 1 --single-branch --branch v10.11.11 \
  https://github.com/jellyfin/jellyfin.git examples/checkouts/jellyfin
git -C examples/checkouts/jellyfin rev-parse HEAD

mkdir -p examples/checkouts/jellyfin/.archguard
cp -n examples/jellyfin.toml examples/checkouts/jellyfin/.archguard/archguard.toml

cargo run -p archguard-cli --locked -- serve examples/checkouts/jellyfin \
  --port 8790 --depth 1
```

Die Commit-ID muss dem oben verlinkten Stand entsprechen. Der lokale Viewer
läuft unter <http://127.0.0.1:8790/>; `Ctrl-C` beendet ihn. Er aktualisiert die
Analyse bei Quelländerungen. Die anderen Beispiele können parallel laufen.

Die [Konfigurationsvorlage](jellyfin.toml) wird beim Setup in den analysierten
Checkout kopiert; `cp -n` erhält eine dort bereits vorhandene Config. Danach wird
`.archguard/archguard.toml` automatisch geladen. Graph, effektive Config und
eigenständiger HTML-Report liegen ebenfalls im Checkout unter `.archguard`, wie
im [Projektstandard](../README_DE.md#konfiguration-und-projektdaten) beschrieben.
`--config examples/jellyfin.toml` bleibt als explizite Alternative verfügbar.

Die Konfiguration analysiert C#-Produktionsquellen aus allen
Projektordnern. Tests, Fuzzing, Build-Ausgaben und 16 reine Assembly-/Versions-
Metadaten-Dateien sind ausgeschlossen. Die logische Hierarchie folgt den
`namespace`-Deklarationen, auch wenn die öffnende Klammer in der nächsten Zeile
steht. Tiefe 1 zeigt unter `Emby`, `Jellyfin` und `MediaBrowser` die jeweiligen
Anwendungs- und Infrastruktur-Namensräume.

## Gemessener Umfang

Analyse vom 27.10.2026 mit obigem Quellstand und dieser Konfiguration:

| Ebene | Ergebnis |
| --- | --- |
| Analysierte C#-Dateien | 1.767 |
| Gerichtete Importbeziehungen | 4.942 mit 4.942 Quellzeilenbelegen |
| Datei-/Namespacegraph einschließlich semantischer Abhängigkeiten | 7.569 Beziehungen mit 10.767 Belegen |
| Zusätzliche semantische Belege | 5.046 interne Aufrufstellen und 779 Vererbungs-/Interfacebeziehungen |
| Zyklische Gruppen auf Dateiebene | 9; größte Gruppe: 17 Dateien |
| Startansicht, Tiefe 1 | 20 Knoten, 100 Beziehungen |
| Zyklische Gruppen in dieser Ansicht | 3; größte Gruppe: 4 Knoten |
| ACD in dieser Ansicht | 8,65 |
| Zusätzliche Platzhalter im vollständigen Graphen | 256 `namespace/*`-Knoten für Namespace-Imports |

Alle analysierten Dateien und gespeicherten Import-, Aufruf- und Deklarationsbelege
wurden mit den Quelldateien abgeglichen. Enthalten ist unter anderem der
Dateizyklus: `ExtraRuleResolver.cs:28` ruft `VideoResolver.IsVideoFile`
auf; `VideoResolver.cs:81` ruft zurück zu `ExtraRuleResolver.GetExtraInfo`.
Die automatisierte Prüfung mit installiertem .NET SDK 8+ lässt sich wiederholen:

```bash
cargo build -p archguard-cli --all-features --locked
python3 scripts/integration/test_jellyfin_example.py
```

Sie lädt nichts herunter und benötigt den oben beschriebenen Checkout. Deshalb
ist sie separat von der Offline-CI ausführbar. Der tatsächliche Viewer-Payload
wurde zusätzlich mit dem vorhandenen DOM-/SVG-Testharness geprüft: Start-KPIs,
100 gerichtete Pfeile, Namespace-Navigation und Auswahl. Keine Browser-Abnahme.

## Einstieg im Viewer

1. **Cycles → scc-3** zeigt `Jellyfin/MediaEncoding` und
   `MediaBrowser/Controller` als kleine Gruppe auf der Starttiefe.
2. Eine Richtung auswählen und die Belege im Inspector lesen:
   `src/Jellyfin.MediaEncoding.Hls/Cache/CacheDecorator.cs:9` importiert
   `MediaBrowser.Controller.Persistence`; in der anderen Richtung importiert
   `MediaBrowser.Controller/Library/IKeyframeManager.cs:5`
   `Jellyfin.MediaEncoding.Keyframes`.
3. Die Pakete aufklappen: `Hls` und `Keyframes` sind unterschiedliche Teile des
   gröberen Knotens `MediaEncoding`. Ein Zyklus auf dieser aggregierten Ebene
   beweist deshalb keinen Datei- oder Assembly-Zyklus. Der äußere Paketkontext
   bleibt beim Navigieren sichtbar und wird separat gekennzeichnet.
4. **Return to overview**, dann im Explorer `Jellyfin/Api/Controllers`,
   `MediaBrowser/Controller/Entities` oder `MediaBrowser/Providers` untersuchen.
   **Review priorities** zeigt begründete Kandidaten für eine genauere Prüfung.

```bash
target/debug/archguard scan examples/checkouts/jellyfin \
  --level file
target/debug/archguard cycles examples/checkouts/jellyfin \
  --format json
target/debug/archguard hotspots examples/checkouts/jellyfin \
  --top 20 --format json
target/debug/archguard calls examples/checkouts/jellyfin \
  --format json
```

Der Scan speichert seinen vollständigen Abhängigkeitsgraph unter
`examples/checkouts/jellyfin/.archguard/graph.json`.

## Grenzen der Aussage

- Der C#-Scanner erfasst explizite `using`-Direktiven und Namensräume. Ein normaler
  Namespace-Import zeigt auf einen `namespace/*`-Platzhalter; er behauptet keinen
  Aufruf einer bestimmten Klasse. Implizite Imports, Typverwendung innerhalb
  eines Namensraums, Präprozessor-Auswertung und `.csproj`-Projektbeziehungen
  sind keine vollständige C#-Compileranalyse.
- **Roslyn ergänzt den Datei- und Paketgraphen.** Der lokale Lauf mit SDK 8.0.204
  liefert 1.797 Typen, 15.055 Methoden und 64.683 Aufrufstellen: 8.531 mit
  Projektziel, 14.938 mit externem Ziel und 41.214 mit unaufgelöstem Ziel.
  Die Abdeckung ist `partial`: Jellyfin verlangt SDK 9, und Referenzen sowie
  Teile seiner Projektkonfiguration fehlen im statisch ausgewerteten Kontext.
  Diese Daten sind kein vollständiger Aufrufgraph. Eindeutig aufgelöste interne
  Aufrufe und bekannte interne Obertypen fließen vor der Zyklensuche, den KPIs
  und den Regeln in den Graphen ein. Aufrufe innerhalb derselben Datei erzeugen
  keine Dateikante; externe und unaufgelöste Aufrufe werden nicht zu internen
  Kanten umgedeutet. Bei Teilklassen werden die tatsächliche Methodendatei und
  die Deklaration der Vererbungsbeziehung verwendet. 75 weitere Obertyp-Beziehungen
  ließen sich in diesem Lauf nicht eindeutig als Dateikante belegen und wurden
  ausgelassen; kein aufgelöster Projektaufruf scheiterte an der Dateizuordnung.
  Die Calls-Ansicht bleibt der ursprüngliche Quellbefund, auch
  wenn im Graphen eine Refactoring-Simulation aktiv ist. **Auch nach dieser
  Integration bedeutet „0 Dateizyklen“ bei eingeschränkter Abdeckung keine
  vollständige Entwarnung.**
- Im Lauf bleiben 1.081 nicht aufgelöste Imports, darunter externe NuGet-
  Namensräume wie `Microsoft.Extensions.Logging` und `TagLib`. Sie sind keine
  analysierten internen Kanten. Die ersten Warnungen erscheinen im CLI.
- Die Projektinventur erkennt 42 Manifestprojekte und `Jellyfin.Server` als eine
  ausführbare Anwendung. Im gepinnten Checkout gibt es keine Compose-Dateien.
  Die übrigen Bibliotheken und Werkzeuge werden nicht als eigenständige Dienste
  ausgegeben; aus Projekt- oder Namespace-Namen werden keine API-Kanten erfunden.
- Der Checkout ist shallow. Git-Churn-Zahlen sind Untergrenzen und kein
  historisches Risikoranking. Für eigene Zeitraumanalysen die Historie bewusst
  mit `git -C examples/checkouts/jellyfin fetch --unshallow` nachladen.
- Die Beispielregel meldet Paketzyklen zur Untersuchung. Sie ist keine Aussage
  über Jellyfins eigene Architekturvorgaben; es werden keine Soll-Schichten erfunden.
