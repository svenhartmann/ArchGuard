# Measurement without website tracking / Messung ohne Website-Tracking

Updated / Stand: 2026-11-20. No analytics script, tracking pixel or new data collection is added to the website.

## What the numbers mean / Aussage der Zahlen

| Signal | Interpret as / Bedeutung | Do not infer / Nicht ableiten |
| --- | --- | --- |
| Release asset downloads | Downloads per asset and release / Abrufe je Asset und Version | Installations, unique people or active users / Installationen, Personen oder aktive Nutzung |
| Stars | Interest at a point in time / Interesse zu einem Zeitpunkt | Retention or successful adoption / Wiederkehrende oder erfolgreiche Nutzung |
| Issues and discussions | Observable questions and feedback / Sichtbare Fragen und Rückmeldungen | All users or satisfaction / Alle Nutzer oder Zufriedenheit |
| Public workflows using the Action | A manually verified lower bound / Manuell geprüfte Untergrenze | Private CI usage or active execution / Private Nutzung oder tatsächlich laufende Jobs |
| GitHub repository traffic | Repository visits, clones and available referrers / Repository-Besuche, Klone und verfügbare Referrer | Website “Get started” clicks / Klicks auf der Website |
| MCP mentions/listings | Discovery evidence / Hinweise auf Auffindbarkeit | MCP installations or sessions / MCP-Installationen oder Sitzungen |

**Correction to the initial plan:** GitHub repository traffic does not measure clicks on the website's “Get started” button. Keep that metric “not measured” unless a separate, explicitly chosen measurement mechanism is introduced. No such mechanism is proposed here.

**Korrektur am Ausgangsplan:** GitHub-Repository-Traffic misst keine Klicks auf „Get started“ auf der Website. Diese Kennzahl bleibt „nicht gemessen“. Es wird hier kein zusätzlicher Tracking-Mechanismus eingeführt.

## Weekly capture / Wöchentliche Erfassung

Run manually with an authenticated GitHub CLI account that can read repository traffic. These commands read GitHub and write local JSON files. Choose a fresh directory for each UTC capture time; keep private traffic snapshots outside public site assets.

Manuell mit einem angemeldeten GitHub-CLI-Konto ausführen, das Repository-Traffic lesen darf. Die Befehle lesen GitHub und schreiben lokale JSON-Dateien. Pro UTC-Aufnahmezeit einen neuen Ordner verwenden; private Traffic-Daten nicht in Website-Assets ablegen.

```sh
mkdir marketing-snapshot-YYYY-MM-DD
cd marketing-snapshot-YYYY-MM-DD
gh api repos/svenhartmann/ArchGuard > repository.json
gh api --paginate --slurp repos/svenhartmann/ArchGuard/releases > releases-pages.json
gh api repos/svenhartmann/ArchGuard/traffic/views > views.json
gh api repos/svenhartmann/ArchGuard/traffic/clones > clones.json
gh api repos/svenhartmann/ArchGuard/traffic/popular/referrers > referrers.json
```

Replace `YYYY-MM-DD` with the actual date. Record capture timestamp, command failures and the published content URLs in a neighbouring note. A denied or unavailable response is missing data, never a zero. GitHub's releases API exposes asset `download_count`; report it per asset rather than summing binary archives, checksums and desktop packages as if they were people. [GitHub release API](https://docs.github.com/en/rest/releases/releases).

Datum ersetzen und Aufnahmezeit, Befehlsfehler sowie veröffentlichte Inhalts-URLs daneben notieren. Verweigerte oder fehlende Antworten sind keine Null. `download_count` steht je Release-Asset zur Verfügung; Binärarchive, Prüfsummen und Desktop-Pakete getrennt auswerten. [GitHub Release-API](https://docs.github.com/en/rest/releases/releases).

Traffic covers a rolling 14-day window; capture weekly and deduplicate daily timestamps when combining snapshots. Do not sum overlapping windows or daily unique counts into “monthly unique users”. GitHub's traffic data describes the repository and uses UTC timestamps. [Traffic API](https://docs.github.com/en/rest/metrics/traffic), [traffic guide](https://docs.github.com/en/repositories/viewing-activity-and-data-for-your-repository/viewing-traffic-to-a-repository).

Traffic umfasst ein rollierendes 14-Tage-Fenster. Wöchentlich sichern und Tageswerte anhand ihrer UTC-Zeitstempel deduplizieren. Überlappende Fenster oder tägliche Unique-Zahlen nicht zu „monatlichen Nutzern“ addieren. [Traffic-API](https://docs.github.com/en/rest/metrics/traffic), [Anleitung](https://docs.github.com/en/repositories/viewing-activity-and-data-for-your-repository/viewing-traffic-to-a-repository).

## Manual adoption log / Manuelles Nutzungsprotokoll

Search GitHub code for `"svenhartmann/ArchGuard" path:.github/workflows`, then inspect each result. Record repository, workflow path, pinned action ref, observation date and whether the configuration actually invokes ArchGuard. Deduplicate repositories; separate this project's tests and forks. Search visibility and indexing limit coverage.

In GitHub Code Search nach `"svenhartmann/ArchGuard" path:.github/workflows` suchen und Treffer einzeln prüfen. Repository, Workflow-Pfad, Action-Ref und Datum notieren. Repositories deduplizieren; eigene Tests und Forks getrennt halten. Sichtbarkeit und Indexierung begrenzen die Aussage.

For MCP, log the exact listing or mention URL, discovery date and relevance. Do not collect user-level telemetry. For issues/discussions, record themes such as installation friction, missing language evidence or useful workflows; avoid labelling criticism as a negative user.

Für MCP die genaue Fundstelle, Datum und Relevanz festhalten. Keine personenbezogene Nutzungs-Telemetrie sammeln. Issues und Diskussionen nach Themen zusammenfassen, etwa Installationshürden, fehlende Sprachbelege oder nützliche Abläufe.

## Week-12 decision / Entscheidung in Woche 12

For each channel, record: work invested, published items, attributable feedback, known measurement gaps and one action (continue, change format, pause). Compare similar post-release windows. If a metric changes after a post, describe the timing rather than claiming causation.

Je Kanal Aufwand, veröffentlichte Inhalte, zuordenbares Feedback, Messlücken und eine Entscheidung notieren: fortführen, Format ändern oder pausieren. Vergleichbare Zeiträume nach Releases betrachten. Eine zeitliche Veränderung nach einem Beitrag beweist keine Ursache.

| Channel / Kanal | Evidence / Beleg | Effort / Aufwand | Gaps / Lücken | Next action / Nächster Schritt |
| --- | --- | --- | --- | --- |
| GitHub | To record / Zu erfassen | To record / Zu erfassen | To record / Zu erfassen | Decide in week 12 / In Woche 12 entscheiden |
| Community posts | To record / Zu erfassen | To record / Zu erfassen | To record / Zu erfassen | Decide in week 12 / In Woche 12 entscheiden |
| Social | To record / Zu erfassen | To record / Zu erfassen | To record / Zu erfassen | Decide in week 12 / In Woche 12 entscheiden |
| Teaching / meetups | To record / Zu erfassen | To record / Zu erfassen | To record / Zu erfassen | Decide in week 12 / In Woche 12 entscheiden |
