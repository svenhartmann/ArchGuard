# GitHub Pages site

A static English introduction to ArchGuard with actual viewer screenshots, three
reproducible examples, language prerequisites and CLI/MCP entry points. No npm
packages, external fonts, analytics or third-party runtime scripts are needed.

## Preview locally

From the repository root, choose a fresh output directory:

```sh
python3 site/build.py --out /tmp/archguard-pages-preview
python3 site/check.py /tmp/archguard-pages-preview
python3 site/preview.py --port 8870 --directory /tmp/archguard-pages-preview
```

Open http://127.0.0.1:8870/. The build refuses to overwrite a nonempty output
folder; use a new path for subsequent previews. Add `--repository owner/name`
to show GitHub links in every page header and on the home page. Without it, the
build removes those links.

## Publish later

1. Set the intended GitHub repository as the remote and push the reviewed changes.
2. In that repository, set **Settings → Pages → Build and deployment → Source**
   to **GitHub Actions**.
3. Run the **GitHub Pages** workflow on `main`, or push a change to `site/`.
4. The deployment job publishes its URL in the `github-pages` environment.

The workflow derives the source link from `GITHUB_REPOSITORY`; no repository name
is hard-coded. All asset paths are relative, so project sites and custom domains
work without rebuilding URLs. Pull requests build and validate but never deploy.
The artifact contains only public site assets; it does not copy repository files,
local reports, source checkouts, absolute machine paths or these maintenance scripts.

Workflow reference: [GitHub's custom Pages workflow documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).
Actions are pinned to verified commit IDs.

## Update screenshots

All gallery images are real captures at **1440 × 960** with the viewer set to
English. `media/capture-views.cjs` captures them reproducibly. It needs an
installed Chromium/Chrome (`CHROME`), Puppeteer Core (`PUPPETEER_MODULE`) and
running viewers for Commons Collections and Microsoft eShop at the pinned
revisions from the walkthroughs under `examples/`:

```sh
archguard serve examples/checkouts/commons-collections --port 8816 --depth 4 \
  --config examples/commons-collections.toml
archguard serve examples/checkouts/eshop --port 8818
node site/media/capture-views.cjs http://127.0.0.1:8816 http://127.0.0.1:8818 site/assets
```

- `assets/world.png` (hero): Commons Collections, Start tab, Problems lens.
- `assets/world-complexity.png`, `world-cbo.png`, `world-wmc.png`, `world-lcom4.png`:
  Start tab, Explore lens with that metric selected.
- `assets/world-package.png`: Explore lens, Complexity, inside the `map` package.
- `assets/graph.png`, `matrix.png`, `calls.png`, `quality.png`: Commons Collections,
  Graph, Matrix, Calls and Quality tabs.
- `assets/eshop-world.png`: eShop, Start tab, deployment
  `aspire:src/eShop.AppHost/AppHost.cs`, inferred event flow, Cycles.
- `assets/eshop-services.png`: eShop, Services tab, inferred event flow, Cycles only,
  details of the first event connection with its source evidence.
- `assets/services.png` (Service Shop example page only): Services tab, API
  configuration relations; captured manually.

The script fails when the eShop cycle or its source evidence is missing. Review
the images before committing: keep partial-evidence indicators visible and update
`about.html` with the capture date and scope.

## Update the flow video

`assets/flow.mp4` is a silent, roughly 30-second recording of one path through
Commons Collections: World View, Explore lens with CBO, the `map` package, its
dependencies in the graph, Calls, Quality and the export menu. A drawn pointer
marks each step because headless capture has no system cursor. With the
Commons Collections viewer from above running and `ffmpeg` on `PATH`:

```sh
node site/media/record-flow.cjs http://127.0.0.1:8816 /tmp/flow.webm
ffmpeg -i /tmp/flow.webm -an -vf fps=30 -c:v libx264 -preset slow -crf 30 \
  -pix_fmt yuv420p -movflags +faststart site/assets/flow.mp4
ffmpeg -ss 0.5 -i site/assets/flow.mp4 -frames:v 1 -q:v 3 site/assets/flow-poster.jpg
```

The page loads the video only on demand, plays it while it is visible and
leaves it paused for visitors who ask for reduced motion.

The source checkout and any generated analysis outputs are intentionally absent
from the published site. Upstream projects keep their own licenses and notices.

## Verification

`check.py` validates local destinations, fragment IDs, portable asset paths,
image alt text and dimensions. Browser checks should additionally cover mobile
layout, all buttons of both galleries (metrics and views), full-resolution screenshot links and copying
commands, including the selected-text fallback when clipboard access is unavailable.

## Documentation guides

`docs/index.html` is the documentation hub, with a client-side topic filter.
Eleven English guides cover installation, tools and prerequisites per language, the viewer, CLI commands, configuration,
product purpose and scope, language prerequisites and metrics, service evidence, MCP prompts, CI and
troubleshooting. The sidebar and all guides work without JavaScript.

Maintain these pages alongside the CLI and viewer. Verify commands against
`archguard --help` and each command's `--help`; verify configuration examples
against the engine schema. Keep missing measurements and inferred connections
explicit rather than promising complete analysis. Never include machine-specific
paths or invented release URLs.

The existing Pages workflow validates documentation links and fragments too.
Before publishing, check the documentation at desktop and narrow mobile widths,
filter topics (including no results and reset), and copy a command.
