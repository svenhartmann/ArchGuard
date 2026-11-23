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

### 2026-11-20: Blazor reads query-only links, one-level helpers and literal fields

Measured on eShop, the most frequent unreadable Blazor targets were helper
calls. Three rules close most of them without guessing:
`NavigationManager.GetUriWithQueryParameter(s)(…)` targets the current screen
(only the query changes); a helper method of the component, or a static
method of exactly one class of that name in the project scope, is inlined one
level when every return is a literal, an interpolated string (holes become
parameter segments) or such a query-only call; a field whose initializer and
assignments are all readable that way yields one transition per value.
Helpers calling helpers, parameters and values from requests (`Referer`) stay
gaps. A literal or helper path now resolves in the application of the page a
shared component is placed on, so a component library can link into the app
that uses it. eShop: navigations resolved 38 → 52 of 61.

### 2026-11-20: role and policy catalog

The simulator offered only role names that appear in conditions, and every
`policy(...)` atom stayed unknown. `UiFlow` gains `roles` (`RoleDef`: name,
kind `role|permission|group`, `includes`, file, line, source) and `policies`
(`PolicyDef`: name, `requires` as a `Condition`, file, line, source); both
serde default, left out when empty, merged as a sorted union. The import file
may carry them.

Evaluation: `resolve_assignment(flow, assignment)` in `archguard-core` is the
one place that applies the catalog. A held role adds every name its defs
`include`, transitively and cycle-safe, and every `permission` def it covers;
a defined policy the assignment does not switch explicitly is set to what its
`requires` evaluates to, when that is known (rounds until nothing changes, so
a policy may require another; a cycle stays unknown). Several defs of one name
that disagree leave the policy unknown (Bitwarden defines `Application`
differently in three services). Nothing defined: the assignment is unchanged.
`evaluate_condition` keeps its signature and takes the assignment as given;
`reachable_screens`, `screen_reaches` and the findings resolve first, and the
findings also simulate roles named only in policy requirements. CLI and MCP
evaluate endpoints with the resolved assignment.

Permission ids follow Squidex's own rule: a held id covers another when its
dot segments are a prefix and each is equal or `*` (`squidex.apps.{app}.*`
and `squidex.apps.{app}` cover `squidex.apps.{app}.contents.read`). Squidex's
alternatives (`a|b`) and exclusions (`^`) are not read. Which role holds which
permission is not derived.

Rules (`ui_flow_roles.rs`): string constants of classes named `Roles`,
`RoleNames`, `AuthoritiesConstants` (roles) and `Permissions`,
`PermissionIds`, `PermissionNames` (permissions); constants and enum members
(`nameof(E.M)`, `E.M.ToString()`) named in `RequireRole`, `[Authorize(Roles =
…)]`, `@PreAuthorize`, `@Secured`, `@RolesAllowed`, `.hasAuthority`,
`.hasRole` and their `Any` forms, recorded at their declaration; an enum is
never a role catalog by itself (Bitwarden's `OrganizationUserType` is used in
handlers, not in role checks, so it is not listed). `new IdentityRole(x)` at
the seeding site. `AddPolicy(name, …)`: `RequireAuthenticatedUser` = signed
in, `RequireRole` = any of the roles, `RequireClaim(t, v…)` = custom `claim t =
v|…` (constants resolved where declared in the scan, else the source text),
anything else unknown with a note; `DefaultPolicy`/`FallbackPolicy = new
AuthorizationPolicyBuilder()…` likewise. Spring `fromHierarchy`/`setHierarchy`
literals give `includes`. TypeScript `enum` or `const` objects named
`Authority`, `Role`, `Roles`, `Permission`, `Permissions` count only for
members used as `Name.MEMBER` in a `.ts` or `.html` file. `ROLE_` is dropped
for Spring and TypeScript, as in the Spring access rules. A name or value that
is not readable is a `note`; notes leave `complete` unchanged, since the
catalog adds to the flow and missing it loses no screen or endpoint.

CLI: `ui-flow roles [PATH]` lists the catalog (text or JSON with
`requires_text`); `--role ROLE_X` reads `X` when only `X` is defined, an
undefined role gets a warning, and the simulation shows the implied roles and
the resolved policies. MCP `ui_flow`: `metadata.roles` (names) and
`metadata.policies` (name to requirement in words), the full defs as `role`
and `policy` rows, `simulation.implied_roles` and `simulation.policies`.

Checked 2026-11-20: CleanArchitecture `Administrator` (constant and seeding;
this checkout has no `Policies.CanPurge`), jhipster `ADMIN`, `USER`,
`ANONYMOUS` and the Angular `Authority.ADMIN`, Squidex 139 permission ids,
Bitwarden 13 policies, eShop none. Not yet done: the desktop role picker and
inspector (`evaluate_condition` there and in `domain_chain`/`ui_flow_replay`
still gets the unresolved assignment), Spring's `RoleHierarchyImpl` builder
API, a Squidex-style `Role` class with role constants (singular names are not
in the list).

### 2026-11-20: shared language base, run context, access fallback

The extractors were written one by one and each carried its own copies of
the text helpers; the copies had drifted (the Vue copy read `import … from
'x'` without `;`, the React copy lost it). They now call one module per
language family in `archguard-engine`: `lang_ts`/`lang_ts_decl`/
`lang_ts_http` (literal parser, imports, hook bindings, `fetch`/`axios`;
Angular, React, Vue and the TypeScript domain reader), `lang_csharp` (one C#
lexer: comments, preprocessor lines, verbatim, interpolated and raw strings,
char literals; Blazor, XAML, backend, domain, view conventions), `lang_java`,
`lang_markup` (XAML/Razor tags) and `lang_blank`/`lang_text` (blanking and
navigation in blanked text). Where copies differed, the more general rule won
and the tests decided (raw strings and `$@"…"` everywhere, a char literal is
`'x'` or an escape, never an apostrophe in markup text). One framework is
still one module; it keeps its rules and calls the base.

Every extractor runs with an `ExtractContext` (root and configuration).
`ui_flow::analyze(root)` reads the root's configuration as before;
`analyze_with` takes a context. `ui-flow`, `ui-flow open/replay/roles`,
`domain` and `check`'s conventions now use the configuration `--config`
names; before, `--config` was ignored and only the project's own file was
read. The MCP tools use the configuration of their root. Extractors never
import the configuration loader (the architecture gate refuses the cycle
through `check`).

`[views.access]` takes a fallback for use sites without arguments:
`ApiPermission = "policy:$arg|authenticated"`. Only a `role:$arg` or
`policy:$arg` form may have one, and the fallback is no `$arg` form; anything
else is a configuration error. The backend and Angular extractors apply it.
Squidex with `ApiPermission = "policy:$arg|authenticated"`,
`ApiPermissionOrAnonymous = "policy:$arg|open"` and
`mustBeAuthenticatedGuard = "authenticated"`: the 51 endpoints whose bare
attribute was `unknown` become 30 signed in and 21 open; 26 stay
`undeclared`. `ui-flow` and `domain` JSON of eShop, Squidex, both Bitwarden
repositories, jhipster, CleanArchitecture, the OpenTelemetry front end and
the Aspire Vue sample are byte-identical before and after the refactor.

### 2026-11-20: notes for agents, code panel and drill-down in the desktop

Every gap, mismatch and undeclared endpoint in the Screens view can carry a
note for the coding agent: decision fix, intentional (accept-risk) or later,
free text and an optional suggested change (appended to the note after
`Suggested change:`). Notes go into the existing review decision register
`.archguard/reviews.json` under the item's stable id: the convention id
`view-<rule>:<file>:<subject>` where a convention covers the item, else
`view-gap:<file>:<message>`. Agents read them through `review-decision list`
and the MCP `review_decisions` tool; attaching them to `check` output is
open. Every file:line opens in a read-only code panel inside the view or in
the configured editor (program plus argument list, never a shell; paths
outside the project are refused). An action drills down to its endpoints,
handler, use cases, transitions, events and integration events across
services, and to the Calls view for the handler's callees. The role picker
lists the roles of the catalog, and every evaluation in the desktop goes
through `resolve_assignment`.

### 2026-11-20: notes in `check`, handler locations

`check` attaches the notes for agents. While `[views] conventions` is not
empty, `check_gate::view_notes` reads the register's `view-…` decisions (a
corrupt register is an error, as everywhere else); otherwise the register is
not read. A convention finding with a note shows it in text as `note
(fix|intentional|later, by <author>): <text>` under the finding, in JSON and
in the MCP `check` tool as `note {decision, text, author, ts}` plus
`accepted_by_note`, in SARIF as `properties.note`. Notes on `view-gap:…` ids
are listed after the findings under "Noted gaps" (`noted_gaps` in JSON, MCP
and the SARIF run properties); whether that gap still occurs is not checked.

Decided: an intentional note (accept-risk) accepts the finding like a
baseline entry. `gate_fails` takes the notes, `accepted_by_note` is the one
rule, an `expires_on` in the past ends the acceptance. Unlike a baselined
finding it stays in the output, marked `accepted: intentional note; does not
fail check`, and in SARIF it is level `note` with `suppressions: [{kind:
"external", justification: <note>}]`. Reason: the note is a reviewer's
recorded decision with author and reason, which is what a baseline entry
stands for, and an agent must still see it. Fix and later notes change
nothing about failing. The summary line still counts the accepted findings
as convention violations.

`UiFlow` gains `handler_sites: [HandlerSite { endpoint_file, endpoint_line,
handler, file, line }]` (serde default, left out when empty, merged as a
sorted union): where the handler method of an endpoint is declared. The
backend extractor records it for minimal-API method groups (also in another
file of the project), controller actions and Spring methods; a lambda has
none. `UiFlow::handler_site(endpoint)` finds it. The list sits on `UiFlow`
instead of `handler_file`/`handler_line` on `EndpointAccess`, so code that
builds endpoints literally (the desktop tests, the other extractors) keeps
compiling, as with `start_screens`. eShop: `POST /api/orders` →
`Ordering.API/Apis/OrdersApi.cs:118` (`CreateOrderAsync`); all 48 endpoints
with a handler have a site.

### 2026-11-20: NestJS, SvelteKit, FastAPI, Django, Laravel and Rust UIs

Six more extractors, one module each, on the shared run context and language
modules (`lang_python`, `lang_php`, `lang_rust` are new):

- NestJS: controllers, global prefix with excludes, URI versioning, guards
  (`@UseGuards`, `APP_GUARD`, per root module), `@Roles`, and project
  decorators followed one level into `SetMetadata` options (Immich's
  `@Authenticated({ permission, admin, public })`).
- SvelteKit: file routes, `redirect` guards in `load` and `hooks.server`,
  `+server` and form actions as endpoints, one-hop route helpers and generated
  fetch clients (`@immich/sdk`). Immich web and server join 152 of 153 calls.
- FastAPI: routers and includes with composed prefixes and dependencies;
  `Security` scopes are all required, as FastAPI hands every scope on.
- Django and DRF: `urlpatterns` from `ROOT_URLCONF`, includes, namespaces,
  routers, template views as screens, mixins, decorators and permission
  classes with the settings default; runtime view registries (NetBox's
  `register_model_view`) stay gaps.
- Laravel: route files, groups, resources, middleware incl. Kernel aliases,
  controller middleware and top-level authorize calls, Blade links with
  `@can`/`@auth` conditions; checks inside `if` blocks are notes.
- Rust: Tauri commands as `INVOKE` endpoints joined to `invoke()` calls with
  capability-based access, Dioxus/Yew routable enums, Leptos routes and
  server functions, egui/iced screen enums.

Project-specific checks (BookStack `checkPermission`, NetBox mixins, Immich's
`AuthGuard`) are read only through `[views.access]`; nothing is inferred from
names.

### 2026-11-20: TanStack Router and hey-api; shared helpers; FastAPI on `lang_python`

React reads TanStack Router file routes and the clients `@hey-api/openapi-ts`
generates. The FastAPI template's frontend goes from 0 to 8 screens and 17
actions; all 17 join a backend endpoint (16 of 23). The other 24 checkouts
are byte-identical.

- Screens: `createFileRoute('<id>')` with `$id` → `:id`, `_x`/`(x)` segments
  dropped; root and `<Outlet>` modules are layouts. `beforeLoad` is read in
  its own extent (arrow with block body, method shorthand, a local or
  imported guard function): `if`/`else`, early `return` and
  `throw redirect(…)`, with the path of enclosing `if`s
  (`if (a) { if (b) redirect }` requires `not(all(a, b))`). Ancestor routes
  apply by id prefix. Any other redirect placement or form is `Unknown` with
  a diagnostic, never no condition.
- Calls: only an import that resolves (relative, index files, the app's own
  `tsconfig` `paths`, re-export chains) to the indexed `sdk.gen.ts` makes a
  call `Http`; a member the index lacks, a non-literal url, or a method that
  is not an HTTP verb after resolving `Method.X` through its declaration is
  `Unresolved` with a diagnostic. The first version guessed from import
  specifiers containing `client`/`sdk` and from enum member names; the review
  showed invented calls and verbs, and both rules were replaced.

The SvelteKit generated-client index moved to `lang_ts_http::generated_calls`,
the `[views.access]` → condition mapping to `extract_ctx::access_condition`,
and `lang_ts_decl` gained `name_path` (rejects `...x`, `a..b`); Immich output
is byte-identical. FastAPI reads Python through `lang_python` instead of its
own lexer; `string_value` now joins adjacent literals across comments and
`\` continuations and also reads parenthesised literals, decodes f-string
escapes as Python does and rejects t-strings. The FastAPI template and NetBox
are byte-identical.

Process: build agents ran on Sonnet, each branch was reviewed by an Opus
agent against the base before merging; two of four reviews sent the branch
back (FastAPI literal joining, React name guesses).

Open:
- An import alias that does not resolve (no `tsconfig`, `extends` not
  followed) leaves calls through it out without a gap.
- Helpers that redirect inside `beforeLoad` (`ensureAuth(ctx)`), `a && b`
  guards (`Unknown`), `export * as ns` and namespace calls into the SDK.
- `lang_python::statements` does not join a CRLF `\` continuation.
- Mutation probes of the shared TS helpers left four survivors (first fetch
  call, `export async`, fixed `policy:` meaning, non-arrow exports); these
  gaps predate the move.

### 2026-11-21: three access states on screens, links and actions; role conventions

Change (a) of the access-logic work, model only (no extractor reads more than
before):

- `Screen.access`, `Transition.condition` and `Action.condition` are an
  `AccessState` like endpoints: `Protected(Condition)`, `Open`, `Undeclared`.
  A missing field or `null` in a file reads as `Undeclared`, a bare condition
  of older files as `Protected`, `"open"` as `Open`. These three fields keep
  writing a protected state as the bare condition, so files stay as they were. Extractors build it with
  `AccessState::from_parts(condition, false)`; nothing produces `Open` for
  these elements yet. `and` (both layers hold; `Undeclared` is the identity)
  and `or_default` (fallback applies only when nothing was declared) are the
  only combinators in the core.
- The simulator no longer reads "no condition" as allowed: `Reach::Undeclared`
  is its own stage (`ScreenReach::Undeclared` in `screen_reaches`), ordered
  between `Sure` and `Possible` so a path is as weak as its weakest step. The
  findings and the desktop still treat an undeclared step like an unguarded
  one, so their output does not change; showing it is change (c).
- `weak: bool` on screens, links and actions marks a guard that only holds as a
  rule of thumb (layout load, parent loader). `Protected(Condition::Never)`
  stays the condition for a read deny; the rules that produce it follow with
  the extractors.
- `RoleConvention` (per application, `UiFlow.role_conventions`) says how names
  are written and compared: `role_prefix`, `scope_prefix`, `role_claim`,
  `role_case`, `policy_case`, `PermissionFormat` (`Exact`, `Dotted`,
  `ActionSubject`), aliases and origins. `holds`, `canonical` and `label` are
  the one place with comparison logic; hierarchy stays in
  `resolve_assignment`, which now also fills `Assignment.convention`
  (`resolve_assignment_for` takes the application). The built-in
  `permission_covers` became `PermissionFormat::Dotted`; the role catalog
  extractor emits that convention when it finds permission ids.
- `RoleKind::Scope`; `RoleDef` and `PolicyDef` carry `application`.

Open: no extractor emits a convention except the dotted one, and policy atoms
are still switches (only `policy_case` applies to them); the fixed `ROLE_`
handling in the desktop and CLI goes with change (c).

### 2026-11-22: waves 1, 2 and 4 and the service level

Summary of the extractor work since the first extractors (facts from the merge
history `2875e9fe..cff184dd`):

- Wave 1 (P0 fixes, merged with the UI flow model): Laravel, Spring, Blazor,
  NestJS, Django and FastAPI.
- Wave 2 (P1 fixes): Django/DRF (mixin order, `method_decorator`, model
  permissions), Svelte (`error()` guards, hooks `handle`/`sequence`), Angular
  (`canLoad`, `mapToCanActivate`), NestJS (passport subclasses, composites),
  Vue/Nuxt (meta counts only through a reading guard), React (wrapper and Next
  middleware bodies), Express (`express-jwt` `unless`, passport), FastAPI
  (Security scopes, Starlette `@requires`), Spring (conventions, method-security
  switches, chain scoping), XAML (guards only from `[views.access]`), Laravel
  (`verified`/`signed`, Sanctum abilities, model-qualified policies), Rust UIs
  (Tauri capabilities and ACL, Leptos, Dioxus/Yew) and ASP.NET (default and
  fallback policy, `AuthorizeFilter`, inherited actions); roles (per-application
  conventions, catalog from use) and Blazor (project attribute bases, transitive).
  A guard an extractor cannot read stays `Unknown` with a gap instead of being
  dropped or guessed.
- Wave 4, new backend extractors (endpoints and their access): Flask, Go (gin,
  chi, echo, gorilla, `net/http`, casbin, oathkeeper), Phoenix (routes,
  pipelines, `live_session` hooks; also screens), Rails (routes, callbacks,
  authorization), Spring WebFlux (`RouterFunction` routes and the reactive
  security chain), Quarkus / Jakarta REST (HTTP permissions, security
  annotations, defaults) and Micronaut (`@Secured`, JSR-250, `intercept-url-map`,
  default deny).
- Service level (`ui_flow_service`, Java and Kotlin only): the checks below an
  endpoint's handler become `ServiceLayer`s on the flow. A check is `Enforced`
  (ANDed into the effective access) only when the handler calls the checked
  method directly and unconditionally on another bean; an early return,
  conditional or indirect call, or differing implementations give `PathOnly`;
  self-invocation, `private`/`final` methods and similar give `Hint`. Service
  layers can only add `Protected` conditions, never open an endpoint.

### 2026-11-23: crate locations

The readers moved out of `archguard-engine` into `archguard-flow-*` crates (pure move, no behaviour
change): shared substrate `archguard-flow-common`, Java/Kotlin/C# kernel `archguard-flow-clike`, Spring/ASP.NET driver `archguard-flow-backend`, then `-jvm`, `-dotnet`, `-python`, `-web`, `-node`, `-server`, `-rust` per language family. Module names are unchanged; the engine keeps the registry (`ui_flow`, `domain_model`).
