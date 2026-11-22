# Distribution brief / Verbreitungsbrief

Prepared 2026-11-20. This is submission copy, not a record of submitted or accepted entries. Read each destination's current contribution rules immediately before submitting.

## MCP directory entry — English

**Name:** ArchGuard

**Short description:** Local architecture analysis for coding agents: explore dependencies, trace findings to source evidence and compare hypothetical refactorings.

**Long description:** ArchGuard is a non-commercial open-source Rust CLI, native desktop app and stdio MCP server. It analyses source code and supported project/deployment configuration. Agents can ask for bounded architecture context, inspect cycle evidence, compare changes and preview graph refactorings. Missing and unresolved evidence stays explicit. Some tools can persist project-local artifacts; query access should not be described as making every tool read-only.

**Repository:** https://github.com/svenhartmann/ArchGuard

**Licence:** MIT OR Apache-2.0 (ArchGuard; third-party projects retain their licences).

**Transport:** local stdio; no hosted endpoint or API key required for the ArchGuard process itself. The chosen agent client has its own provider and data-handling settings.

**Launch command:** `archguard mcp --root /absolute/path/to/project`

**Prerequisites:** installed ArchGuard binary; supported language tools for deeper compiler analysis. See the repository README and site MCP/prerequisite guides.

**Tags to adapt to the directory's vocabulary:** architecture, dependencies, static-analysis, developer-tools, rust, mcp.

**Demo request:** “Show one dependency cycle, the source location for every edge, and the analysis gaps. Do not edit source files.”

## MCP-Verzeichniseintrag — Deutsch

**Kurzbeschreibung:** Lokale Architekturanalyse für Coding-Agents: Abhängigkeiten erkunden, Befunde bis zur Quelle verfolgen und hypothetische Refactorings vergleichen.

**Langbeschreibung:** ArchGuard ist ein nicht-kommerzielles Open-Source-Projekt mit Rust-CLI, nativer Desktop-App und stdio-MCP-Server. Es analysiert Quellcode und unterstützte Projekt-/Deployment-Konfiguration. Agents können begrenzten Architekturkontext abfragen, Zyklusbelege untersuchen, Änderungen vergleichen und Graphumbauten simulieren. Fehlende und unaufgelöste Evidenz bleibt sichtbar. Einige Werkzeuge schreiben lokale Projektartefakte; nicht alle Tools sind ausschließlich lesend.

Repository, Lizenz und Startbefehl entsprechen dem englischen Eintrag. Der ArchGuard-Prozess benötigt keinen gehosteten Endpunkt oder eigenen API-Schlüssel. Für den gewählten Agenten-Client gelten dessen Anbieter- und Datenverarbeitungseinstellungen.

**Demo-Frage:** „Zeige einen Abhängigkeitszyklus, die Quellstelle jeder Kante und die Analyse-Lücken. Ändere keinen Quellcode.“

## Destinations / Ziele

| Destination | Prepared action / vorbereiteter Schritt |
| --- | --- |
| [Official MCP Registry](https://github.com/modelcontextprotocol/registry) | Check supported distribution and ownership verification; create validated metadata only for an actually available package. / Unterstützte Distribution und Eigentumsnachweis prüfen; Metadaten nur für ein wirklich verfügbares Paket erstellen. |
| [MCP servers collection](https://github.com/modelcontextprotocol/servers) | Follow its current route for community listings; do not present ArchGuard as an official reference server. / Aktuellen Weg für Community-Einträge beachten; keine offizielle Referenzimplementierung behaupten. |
| [Awesome Rust](https://github.com/rust-unofficial/awesome-rust) | Propose one descriptive line in the applicable development-tools category after checking its criteria. / Nach Prüfung der Kriterien eine sachliche Zeile in der passenden Werkzeugrubrik vorschlagen. |
| [Awesome Software Architecture](https://github.com/simonaronsson/awesome-software-architecture) | Check category fit and contribution rules before preparing a PR. / Rubrik und Beitragsregeln vor einem PR prüfen. |

The official registry stores metadata and validates package ownership; a local executable path is not proof of a publishable package. Distribution packaging and authenticated submission remain separate work. [Official requirements](https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/official-registry-requirements.md).

Die offizielle Registry speichert Metadaten und prüft Paketeigentum; ein lokaler Binärpfad belegt kein veröffentlichbares Paket. Paketierung und authentifizierte Einreichung bleiben eigene Schritte. [Offizielle Anforderungen](https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/official-registry-requirements.md).

Suggested Awesome-list line / Vorschlag:

> [ArchGuard](https://github.com/svenhartmann/ArchGuard) — Source-based architecture analysis with a native desktop app, CLI gates and a local MCP server; explicit evidence and coverage limits.

GitHub topics to review manually: `software-architecture`, `static-analysis`, `dependency-analysis`, `rust`, `mcp-server`, `arc42`. No topic changes have been made.

## Meetup and conference abstract — English

### From a dependency map to an architecture decision

A graph can reveal a cycle without explaining whether it matters. In this live session we use ArchGuard on a pinned open-source project, follow one relationship to its source and distinguish configured service connections from recorded runtime evidence. We ask a coding agent to inspect the same finding, then simulate a change and discuss the decisions that the graph cannot make for us. Attendees leave with a repeatable review exercise and a way to separate missing measurements from reassuring-looking zeroes.

Format: 25-minute talk plus 10-minute discussion. Audience: developers, architects and platform engineers. Use an offline-ready eShop or OpenTelemetry Demo checkout. Venue candidates from the plan: OOP, JAX, iSAQB and local Rust/architecture meetups; no event date or acceptance is assumed.

## Vortragseinreichung — Deutsch

### Von der Abhängigkeitskarte zur Architekturentscheidung

Ein Graph zeigt einen Zyklus, erklärt aber noch nicht seine Bedeutung. In dieser Live-Session untersuchen wir mit ArchGuard ein gepinntes Open-Source-Projekt, verfolgen eine Beziehung bis zur Quelle und trennen konfigurierte Service-Verbindungen von aufgezeichneten Laufzeitbelegen. Ein Coding-Agent untersucht denselben Befund. Anschließend simulieren wir eine Änderung und diskutieren die Entscheidungen, die uns der Graph nicht abnimmt. Die Teilnehmenden erhalten eine reproduzierbare Review-Übung und einen Umgang mit fehlenden Messwerten.

Format: 25 Minuten plus 10 Minuten Diskussion. Zielgruppe: Entwicklung, Architektur und Platform Engineering. eShop oder OpenTelemetry Demo vorab offline bereitstellen. OOP, JAX, iSAQB und lokale Meetups sind mögliche Ziele; Termine und Zusagen sind offen.

## Demo run sheet / Demo-Ablauf

| Minute | English | Deutsch |
| --- | --- | --- |
| 0–3 | State question, revision and analysis scope | Frage, Commit und Scope nennen |
| 3–8 | Explore service overview, then one package | Serviceübersicht, dann ein Paket erkunden |
| 8–13 | Trace a cycle edge to its source | Zykluskante zur Quelle verfolgen |
| 13–18 | Ask MCP for the same evidence and its gaps | MCP nach demselben Beleg und seinen Lücken fragen |
| 18–22 | Preview one change; explain what remains unproven | Änderung simulieren; offene Nachweise nennen |
| 22–25 | Show CI/documentation handoff and the next decision | Übergang zu CI/Dokumentation und nächste Entscheidung |

Fallback: use the real recorded clip and saved outputs from the same revision if a live tool fails. State that the recording is a fallback; do not act out a successful live response.

Bei einer Live-Störung den echten aufgezeichneten Clip und gespeicherte Ergebnisse desselben Commits verwenden. Den Wechsel zur Aufnahme offen benennen; keine erfolgreiche Live-Antwort nachspielen.
