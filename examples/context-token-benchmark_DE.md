# Kontext-Tokens mit und ohne MCP

[English](context-token-benchmark.md) | [Deutsch](context-token-benchmark_DE.md)

Gemessen am 28.10.2026: Gegen eine gezielte Suche mit `rg` und Dateiabrufen ist
für diese drei Beispiele **keine allgemeine Tokenersparnis nachgewiesen**.
Die MCP-Antworten enthalten zusätzliche strukturierte Belege und Metadaten;
die beiden Verfahren liefern nicht dieselbe Analyseabdeckung.

## Ergebnis

`tiktoken 0.14.0`, Encoding `o200k_base`. Gezählt wurden die tatsächlichen
Abrufargumente/Befehle und Antworttexte, einschließlich aller Folgeseiten.
Beide Verfahren enthalten dieselbe vollständige Lektüre der jeweiligen
Zieldatei mit Zeilennummern. Die Zahlen enthalten noch keine Tool-Schemas.

| Zieldatei / Projekt | Gezielte Suche | MCP, 4 KiB | MCP, 16 KiB |
| --- | ---: | ---: | ---: |
| `src/a/alpha.ts` / TS-Zyklus | 613 | 909 | 948 |
| `AbstractBagDecorator.java` / Commons Collections | 2.721 | 4.191 | 3.321 |
| `ExtraRuleResolver.cs` / Jellyfin | 2.554 | 2.595 | 2.339 |
| **Summe** | **5.888** | **7.695** | **6.608** |
| Mehrbedarf gegenüber Suche | — | **30,7 %** | **12,2 %** |

Alle MCP-Seiten sind vollständig, ohne Auslassungen und innerhalb einer Variante
revisionsgebunden. Beide Varianten enthalten dieselben Items. Die 4-KiB-Variante
braucht 1/4/2 Aufrufe; die 16-KiB-Variante jeweils einen. Die größere Variante
übergibt schon beim ersten Aufruf die Revision des Standardlaufs, um denselben
Datenstand zu prüfen. Diese zusätzlichen Argument-Tokens sind mitgezählt; deshalb
ist sie beim kleinen TS-Beispiel etwas größer.

Einmalig hinzugefügte, kompakt serialisierte Tool-Beschreibungen benötigen
weitere **549 Text-Tokens** für `change_context` oder **4.847** für den tatsächlichen
Katalog mit 26 Tools. Bei den drei 16-KiB-Abrufen ergibt das 7.157 beziehungsweise
11.455 Tokens statt 5.888 für die Suche. Das ist eine separate Rechnung für
einmalig bereitgestellten Schemakontext, keine Annahme über einen bestimmten Client.

Mit `cl100k_base` bleibt der Befund bestehen: Suche 5.758, MCP mit 4 KiB 7.505
(30,3 % mehr), mit 16 KiB 6.428 (11,6 % mehr).

## Unterschiedliche Belege

Die Aufgabe war jeweils, direkte Aufrufer und Abhängigkeiten einer vorgegebenen
Datei, belegbare Zyklen und Konfiguration zu untersuchen. Die Suchpläne wurden
unabhängig von den MCP-Ergebnissen erstellt; sie lesen keine generierten Graphen.
Es wurde keine Codeänderung bis zum erfolgreichen Testabschluss verglichen.

- TS: Beide Wege belegen zwei eingehende und eine ausgehende Abhängigkeit sowie
  den Zyklus `alpha ↔ beta`. MCP nennt zusätzlich effektive Standard-Gates.
- Commons: Die Suche findet drei Unterklassen und drei Imports. MCP liefert
  zusätzlich `CollectionSortedBag.java` als Aufrufer anhand eines geerbten
  Methodenaufrufs. Die Suche beweist keine Zyklusfreiheit.
- Jellyfin: Beide belegen zwei Aufrufer und den Datei-Zyklus
  `ExtraRuleResolver ↔ VideoResolver`. Die Suche lokalisiert sieben explizite
  lokale Typabhängigkeiten; MCP liefert nur drei ausgehende Dateien. Für die vier
  fehlenden Typen `NamingOptions`, `ExtraRule`, `MediaType` und `ExtraRuleType`
  wurde ein zusätzlicher tatsächlicher `rg`-Abruf gemessen: **718 Tokens**.
  Damit steigt die 16-KiB-Variante auf **3.057 Tokens**, also **19,7 % mehr** als
  die Suche. Die rohe Ersparnis von 8,4 % in der Tabelle ist deshalb kein Nachweis
  gleichwertiger Dependency-Ermittlung.

Der semantische Status bleibt `unavailable` für TS und `partial` für Java/C#.
Auch vollständige MCP-Seiten sind kein vollständiger semantischer Impact-Report:
Kantenbelege und Zyklusbeispiele sind begrenzt, reine Typreferenzen und dynamische
Aufrufe werden nicht vollständig erfasst. Die ergänzte Suche stellt ebenfalls
keine vollständige semantische Gleichwertigkeit her.

## Was gezählt wurde

Für die Suche: Summe der einzeln tokenisierten Befehlsstrings, `stdout` und
`stderr`. Für MCP: Summe der originalen kompakten `arguments_text` und
`response_text` aller Aufrufe plus identischer Zieldatei-Lektüre. Antwort-JSON
wurde nicht für die Zählung umformatiert. Größere Budgets sind Byte-Budgets,
keine Token-Budgets. Prozentwerte beziehen sich auf die Summen.

Nicht enthalten: gemeinsame Aufgabenbeschreibung und Shell-Toolbeschreibung,
Modellantworten, Reasoning, RPC-Hüllen, Client-Formatierung und erneut übermittelte
Gesprächshistorie. Der Schemaaufschlag zählt JSON-Text; er ist kein exaktes
Modellprotokoll. Es wurde kein API-Aufruf mit Usage-Abrechnung ausgeführt und
keine Laufzeitersparnis gemessen. Keines der Encodings wird als exakter
Tokenizer des hier verwendeten Codex-Modells ausgegeben. OpenAI unterscheidet
lokale Textzählung von vollständiger API-Eingabe einschließlich Rollen und
Tool-Formatierung: [Token counting](https://developers.openai.com/api/docs/guides/token-counting).

## Wiederholen

Voraussetzung: die drei Beispiele aus `scripts/benchmarks/context_benchmark_cases.json`
liegen lokal vor, das CLI ist mit MCP-Unterstützung gebaut. Die feste Baseline
spielt die aufgezeichneten Suchbefehle ohne Shell nach; sie ist kein allgemeiner
Suchagent. Quellen, Compilerstand und Konfiguration beeinflussen die Ergebnisse.

```sh
python3 scripts/benchmarks/capture_context_baseline.py
python3 scripts/benchmarks/capture_context_benchmark.py
python3 -m venv /tmp/archguard-token-benchmark-venv
/tmp/archguard-token-benchmark-venv/bin/python -m pip install tiktoken==0.14.0
TIKTOKEN_CACHE_DIR=/tmp/archguard-token-benchmark-cache /tmp/archguard-token-benchmark-venv/bin/python scripts/benchmarks/benchmark_context_tokens.py
python3 -m unittest discover -s scripts -p 'test_*context*.py'
```

Tokenizervokabular wird beim ersten Gebrauch heruntergeladen. Es werden keine
Quelltexte an eine LLM-API übertragen. MCP kann seine normalen lokalen Caches
aktualisieren. Die 13 Regressionstests benötigen weder Netzwerk noch Tokenizer.

Die lokalen Messartefakte liegen unter
`.archguard/reports/context-token-benchmark/`: `tokens.json` mit beiden Encodings,
originale Baseline-/MCP-Transkripte, Quell-/Artefakt-Hashes und `provenance.json`
mit Binary-Hash und Git-Ständen. Diese generierten Dateien sind gitignoriert.
Der separate Jellyfin-Zusatzabruf ist als `baseline/jellyfin-mcp-followup.json`
gesichert und wird vom Zähler berücksichtigt, falls vorhanden; er gehört nicht
zum festen Baseline-Replay. Kommando:

```sh
rg -n -B 6 -g '*.cs' -g '!tests/**' -g '!fuzz/**' -g '!**/obj/**' -g '!**/bin/**' -g '!**/AssemblyInfo.cs' -g '!SharedVersion.cs' '(class|enum) (NamingOptions|ExtraRule|MediaType|ExtraRuleType)\b' .
```

Dieser Abruf läuft im Jellyfin-Checkout. Für die gemessenen Beispiele reduziert
das größere Budget den Textumfang der MCP-Variante einschließlich Zieldatei-
Lektüre um 14,1 %. Weitere Optimierungen
müssen gegen diese gezielte Suchbaseline und die Belegabdeckung geprüft werden;
eine kleine erste Seite allein ist kein Nachweis einer Ersparnis.

## Gezielter Graph-DB-Abruf

Anschließender Versuch am selben Commons-Collections-Beispiel: einen separaten
Mirror mit `db sync` erzeugt (390 Knoten, 1121 Kanten), anschließend über das
vorhandene MCP-Tool `graph_query` nur direkte Nachbarn abgefragt. Die Projektion
liefert `direction`, `file` und `refs`. Alle sieben Beziehungen und ihre
Referenzanzahlen stimmen mit den direkten Nachbarn des gespeicherten
`change_context` überein; die Antwort ist nicht abgeschnitten.

| Abruf, ohne Zieldatei-Lektüre und Schema | Text-Tokens, o200k_base |
| --- | ---: |
| Gezielte DB-Abfrage: Argumente | 156 |
| Gezielte DB-Abfrage: Antwort | 156 |
| **Gezielte DB-Abfrage insgesamt** | **312** |
| Vollständiges `change_context`, 16 KiB | 2.324 |

Das `graph_query`-Schema würde separat 336 Text-Tokens hinzufügen. Der Versuch
zeigt eine kleinere Antwort auf eine engere Frage, **keine gleichwertige
Ersetzung des vollständigen Kontextpakets**: Die DB-Antwort enthält weder
Quellzeilenbelege noch Regeln, Revision oder semantische Abdeckung. `file` ist
hier ein logischer Graphname, nicht zwingend ein physischer Dateipfad. Sync und
Schemawissen sind Vorbereitung; sie sind nicht als Query-Tokens eingerechnet.
Die Datenbank wurde für diesen Versuch unmittelbar davor synchronisiert.

Die aktuelle DB ist ein separater Spiegel; `change_context` verwendet ihn nicht.
Er speichert Knoten, Abhängigkeiten, Findings und Snapshot-Summen, aber keine
Methoden oder Quellbelege. `graph_query` prüft keine Aktualität. Außerdem wird
beim Sync der vollständige Graph mit einem Modulreport kombiniert: für
Dateiknoten gespeicherte Metriken und Zyklusflags sind deshalb nicht als
verlässliche Datei-Analyse zu verwenden. Der Versuch nutzt ausschließlich
`depends_on` und deren Referenzanzahlen.

Ein sinnvoller Ausbau wäre eine kleine revisionsgebundene Übersicht mit
gezieltem Nachladen ausgewählter Kantenbelege. Dafür fehlen im DB-Mirror
Quellzuordnung, Belege, Abdeckungsstatus und ein atomar veröffentlichter
Datenstand. Das bestehende `edge_evidence` arbeitet auf der Modulansicht und
kann diese Datei-Belege nicht direkt ergänzen. Reine Typabhängigkeiten entstehen
nicht durch einen Wechsel des Speichers. Die Tokenreduktion kommt von Auswahl
und Antwortprojektion; derselbe Mechanismus ist auch auf dem In-Memory-Graphen
möglich.

Originalabfrage, Antwort, Zähler und Datenbank-Hash sind lokal unter
`.archguard/reports/context-token-benchmark/graph-db/commons-neighbors.json`
gesichert, der Sync-Lauf unter `commons-sync.json`. Die isolierte Datenbank liegt
im Commons-Checkout unter `.archguard/reports/context-token-benchmark/graph.db`.
