# Analysebeispiele

[English](README.md) | [Deutsch](README_DE.md)

- [Service-Shop](service-shop/README_DE.md): ausführbares Demo-Monorepo mit 14 Diensten,
  drei Compose-Stacks, zwei unterschiedlichen API-Zyklen und Infrastruktur.
- Apache Commons Collections: reales Open-Source-Projekt für die Paket-/Klassenanalyse,
  siehe Anleitung unten.
- [Jellyfin](jellyfin_DE.md): reales C#-Medienserver-Projekt mit 1.767 analysierten
  Quelldateien, Namespace-Hierarchie und drei zyklischen Gruppen auf der Starttiefe.
- [Apache Kafka](kafka_DE.md): größerer Testfall mit 3.369 Java-Produktionsdateien,
  unabhängig geprüften Dateizyklen und drehbarer Architekturlandschaft.

## Apache Commons Collections

[Apache Commons Collections](https://commons.apache.org/proper/commons-collections/)
ergänzt Javas Collections unter anderem um Maps, Bags, Iteratoren, Tries und
Bloomfilter. Die Paketstruktur und expliziten Java-Imports eignen sich für den
gegenwärtigen ArchGuard-Scanner.

Verwendet wird **4.6.0**, Tag `rel/commons-collections-4.6.0`, festgehalten auf
Commit [`4107624de41aeed37985d2a069fad41992fd71a2`](https://github.com/apache/commons-collections/tree/4107624de41aeed37985d2a069fad41992fd71a2).
Upstream-Lizenz: Apache-2.0; `LICENSE.txt` und `NOTICE.txt` bleiben im Checkout.
Der fremde Quellcode liegt unter `checkouts/`, das im ArchGuard-Repository ignoriert
wird. Die Analyse benötigt keinen Build und keine installierten Java-Abhängigkeiten.

## Starten

Alle Befehle vom ArchGuard-Repository-Root ausführen. Den Clone-Befehl einmal
für eine frische Arbeitskopie ausführen.

```bash
git clone --depth 1 --single-branch --branch rel/commons-collections-4.6.0 \
  https://github.com/apache/commons-collections.git examples/checkouts/commons-collections
git -C examples/checkouts/commons-collections rev-parse HEAD

mkdir -p examples/checkouts/commons-collections/.archguard
cp -n examples/commons-collections.toml \
  examples/checkouts/commons-collections/.archguard/archguard.toml

cargo run -p archguard-cli --locked -- serve examples/checkouts/commons-collections \
  --port 8788 --depth 4
```

Die ausgegebene Commit-ID muss mit dem oben verlinkten Stand übereinstimmen.
Viewer: <http://127.0.0.1:8788/>. Port 8787 kann parallel das kleine Beispiel zeigen.
`Ctrl-C` beendet den Server.

Die [Konfigurationsvorlage](commons-collections.toml) wird beim Setup in den
analysierten Checkout kopiert; `cp -n` erhält eine dort bereits vorhandene Config.
Danach wird `.archguard/archguard.toml` automatisch geladen. Graph, effektive
Config und eigenständiger HTML-Report liegen ebenfalls im Checkout unter
`.archguard`, wie im [Projektstandard](../README_DE.md#konfiguration-und-projektdaten)
beschrieben. `--config examples/commons-collections.toml` bleibt als explizite
Alternative verfügbar.

Die Konfiguration schließt `src/test/**` aus. Tiefe 4
zeigt die Kinder von `org/apache/commons/collections4`: Unterpakete sowie die direkt
in diesem Paket liegenden Dateien. `--depth 4` ist beim Server ausdrücklich nötig,
weil dessen Starttiefe standardmäßig 0 ist.
Der Graph startet mit den 18 direkten Unterpaketen; die 55 direkten Dateien lassen sich
über **Show files** einblenden. Die Analyse umfasst weiterhin alle 73 Knoten.

## Gemessener Umfang

Analyse vom 27.10.2026 mit ArchGuard 0.1.0 und der obigen Konfiguration:

| Ebene | Ergebnis |
| --- | --- |
| Produktionsquellen | 367 Java-Dateien, einschließlich `package-info.java` |
| Deklarierte Java-Pakete | 20 |
| Hierarchie | 23 Modulknoten einschließlich `org`, `org/apache`, `org/apache/commons` |
| Dateiverbindungen | 700 gerichtete Kanten mit 701 Importreferenzen |
| Sicht auf Tiefe 4 | 73 Knoten, 192 Kanten, 3 zyklische Gruppen; größte Gruppe: 16 Knoten |
| Ausgeschlossen | 310 Java-Testdateien |
| Calls mit Java-17-Basis | 601 Klassen/Typen (auch verschachtelte und anonyme), 5.040 Methoden-/Konstruktordeklarationen |
| Aufrufstellen | 10.291: 5.867 projektintern, 4.424 extern; keine unaufgelösten Ziele oder Compilerdiagnosen in diesem Lauf |

Alle 367 Datei-Knoten stimmen mit den Produktionsquellen überein. Die 701
gespeicherten Importbelege wurden gegen ihre jeweiligen Quellzeilen geprüft.
Der Live-Payload sowie dessen Start mit der echten gepinnten Graph-Bibliothek und
DOM-/Canvas-Testdoubles wurden geprüft; keine visuelle Browserabnahme.
Zusätzlich wurden alle 10.291 Aufrufbelege gegen Datei, Zeile, Spalte und
Quelltext geprüft. Diese Calls-Daten sind unabhängig von den Importkanten.

## Einstieg im Viewer

1. **Cycles → scc-2** fokussiert den kleinen Zyklus zwischen `ComparatorUtils.java`
   und dem Paket `comparators`. Er eignet sich als überschaubarer Einstieg.
2. Eine gerichtete Kante auswählen und die Importzeilen im Inspektor ansehen.
   Ein konkretes Dateipaar ist `ComparatorUtils.java:26 → NullComparator` und
   `comparators/NullComparator.java:23 → ComparatorUtils`.
3. **Return to overview** stellt die Paketübersicht wieder her. Im Explorer
   `map`, `list`, `trie` oder `bloomfilter` suchen und aufklappen.
4. Einen Paketvorschlag mit **Simulate suggested move** bestätigen und anschließend
   **Undo** verwenden; Importbeziehungen werden durch Verschieben nicht entfernt.

Für Klassen und Methoden **Calls** öffnen und das Paket
`org.apache.commons.collections4` auswählen. Nach `CollectionUtils` suchen und
`CollectionUtils → AbstractMultiSet` öffnen: Quellbelege wie
`CollectionUtils.java:1830` begründen einen Interface-Prüfplan. Dabei zuerst
vorhandene Interfaces prüfen. `BagUtils → CollectionBag` zeigt einen
Verschiebekandidaten in Richtung `bag`, unter anderem anhand von `BagUtils.java:44`.
**Copy review plan** kopiert den Vorschlag; Quellcode wird nicht automatisch geändert.
Voraussetzung für die Erzeugung dieser Daten ist ein JDK 17+ auf PATH; Grenzen
der statischen Analyse stehen in der [Viewer-Anleitung](../README_DE.md).

CLI-Abfragen für denselben Stand:

```bash
target/debug/archguard scan examples/checkouts/commons-collections \
  --level file
target/debug/archguard cycles examples/checkouts/commons-collections \
  --format json
target/debug/archguard hotspots examples/checkouts/commons-collections \
  --top 20 --format json
target/debug/archguard performance examples/checkouts/commons-collections \
  --format json
target/debug/archguard co-change examples/checkouts/commons-collections \
  --since 6.months --format json
```

Der Scan speichert seinen vollständigen Importgraph unter
`examples/checkouts/commons-collections/.archguard/graph.json`.

**Review priorities** öffnet im Viewer eine begründete Rangliste. Gelbe P1/P2-Marken
zeigen Kandidaten, der Inspektor erklärt Kopplung, Zyklusbeitrag, transitive Nutzer
und verfügbare Git-Dateiberührungen. Die ausklappbaren KPIs ergänzen CCD/NCCD,
größte zyklische Gruppe und Quellenabdeckung. **Source & build indicators** zeigt
unter anderem Java-Abstraktheit und normierte Main-Sequence-Distanz bei vollständigen
Compiler-Daten. Diese Quellenbelege gehören zum ursprünglichen Report; Struktur-KPI
und Review-Rangfolge folgen dagegen der aktuellen Navigation und Simulation.

Der oben dokumentierte Clone ist **shallow**. Git-Zählungen sind deshalb
Untergrenzen; ein leerer Co-Change-Report aus diesem Checkout belegt keine fehlende
historische Kopplung. Für eine längere Auswertung kann die Historie ausdrücklich
nachgeladen werden:

```bash
git -C examples/checkouts/commons-collections fetch --unshallow
```

Die neue Auswertung benennt ihre tatsächliche Historien-/Quellabdeckung in JSON
und im Viewer. Es werden keine neuen Messwerte für dieses Beispiel behauptet;
der obige Umfang ist der separat dokumentierte Lauf. Score, Konfigurationsregeln
und Grenzen sind in der [Review-Anleitung](../README_DE.md#häufige-arbeitsabläufe)
beschrieben.

## Grenzen der Aussage

Dies ist eine Importanalyse, keine vollständige Java-Typanalyse: implizite
Beziehungen innerhalb desselben Pakets und vollständig qualifizierte Typzugriffe
ohne Import werden nicht erfasst. Sieben Importwarnungen bleiben: vier betreffen
`org.w3c.dom`, drei verschachtelte Typen (`TreeBidiMap.DataElement` und
`AbstractHashedMap.HashEntry`). Die beiden `DataElement`-Imports liegen innerhalb
derselben Quelldatei. Diese Grenzen werden nicht durch veränderten Upstream-Code
kaschiert.

Die strenge Beispielregel meldet zyklische Gruppen als Findings; das ist keine
Behauptung, das Upstream-Projekt verletze seine eigenen Architekturvorgaben.
Zyklusbeispiele und Cut-Vorschläge sind begrenzt gesampelt. Es werden keine
beabsichtigten Schichten erfunden; `derive_layers` ist für dieses Beispiel aus.
