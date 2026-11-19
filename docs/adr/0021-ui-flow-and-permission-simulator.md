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
