# GitHub Pages site

A static English and German introduction and handbook for ArchGuard with actual desktop app screenshots,
reproducible examples, practical guides and CLI/MCP reference. No npm
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
to show repository links and release installation commands. Without it, the
build omits repository-specific links and commands.

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

The gallery screenshots come from the ArchGuard desktop app. `media/capture-app.sh`
recreates the 18 gallery views at 2960 × 1840 pixels with English labels. It needs
the pinned checkouts from the walkthroughs under `examples/` (Commons Collections
and Kafka with their example configuration copied into the checkout), and an app
built with the snapshot feature; without it a real window opens. The additional
Screens, Domain and Security captures have their own record in
`assets/screenshot-provenance.json`; Domain uses 4400 × 1840 pixels.

```sh
cargo build --release -p archguard-desktop --features snapshot
site/media/capture-app.sh target/release/archguard-desktop
```

The script lists what each image shows. Review the images before committing: keep
partial-evidence indicators visible, check that the values named in `index.html`
and `site.js` still match, and update `about.html` with the capture date and scope.

The source checkout and any generated analysis outputs are intentionally absent
from the published site. Upstream projects keep their own licenses and notices.

## Verification

`check.py` validates local destinations, fragment IDs, portable asset paths,
image alt text and dimensions. Browser checks should additionally cover mobile
layout, all buttons of both galleries (metrics and views), full-resolution screenshot links and copying
commands, including the selected-text fallback when clipboard access is unavailable.

## Documentation guides

`docs/index.html` and `de/docs/index.html` are the handbook hubs. The guides cover
installation and a first analysis, desktop use, UI flows, permissions, domain models,
simulation, architecture exports, runtime evidence, quality, CLI/MCP and CI.

`data/docs-navigation.json` defines the shared chapter groups and previous/next order.
Add a page in both locales and register its slug there. The build rejects missing
or unregistered chapters. `docs_build.py` renders a common responsive shell and
derives each page's table of contents from its H2 headings. Existing explicit
fragment IDs are preserved, including the former development overview's links.

After resolving locale and repository placeholders, the build indexes article
sections in each locale's `docs/search-index.json`. `docs.js` searches headings,
prose and command examples locally; it does not send queries to a service.
Results link to the matching section. The sidebar and article content remain
usable without JavaScript or if the search index cannot be fetched.

Maintain these pages alongside the CLI and desktop app. Verify commands against
`archguard --help` and each command's `--help`; verify configuration examples
against the engine schema. Keep missing measurements and inferred connections
explicit rather than promising complete analysis. Never include machine-specific
paths or invented release URLs.

The existing Pages workflow validates documentation links and fragments too.
Before publishing, check the documentation at desktop and narrow mobile widths,
search article text (including no results, reset and network failure), open an
image overlay, and copy a command. Source verification is not a claim that all
documented commands are present in a published release: `development.html#status`
records the inspected source snapshots, including the separate load branch.

## Marketing content and localisation

`data/languages.json` is the sole language inventory for the home page and language/prerequisite tables. The build derives the language-group count and the English guide count. TypeScript/JavaScript and C/C++ each count as one group. Update both locale fields when changing an adapter description; verify against the current README and CLI, not an older installed binary.

New marketing journeys live in `for/`, `compare/`, `stories/` and `languages/`, with their German counterparts in `de/`. Both documentation locales have their own chapters and search index. All navigation uses relative paths for GitHub project Pages. The build adds the evidence motto to pages whose footer does not already state it.

`python3 site/test_build.py` verifies build-time inventory/counts, locale output, repository-link handling, documentation navigation/search targets and directory-index fragment validation. Run it alongside `site/check.py` on a fresh build, both with and without `--repository`. Browser QA covers desktop/mobile, language switching, search, galleries, copy buttons and reduced-motion behaviour.

Campaign drafts and the measurement workflow are in `docs/marketing/` at repository root; the static build deliberately excludes them. They are preparation for the twelve-week rollout, not a record of posts sent or metrics observed.
