# ADR 0021: UI flow and permission simulator

- **Status:** accepted
- **Date:** 2026-11-19

## Context

ArchGuard shows architecture views read from code by fixed rules: layers,
dependencies, HTTP and GraphQL contracts, datastores, deployments. It has no
view of the user interface and none of permissions: `api_contracts` states
that authentication is not compared, and no crate reads routes, screens or
navigation.

We want a view that answers three questions without an AI summary:

- Which screens does an application have, and which click leads where?
- Which backend call does an action on a screen trigger?
- What can a given role reach, and where do frontend and backend disagree
  (a button the role sees but the backend refuses; an action the frontend
  hides but the backend does not protect)?

The view must work for web and desktop user interfaces and must not be tied
to one framework. From a screen in the view it should be possible to open
that screen in the locally running application.

Checked 2026-11-19 in `examples/checkouts/eshop`: one repository holds a
Blazor web app (13 `@page` components, `@attribute [Authorize]`,
`<AuthorizeView>`, `Nav.NavigateTo("user/orders")`), a .NET MAUI client
(`AppShell.xaml`, a navigation service) and ASP.NET backends
(`RequireAuthorization()`, `[Authorize]`). It covers all three sides.

## Decision

1. **One neutral model** `ui-flow/v1` in `archguard-core` (`ui_flow.rs`).
   Every framework maps onto it; nothing downstream knows a framework.
   - `Screen`: id, name, kind (`page`, `window`, `dialog`, `view`), optional
     route pattern (`/order/:id`), owning application, file, line, optional
     access condition.
   - `Transition`: source screen, target (a screen id, or `unresolved` with
     the source text), optional trigger (element id or label), file, line,
     optional condition.
   - `Action`: screen, optional trigger, the call it makes (HTTP method and
     path pattern, or a method id), file, line, optional condition.
   - `EndpointAccess`: HTTP method, path pattern, access condition, file,
     line. This is the backend side.
   - `Condition`: a boolean expression of `all`, `any`, `not` over atoms
     `authenticated`, `role(name)`, `policy(name)`, `custom(name)`, plus
     `unknown(text)` for anything the rules cannot read.
   - `UiFlow` carries the lists, `diagnostics` and `complete`.
2. **Access has three states**, never two: `protected` (a condition),
   `open` (declared anonymous, e.g. `[AllowAnonymous]`, `permitAll`) and
   `undeclared` (no declaration found). `undeclared` is never shown as
   allowed: a backend that checks by hand in code must not look open.
3. **Nothing is guessed.** A route, navigation target or role name that is
   not a literal becomes `unresolved`/`unknown`, adds a diagnostic with file
   and line and sets `complete: false`. The view reports "n of m navigations
   resolved". The rule follows ADR 0012.
4. **Two sources feed the model**, and they merge:
   - Built-in extractors per framework. First: Blazor (`@page`,
     `NavigateTo`, `NavLink`/`href`, `[Authorize]`, `<AuthorizeView>`),
     Angular (`Routes` arrays, `routerLink`, `router.navigate`,
     `canActivate`), and for the backend ASP.NET (`[Authorize]`,
     `[AllowAnonymous]`, `RequireAuthorization`) and Spring
     (`@PreAuthorize`, `@Secured`, `@RolesAllowed`).
   - An import file `.archguard/ui-flow.json` in the same schema. Any
     framework without an extractor, a build plugin or a recorder can write
     it. Each entry records its source (`extractor` name or `import`).
   A new framework is one more extractor module; the model, the simulator
   and the views stay unchanged.
5. **The simulator is a pure function** in `archguard-core`: a condition and
   an assignment of atoms (the chosen role sets `role(...)` and
   `authenticated`; every other atom is a switch the user sets) evaluate in
   three-valued logic to `allowed`, `denied` or `unknown`. Reachability from
   a start screen follows transitions whose condition is not `denied`.
   Joining an `Action` to an `EndpointAccess` by method and path pattern
   yields the mismatch findings named in the context.
6. **Conventions close the gaps, and `check` enforces them** (later phase,
   opt-in per project): navigation targets are statically readable;
   permission checks go through functions declared in the configuration;
   navigating elements carry a stable id (`data-testid`, `AutomationId`).
   An agent that is checked after each step keeps them.
7. **Jump to the running application** (later phase): for web, open the
   configured local base URL plus the screen's route. For everything else,
   replay the shortest transition path from the start screen through a
   driver (Playwright for web, the platform accessibility API for desktop).
   A failed replay is a finding: view and application disagree. The same
   replay serves as the runtime cross-check of the simulator's predictions.
   Test users per role come from local configuration; passwords from the
   keychain or the environment, never from a file.

### Phases

1. Model, three-valued evaluation, reachability, import file (core, engine).
2. In parallel: Blazor extractor, Angular extractor, backend access
   extractor (ASP.NET, Spring).
3. Join and mismatch findings; CLI and MCP output.
4. Desktop app: flow view with role selection and switches.
5. Web jump by URL; path replay with Playwright.
6. Desktop extractors (XAML family first: MAUI Shell, WPF, WinUI,
   Avalonia), then React/Next/Vue, QML, SwiftUI, Slint; desktop driver.
7. Convention checks in `check`; compiler-backed resolution of non-literal
   targets (TypeScript compiler API, Roslyn) behind the existing trust
   opt-in.

## Alternatives

- **One extractor per framework with its own output.** Rejected: simulator,
  views and checks would be written once per framework.
- **Record the flow only at run time** (crawler, instrumented app). Shows
  what happened, not what the code allows; needs a running app and test
  data for every state. Kept as cross-check (decision 7), not as source.
- **Let a model summarize the UI.** Rejected: the view must be reproducible
  and every element must point to a file and line.
- **Start with compiler-backed extraction.** More targets resolve, but it
  needs the trust opt-in and a toolchain per language. The literal patterns
  come first; phase 7 adds the compilers where the diagnostics show a need.

## Consequences

- The view is static evidence. A screen marked reachable can still fail at
  run time on data the simulator does not know; switches make that explicit.
- Hand-written checks in backend code stay `undeclared` until the project
  adopts the convention of decision 6.
- Path patterns join by shape (`/order/:id` equals `/order/{id}`); two
  routes of the same shape in different applications need the owning
  application to tell them apart.
- Each framework costs one extractor. Until it exists, the import file is
  the way in.

## Open points

- Which desktop technique and operating system come first for the driver
  (decision 7) is not decided.
- How far the TypeScript checker resolves route helper functions is not
  measured; phase 7 starts with that measurement.

## Verlauf

### 2026-11-19: model refinements after the first extractors

The Blazor, Angular and backend extractors reported gaps in the model. Since
`ui-flow/v1` is unreleased, the model changed instead of the extractors
working around it.

1. **Every screen as source.** `Transition.from` and `Action.screen` are an
   `Origin`: `{"screen": id}` or `{"any_screen": {"application": name}}` for
   layouts and menus (`""` for screens without an application). Reachability
   follows an `any_screen` transition from every reached screen of that
   application; the pseudo id `<app>:*` is gone.
2. **Literal path against route pattern.** A parameter segment of a route
   matches any single literal segment (`/orders/42` hits `/orders/:id`);
   pattern against pattern still compares shape. The most literal route wins
   (`/item/new` beats `/item/{id}`); a tie stays unresolved with a diagnostic.
3. **Unresolved action.** `Call::Unresolved(text)` keeps a call whose URL is
   not a literal as an action; the join skips it.
4. **Any HTTP method.** `ANY` (Spring `@RequestMapping` without `method`) is
   a wildcard in the join and matches every action method.
5. **False atom.** `Condition::Never` evaluates to `denied`; `@DenyAll` maps
   to it instead of `unknown`.
6. **Application on endpoints.** `EndpointAccess.application` is the project
   directory name (nearest `.csproj`, `pom.xml`, `build.gradle`). Routes rank
   within each application. When a hit belongs to the action's own
   application, only those hits stay; otherwise hits of all applications
   stay, since frontends call other services.
7. **Diagnostic severity.** `severity` is `gap` (an element is missing or
   unknown; the flow is not complete) or `note` (information, e.g. the
   `SecurityFilterChain` hint). A missing field reads as `gap`.

### 2026-11-19: every simulated screen says why it is not reached

On eShop, `ui-flow --anonymous` showed the open page `/item/{itemId:int}` as
"not reached": the catalog links to it only through helper-built URLs that
stay `Unresolved`. Readers took "not reached" as "denied". The simulation now
gives every screen one reach from `screen_reaches` in `archguard-core`, the
only place that decides it (CLI and MCP render it):

- `sure`, `possible`: as before.
- `denied`: the screen's own access evaluates `denied` for the assignment.
- `no_known_path`: not denied, but no resolved path from a `/` screen;
  `unresolved_transitions_in_application` counts the transitions of the
  screen's application whose target stayed unresolved, so the reader sees
  that the gap may be the extractor's, not the application's. A screen behind
  a denied transition condition also lands here.
- `no_start_screen`: the application has no screen routed `/`.

JSON: `{"reach": "no_known_path", "unresolved_transitions_in_application":
2}` merged into the screen row. CLI text: "reached surely", "reached
possibly", "denied (needs: signed in)", "no known path (2 links in this app
unresolved)", "no start screen". `not_reached` is gone. The MCP `ui_flow`
tool now reaches screens from every `/` screen, also with `application` set;
that parameter only narrows the rows. The findings are unchanged.

### 2026-11-19: conventions in `check` (decision 6, first part)

`[views] conventions` in the configuration switches the checks on, one name
per convention; absent means none, an unknown name is a configuration error.
For this ADR: `navigation_literal` (a transition target stayed
`unresolved`), `endpoint_access_declared` (an `undeclared` endpoint),
`http_url_literal` (a `Call::Unresolved` action) and `frontend_backend_agree`
(a `shown_but_refused` or `hidden_but_unprotected` finding). Each break is a
`target_architecture` finding with severity `error`, file:line, a fix hint
for the agent, and the id `view-<rule>:<file>:<subject>`, without the line.
An invalid import file is itself a finding, so the gate does not pass
silently. The baseline still accepts only cycles. Not yet done: permission
checks through configured functions and stable element ids (`data-testid`,
`AutomationId`); a finding kind of its own instead of `target_architecture`.

### 2026-11-19: start screens without `/`, catch-all routes

MAUI and desktop applications route no screen `/`, so eShop's `ClientApp`
was `no_start_screen` and none of its screens had a reach. `UiFlow` gains
`start_screens` (screen ids, serde default, left out of the JSON when empty;
merged as a sorted union). The start screens are those listed there plus
those routed `/` (`start_screen_ids` in `archguard-core`, the one place that
decides it); `screen_reaches`, the findings, CLI and MCP start there, and
`no_start_screen` now means "neither". A union rather than "marked, else
`/`": a web `/` stays an entry next to a marked one. The XAML extractor marks
one screen per application by a fixed rule: with a MAUI Shell, the page of
the first `ShellContent` of the first Shell file in document order;
otherwise the `StartupUri` of the `Application` XAML, else the first screen
class created with `new` in `OnStartup`, `OnLaunched`,
`OnFrameworkInitializationCompleted` or `CreateWindow`. Nothing found is a
`note` on the project folder. On eShop `ClientApp` starts at `LoginView`,
`HybridApp` at `MainPage` (`CreateWindow`). The flag lives on `UiFlow`, not on
`Screen`, so code that builds `Screen` literally keeps compiling.

A trailing `*` or `**` route segment (React `[...slug]` and `path="*"`,
Angular `**`, Spring `/**`) now matches one or more remaining segments
instead of being a literal. `best_route` and the action-endpoint join rank
any match without a catch-all above every catch-all match;
`path_match_score` still returns the equal literal segments. Consequence:
an Angular link to an unknown path resolves to the `**` route, as in the
running application, instead of staying unresolved.

### 2026-11-19: jump and replay (decision 7, phase 5, web)

`[views.run.<application>]` holds `base_url`, an optional `login_path` and
per role a test user `users.<role>` with `login` and exactly one of
`password_env` and `password_keychain`; a literal `password` is a
configuration error. `ui-flow open SCREEN` prints base URL plus route, route
parameters from `--param`, and opens it with the platform opener.
`ui-flow replay SCREEN` takes the shortest path of resolved transitions from
a `/` screen (breadth first, transitions whose condition or target access is
`denied` skipped) and writes a Playwright test: sign in by filling the first
password field and the user field (password from `process.env`; a keychain
password reaches the script through `ARCHGUARD_REPLAY_PASSWORD`, which the
CLI fills), then per step click the trigger by test id, else aria-label, else
text, and wait for the next route's URL. A step without a trigger navigates
directly and says so in the script; that is weaker evidence, and a direct step
to a route with parameters fails with the missing parameter. Only `--run`
executes `npx playwright test` and prints each step; a failed step is the
finding "view and application disagree at step n". The script is written to
the system temp directory unless `--out` says otherwise. Not yet done: the
desktop buttons "Open in browser" and "Replay path", the desktop driver, and
the generic sign-in fits forms with one password field only.

### 2026-11-19: baseline for conventions

Convention findings have their own kind `convention` (SARIF rule id
`archguard/convention/<rule>`) instead of `target_architecture`. The baseline
file gains an optional `conventions` list of finding ids, written by
`baseline write`; files without it still read, and the field is left out when
empty. `check --baseline` (CLI and MCP) drops accepted convention findings
from the list, fails on the others, and turns each baselined id without a
finding into an `info` note "remove it from the baseline", which does not
fail. The `view-conventions:…` finding for an unreadable import is never
recorded. `baseline diff` still compares cycles only; no ratchet beyond that.
