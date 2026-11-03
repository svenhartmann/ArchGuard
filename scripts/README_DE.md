# Entwicklungsskripte

Diese Werkzeuge unterstützen Entwicklung und Release-Prüfungen. Für die normale Nutzung von CLI und Viewer sind sie nicht erforderlich. Befehle werden aus dem Repository-Hauptverzeichnis gestartet.

| Verzeichnis | Zweck |
| --- | --- |
| `viewer/` | Node.js-Regressionstests für Viewer-Verhalten und erzeugtes HTML. |
| `browser/` | Interaktions- und Darstellungstests mit Chromium und Puppeteer. |
| `integration/` | Integrationsprüfungen für CLI, MCP, Sprachanalyse und Beispiele sowie gemeinsame Payload-Helfer. |
| `benchmarks/` | Viewer- und Kontext-Token-Messungen, Baseline-Erfassung, Testfälle und zugehörige Tests. |
| `checks/` | Prüfungen der Repository-Architektur samt Tests. |
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

Die Workflows in [`.github/workflows`](../.github/workflows/) definieren die CI-, Sicherheits- und Release-Aufrufe. Weitere Informationen stehen in der [Release-Dokumentation](../RELEASES.md) und der [Benchmark-Dokumentation](../examples/context-token-benchmark_DE.md).
