# Development scripts

These tools support development and release verification. Normal CLI and viewer use does not require running them. Run commands from the repository root.

| Directory | Purpose |
| --- | --- |
| `viewer/` | Node.js regression tests for viewer behavior and generated HTML. |
| `browser/` | Browser interaction and rendering tests; require Chromium and Puppeteer. |
| `integration/` | CLI, MCP, language analysis and example integration tests; includes shared payload helpers. |
| `benchmarks/` | Viewer and context-token measurements, baseline capture, cases and their tests. |
| `checks/` | Repository architecture checks and their tests. |
| `release/` | Release packaging, verification, smoke tests, malware scans and report generation, with their tests. |

## Common commands

```sh
python3 scripts/checks/check_architecture.py
node scripts/viewer/test_viewer_world.js
python3 scripts/integration/smoke_cli.py target/debug/archguard --db
```

Run Python unit tests separately by directory so each suite can import its adjacent helpers:

```sh
for suite in checks release benchmarks; do
  python3 -m unittest discover -s "scripts/$suite" -p 'test_*.py' || exit 1
done
python3 -m unittest discover -s scripts/integration -p test_viewer_payload.py
python3 -m unittest discover -s scripts/integration -p test_jellyfin_example_import.py
```

Language and live integration tests can require a built CLI, language SDKs or a running viewer. Browser tests take report/URL, browser, Puppeteer and output arguments; see each script's usage header. Do not run every script blindly: some capture baselines, build archives or need external tools.

The workflows in [`.github/workflows`](../.github/workflows/) define the CI, security and release commands. See [release documentation](../RELEASES.md) for packaging and scans, and [benchmark documentation](../examples/context-token-benchmark.md) for context measurements.
