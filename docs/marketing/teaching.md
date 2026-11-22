# Teaching exercise / Lehrübung

**Duration / Dauer:** 60 minutes / Minuten. **Prerequisites / Voraussetzungen:** installed ArchGuard CLI, optional desktop app, a local ArchGuard repository checkout. No containers, hosted services or compiler trust opt-ins are needed for the base exercise. Learners work on the included Service Shop example; it is a teaching fixture, not a production system or an external endorsement.

## English: from a line to a defensible claim

Learning objective: distinguish an import edge, a configured HTTP relationship, a startup dependency and an observed runtime call; support one architecture claim with source evidence.

1. **0–10 minutes — Frame the scope.** Record the ArchGuard version and repository commit. Read `examples/service-shop/README.md`. Write down which parts of the shop are mocked and what the static analysis can establish.
2. **10–25 — Inspect evidence.** Run `archguard services examples/service-shop --format json`, optionally open `archguard-desktop examples/service-shop`, and choose one configured relationship. Record its direction, file and configuration key. Find a startup dependency and explain how its meaning differs.
3. **25–40 — Explain a cycle.** Follow either `catalog → pricing → promotions → catalog` or `orders → payments → risk → orders`. Check the current output rather than assuming all edges exist. Explain why a cycle in the service structure does not necessarily mean endless recursion in one request.
4. **40–50 — Propose a change.** Sketch one way to remove a return dependency. State the data that would cross the new boundary, its owner and a freshness/trust tradeoff. Do not edit code just to make the diagram green.
5. **50–60 — Peer review.** Exchange claims. The reviewer must reproduce the source edge and identify one thing the evidence does not establish.

Deliverable: one page with revision/tool version, selected source evidence, cycle explanation, proposed change and one explicit unknown. A screenshot can accompany it but does not replace source references.

### Instructor notes and assessment

The fixture README explains two candidate changes: pass product category into the pricing/promotions flow, or provide order/customer context to the risk check. Either introduces a contract and ownership decision. Reward alternatives with explicit tradeoffs; there is no single mandatory refactoring.

| Criterion | Points |
| --- | --- |
| Reproducible revision and scope | 2 |
| Correct direction and source evidence | 2 |
| Distinguishes configuration/startup/runtime evidence | 2 |
| Cycle explanation avoids equating it with recursion | 2 |
| Proposed boundary names a tradeoff and an unknown | 2 |

Extension: use the [pinned Commons Collections walkthrough](../../site/examples/commons-collections.html) to compare package cycles and the matrix. A fresh clone and upstream licence/notices are required. Compiler-backed metrics need the documented language tools; missing metrics are not zeroes.

## Deutsch: von einer Linie zur begründeten Aussage

Lernziel: Importkante, konfigurierte HTTP-Beziehung, Startabhängigkeit und beobachteten Laufzeitaufruf unterscheiden; eine Architekturaussage mit Quellbelegen stützen.

1. **0–10 Minuten — Scope klären.** ArchGuard-Version und Repository-Commit notieren. `examples/service-shop/README_DE.md` lesen. Welche Teile sind simuliert, welche Aussagen erlaubt statische Analyse?
2. **10–25 — Belege prüfen.** `archguard services examples/service-shop --format json` ausführen, optional `archguard-desktop examples/service-shop` öffnen und eine konfigurierte Beziehung auswählen. Richtung, Datei und Konfigurationsschlüssel festhalten. Eine Startabhängigkeit finden und den Unterschied erklären.
3. **25–40 — Zyklus erklären.** Entweder `catalog → pricing → promotions → catalog` oder `orders → payments → risk → orders` verfolgen. Den aktuellen Output prüfen, statt Kanten vorauszusetzen. Warum bedeutet ein Strukturzyklus nicht zwangsläufig eine Endlosrekursion pro Anfrage?
4. **40–50 — Änderung vorschlagen.** Einen Weg zum Entfernen einer Rückabhängigkeit skizzieren. Welche Daten überschreiten die neue Grenze, wem gehören sie und welche Frage zu Aktualität oder Vertrauen entsteht? Nicht nur für ein grünes Diagramm Code ändern.
5. **50–60 — Gegenseitiges Review.** Die Partnerperson reproduziert den Quellbeleg und benennt eine Aussage, die aus den vorhandenen Daten nicht folgt.

Abgabe: eine Seite mit Commit/Werkzeugversion, Quellbeleg, Zykluserklärung, Änderungsvorschlag und einem ausdrücklich unbekannten Punkt. Ein Screenshot ergänzt die Quellstellen, ersetzt sie aber nicht.

### Hinweise und Bewertung

Die Beispiel-Dokumentation nennt zwei mögliche Änderungen: Produktkategorie im Pricing-/Promotions-Ablauf mitgeben oder Bestell-/Kundenkontext an die Risikoprüfung übergeben. Beide erfordern Entscheidungen zu Vertrag und Verantwortung. Alternative Lösungen mit klaren Abwägungen sind gleichwertig.

Je zwei Punkte für reproduzierbaren Scope, richtige Richtung/Quelle, korrekte Trennung der Evidenzarten, Zykluserklärung ohne Rekursionsfehlschluss sowie einen Vorschlag mit Abwägung und offener Frage.

Erweiterung: Mit dem [gepinnten Commons-Collections-Rundgang](../../site/examples/commons-collections.html) Paketzyklen und Matrix vergleichen. Frischer Klon, Lizenz und Notices gehören dazu. Compiler-Metriken benötigen die dokumentierten Werkzeuge; fehlende Werte bleiben unbekannt.
