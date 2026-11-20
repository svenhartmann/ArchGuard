# Launch-Texte — Deutsch

Status: redaktioneller Entwurf. Erst nach Veröffentlichung der überarbeiteten Website und der beschriebenen Version verwenden. Repository-Link ist bekannt; die endgültige Website-URL ergänzen. Funktionen nur dann als neu bezeichnen, wenn die Release Notes das belegen.

## Blog-Einstieg

### Architektur für Mensch und Coding-Agent

„Warum hängt dieses Paket von jenem ab?“ ist eine gute Frage für Entwickler:innen und Coding-Agents. Beide brauchen dieselben Grundlagen: die Beziehung, die Quellstelle dahinter und die Grenzen der Analyse.

ArchGuard beginnt mit Quellcode und Projektkonfiguration. In der nativen Desktop-App kannst du die World View erkunden, Zyklen verfolgen und die Belege zu Abhängigkeiten öffnen. Die CLI bringt Architekturregeln in den Build. Der lokale MCP-Server stellt dieselbe Analyse einem Coding-Agenten zur Verfügung.

Eine konfigurierte Verbindung und ein aufgezeichneter Laufzeitaufruf sind unterschiedliche Aussagen. Ein unaufgelöstes Ziel ist kein Nullwert. Diese Unterschiede sichtbar zu halten, ist der Grundsatz des Projekts: Fehlende Evidenz ist keine Null.

Das enthaltene Service-Shop-Beispiel lässt sich ohne laufende Container analysieren:

```sh
archguard-desktop examples/service-shop
archguard services examples/service-shop --format json
```

Für ein echtes polyglottes Projekt führt die Website durch einen gepinnten Stand der OpenTelemetry Demo. Die Grenzen der Analyse gehören zum Beispiel.

ArchGuard ist ein nicht-kommerzielles Open-Source-Projekt unter MIT OR Apache-2.0. Hilfreiches Feedback nennt ein reproduzierbares Repository, einen Commit und eine konkrete Architekturfrage.

Quellcode und Installation: https://github.com/svenhartmann/ArchGuard

## Community-Post

Titel: **ArchGuard: Architektur lokal erkunden und per MCP abfragen**

Ich arbeite an ArchGuard, einem nicht-kommerziellen Open-Source-Werkzeug zur Architekturanalyse in Rust. Es bietet eine native Desktop-App, eine CLI und einen lokalen MCP-Server. Du kannst Paketabhängigkeiten und Zyklen verfolgen, unterstützte Service-Konfigurationen untersuchen und einen Coding-Agenten dieselbe Analyse abfragen lassen.

Mir ist die Unterscheidung zwischen „Null gemessen“ und „nicht gemessen“ wichtig. Compiler-Abdeckung, unaufgelöste Aufrufe und importierte Laufzeitdaten bleiben sichtbar. Simulationen zeigen mögliche Änderungen am Graphen; sie verändern keinen Quellcode.

Repository und Website enthalten reproduzierbare Beispiele und echte App-Aufnahmen. Ich freue mich über konkrete Fälle, in denen eine Kante falsch ist oder der Analyseumfang unklar bleibt.

https://github.com/svenhartmann/ArchGuard

Redaktion: Website-Link und echten Clip ergänzen. Für englischsprachige Foren [englische Fassung](launch-en.md) verwenden. Aktuelle Kanalregeln prüfen und die eigene Beteiligung offenlegen.

## Mastodon / LinkedIn

Ein Abhängigkeitsdiagramm wird nützlich, wenn sich jede Kante erklären lässt. ArchGuard verbindet eine native Architekturansicht, CLI-Prüfungen und einen lokalen MCP-Server. Echte Quellbelege; unbekannte Messwerte bleiben unbekannt. Nicht-kommerziell und Open Source.

https://github.com/svenhartmann/ArchGuard

Clip-Beschreibung: **Von der Paketlandschaft zur Quelle hinter einem Zyklus. Echte ArchGuard-Aufnahmen von Apache Commons Collections.**

Alternativtext: **ArchGuard zeigt die World View, den Abhängigkeitsgraphen und Quellbelege für Apache Commons Collections. Das Video hat keinen Ton.**
