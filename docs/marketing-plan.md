# Marketingplan Website

Stand: 2026-11-20 · Status: Entwurf, keine Umsetzung

**Kurz vorweg:** ArchGuard ist nicht-kommerziell. Ziel des Marketings ist deshalb Nutzung (Downloads, Stars, CI-Einsätze, MCP-Nutzer), kein Umsatz. Die Website zeigt heute etwa die Hälfte der Features, und manche Angaben sind veraltet. Der größte Hebel ist, sie auf den Stand des [README](../README.md) zu bringen, nach Zielgruppen zu ordnen und jedes Feature mit einem echten Screenshot aus einem passenden Open-Source-Projekt zu belegen (Abschnitt 8).

## 1. Ziele und Messgrößen

| Ziel | Messgröße | Quelle |
|---|---|---|
| Erstnutzung | Installationen und Release-Downloads | GitHub Releases API |
| Wiederkehrende Nutzung | Issues, Diskussionen, Release-Downloads je Version | GitHub |
| Einsatz in CI | Repos, die `action.yml` nutzen | GitHub Code Search |
| Agent-Nutzung | Erwähnungen des MCP-Servers, Einträge in MCP-Verzeichnissen | manuell |
| Sichtbarkeit | Stars, Referrer, Klicks auf „Get started“ | GitHub Traffic |

Website-Tracking schlage ich bewusst nicht vor. Das passt zum Datenschutzversprechen des Projekts (keine Secrets, kein Cluster-Zugriff, keine Telemetrie).

## 2. Zielgruppen und Botschaften

| Zielgruppe | Problem | Kernbotschaft | Einstieg |
|---|---|---|---|
| Software-Architekt:innen | Die Architektur driftet, die Doku veraltet | „arc42 aus dem Code statt aus dem Gedächtnis“ | arc42, Zielarchitektur, Qualitätsziele, Review-Export |
| Tech Leads / Entwickler:innen | Zyklen, Kopplung, „wer hängt an wem?“ | „Ohne Konfiguration sofort sehen, wie das System zusammenhängt“ | Desktop, World View, Matrix, Quellbelege |
| Platform / DevOps | Microservices, K8s und Terraform sind unübersichtlich | „Service-Topologie aus Compose, K8s, Helm und Terraform, ohne Cluster-Zugriff“ | Services, Datastores, Shared DB/Table |
| CI / Qualitätsverantwortliche | Architekturregeln werden nicht durchgesetzt | „Architektur als CI-Gate“ | `archguard check`, GitHub Action, Baseline, PR-Kommentar |
| Nutzer:innen von Coding-Agents | Der Agent versteht die Architektur nicht | „Dein Agent fragt dieselbe Analyse ab wie du“ | MCP-Server |
| Lehre (HSE) | Architektur ist abstrakt | „Architektur an echten OSS-Projekten zeigen“ | Beispiele: Kafka, eShop, OTel-Demo, PetClinic |
| Security / Compliance | Lizenzen und Abhängigkeiten | „SBOM offline, Lizenz-Policy“ | `sbom`, Lizenz-Policy, Advisories |

Positionierung in einem Satz: **„Evidenzbasierte Architekturanalyse, die sagt, was sie misst und was nicht.“** Das „Missing evidence is not a zero“ ist das Alleinstellungsmerkmal und sollte auf jeder Seite sichtbar sein.

## 3. Feature-Inventar: was die Website zeigt

✅ = auf der Website, ⚠️ = teilweise oder veraltet, ❌ = fehlt

| Feature-Gruppe | Features | Website |
|---|---|---|
| Abhängigkeiten | Paket- und Dateigraph, Zyklen (SCC), Schnittvorschläge, Tree Matrix (DSM), Layer-Checks | ✅ |
| World View | 3D-Landkarte mit Metrik-Linsen (Complexity, CBO, WMC, LCOM4), Drilldown Service → Paket → Quelle | ✅ |
| Klassenmetriken | CBO, RFC, DIT, NOC, WMC, LCOM4, Cognitive, Halstead, Maintainability | ⚠️ nur vier gezeigt |
| Architekturmetriken | Martin-A/I-Zonen, Propagation Cost, SDP-Verletzungen, Tangledness, Health Index, Layer Drift, Leaky Encapsulation | ❌ |
| Sprachen | Java, C#, TS/JS, Go, Rust, Python, PHP, Kotlin, C/C++, Swift, Ruby, Scala | ⚠️ Die Seite sagt „Nine languages“ und für Kotlin „Import graph only“. Beides ist veraltet. |
| Services | Compose, Kubernetes, Helm/Kustomize (Opt-in), Terraform (AWS/GCP/Azure) | ⚠️ nur Compose sichtbar |
| Datastores | Datastore-Knoten, Shared Database / Shared Server / Shared Table, Tabellen aus EF Core, JPA, TypeORM, SQLAlchemy und SQL-Migrationen | ❌ |
| API-Verträge | OpenAPI-Import, GraphQL aus den Quellen, Erkennung von Breaking Changes | ❌ |
| Runtime | OTel-Traces, N+1, chatty Endpoints, langsame Statements, Sampling-Profiler-Probes | ⚠️ nur OTel-Screenshots |
| Doku und Export | arc42 (12 Kapitel, DE/EN), Diagrammkatalog, AsciiDoc/Word (Confluence), Structurizr/C4, Markdown-Report mit Mermaid | ⚠️ nur arc42 |
| Governance | Regeln, Baseline, Git-Vergleich, Review-Entscheidungsregister mit ADR-Pflicht, Ownership (CODEOWNERS), PR-Kommentar | ⚠️ „CI“ nur kurz |
| Simulation | Refactorings virtuell durchspielen (cut/move/merge) | ❌ |
| Imports | Coverage, SARIF, JUnit, CPD und eingebauter Duplikat-Scanner, Mutation Testing | ❌ |
| Supply Chain | CycloneDX-SBOM offline, Lizenz-Policy, Advisories | ❌ |
| Hotspots | Git-Churn × Kopplung, Co-Change ohne Import-Kante | ❌ |
| Qualitätsziele | ISO-25010-Ziele, Abgleich mit Expertenbewertung (Recall/Precision) | ❌ |
| MCP | Über 40 Tools für Coding-Agents | ⚠️ nur Doku-Guide |
| Desktop | Nativ (Slint), DE/EN, System-Theme, Export | ✅ |
| In Planung | Domain View ([ADR 0022](adr/0022-domain-view.md)), UI-Flow- und Berechtigungs-Simulator ([ADR 0021](adr/0021-ui-flow-and-permission-simulator.md)) | ❌ (bewusst erst nach Release) |

## 4. Website-Maßnahmen

**Priorität 1: Korrektheit**

1. Sprachblock auf 12 Sprachen aktualisieren, Kotlin korrigieren, die Zahl an genau einer Stelle pflegen.
2. „Eleven guides“ und andere Zählungen gegen den echten Stand prüfen.

**Priorität 2: neue Sektionen auf der Startseite** (jeweils ein echter Screenshot aus Abschnitt 8 und eine Leitfrage, im Stil der bestehenden „Real questions“)

3. **„From Compose to Terraform“**: Deployment-Evidenz, Datastores, Shared Table. Leitfrage: „Teilen sich zwei Services heimlich eine Tabelle?“
4. **„Architecture as a CI gate“**: GitHub Action, Baseline, PR-Kommentar mit KPI-Deltas.
5. **„Your agent knows the architecture“**: MCP mit einem echten Dialogausschnitt.
6. **„Try before you refactor“**: Simulation mit Vorher/Nachher-Zyklen.
7. **„Docs that stay true“**: arc42, Word/Confluence, C4/Structurizr, Mermaid-Report.

**Priorität 3: Zielgruppen-Landingpages** (`/for/architects`, `/for/platform`, `/for/agents`, `/for/teaching`), jeweils mit Problem, drei Features, einem Beispiel und einem Kommando.

**Priorität 4: Vergleichsseite** „ArchGuard vs. …“ (z. B. Structure101, Sonargraph, jQAssistant, dependency-cruiser), fair und belegbar, mit ehrlichen Grenzen.

## 5. Kanäle

| Kanal | Inhalt | Rhythmus |
|---|---|---|
| GitHub (README, Releases, Topics) | Changelog als Feature-Story mit GIF | pro Release |
| MCP-Verzeichnisse und Awesome-Listen | Einträge für MCP, Architektur und Rust-Tools | einmalig, dann pflegen |
| Show HN, r/rust, r/softwarearchitecture, Lobsters | Launch „World View + MCP“ | einmal pro großem Meilenstein |
| Blog oder Fallstudien auf der Website | z. B. „Wir haben Kafka analysiert: 3.369 Dateien, so weit reichen die Zyklen“ | monatlich |
| Konferenzen und Meetups (OOP, JAX, Rust-Meetups, iSAQB) | Live-Demo mit OTel-Demo oder eShop | 2–3 pro Jahr |
| Hochschule (HSE) | Übungsblätter mit den Beispielprojekten | pro Semester |
| Mastodon und LinkedIn | 30-Sekunden-Clips aus `flow.mp4`, Metrik-Screenshots | wöchentlich |

## 6. Content-Plan (erste 12 Wochen)

| Woche | Thema | Format | Beispielprojekt |
|---|---|---|---|
| 1–2 | Website-Korrekturen und neue Sektionen | Website | – |
| 3 | Launch-Post „Architekturanalyse für Mensch und Agent“ | Blog, HN, Reddit | OTel-Demo |
| 4 | Fallstudie Kafka | Blog und Clip | Apache Kafka |
| 5 | Shared Tables finden | Blog | eShop, Sock Shop |
| 6 | MCP-Tutorial: Zyklus mit Claude Code reparieren | Video | Commons Collections |
| 7 | arc42 automatisch nach Confluence | Blog | eShop |
| 8 | Terraform, Helm und K8s ohne Cluster-Zugriff | Blog | Online Boutique |
| 9 | GitHub Action in 5 Minuten | Docs und Clip | Spring PetClinic |
| 10 | „Missing evidence is not a zero“: warum wir ehrlich messen | Essay | OTel-Demo (Polyglot-Lücken) |
| 11 | SBOM und Lizenz-Policy offline | Blog | Mastodon |
| 12 | Retro: Kennzahlen auswerten, Kanäle nachsteuern | intern | – |

## 7. Grundsätze

- Nur echte Captures aus der Desktop-App, keine gemalten Daten (wie bisher).
- Jede Behauptung mit Beispiel oder Kommando belegen und Grenzen offen nennen.
- Inhalte zweisprachig (DE/EN), wie die App.
- Lizenz und Commit jedes Drittprojekts auf der Seite [About these examples](../site/about.html) ausweisen.

## 8. Beispielprojekte für Screenshots

Bereits auf der Website: **Service Shop** (eigene Demo), **Apache Commons Collections**, **Microsoft eShop**, **Apache Kafka**, **OpenTelemetry Demo**. Die neuen Kandidaten unten sind nach Bekanntheit, passender Struktur und freier Lizenz ausgewählt.

**Vor jedem Capture prüfen:** Lizenz und aktuellen Repo-Inhalt (z. B. ob Helm, Terraform oder GraphQL noch enthalten sind), außerdem den Commit-Hash. Die Projekte werden frisch geklont und nicht aus anderen lokalen Projekten übernommen.

### 8.1 Pro Feature

| Feature | Projekt | Repo | Lizenz | Warum es passt / Screenshot-Idee |
|---|---|---|---|---|
| Zyklen, Schnittvorschläge, Simulation | Apache Commons Collections ✅ | apache/commons-collections | Apache-2.0 | vorhanden; neu: Vorher/Nachher mit `cut_edge` |
| Layer-Checks, CI-Gate, PR-Kommentar | Spring PetClinic | spring-projects/spring-petclinic | Apache-2.0 | klein, klassisch geschichtet; Regel verletzen, Check wird rot |
| Klassenmetriken (RFC, DIT, NOC, Halstead) | Apache Commons Collections ✅ | apache/commons-collections | Apache-2.0 | vorhandene World-Captures um die fehlenden Linsen ergänzen |
| Architekturmetriken (Zonen, Propagation Cost, SDP, Health Index) | Apache Kafka ✅ | apache/kafka | Apache-2.0 | groß genug für aussagekräftige KPIs; Java liefert Abstraktheit |
| Hotspots, Co-Change | Apache Kafka ✅ oder Django | apache/kafka, django/django | Apache-2.0 / BSD-3 | lange Git-Historie nötig |
| Compose-Services, Service-Zyklen | Service Shop ✅, eShop ✅ | – / dotnet/eShop | – / MIT | vorhanden |
| Kubernetes, Helm, Kustomize, Terraform | Online Boutique | GoogleCloudPlatform/microservices-demo | Apache-2.0 | K8s-Manifeste, Helm-Chart, Kustomize-Overlays und Terraform in einem Repo; 11 Services in 5 Sprachen |
| Datastores, Shared Database / Table | Sock Shop, eShop ✅ | microservices-demo/microservices-demo, dotnet/eShop | Apache-2.0 / MIT | Sock Shop: MongoDB und MySQL je Service; eShop: EF Core gegen PostgreSQL |
| JPA-Tabellen pro Service | Spring PetClinic Microservices | spring-petclinic/spring-petclinic-microservices | Apache-2.0 | Spring-Datasource und `@Entity` je Service |
| GraphQL-Verträge, Breaking Changes | Apollo Supergraph Demo (Federation 2) | apollographql/supergraph-demo-fed2 | MIT | mehrere Subgraphen mit Schemas und Operationen; Breaking Change per Baseline zeigen |
| OpenAPI-Import | Swagger Petstore | swagger-api/swagger-petstore | Apache-2.0 | Referenz-OpenAPI, sofort erkennbar |
| Runtime: OTel, gRPC | OpenTelemetry Demo ✅ | open-telemetry/opentelemetry-demo | Apache-2.0 | vorhanden |
| Runtime: N+1, langsame Statements | Spring PetClinic (JPA) | spring-projects/spring-petclinic | Apache-2.0 | eigene Aufzeichnung mit Probe nötig (ADR 0004); Owner-Liste mit Pets als N+1-Kandidat |
| arc42, C4/Structurizr, Word/Confluence | eShop ✅, Online Boutique | dotnet/eShop, GoogleCloudPlatform/microservices-demo | MIT / Apache-2.0 | Kontext- und Bausteinsicht mit echten Services |
| SBOM, Lizenz-Policy | Mastodon | mastodon/mastodon | AGPL-3.0 | Gems und npm in einem Repo, viele Lizenzen; Policy-Verstoß zeigt die AGPL-Thematik |
| Coverage-, JUnit- und Mutation-Import | Apache Commons Lang | apache/commons-lang | Apache-2.0 | Maven mit JaCoCo; PIT-Lauf lokal erzeugen |
| Duplikate (eingebauter Scanner) | Apache Kafka ✅ | apache/kafka | Apache-2.0 | große Codebasis mit realen Wiederholungen |
| Ownership (CODEOWNERS) | Django oder Kafka ✅ | django/django, apache/kafka | BSD-3 / Apache-2.0 | vor Capture prüfen, ob CODEOWNERS vorhanden; sonst Microsoft eShop |
| MCP-Dialog | Commons Collections ✅ | apache/commons-collections | Apache-2.0 | „Warum hängt `map` an `bag`?“ als echter Claude-Code-Verlauf |

### 8.2 Pro Sprache (Sprach-Galerie)

Je Sprache ein World-View-Screenshot mit Complexity-Linse und ein Ausschnitt aus der Matrix.

| Sprache | Projekt | Repo | Lizenz | Hinweis |
|---|---|---|---|---|
| Java | Apache Kafka ✅ | apache/kafka | Apache-2.0 | vorhanden |
| C# | Microsoft eShop ✅, Jellyfin | dotnet/eShop, jellyfin/jellyfin | MIT / GPL-2.0 | Jellyfin als große Monolith-Ergänzung |
| TypeScript | NestJS | nestjs/nest | MIT | Monorepo mit Paketen, gut für Drilldown |
| Go | Prometheus | prometheus/prometheus | Apache-2.0 | klare Paketstruktur |
| Rust | ripgrep | BurntSushi/ripgrep | MIT / Unlicense | Workspace mit mehreren Crates |
| Python | Django | django/django | BSD-3 | bekannte Paketzyklen |
| PHP | Symfony | symfony/symfony | MIT | viele Komponenten, Namespaces |
| Kotlin (+ Java) | OkHttp | square/okhttp | Apache-2.0 | Kotlin mit Java-Anteilen |
| C/C++ | LevelDB, Valkey | google/leveldb, valkey-io/valkey | BSD-3 | LevelDB mit CMake; Valkey als BSD-lizenzierter Redis-Fork (Redis selbst ist nicht mehr BSD) |
| Swift | SwiftNIO | apple/swift-nio | Apache-2.0 | viele SwiftPM-Targets |
| Ruby | Mastodon, Discourse | mastodon/mastodon, discourse/discourse | AGPL-3.0 / GPL-2.0 | Rails mit Zeitwerk; Mastodon dient auch für SBOM und Compose |
| Scala | Apache Pekko | apache/pekko | Apache-2.0 | Scala 2/3, viele Module |

### 8.3 Zuordnung zu den neuen Startseiten-Sektionen

| Sektion (Abschnitt 4) | Hauptbild | Zweitbild |
|---|---|---|
| From Compose to Terraform | Online Boutique: Service-Map mit Terraform/K8s-Evidenz | Sock Shop: Shared Database |
| Architecture as a CI gate | PetClinic: roter Layer-Check und PR-Kommentar | – |
| Your agent knows the architecture | Commons Collections: MCP-Dialog | – |
| Try before you refactor | Commons Collections: Simulation vorher/nachher | Kafka: Zyklusgröße |
| Docs that stay true | eShop: arc42-Kontextsicht | Online Boutique: Structurizr-Export |
| Sprach-Galerie | Abschnitt 8.2 | – |
