# Entwicklungsskripte

Diese Werkzeuge unterstützen Entwicklung und Release-Prüfungen. Für die normale Nutzung von CLI und Viewer sind sie nicht erforderlich. Befehle werden aus dem Repository-Hauptverzeichnis gestartet.

| Verzeichnis | Zweck |
| --- | --- |
| `viewer/` | Node.js-Regressionstests für Viewer-Verhalten und erzeugtes HTML. |
| `browser/` | Interaktions- und Darstellungstests mit Chromium und Puppeteer. |
| `integration/` | Integrationsprüfungen für CLI, MCP, Sprachanalyse und Beispiele sowie gemeinsame Payload-Helfer. |
| `benchmarks/` | Viewer- und Kontext-Token-Messungen, Baseline-Erfassung, Testfälle und zugehörige Tests. |
| `checks/` | Prüfungen der Repository-Architektur samt Tests. |
| `hooks/` | Git-Pre-Commit-Hook, der `archguard check` und eine Kompaktheits-Ratsche für Methoden und Klassen auf den gestagten Dateien ausführt, samt Test. |
| `release/` | Paketierung, Verifikation, Smoke-Tests, Malware-Scans und Berichte samt Tests. |

## Häufige Befehle

```sh
python3 scripts/checks/check_architecture.py
node scripts/viewer/test_viewer_world.js
python3 scripts/integration/smoke_cli.py target/debug/archguard --db
```

Python-Unit-Tests getrennt nach Verzeichnis starten, damit jede Suite ihre benachbarten Hilfsmodule importieren kann:

```sh
for suite in checks release benchmarks; do
  python3 -m unittest discover -s "scripts/$suite" -p 'test_*.py' || exit 1
done
python3 -m unittest discover -s scripts/integration -p test_viewer_payload.py
python3 -m unittest discover -s scripts/integration -p test_jellyfin_example_import.py
```

Sprach- und Live-Integrationstests benötigen teilweise eine gebaute CLI, Sprach-SDKs oder einen laufenden Viewer. Browsertests erhalten Bericht/URL, Browser, Puppeteer und Ausgabeziele als Argumente; die Aufrufbeschreibung steht am Anfang des jeweiligen Skripts. Nicht alle Skripte pauschal ausführen: Einige erfassen Baselines, bauen Archive oder benötigen externe Werkzeuge.

## Pre-Commit-Hook

`scripts/hooks/pre-commit` führt das CI-Gate `archguard check` auf den gestagten Dateien aus: Zyklen, Schichtregeln und die `[review]`-Obergrenzen aus [`.archguard/archguard.toml`](../.archguard/archguard.toml). Nicht gestagte Änderungen bleiben unberücksichtigt. Verwendet wird `$ARCHGUARD_BIN`, sonst `target/release`, `target/debug`, zuletzt `archguard` im `PATH`.

Danach läuft [`hooks/compact_metrics.py`](hooks/compact_metrics.py) und vergleicht Methoden- und Klassenmetriken von `HEAD` und dem gestagten Stand: Zeilen, zyklomatische und kognitive Komplexität, Verschachtelung, Parameter und Methoden pro Klasse. Neue Methoden und Klassen müssen unter den Grenzwerten oben in diesem Skript bleiben; bestehende Ausreißer dürfen bleiben oder schrumpfen, aber nicht wachsen.

```sh
git config core.hooksPath scripts/hooks
python3 scripts/hooks/test_pre_commit.py target/debug/archguard
```

`git commit --no-verify` überspringt den Hook.

Die Workflows in [`.github/workflows`](../.github/workflows/) definieren die CI-, Sicherheits- und Release-Aufrufe. Weitere Informationen stehen in der [Release-Dokumentation](../RELEASES.md) und der [Benchmark-Dokumentation](../examples/context-token-benchmark_DE.md).
