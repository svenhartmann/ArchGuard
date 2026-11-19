# ADR 0022: Domain view read from code

- **Status:** accepted
- **Date:** 2026-11-19

## Context

ADR 0021 adds the user-interface view (screens, transitions, actions,
permissions). The domain side is still missing: ArchGuard shows layers,
contracts and datastores, but not the domain model, the use cases, the
states an entity passes through, the rules that guard them or the events a
change raises. ADR 0012 already reads entity mappings for tables (EF Core
`DbSet`, JPA/TypeORM `@Entity`, SQLAlchemy), so entities are partly found;
their fields and relations are not kept.

A request like "after the payout, the person gets a message" names a state
change (payout), an event and a reaction. To change that safely, an agent
and a reviewer need the chain from the screen down to the domain:
action → endpoint → use case → entity method → state transition → event →
handlers. Each link must come from code by a fixed rule and point to file
and line, as in ADR 0021.

Checked 2026-11-19 in `examples/checkouts/eshop/src/Ordering.*`: aggregates
under `AggregatesModel/`, `enum OrderStatus`, commands with
`*CommandHandler`, domain events raised with `AddDomainEvent(new
…DomainEvent(…))` and `*DomainEventHandler` classes. Transitions follow one
shape:

```csharp
public void SetPaidStatus() {
    if (OrderStatus == OrderStatus.StockConfirmed) {
        AddDomainEvent(new OrderStatusChangedToPaidDomainEvent(Id, OrderItems));
        OrderStatus = OrderStatus.Paid;
    }
}
```

## Decision

1. **One neutral model** `domain-model/v1` in `archguard-core`
   (`domain_model.rs`), framework-free like `ui-flow/v1`:
   - `Entity`: id, name, kind (`aggregate_root`, `entity`, `value_object`,
     `unknown`), fields (name, type, collection flag), file, line.
   - `Relation`: from entity, to entity, kind (`has_one`, `has_many`,
     `references`), via field.
   - `StateMachine`: entity, state field, enum type, states, initial state
     when assigned in a constructor, and `StateTransition`s.
   - `StateTransition`: from states (empty = any), to state, the method that
     performs it, the guard as source text, events raised in the same
     method, file, line.
   - `UseCase`: id, name, kind (`command`, `query`, `handler`, `service`),
     the request type, the entity methods it calls, file, line.
   - `DomainEvent`: type, raised by (entity method or use case), handled by
     (handler classes).
   - `diagnostics` and `complete` with the same meaning as in ADR 0021.
2. **Rules, not guesses.** An element enters the model only through a
   declared pattern:
   - Entities: the ADR 0012 mappings, plus types implementing a configured
     marker (`IAggregateRoot`, `AggregateRoot`, `@AggregateRoot`, base class
     `Entity`); `value_object` by marker or by `record`/`@Embeddable`.
     Relations from fields whose type is another entity or a collection of
     one.
   - State machines: a field whose type is an enum, assigned an enum
     literal inside an entity method. The guard is the condition of the
     enclosing `if`/`switch` that compares the same field with literals
     (`==`, `!=`, `||`, `&&`, `is`, `case`). A `!=` guard that throws, as in
     `SetShippedStatus`, yields the complement. An assignment from a
     variable or a computed value is not guessed: diagnostic, `complete:
     false`.
   - Use cases: classes implementing a configured handler interface
     (MediatR `IRequestHandler<,>`, `ICommandHandler<>`, Spring
     `@Service` methods only when configured), or named `*CommandHandler`,
     `*QueryHandler`, `*UseCase`. Which entity methods they call comes from
     the existing compiler-backed call model (C#, Java), not from text.
   - Events: `new X(...)` passed to a configured raise function
     (`AddDomainEvent`, `registerEvent`, `publishEvent`, `apply`); handlers
     by a configured interface (`INotificationHandler<X>`,
     `IIntegrationEventHandler<X>`, `@EventListener`, `@TransactionalEventListener`).
   The marker names live in the configuration with these defaults; a
   project adds its own instead of ArchGuard guessing them.
3. **Import file** `.archguard/domain-model.json` in the same schema, merged
   like `.archguard/ui-flow.json`. It is the way in for any language
   without an extractor.
4. **The chain to the UI.** `UiFlow` `EndpointAccess` and an `UseCase` join
   when the endpoint's handler method calls the use case (call model) or
   sends its request type (`mediator.Send(new XCommand…)`). The result is
   the path action → endpoint → use case → transitions → events → handlers,
   shown in both directions: "what does this button change" and "which
   screens can trigger `Paid`". Rights from ADR 0021 apply along the path,
   so the simulator answers "can role X cause state Paid".
5. **Rules as a view, not as logic.** Guards are shown as source text on the
   transition. Turning arbitrary business rules into evaluable formulas is
   out of scope; a project that wants them evaluable writes them as named
   specification or policy classes, which then appear as atoms.
6. **Conventions in `check`** (later phase, opt-in): every assignment to a
   state field happens in a method of its entity; events are raised through
   the configured function; handlers are found by interface. An agent kept
   to these conventions keeps the view complete.

### Phases

1. Model, merge, import (core, engine). No extractor.
2. C# extractor (entities, relations, state machines, use cases, events)
   against `eshop/Ordering` as reference.
3. Join to ADR 0021 (endpoint → use case) and the chain in CLI and MCP.
4. Java/Kotlin extractor (JPA, Spring events, Axon/Spring Modulith markers).
5. Desktop view: entity diagram, state diagram per entity, chain view.
6. TypeScript, Python, PHP extractors; convention checks.

## Alternatives

- **Read the domain from the database schema.** Tables lose aggregates,
  value objects, states and events; ADR 0012 already covers tables.
- **Require annotations for everything** (`@StateTransition(from, to)`).
  Exact, but existing code like eshop would show nothing. Patterns first,
  annotations only as an optional way to make a case explicit.
- **Let a model summarize the domain.** Rejected for the same reason as in
  ADR 0021: the view must be reproducible and point to code.

## Consequences

- State machines written as tables, in the State pattern or through a
  library (Stateless, Spring Statemachine) are not found until an
  extractor for that form exists; until then the import file covers them.
- A guard reads as text, so the view can show a transition whose guard can
  never be true.
- Use-case detection depends on the call model, so it needs the compiler
  opt-in where the call model does.

## Open points

- Which further state machine forms (State pattern, Stateless, Spring
  Statemachine) come first.
- Whether Spring `@Service` methods count as use cases by default.

## Verlauf

### 2026-11-19: a guard reads as the allowed condition

`domain` showed `Order.SetCancelledStatus` with the guard `OrderStatus ==
OrderStatus.Paid || OrderStatus == OrderStatus.Shipped`: the from-set was
right, but the text was the condition under which the method throws. The C#
rules now write a guard taken from an exit `if` (throw, return, throwing
helper) or from an `else` as the literal negation of the source text,
`!(OrderStatus == OrderStatus.Paid || OrderStatus == OrderStatus.Shipped)`,
never simplified. A guard from an enclosing `if` stays as written. So
`guard` always means "allowed when"; CLI and MCP label it so. The model is
unchanged; a separate field for the source form was not added.

### 2026-11-19: the chain from endpoint to event handler (decision 4)

`EndpointAccess` gains `handler` (method id, e.g. `OrdersApi.CreateOrderAsync`;
none for a lambda) and `sends` (both serde-default, so older `ui-flow.json`
reads). The backend extractor fills them from the handler body: minimal-API
method groups (found in the project) and lambdas, controller actions, Spring
controller methods. `sends` holds the argument type of `x.Send(…)`,
`SendAsync`, `send`, `sendAndWait` when the receiver name contains
`mediator`, `sender`, `gateway` or `bus`: `new X(…)`, or a variable assigned
`new X(…)` or declared `X` (local, parameter, field). A generic wrapper
`new IdentifiedCommand<CreateOrderCommand, bool>(cmd, id)` yields the wrapper
as written and every argument whose type is one of its type arguments, so
eShop records both. A field typed `*UseCase`, `*CommandHandler` or
`*QueryHandler` whose method the handler calls is recorded as that class.
A computed argument (factory call) adds nothing; Kotlin constructor calls
without `new` are not read. This stands in for the call model named in
decision 4 until that model covers endpoint handlers.

`archguard_engine::domain_chain::chains` joins endpoint → use cases (request
or class name, generics and namespace stripped, every match) → transitions
whose method a use case calls → `event_chain`; one chain per endpoint and
calling action, endpoints without an action included.
`triggers_of_state(flow, model, entity, state)` keeps the chains with a
transition into that state. CLI `domain --chain` and `--entity E --state S`,
MCP `domain_model` with `chain` and `state`, show them.

On eShop, `Paid` is set by `SetPaidOrderStatusCommandHandler`, which an
integration-event handler (payment succeeded) sends, not an endpoint; the
reverse query therefore finds no endpoint for `Paid`. Integration-event
handlers as chain starts are open. Rights along the path (simulating "can
role X cause Paid") are not evaluated yet.

### 2026-11-19: conventions in `check` (decision 6, first part)

Switched on like the ADR 0021 conventions through `[views] conventions`:
`state_changes_in_entity` reports, in a C# file other than the entity's,
`x.Field = Enum.Value` on a state field of a found state machine, plus every
non-literal state assignment the extractor reports; `events_through_raise_function`
reports `new X(…)` of a domain-event type (named `*DomainEvent` or listing
`INotification` as a base) that is neither an `AddDomainEvent(…)` argument nor
a variable the same file passes to it, plus every raise argument the
extractor could not read. The scan is text-level like the C# extractor. The
raise function is the extractor's `AddDomainEvent`, not yet configurable.
Not yet done: Java/Kotlin, object initializers, and "handlers are found by
interface".

### 2026-11-19: message handlers as chain starts, rights along the chain

Decision 4 named endpoints as the only chain start, so `Order → Paid` on
eShop had no trigger. `DomainModel` gains `message_handlers` (serde default,
left out when empty): `MessageHandler { id, message, sends, file, line,
source }`. The C# rules record a class per handled `T` of
`IIntegrationEventHandler<T>`, MassTransit `IConsumer<T>` and NServiceBus
`IHandleMessages<T>`; the Java rules a method annotated `@KafkaListener`,
`@RabbitListener`, `@JmsListener` or `@StreamListener` (id `Class.method`,
message = first parameter type). `sends` uses the send rule of the backend
extractor (the same `sends` function, made reachable through
`ui_flow_backend::sends_in`, not a copy), so `context.Send`/`Publish` of
MassTransit and NServiceBus are not read, and Spring Cloud Stream functional
beans are not found. The handlers are part of the model, not only of
`domain_chain`, so the import file can add them and other views can show
them.

`domain_chain::Chain` now has a `start`: `Endpoint { action, endpoint }` or
`Message { message_handler }`, serialized flat as before (`endpoint`,
`action`) or as `message_handler`. `chains` lists endpoint chains, then one
chain per message handler; `triggers_of_state` keeps both. On eShop `domain
--entity Order --state Paid` now finds `message
OrderPaymentSucceededIntegrationEvent → SetPaidOrderStatusCommandHandler`.

Rights along the path (decision 4, last sentence): `domain_chain::rights`
gives each chain a `verdict` and `reasons` for an assignment. For an
endpoint chain it joins in Kleene logic the endpoint access (`undeclared` is
unknown), and with an action the reach of its screen (`sure` allowed,
`possible`/`no_known_path`/`no_start_screen` unknown, `denied` denied) and
the action's condition. A message-handler chain is `not_a_user_path`: it
runs inside the system, no role starts it. CLI `domain --chain|--state …
--role R|--anonymous` prints `right: …` per chain and adds `right` and
`simulated` to the JSON; MCP `domain_model` takes `roles`/`anonymous` with
`chain` or `state`. Not done: following a message back to the endpoint whose
use case publishes it (the integration event's raiser).

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
