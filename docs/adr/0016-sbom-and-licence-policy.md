# ADR 0016: SBOM and licence policy from local lockfiles

- **Status:** accepted
- **Date:** 2026-11-18

## Context

Teams want to know which third-party packages each project or service
ships and whether their licences are acceptable, as a CI gate next to the
architecture rules. ArchGuard already counted external dependencies for
the review KPIs (`dep_inventory.rs`: `Cargo.lock`, `package-lock.json`,
`requirements*.txt`, `pyproject.toml`, `composer.json`, `pom.xml`) and runs
Trivy explicitly for advisories (`archguard security`). Neither produced a
standard SBOM, and neither knew licences.

Constraints: ArchGuard stays offline and does not guess. Most lockfiles
do not state licences (`Cargo.lock`, `yarn.lock`, `pnpm-lock.yaml`,
`go.mod`, `poetry.lock`, `uv.lock`, `packages.lock.json`, `gradle.lockfile`);
`package-lock.json` (v2/v3 `license`) and `composer.lock` (`license` array)
do.

## Decision

1. **One parse for inventory and SBOM.** The lockfile parsers live in
   `archguard-engine/src/lockfiles.rs` and return name, version, stated
   licences, scope and a first-party flag. `dep_inventory.rs` finds the
   files and keeps counting for the KPI; `sbom.rs` builds the SBOM from the
   same result. New formats: `yarn.lock` (v1 and berry), `pnpm-lock.yaml`
   (v5/v6/v9), `packages.lock.json`, `*.csproj` `PackageReference`,
   `gradle.lockfile`, `go.mod`, `poetry.lock`, `uv.lock`, `composer.lock`.
   A lockfile supersedes its manifest in the same directory
   (`composer.json`, `pyproject.toml`/`requirements*.txt`, `*.csproj`).
   `go.sum` is not read: it also lists versions outside the build list,
   which `go.mod` (Go ≥ 1.17) holds completely.
2. **Third-party only.** Cargo packages without `source`, npm `link`
   entries, yarn `workspace:`/`link:`/`portal:`/`file:`, uv
   editable/virtual/directory/path sources, NuGet `Project` entries and
   composer `path` dists are first-party and not components. The search
   skips `fixtures/` (test data) and `.claude/` (agent worktrees) like
   build output and dependency caches.
3. **Scope per project/service.** Each component carries the
   project-relative manifests that state it (CycloneDX property
   `archguard:manifest`). Components are deduplicated by purl. CycloneDX
   `scope` is set only where the file says: npm `dev`/`devOptional`,
   composer `packages-dev`, poetry `category = "dev"`, Maven `test` scope,
   pnpm v5/v6 `dev`, Gradle test-only classpaths → `optional`; the
   others of these files → `required`; otherwise absent.
4. **Licences only from the files.** Several stated licences (composer)
   are joined with `OR`, as composer means them. No registry, cache or
   `node_modules` is read; anything not stated is `unknown`. In CycloneDX a
   licence that parses as an SPDX expression is written as `expression`,
   other text as `license.name` (never as `license.id`, which the schema
   checks against the SPDX list ArchGuard does not ship).
5. **Output.** `archguard sbom [PATH] --format cyclonedx|json|text`.
   CycloneDX 1.6 JSON without serial number and timestamp, so the output
   is deterministic. Components have `type`, `bom-ref` (= purl), `name`
   (`group` for Maven), `version`, `purl`, `scope`, `licenses`,
   `properties`. `sbom` itself never fails on licences.
6. **Policy** in `[licenses]` (core `LicensesConfig`):
   `allow`/`deny` lists of SPDX ids (case-insensitive) and
   `unknown = "allow" | "deny"` (default `allow`). The lists compare exact
   ids, so deprecated forms a lockfile may state (`GPL-3.0`, `GPL-2.0+`)
   must be listed too. States: `denied`,
   `not_allowed` (allow list set, licence not on it), `unlisted` (no list
   matches, no allow list), `allowed`, `unknown`. Expressions are
   evaluated: `OR` takes the best branch, `AND` the worst, `WITH` counts as
   its licence. Text that is no expression is one opaque name that no list
   matches. Violations are `denied`, `not_allowed`, and `unknown` only with
   `unknown = "deny"`.
7. **Gate.** `archguard check` adds one error finding of the new kind
   `license` per violating component (id `license:<purl>`, evidence =
   manifests), so text, JSON and SARIF (`archguard/license`) carry it and
   the exit code is 1. Like the `[review]` KPI gates it speaks about the
   current state and is also evaluated with `--base`. An unconfigured
   `[licenses]` section reads no lockfile in `check`.
8. **MCP.** Tool `sbom`: ecosystem and licence-state counts, violation
   count, and 4 KiB pages of components filtered by `license_state`,
   `violations_only` or a name/purl query.

## Alternatives

- **Trivy or Syft for the SBOM.** Trivy is already used for advisories and
  can emit CycloneDX, but it is an external program and its licence data
  for some ecosystems comes from its database or installed packages.
  Rejected for the default path: the SBOM must work without extra tools
  and only from repository files.
- **Look licences up in local caches** (`~/.cargo/registry`,
  `node_modules`, `~/.m2`). These are present on some machines and not on
  CI, so the same commit would give different results. Rejected; a later
  explicit import (like `.archguard/dep-meta.json`) remains possible.
- **Ship the SPDX licence list** to validate ids and write `license.id`.
  Rejected for now: an extra data file to keep current; expressions are
  valid CycloneDX without it.
- **Unknown as violation by default.** Rejected: with most lockfiles
  stating no licence, the gate would fail nearly every project.

## Consequences

- Rust, Go, .NET, Python and Gradle projects get mostly `unknown`
  licences. The SBOM is still useful as an inventory, and `unknown`
  stays visible as its own count instead of passing silently.
- The review KPI `dependencies` now also counts the new lockfile formats
  and no longer counts packages in `fixtures/` or `.claude/`.
- Manifests without a lockfile give components without version; their
  purl has no version.
- `FindingKind` has a new variant `license`; every exhaustive match
  (text, SARIF, MCP, store, evaluation, goal mapping) names it. Licence
  findings map to maintainability, but they exist only in `check` output,
  not in the analysis report.
- The CycloneDX output of the fixture `fixtures/sbom-polyglot`, of this
  repository and of a sample with non-SPDX licence text was validated
  against the official `bom-1.6.schema.json` (with `spdx.schema.json`
  and `jsf-0.82.schema.json`) on 2026-11-18. There is no schema test in
  the build; the schema is not vendored.

## History

- **2026-11-18, review fixes.** First-party also covers poetry sources of
  `type = "directory"`/`"file"`. npm aliases (`package-lock.json` entry
  `name`, yarn `alias@npm:real@range`) are the real package. pypi purls
  only lowercase and replace `_` (`zope.interface` keeps its dot). A
  leading byte order mark in a lockfile is ignored. The manifest search
  goes 12 directory levels deep (was 4) and does not follow symlinked
  directories; when `[licenses]` is active and no lockfile was found or
  the search hit that bound, `check` adds a `license-coverage` warning
  instead of passing silently. MCP `sbom` always returns at least one
  component; one whose manifest list does not fit the page keeps the first
  manifests and states `manifests_total`.
