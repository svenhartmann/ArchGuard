//! Java and Kotlin extractor for domain-model/v1 (ADR 0022, phase 4): a
//! text-level scanner (comments and string contents blanked, brackets
//! matched), no compiler. Every entry has `source: "java"`, Kotlin ones too.
//!
//! Recognized:
//! - Files: `*.java` and `*.kt` below the root, skipping `target`, `build`,
//!   the directories the service scan excludes and test paths; files over
//!   1 MiB, unreadable files and files past the first 5 000 are diagnostics.
//! - Entities: a class annotated `@AggregateRoot` (jMolecules) or `@Aggregate`
//!   (Axon), implementing `AggregateRoot<…>` (jMolecules/Axon) or extending
//!   `AbstractAggregateRoot<…>` (Spring Data) → `aggregate_root`; `@Entity` →
//!   `entity`; `@Embeddable` → `value_object`; a Java `record` or Kotlin `data
//!   class` in the package of an aggregate root → `value_object`. Classes named
//!   like a marker are the markers and skipped. `id` is the class name,
//!   package-qualified when two entities share a name (with a diagnostic).
//! - Fields: Java fields and record components, Kotlin `val`/`var`
//!   properties and primary-constructor properties (type written, or `E.V`
//!   initializer → `E`); `List`, `Set`, `Collection`, `Iterable`, their
//!   mutable/hash/tree forms, arrays and a field annotated
//!   `@OneToMany`/`@ManyToMany` are `collection` with the element type.
//! - Relations: a field whose type is a found entity → `has_one`
//!   (`@ManyToOne`, `@OneToOne`, `@Embedded`), a collection of one →
//!   `has_many`. A `Long`/`long`/`UUID` (nullable too) `xId` with `X` an
//!   aggregate root in another package → `references`.
//! - State machines: an enum-typed field (`@Enumerated` or not) assigned inside
//!   a method of its entity (`status = OrderStatus.PAID`, `this.status = …`,
//!   `status = PAID` when `OrderStatus.PAID` or `OrderStatus.*` is (statically)
//!   imported). From states come from the nearest enclosing `if` comparing the
//!   field with enum literals (`==` joined by `||`, `!=` joined by `&&`, one
//!   `==`/`!=`), from the `else` of such an `if` (negated), or from the nearest
//!   preceding `if` on the field whose block throws, returns or calls an entity
//!   method that only throws (negated). The guard text is the allowed
//!   condition: a negated guard is the literal `!(source condition)`, never
//!   simplified. Without such an `if`, from = [] (any). A constructor or Kotlin
//!   `init` assignment is `initial`, else the field initializer. A non-literal
//!   value, a `switch`/`when` around the assignment or a guard of another
//!   shape is a diagnostic and makes the model incomplete.
//! - Events: `registerEvent(…)`, `publishEvent(…)` and `apply(…)` (bare or
//!   `AggregateLifecycle.apply`) with `new X(…)`, Kotlin `X(…)` or a variable
//!   assigned one earlier in the method → `raised_by: "Class.method"` for any
//!   class, and `X` on that method's transitions. Handlers: classes with a
//!   method annotated `@EventListener`, `@TransactionalEventListener`,
//!   `@EventHandler` or `@ApplicationModuleListener`; the event is the type of
//!   its first parameter.
//! - Use cases: methods annotated `@CommandHandler`/`@QueryHandler` (id
//!   `Class.method`, request = first parameter type); otherwise classes named
//!   `*CommandHandler`, `*QueryHandler`, `*UseCase`, and `@Service` classes
//!   only when [`SERVICE_USE_CASES`] is set (ADR 0022 open point, default off).
//!   `calls`: `.m(` in the use case where `m` is a public method of exactly one
//!   entity, accessors (`getX`/`setX`/`isX` of a field), `equals`, `hashCode`
//!   and `toString` left out. A name on several entities is a diagnostic.
//! - Message handlers (chain starts without an endpoint): methods annotated
//!   `@KafkaListener`, `@RabbitListener`, `@JmsListener` or `@StreamListener`
//!   (id `Class.method`, message = first parameter type without generics, so
//!   `ConsumerRecord<K, V>` reads `ConsumerRecord`); `sends` by the backend
//!   extractor's send rule over the method body. Spring Cloud Stream
//!   functional beans (`Consumer<T>` beans) are not read.
//! - Rules, Bean Validation and broker publish sites: see `rich`.
//!
//! Not recognized: nested-type qualification, state changes outside the
//! entity's own methods (setters called from services assign a variable and
//! are diagnostics), `equals`/`in`/`when` guards, Kotlin `check`/`require` and
//! Spring `Assert` as exit forms, `else if` chains as negations, Spring
//! Statemachine and the State pattern, request types of `*CommandHandler`
//! classes. The marker names are the constants below with the ADR 0022
//! defaults; reading them from the configuration comes later.
use crate::extract_ctx::{ExtractContext, domain_sources, with_root_config};
use crate::lang_text::*;
use crate::ui_flow_backend::sends_in;
use archguard_core::domain_model::{
    DomainDiagnostic, DomainEvent, DomainModel, Entity, EntityKind, EventHandlerSite, Field,
    MessageHandler, Relation, RelationKind, StateMachine, StateTransition, UseCase, UseCaseKind,
};
use members::{Kind, Member, field, param_types};
use states::{Rules, Step, assignments, created, raise_calls};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SOURCE: &str = "java";
const AGGREGATE_ANNOTATIONS: [&str; 2] = ["AggregateRoot", "Aggregate"];
const AGGREGATE_BASES: [&str; 2] = ["AggregateRoot", "AbstractAggregateRoot"];
const ENTITY_ANNOTATION: &str = "Entity";
const VALUE_OBJECT_ANNOTATION: &str = "Embeddable";
const TO_MANY: [&str; 2] = ["OneToMany", "ManyToMany"];
const RAISE_FUNCTIONS: [&str; 3] = ["registerEvent", "publishEvent", "apply"];
/// `apply` counts only bare or with this qualifier (Axon), not `function.apply(x)`.
const APPLY_QUALIFIER: &str = "AggregateLifecycle";
const EVENT_LISTENERS: [&str; 4] = [
    "EventListener",
    "TransactionalEventListener",
    "EventHandler",
    "ApplicationModuleListener",
];
const MESSAGE_LISTENERS: [&str; 4] = [
    "KafkaListener",
    "RabbitListener",
    "JmsListener",
    "StreamListener",
];
const COMMAND_HANDLER: &str = "CommandHandler";
const QUERY_HANDLER: &str = "QueryHandler";
const USE_CASE_SUFFIXES: [&str; 3] = ["CommandHandler", "QueryHandler", "UseCase"];
const SERVICE_ANNOTATION: &str = "Service";
/// Whether `@Service` classes count as use cases (ADR 0022 open point).
const SERVICE_USE_CASES: bool = false;
const NOT_CALLS: [&str; 3] = ["equals", "hashCode", "toString"];
const COLLECTIONS: [&str; 16] = [
    "List",
    "Set",
    "Collection",
    "Iterable",
    "ArrayList",
    "LinkedList",
    "HashSet",
    "LinkedHashSet",
    "TreeSet",
    "SortedSet",
    "MutableList",
    "MutableSet",
    "MutableCollection",
    "MutableIterable",
    "Array",
    "Bag",
];
const ID_TYPES: [&str; 3] = ["Long", "long", "UUID"];
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const MODIFIERS: [&str; 30] = [
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "final",
    "abstract",
    "transient",
    "volatile",
    "synchronized",
    "native",
    "default",
    "strictfp",
    "open",
    "sealed",
    "data",
    "enum",
    "inner",
    "override",
    "lateinit",
    "const",
    "annotation",
    "value",
    "inline",
    "suspend",
    "operator",
    "infix",
    "tailrec",
    "external",
    "companion",
];

/// Byte range in a file's text.
type Span = (usize, usize);
/// Diagnostics found on the way: file index, offset, message, drops an element.
type Pending = Vec<(usize, usize, String, bool)>;

/// The domain model of the Java and Kotlin sources below `root`.
pub fn extract(root: &Path) -> DomainModel {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> DomainModel {
    let root = ctx.root;
    let mut x = Extractor::default();
    x.read(root);
    for f in 0..x.files.len() {
        x.declarations(f);
    }
    x.entities();
    x.relations();
    x.machines_and_events();
    x.use_cases();
    x.message_handlers();
    x.model.rules = rich::rules(&x);
    x.model.validations = rich::validations(&x);
    x.model.integration_events = rich::integration_events(&x);
    x.finish()
}

struct Src {
    rel: String,
    /// Source with comments and string contents blanked; offsets unchanged.
    text: String,
    kotlin: bool,
    /// `a.b.C.*` / `a.b.C.D` per `import` line, `static` dropped.
    imports: Vec<String>,
    /// The source as read, for messages and rule text.
    raw: String,
    /// The folder of the nearest `pom.xml`/`build.gradle(.kts)`.
    project: String,
}

struct Decl {
    file: usize,
    /// `class`, `interface`, `enum`, `record` or `object`.
    kw: &'static str,
    /// Kotlin `data class`.
    data: bool,
    name: String,
    /// Package, or the folder when the file declares none.
    scope: String,
    pos: usize,
    annotations: Vec<String>,
    bases: Vec<String>,
    /// Record components or Kotlin primary-constructor parameters.
    params: Option<Span>,
    body: Option<Span>,
    members: Vec<Member>,
}
impl Decl {
    fn methods(&self) -> impl Iterator<Item = (&Member, &str, Span, Span, bool)> {
        self.members.iter().filter_map(|m| match &m.kind {
            Kind::Method {
                name,
                params,
                body,
                ctor,
            } => Some((m, name.as_str(), *params, *body, *ctor)),
            Kind::Data { .. } => None,
        })
    }
    /// Methods whose body only throws, so calling them exits like `throw`.
    fn throwing<'a>(&'a self, s: &str) -> BTreeSet<&'a str> {
        self.methods()
            .filter(|(.., body, _)| s[body.0..body.1].trim_start().starts_with("throw"))
            .map(|(_, n, ..)| n)
            .collect()
    }
}

struct Ent {
    id: String,
    name: String,
    kind: EntityKind,
    decl: usize,
    fields: Vec<Field>,
}

#[derive(Default)]
struct Extractor {
    model: DomainModel,
    files: Vec<Src>,
    decls: Vec<Decl>,
    ents: Vec<Ent>,
    /// Event types per (declaration, method).
    raised_in: BTreeMap<(usize, String), Vec<String>>,
    pending: Pending,
}

impl Extractor {
    fn read(&mut self, root: &Path) {
        let java = |rel: &str| rel.ends_with(".java") || rel.ends_with(".kt");
        let (sources, gaps) = domain_sources(root, &[], java, "Java/Kotlin");
        for (file, message) in gaps {
            self.push_diag(&file, message);
        }
        let mut projects = BTreeMap::new();
        for (rel, raw) in sources {
            let text = crate::lang_java::mask(&raw);
            let imports = decls::imports(&text);
            let kotlin = rel.ends_with(".kt");
            let project =
                crate::domain_csharp::code::project_of(root, &rel, &BUILD_FILES, &mut projects);
            self.files.push(Src {
                rel,
                text,
                kotlin,
                imports,
                raw,
                project,
            });
        }
    }

    fn push_diag(&mut self, file: &str, message: String) {
        self.model.diagnostics.push(DomainDiagnostic {
            file: file.into(),
            line: 0,
            message,
        });
        self.model.complete = false;
    }

    fn declarations(&mut self, f: usize) {
        let src = &self.files[f];
        let dir = src.rel.rsplit_once('/').map_or("", |(d, _)| d);
        let found = decls::parse(&src.text, src.kotlin, dir);
        self.decls
            .extend(found.into_iter().map(|d| Decl { file: f, ..d }));
    }

    fn entities(&mut self) {
        let kinds = self.classify();
        let mut ents: Vec<Ent> = kinds
            .into_iter()
            .map(|(d, kind)| {
                let decl = &self.decls[d];
                Ent {
                    id: decl.name.clone(),
                    name: decl.name.clone(),
                    kind,
                    decl: d,
                    fields: self.fields(decl),
                }
            })
            .collect();
        self.pending.extend(qualify(&mut ents, &self.decls));
        self.model.entities = ents
            .iter()
            .map(|e| {
                let d = &self.decls[e.decl];
                Entity {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    kind: e.kind,
                    fields: e.fields.clone(),
                    file: self.files[d.file].rel.clone(),
                    line: line(&self.files[d.file].text, d.pos),
                    source: SOURCE.into(),
                    module: d.scope.clone(),
                }
            })
            .collect();
        self.ents = ents;
    }

    /// Declarations with a marker, then records/data classes in an aggregate
    /// root's package.
    fn classify(&self) -> Vec<(usize, EntityKind)> {
        let mut kinds: Vec<(usize, EntityKind)> = (self.decls.iter().enumerate())
            .filter_map(|(d, decl)| marker_kind(decl).map(|k| (d, k)))
            .collect();
        let roots: BTreeSet<&str> = (kinds.iter())
            .filter(|(_, k)| *k == EntityKind::AggregateRoot)
            .map(|&(d, _)| self.decls[d].scope.as_str())
            .collect();
        let values: Vec<(usize, EntityKind)> = (self.decls.iter().enumerate())
            .filter(|(d, decl)| {
                (decl.kw == "record" || decl.data)
                    && !kinds.iter().any(|k| k.0 == *d)
                    && roots.contains(decl.scope.as_str())
            })
            .map(|(d, _)| (d, EntityKind::ValueObject))
            .collect();
        kinds.extend(values);
        kinds
    }

    /// Record components / constructor properties, then body fields.
    fn fields(&self, d: &Decl) -> Vec<Field> {
        let src = &self.files[d.file];
        let mut out: Vec<Field> = match (d.params, src.kotlin) {
            (Some(p), false) if d.kw == "record" => members::record_components(&src.text, p),
            _ => vec![],
        };
        out.extend(d.members.iter().filter_map(|m| match &m.kind {
            Kind::Data { ty, name, .. } => Some(field(ty, name, m.has_any(&TO_MANY))),
            Kind::Method { .. } => None,
        }));
        out
    }

    /// The entity a type name means as seen from entity `from`; `None` when no
    /// entity or several outside `from`'s package have that name.
    fn resolve(&self, name: &str, from: usize) -> Option<usize> {
        let all: Vec<usize> = (0..self.ents.len())
            .filter(|&e| self.ents[e].name == name)
            .collect();
        match all.as_slice() {
            [one] => Some(*one),
            _ => all.into_iter().find(|&e| self.scope(e) == self.scope(from)),
        }
    }

    fn scope(&self, e: usize) -> &str {
        &self.decls[self.ents[e].decl].scope
    }

    fn relations(&mut self) {
        let mut found = vec![];
        for (e, ent) in self.ents.iter().enumerate() {
            for f in &ent.fields {
                if let Some((to, kind)) = self.relation_of(e, f) {
                    found.push(Relation {
                        from: ent.id.clone(),
                        to: self.ents[to].id.clone(),
                        kind,
                        via: f.name.clone(),
                        source: SOURCE.into(),
                        external: false,
                    });
                }
            }
        }
        self.model.relations = found;
    }

    fn relation_of(&self, e: usize, f: &Field) -> Option<(usize, RelationKind)> {
        if let Some(to) = self.resolve(&simple(&f.type_name), e) {
            let kind = if f.collection {
                RelationKind::HasMany
            } else {
                RelationKind::HasOne
            };
            return Some((to, kind));
        }
        let x = f
            .name
            .strip_suffix("Id")
            .filter(|_| !f.collection && ID_TYPES.contains(&f.type_name.as_str()))?;
        let to = self.resolve(&capitalize(x), e).filter(|&to| {
            self.ents[to].kind == EntityKind::AggregateRoot && self.scope(to) != self.scope(e)
        })?;
        Some((to, RelationKind::References))
    }

    /// The constants of the enum `name` as seen from entity `e`.
    fn enum_states(&self, name: &str, e: usize) -> Option<Vec<String>> {
        let all: Vec<&Decl> = (self.decls.iter())
            .filter(|d| d.kw == "enum" && d.name == name)
            .collect();
        let decl = match all.as_slice() {
            [one] => *one,
            _ => *all.iter().find(|d| d.scope == self.scope(e))?,
        };
        let s = &self.files[decl.file].text;
        let (a, z) = decl.body?;
        let constants = &s[a + 1..z];
        let end = find_top(constants, ";").unwrap_or(constants.len());
        Some(
            split_top(&constants[..end], ",")
                .into_iter()
                .filter_map(|m| {
                    let m = members::skip_annotations(m);
                    idents(m, 0, m.len()).first().map(|x| x.1.to_string())
                })
                .collect(),
        )
    }

    fn machines_and_events(&mut self) {
        let mut raised: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut pending = Pending::new();
        for d in 0..self.decls.len() {
            self.raised_events(d, &mut raised, &mut pending);
        }
        let mut machines = vec![];
        for e in 0..self.ents.len() {
            for m in &self.decls[self.ents[e].decl].members {
                if let Kind::Data { ty, name, init } = &m.kind
                    && let Some(machine) = self.machine_of(e, (ty, name, *init), &mut pending)
                {
                    machines.push(machine);
                }
            }
        }
        self.model.state_machines = machines;
        (self.model.events, self.model.event_handler_sites) =
            domain_events(&self.decls, &self.files, raised);
        self.pending.extend(pending);
    }

    /// Event types raised per method of declaration `d`; adds the raisers to `raised`.
    fn raised_events(
        &mut self,
        d: usize,
        raised: &mut BTreeMap<String, BTreeSet<String>>,
        pending: &mut Pending,
    ) {
        let decl = &self.decls[d];
        let src = &self.files[decl.file];
        let s = &src.text;
        for (_, name, _, body, _) in decl.methods() {
            for (pos, call, arg) in raise_calls(s, body) {
                let Some(t) = created(s, (body.0, pos), arg) else {
                    if call != "apply" {
                        let message = format!(
                            "{}.{name}: {call} argument is neither a constructor call nor a variable assigned one; the event is not read.",
                            decl.name
                        );
                        pending.push((decl.file, pos, message, true));
                    }
                    continue;
                };
                raised
                    .entry(t.clone())
                    .or_default()
                    .insert(format!("{}.{name}", decl.name));
                self.raised_in
                    .entry((d, name.to_string()))
                    .or_default()
                    .push(t);
            }
        }
    }

    /// The state machine of field `(type, name, initializer)` of entity `e`,
    /// when the type is an enum and a method of the entity assigns the field.
    fn machine_of(
        &self,
        e: usize,
        (ty, field_name, init): (&str, &str, Option<Span>),
        pending: &mut Pending,
    ) -> Option<StateMachine> {
        let ent = &self.ents[e];
        let decl = &self.decls[ent.decl];
        if field(ty, "", false).collection {
            return None;
        }
        let enum_type = simple(ty);
        let states = self.enum_states(&enum_type, e)?;
        let src = &self.files[decl.file];
        let s = &src.text;
        let helpers = decl.throwing(s);
        let rules = Rules::new(s, (field_name, &enum_type), &states, &helpers, &src.imports);
        let mut machine = StateMachine {
            entity: ent.id.clone(),
            field: field_name.into(),
            enum_type: enum_type.clone(),
            states: states.clone(),
            initial: None,
            transitions: vec![],
        };
        let mut assigned = false;
        for (_, method, _, body, ctor) in decl.methods() {
            for (stmt, rhs) in assignments(s, body, field_name, src.kotlin) {
                assigned = true;
                let at = format!("{}.{method}", ent.id);
                match rules.step(body, stmt, rhs, ctor) {
                    Step::NonLiteral(v) => pending.push((decl.file, stmt, format!("{at} assigns {field_name} the non-literal `{v}`; the transition is not read."), true)),
                    Step::Initial(to) => {
                        machine.initial.get_or_insert(to);
                    }
                    Step::Transition { from, to, guard, unread } => {
                        if let Some(why) = unread {
                            pending.push((decl.file, stmt, format!("{at}: guard of {field_name} not read ({why}); from states unknown."), true));
                        }
                        machine.transitions.push(StateTransition {
                            from,
                            to,
                            method: at,
                            guard,
                            events: (self.raised_in.get(&(ent.decl, method.to_string())))
                                .cloned()
                                .unwrap_or_default(),
                            file: src.rel.clone(),
                            line: line(s, stmt),
                        });
                    }
                }
            }
        }
        if machine.initial.is_none() {
            machine.initial = init.and_then(|(a, z)| rules.literal(&collapse(&s[a..z])));
        }
        machine.transitions.sort();
        assigned.then_some(machine)
    }

    fn use_cases(&mut self) {
        let mut owners: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for e in &self.ents {
            let decl = &self.decls[e.decl];
            for (m, name, _, _, ctor) in decl.methods() {
                if m.public && !ctor && !accessor(name, &e.fields) {
                    owners.entry(name).or_default().insert(&e.id);
                }
            }
        }
        let mut pending = Pending::new();
        let mut cases = vec![];
        for d in &self.decls {
            cases.extend(self.use_cases_of(d, &owners, &mut pending));
        }
        self.model.use_cases = cases;
        self.pending.extend(pending);
    }

    /// Annotated handler methods of `d`, else `d` itself when named or marked.
    fn use_cases_of(
        &self,
        d: &Decl,
        owners: &BTreeMap<&str, BTreeSet<&str>>,
        pending: &mut Pending,
    ) -> Vec<UseCase> {
        let src = &self.files[d.file];
        let s = &src.text;
        let mut make = |name: String, kind, request, body: Span, pos| {
            let (calls, ambiguous) = calls_in(s, body, owners);
            for (at, message) in ambiguous {
                pending.push((d.file, at, format!("{name}: {message}"), true));
            }
            UseCase {
                id: name.clone(),
                name,
                kind,
                request,
                calls,
                file: src.rel.clone(),
                line: line(s, pos),
                source: SOURCE.into(),
            }
        };
        let mut out = vec![];
        for (m, name, params, body, _) in d.methods() {
            let kind = if m.has(COMMAND_HANDLER) {
                UseCaseKind::Command
            } else if m.has(QUERY_HANDLER) {
                UseCaseKind::Query
            } else {
                continue;
            };
            let request = param_types(s, params, src.kotlin).into_iter().next();
            out.push(make(
                format!("{}.{name}", d.name),
                kind,
                request,
                body,
                body.0,
            ));
        }
        let named = USE_CASE_SUFFIXES.iter().any(|x| d.name.ends_with(x));
        let service = SERVICE_USE_CASES && d.annotations.iter().any(|a| a == SERVICE_ANNOTATION);
        if out.is_empty() && matches!(d.kw, "class" | "object") && (named || service) {
            let kind = if named {
                use_case_kind(&d.name)
            } else {
                UseCaseKind::Service
            };
            let body = d.body.unwrap_or((d.pos, d.pos));
            out.push(make(d.name.clone(), kind, None, body, d.pos));
        }
        out
    }

    /// Broker listener methods, sorted.
    fn message_handlers(&mut self) {
        let mut found = vec![];
        for d in &self.decls {
            let src = &self.files[d.file];
            for (m, name, params, body, _) in d.methods() {
                let message = param_types(&src.text, params, src.kotlin)
                    .into_iter()
                    .next();
                let (true, Some(message)) = (m.has_any(&MESSAGE_LISTENERS), message) else {
                    continue;
                };
                found.push(MessageHandler {
                    id: format!("{}.{name}", d.name),
                    message,
                    sends: sends_in(&src.text, body),
                    file: src.rel.clone(),
                    line: line(&src.text, body.0),
                    source: SOURCE.into(),
                    service: src.project.clone(),
                });
            }
        }
        found.sort();
        self.model.message_handlers = found;
    }

    fn finish(mut self) -> DomainModel {
        for (f, pos, message, drops) in std::mem::take(&mut self.pending) {
            let src = &self.files[f];
            self.model.diagnostics.push(DomainDiagnostic {
                file: src.rel.clone(),
                line: line(&src.text, pos),
                message,
            });
            self.model.complete &= !drops;
        }
        archguard_core::domain_model::link_integration_events(&mut self.model);
        let m = &mut self.model;
        m.rules.sort();
        m.validations.sort();
        m.entities.sort();
        m.relations.sort();
        m.relations.dedup();
        m.state_machines.sort();
        m.use_cases.sort();
        m.events.sort();
        m.diagnostics.sort();
        m.diagnostics.dedup();
        self.model
    }
}

fn marker_kind(decl: &Decl) -> Option<EntityKind> {
    let name = decl.name.as_str();
    if !matches!(decl.kw, "class" | "record")
        || AGGREGATE_BASES.contains(&name)
        || AGGREGATE_ANNOTATIONS.contains(&name)
    {
        return None;
    }
    let has = |n: &str| decl.annotations.iter().any(|a| a == n);
    let based = decl
        .bases
        .iter()
        .any(|b| AGGREGATE_BASES.contains(&simple(b).as_str()));
    if based || AGGREGATE_ANNOTATIONS.iter().any(|a| has(a)) {
        Some(EntityKind::AggregateRoot)
    } else if has(ENTITY_ANNOTATION) {
        Some(EntityKind::Entity)
    } else if has(VALUE_OBJECT_ANNOTATION) {
        Some(EntityKind::ValueObject)
    } else {
        None
    }
}

/// Qualifies the ids of entities sharing a name; one diagnostic each.
fn qualify(ents: &mut [Ent], decls: &[Decl]) -> Pending {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for e in ents.iter() {
        *count.entry(e.name.clone()).or_default() += 1;
    }
    let mut out = Pending::new();
    for e in ents.iter_mut().filter(|e| count[&e.name] > 1) {
        let d = &decls[e.decl];
        e.id = format!("{}.{}", d.scope, e.name);
        let message = format!(
            "Several entities are named {}; ids are package-qualified ({}).",
            e.name, e.id
        );
        out.push((d.file, d.pos, message, false));
    }
    out
}

/// The events, and where each listener method is declared (sorted).
fn domain_events(
    decls: &[Decl],
    files: &[Src],
    raised: BTreeMap<String, BTreeSet<String>>,
) -> (Vec<DomainEvent>, Vec<EventHandlerSite>) {
    let mut handled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut sites = vec![];
    for d in decls {
        let src = &files[d.file];
        for (m, _, params, ..) in d.methods() {
            if !m.has_any(&EVENT_LISTENERS) {
                continue;
            }
            if let Some(t) = param_types(&src.text, params, src.kotlin)
                .into_iter()
                .next()
            {
                sites.push(EventHandlerSite {
                    event: t.clone(),
                    handler: d.name.clone(),
                    file: src.rel.clone(),
                    line: line(&src.text, params.0),
                });
                handled.entry(t).or_default().insert(d.name.clone());
            }
        }
    }
    sites.sort();
    let types: BTreeSet<&String> = raised.keys().chain(handled.keys()).collect();
    let events = types
        .into_iter()
        .map(|t| DomainEvent {
            type_name: t.clone(),
            raised_by: (raised.get(t)).map_or(vec![], |x| x.iter().cloned().collect()),
            handled_by: (handled.get(t)).map_or(vec![], |x| x.iter().cloned().collect()),
            source: SOURCE.into(),
        })
        .collect();
    (events, sites)
}

fn use_case_kind(name: &str) -> UseCaseKind {
    if name.contains("Command") {
        UseCaseKind::Command
    } else if name.contains("Query") {
        UseCaseKind::Query
    } else {
        UseCaseKind::Handler
    }
}

/// `getX`/`setX`/`isX` of a field `x`, or `equals`/`hashCode`/`toString`.
fn accessor(name: &str, fields: &[Field]) -> bool {
    let rest = ["get", "set", "is"].iter().find_map(|p| {
        name.strip_prefix(p)
            .filter(|r| r.starts_with(char::is_uppercase))
    });
    NOT_CALLS.contains(&name)
        || rest.is_some_and(|r| fields.iter().any(|f| capitalize(&f.name) == r))
}

fn capitalize(x: &str) -> String {
    let mut c = x.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// `Entity.m` for each `.m(` in `body` whose `m` one entity owns, plus
/// (offset, message) for names several entities own.
fn calls_in(
    s: &str,
    body: Span,
    owners: &BTreeMap<&str, BTreeSet<&str>>,
) -> (Vec<String>, Vec<(usize, String)>) {
    let b = s.as_bytes();
    let mut calls = BTreeSet::new();
    let mut ambiguous = vec![];
    for (pos, w) in idents(s, body.0, body.1) {
        let dotted = prev_nonws(b, pos).is_some_and(|k| b[k] == b'.');
        let call = next_nonws(b, pos + w.len()).is_some_and(|k| b[k] == b'(');
        let Some(set) = owners.get(w).filter(|_| dotted && call) else {
            continue;
        };
        let list: Vec<&str> = set.iter().copied().collect();
        match list.as_slice() {
            [one] => {
                calls.insert(format!("{one}.{w}"));
            }
            _ => ambiguous.push((
                pos,
                format!(
                    "call .{w}( matches methods of several entities ({}); not recorded.",
                    list.join(", ")
                ),
            )),
        }
    }
    (calls.into_iter().collect(), ambiguous)
}

/// Type declarations, imports and annotations.
mod decls {
    use super::*;

    /// Declarations of one file (file index left 0).
    pub(super) fn parse(s: &str, kotlin: bool, dir: &str) -> Vec<Decl> {
        let toks = idents(s, 0, s.len());
        let scope = package(s, &toks).unwrap_or_else(|| dir.to_string());
        (0..toks.len())
            .filter_map(|t| decl_at(s, &toks, t, kotlin))
            .map(|d| {
                let members = match (d.body, kotlin) {
                    (Some(body), false) => members::java(s, body, &d.name),
                    (body, true) => members::kotlin(s, d.params, body),
                    (None, false) => vec![],
                };
                Decl {
                    scope: scope.clone(),
                    members,
                    ..d
                }
            })
            .collect()
    }

    fn package(s: &str, toks: &[(usize, &str)]) -> Option<String> {
        let &(p, w) = toks.iter().find(|x| x.1 == "package")?;
        let rest = &s[p + w.len()..];
        Some(
            rest[..rest.find([';', '\n']).unwrap_or(rest.len())]
                .trim()
                .to_string(),
        )
    }

    /// `import [static] a.b.C.*;` → `a.b.C.*`.
    pub(super) fn imports(s: &str) -> Vec<String> {
        s.lines()
            .filter_map(|l| l.trim().strip_prefix("import "))
            .map(|l| {
                let l = l.trim().trim_end_matches(';').trim();
                let l = l.strip_prefix("static ").unwrap_or(l).trim();
                l.split_whitespace().next().unwrap_or("").to_string()
            })
            .collect()
    }

    fn kind_of(s: &str, pos: usize, word: &str, kotlin: bool) -> Option<(&'static str, bool)> {
        let b = s.as_bytes();
        if prev_nonws(b, pos).is_some_and(|k| matches!(b[k], b'.' | b':' | b'@')) {
            return None;
        }
        let before = prev_word(s, pos);
        match (word, kotlin) {
            ("class", true) => match before {
                "enum" => Some(("enum", false)),
                "annotation" => None,
                _ => Some(("class", before == "data")),
            },
            ("object", true) if before != "companion" => Some(("object", false)),
            ("class", false) => Some(("class", false)),
            ("enum", false) => Some(("enum", false)),
            ("record", false) => Some(("record", false)),
            ("interface", _) => Some(("interface", false)),
            _ => None,
        }
    }

    fn decl_at(s: &str, toks: &[(usize, &str)], t: usize, kotlin: bool) -> Option<Decl> {
        let (pos, word) = toks[t];
        let (kw, data) = kind_of(s, pos, word, kotlin)?;
        let &(npos, name) = toks.get(t + 1).filter(|x| gap(s, toks[t], x.0))?;
        let b = s.as_bytes();
        let after = next_nonws(b, npos + name.len()).map(|k| b[k]);
        if kw == "record" && !matches!(after, Some(b'(' | b'<')) {
            return None;
        }
        let (params, bases, body) = if kotlin {
            kotlin_header(s, npos + name.len())
        } else {
            java_header(s, npos + name.len())
        };
        Some(Decl {
            file: 0,
            kw,
            data,
            name: name.into(),
            scope: String::new(),
            pos,
            annotations: annotations_before(s, pos).0,
            bases,
            params,
            body,
            members: vec![],
        })
    }

    fn skip_generics(b: &[u8], i: usize) -> usize {
        match next_nonws(b, i).filter(|&k| b[k] == b'<') {
            Some(k) => matching(b, k).map_or(i, |e| e + 1),
            None => i,
        }
    }

    fn parens(b: &[u8], i: usize) -> Option<(Span, usize)> {
        let k = next_nonws(b, i).filter(|&k| b[k] == b'(')?;
        let e = matching(b, k)?;
        Some(((k + 1, e), e + 1))
    }

    /// Record components, `extends`/`implements` list and body after a name.
    fn java_header(s: &str, i: usize) -> (Option<Span>, Vec<String>, Option<Span>) {
        let b = s.as_bytes();
        let mut i = skip_generics(b, i);
        let params = parens(b, i).map(|(p, e)| {
            i = e;
            p
        });
        let stop = (i..s.len())
            .find(|&j| matches!(b[j], b'{' | b';'))
            .unwrap_or(s.len());
        let text = format!(" {} ", collapse(&s[i..stop]));
        let text = &text[..text.find(" permits ").unwrap_or(text.len())];
        let text = text.replace(" extends ", ",").replace(" implements ", ",");
        let bases = split_top(&text, ",")
            .into_iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        let body = (stop < s.len() && b[stop] == b'{')
            .then(|| matching(b, stop).map(|c| (stop, c)))
            .flatten();
        (params, bases, body)
    }

    /// Primary constructor, supertypes and body after a Kotlin class name.
    fn kotlin_header(s: &str, i: usize) -> (Option<Span>, Vec<String>, Option<Span>) {
        let b = s.as_bytes();
        let mut i = skip_generics(b, i);
        let mut k = next_nonws(b, i);
        while let Some(at) = k.filter(|&at| b[at] == b'@' || is_id(b[at])) {
            let word = &s[at..ident_end(b, at + usize::from(b[at] == b'@'))];
            if !(word.starts_with('@') || word == "constructor" || MODIFIERS.contains(&word)) {
                break;
            }
            i = at + word.len();
            if word.starts_with('@')
                && let Some((_, e)) = parens(b, i)
            {
                i = e;
            }
            k = next_nonws(b, i);
        }
        let params = parens(b, i).map(|(p, e)| {
            i = e;
            p
        });
        let mut bases = vec![];
        if let Some(colon) = next_nonws(b, i).filter(|&c| b[c] == b':') {
            let stop = bases_end(b, colon + 1);
            bases = split_top(&s[colon + 1..stop], ",")
                .into_iter()
                .map(|x| x.split(" by ").next().unwrap_or(x).trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
            i = stop;
        }
        let body = next_nonws(b, i)
            .filter(|&o| b[o] == b'{')
            .and_then(|o| matching(b, o).map(|c| (o, c)));
        (params, bases, body)
    }

    /// End of a Kotlin supertype list: `{` or a line break that does not
    /// continue the list.
    fn bases_end(b: &[u8], from: usize) -> usize {
        let mut depth = 0i32;
        for k in from..b.len() {
            match b[k] {
                b'(' | b'<' | b'[' => depth += 1,
                b')' | b'>' | b']' => depth -= 1,
                b'{' if depth == 0 => return k,
                b'\n' if depth == 0 => {
                    let prev = prev_nonws(b, k).map(|p| b[p]);
                    let next = next_nonws(b, k).map(|p| b[p]);
                    if !matches!(prev, Some(b',' | b':')) && !matches!(next, Some(b',')) {
                        return k;
                    }
                }
                _ => {}
            }
        }
        b.len()
    }

    /// Annotation simple names and modifiers directly before offset `pos`
    /// (walking back over `@a.b.X(…)`, `@field:X` and modifiers).
    pub(super) fn annotations_before(s: &str, pos: usize) -> (Vec<String>, Vec<String>) {
        let b = s.as_bytes();
        let (mut annotations, mut modifiers) = (vec![], vec![]);
        let mut k = pos;
        while let Some(mut p) = prev_nonws(b, k) {
            let args = b[p] == b')';
            if args {
                let Some(open) = matching_back(b, p) else {
                    break;
                };
                let Some(q) = prev_nonws(b, open).filter(|&q| is_id(b[q])) else {
                    break;
                };
                p = q;
            }
            if !is_id(b[p]) {
                break;
            }
            let word = prev_word(s, p + 1);
            let start = p + 1 - word.len();
            match annotation_start(s, start) {
                Some(at) => {
                    annotations.push(word.to_string());
                    k = at;
                }
                None if !args && MODIFIERS.contains(&word) => {
                    modifiers.push(word.to_string());
                    k = start;
                }
                None => break,
            }
        }
        (annotations, modifiers)
    }

    /// The `@` of an annotation whose last name segment starts at `start`.
    fn annotation_start(s: &str, start: usize) -> Option<usize> {
        let b = s.as_bytes();
        let mut j = start;
        loop {
            match j.checked_sub(1).map(|p| b[p]) {
                Some(b'@') => return Some(j - 1),
                Some(b'.' | b':') => {
                    let w = prev_word(s, j - 1);
                    if w.is_empty() || b[j - 2].is_ascii_whitespace() {
                        return None;
                    }
                    j = j - 1 - w.len();
                }
                _ => return None,
            }
        }
    }
}

/// Members of a type body.
mod members {
    use super::*;
    use decls::annotations_before;

    const NOT_MEMBERS: [&str; 5] = ["class", "interface", "enum", "record", "new"];

    pub(super) enum Kind {
        Data {
            ty: String,
            name: String,
            /// The initializer value.
            init: Option<Span>,
        },
        Method {
            name: String,
            params: Span,
            body: Span,
            ctor: bool,
        },
    }

    pub(super) struct Member {
        pub(super) kind: Kind,
        pub(super) public: bool,
        pub(super) annotations: Vec<String>,
    }
    impl Member {
        pub(super) fn has(&self, name: &str) -> bool {
            self.annotations.iter().any(|a| a == name)
        }
        pub(super) fn has_any(&self, names: &[&str]) -> bool {
            names.iter().any(|n| self.has(n))
        }
    }

    /// Fields, methods and constructors of the Java body `{…}` at `open..close`.
    pub(super) fn java(s: &str, (open, close): Span, class: &str) -> Vec<Member> {
        let b = s.as_bytes();
        let mut out = vec![];
        let mut start = open + 1;
        let mut depth = 0i32;
        let mut i = start;
        while i < close {
            match b[i] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'{' if depth == 0 => {
                    let end = matching(b, i).unwrap_or(close);
                    out.extend(java_member(s, (start, i), Some((i, end)), class));
                    start = end + 1;
                    i = start;
                    continue;
                }
                b';' if depth == 0 => {
                    out.extend(java_member(s, (start, i), None, class));
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    fn java_member(s: &str, (a, z): Span, block: Option<Span>, class: &str) -> Option<Member> {
        let raw = &s[a..z];
        let annotations = annotation_names(raw);
        let header = blank_annotations(raw);
        let decl = &header[..first_assign(&header).unwrap_or(header.len())];
        let words: Vec<&str> = idents(decl, 0, decl.len())
            .into_iter()
            .map(|x| x.1)
            .collect();
        if words.is_empty() || words.iter().any(|w| NOT_MEMBERS.contains(w)) {
            return None;
        }
        let kind = match find_top(decl, "(") {
            Some(paren) => {
                let (o, c) = block?;
                let close = matching(header.as_bytes(), paren)?;
                let name = idents(decl, 0, paren).last()?.1.to_string();
                if name == class && words.contains(&"static") {
                    return None;
                }
                Kind::Method {
                    ctor: name == class,
                    name,
                    params: (a + paren + 1, a + close),
                    body: (o + 1, c),
                }
            }
            None => {
                let (ty, name) = data(decl)?;
                let init = first_assign(&header).map(|e| (a + e + 1, z));
                Kind::Data { ty, name, init }
            }
        };
        Some(Member {
            kind,
            public: words.contains(&"public"),
            annotations,
        })
    }

    /// Simple names of the annotations in `raw`.
    fn annotation_names(raw: &str) -> Vec<String> {
        let b = raw.as_bytes();
        (0..b.len())
            .filter(|&k| b[k] == b'@')
            .filter_map(|k| {
                let end = (k + 1..b.len())
                    .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
                    .unwrap_or(b.len());
                let name = simple(&raw[k + 1..end]);
                (!name.is_empty()).then_some(name)
            })
            .collect()
    }

    /// `raw` with every `@a.B(…)` replaced by spaces; offsets unchanged.
    fn blank_annotations(raw: &str) -> String {
        let mut b = raw.as_bytes().to_vec();
        let mut k = 0;
        while k < b.len() {
            if b[k] != b'@' {
                k += 1;
                continue;
            }
            let mut end = (k + 1..b.len())
                .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
                .unwrap_or(b.len());
            if let Some(o) = next_nonws(&b, end).filter(|&o| b[o] == b'(') {
                end = matching(&b, o).map_or(end, |c| c + 1);
            }
            for c in &mut b[k..end] {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
            k = end;
        }
        String::from_utf8(b).unwrap_or_default()
    }

    pub(super) fn skip_annotations(s: &str) -> &str {
        let mut s = s.trim_start();
        while s.starts_with('@') {
            let b = s.as_bytes();
            let mut end = (1..b.len())
                .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
                .unwrap_or(b.len());
            if let Some(o) = next_nonws(b, end).filter(|&o| b[o] == b'(') {
                end = matching(b, o).map_or(end, |c| c + 1);
            }
            s = s[end..].trim_start();
        }
        s
    }

    /// Members of a Kotlin class: primary-constructor properties and the
    /// `val`/`var`/`fun`/`init`/`constructor` at depth 0 of the body.
    pub(super) fn kotlin(s: &str, params: Option<Span>, body: Option<Span>) -> Vec<Member> {
        let b = s.as_bytes();
        let mut out = vec![];
        if let Some((a, z)) = params {
            for part in split_top(&s[a..z], ",") {
                let from = offset(s, part);
                let to = from + part.len();
                if let Some(&(k, _)) = idents(s, from, to)
                    .iter()
                    .find(|x| matches!(x.1, "val" | "var"))
                {
                    out.extend(kotlin_property(s, k, to));
                }
            }
        }
        let Some((open, close)) = body else {
            return out;
        };
        let mut depth = 0i32;
        let mut k = open + 1;
        while k < close {
            if depth == 0 && is_id(b[k]) && !is_id(b[k - 1]) {
                let end = ident_end(b, k);
                out.extend(match &s[k..end] {
                    "val" | "var" => kotlin_property(s, k, close),
                    "fun" => kotlin_fun(s, k, close),
                    "init" | "constructor" => kotlin_ctor(s, k, end, close),
                    _ => None,
                });
                k = end;
                continue;
            }
            match b[k] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            k += 1;
        }
        out
    }

    fn private(modifiers: &[String]) -> bool {
        modifiers
            .iter()
            .any(|m| matches!(m.as_str(), "private" | "protected" | "internal"))
    }

    /// `val|var name[: Type] [= init]` with the keyword at `kw`, before `limit`.
    fn kotlin_property(s: &str, kw: usize, limit: usize) -> Option<Member> {
        let b = s.as_bytes();
        let n = next_nonws(b, kw + 3).filter(|&k| k < limit && is_id(b[k]))?;
        let name_end = ident_end(b, n);
        let colon = next_nonws(b, name_end).filter(|&k| k < limit && b[k] == b':');
        let type_end = colon.map_or(name_end, |c| type_end(b, c + 1, limit));
        let init = next_nonws(b, type_end)
            .filter(|&k| k < limit && b[k] == b'=' && b.get(k + 1) != Some(&b'='))
            .map(|k| (k + 1, states::value_end(b, k + 1, limit, true)));
        let ty: String = match colon {
            Some(c) => s[c + 1..type_end]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
            // `var status = OrderStatus.NEW`: the type is the qualifier.
            None => {
                let v = collapse(&s[init?.0..init?.1]);
                let (t, c) = v.rsplit_once('.')?;
                let plain = |x: &str| !x.is_empty() && x.bytes().all(is_id);
                (plain(t) && plain(c) && t.starts_with(char::is_uppercase))
                    .then(|| t.to_string())?
            }
        };
        let (annotations, modifiers) = annotations_before(s, kw);
        Some(Member {
            kind: Kind::Data {
                ty,
                name: s[n..name_end].to_string(),
                init,
            },
            public: !private(&modifiers),
            annotations,
        })
    }

    /// End of a Kotlin type after `:`.
    fn type_end(b: &[u8], from: usize, limit: usize) -> usize {
        let mut depth = 0i32;
        for k in from..limit {
            match b[k] {
                b'(' | b'<' | b'[' => depth += 1,
                b'>' if b[k - 1] == b'-' => {}
                b')' | b'>' | b']' if depth > 0 => depth -= 1,
                b')' | b'>' | b']' | b'=' | b'\n' | b';' | b'{' | b',' if depth == 0 => return k,
                b'b' if depth == 0 && b[k..].starts_with(b"by ") && !is_id(b[k - 1]) => return k,
                _ => {}
            }
        }
        limit
    }

    /// `fun [<T>] [Recv.]name(params)[: R] { … }` or `= expr`.
    fn kotlin_fun(s: &str, kw: usize, limit: usize) -> Option<Member> {
        let b = s.as_bytes();
        let open = (kw..limit).find(|&j| b[j] == b'(')?;
        let name = idents(s, kw + 3, open).last()?.1.to_string();
        let close = matching(b, open)?;
        let body = kotlin_body(s, close + 1, limit)?;
        let (annotations, modifiers) = annotations_before(s, kw);
        Some(Member {
            kind: Kind::Method {
                name,
                params: (open + 1, close),
                body,
                ctor: false,
            },
            public: !private(&modifiers),
            annotations,
        })
    }

    /// `init { … }` or `constructor(params) [: this(…)] { … }`.
    fn kotlin_ctor(s: &str, kw: usize, end: usize, limit: usize) -> Option<Member> {
        let b = s.as_bytes();
        let params = match next_nonws(b, end).filter(|&o| b[o] == b'(') {
            Some(o) => (o + 1, matching(b, o)?),
            None => (end, end),
        };
        let body = kotlin_body(s, params.1 + 1, limit)?;
        Some(Member {
            kind: Kind::Method {
                name: s[kw..end].to_string(),
                params,
                body,
                ctor: true,
            },
            public: true,
            annotations: vec![],
        })
    }

    /// The block or expression body after a Kotlin signature, skipping a
    /// return type or delegation call; `None` at a line end without one.
    fn kotlin_body(s: &str, from: usize, limit: usize) -> Option<Span> {
        let b = s.as_bytes();
        let mut depth = 0i32;
        for k in from..limit {
            match b[k] {
                b'(' | b'<' | b'[' => depth += 1,
                b'>' if b[k - 1] == b'-' => {}
                b')' | b'>' | b']' => depth -= 1,
                b'{' if depth == 0 => return Some((k + 1, matching(b, k)?)),
                b'=' if depth == 0 => {
                    return Some((k + 1, states::value_end(b, k + 1, limit, true)));
                }
                b'\n' if depth == 0 => {
                    let next = next_nonws(b, k).map(|j| b[j]);
                    let prev = prev_nonws(b, k).map(|j| b[j]);
                    if !matches!(next, Some(b'{' | b'=' | b':')) && prev != Some(b':') {
                        return None;
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Java record components as fields.
    pub(super) fn record_components(s: &str, (a, z): Span) -> Vec<Field> {
        split_top(&s[a..z], ",")
            .into_iter()
            .filter_map(|p| data(&blank_annotations(p)))
            .map(|(ty, n)| field(&ty, &n, false))
            .collect()
    }

    /// Simple types of the parameters at `params` (Java `T x`, Kotlin `x: T`).
    pub(super) fn param_types(s: &str, (a, z): Span, kotlin: bool) -> Vec<String> {
        split_top(&s[a..z], ",")
            .into_iter()
            .filter(|p| !p.trim().is_empty())
            .filter_map(|p| {
                let p = blank_annotations(p);
                if kotlin {
                    let t = p.split_once(':')?.1;
                    Some(simple(t.split('=').next().unwrap_or(t)))
                } else {
                    data(&p).map(|(ty, _)| simple(&ty))
                }
            })
            .collect()
    }

    /// `[modifiers] Type name` → (type without whitespace, name).
    pub(super) fn data(decl: &str) -> Option<(String, String)> {
        let decl = decl.trim();
        if split_top(decl, ",").len() > 1 {
            return None;
        }
        let name_start = decl
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .map_or(0, |k| k + 1);
        let name = &decl[name_start..];
        let mut ty = decl[..name_start].trim();
        while let Some((first, rest)) = ty.split_once(char::is_whitespace)
            && MODIFIERS.contains(&first)
        {
            ty = rest.trim_start();
        }
        let ty: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        let valid = !name.is_empty()
            && !ty.is_empty()
            && !MODIFIERS.contains(&ty.as_str())
            && !name.starts_with(|c: char| c.is_ascii_digit());
        valid.then(|| (ty, name.to_string()))
    }

    /// A field; `to_many` (`@OneToMany`/`@ManyToMany`) makes a generic type a
    /// collection of its argument.
    pub(super) fn field(ty: &str, name: &str, to_many: bool) -> Field {
        let t = ty.trim_end_matches('?');
        let inner = t
            .find('<')
            .filter(|&lt| {
                t.ends_with('>') && (to_many || COLLECTIONS.contains(&simple(&t[..lt]).as_str()))
            })
            .map(|lt| &t[lt + 1..t.len() - 1])
            .filter(|inner| !inner.contains(','))
            .or_else(|| t.strip_suffix("[]"));
        Field {
            name: name.into(),
            type_name: inner.unwrap_or(t).trim_end_matches('?').to_string(),
            collection: inner.is_some(),
        }
    }
}

/// State assignments, their guards and raised events inside method bodies.
mod states {
    use super::*;

    pub(super) enum Step {
        Initial(String),
        NonLiteral(String),
        Transition {
            from: Vec<String>,
            to: String,
            guard: Option<String>,
            unread: Option<&'static str>,
        },
    }

    enum Guard {
        Any,
        /// States for which the transition may start, and the condition text.
        From(BTreeSet<String>, String),
        Unread(Option<String>, &'static str),
    }

    enum Head {
        If(Span),
        /// `switch (…)`/`when (…)`, or a `when` without subject.
        Switch(Option<Span>),
        /// `else`, with the condition of its braced `if` when found.
        Else(Option<Span>),
        Other,
    }

    /// One state field of one entity.
    pub(super) struct Rules<'a> {
        s: &'a str,
        field: &'a str,
        enum_type: &'a str,
        states: &'a [String],
        all: BTreeSet<String>,
        /// Constants usable without qualifier through an import.
        bare: BTreeSet<String>,
        helpers: &'a BTreeSet<&'a str>,
    }

    impl<'a> Rules<'a> {
        pub(super) fn new(
            s: &'a str,
            (field, enum_type): (&'a str, &'a str),
            states: &'a [String],
            helpers: &'a BTreeSet<&'a str>,
            imports: &[String],
        ) -> Self {
            let all: BTreeSet<String> = states.iter().cloned().collect();
            let imported = |c: &str| {
                imports.iter().any(|i| {
                    let mut parts = i.rsplit('.');
                    let (last, owner) = (parts.next(), parts.next());
                    owner == Some(enum_type) && (last == Some(c) || last == Some("*"))
                })
            };
            let bare = all.iter().filter(|c| imported(c)).cloned().collect();
            Self {
                s,
                field,
                enum_type,
                states,
                all,
                bare,
                helpers,
            }
        }

        /// What the assignment at `stmt` with value `rhs` means.
        pub(super) fn step(&self, body: Span, stmt: usize, rhs: Span, ctor: bool) -> Step {
            let value = collapse(&self.s[rhs.0..rhs.1]);
            let Some(to) = self.literal(&value) else {
                return Step::NonLiteral(value);
            };
            if ctor {
                return Step::Initial(to);
            }
            let (from, guard, unread) = match self.guard(body, stmt) {
                Guard::Any => (vec![], None, None),
                Guard::From(set, text) => (self.ordered(&set), Some(text), None),
                Guard::Unread(text, why) => (vec![], text, Some(why)),
            };
            Step::Transition {
                from,
                to,
                guard,
                unread,
            }
        }

        /// `E.V` or an imported bare `V`.
        pub(super) fn literal(&self, value: &str) -> Option<String> {
            match value.rsplit_once('.') {
                Some((t, v)) => {
                    (simple(t) == self.enum_type && self.all.contains(v)).then(|| v.to_string())
                }
                None => self.bare.contains(value).then(|| value.to_string()),
            }
        }

        fn ordered(&self, set: &BTreeSet<String>) -> Vec<String> {
            (self.states.iter())
                .filter(|x| set.contains(*x))
                .cloned()
                .collect()
        }

        fn guard(&self, body: Span, stmt: usize) -> Guard {
            let stack = open_blocks(self.s.as_bytes(), body.0, stmt);
            if let Some(g) = self.enclosing(stmt, &stack) {
                return g;
            }
            let start = stack.last().map_or(body.0, |&k| k + 1);
            self.preceding_exit(start, stmt)
                .map_or(Guard::Any, |c| self.read(c, true))
        }

        /// The guard from the nearest enclosing `if`, `else` or `switch`/`when`
        /// on the field.
        fn enclosing(&self, stmt: usize, stack: &[usize]) -> Option<Guard> {
            let on_field = |c: Span| mentions(&self.s[c.0..c.1], self.field);
            for h in std::iter::once(stmt).chain(stack.iter().rev().copied()) {
                match head(self.s, h) {
                    Head::If(c) if on_field(c) => return Some(self.read(c, false)),
                    Head::Else(Some(c)) if on_field(c) => return Some(self.read(c, true)),
                    Head::Switch(Some(c)) if on_field(c) => {
                        let text = collapse(&self.s[c.0..c.1]);
                        return Some(Guard::Unread(Some(text), "switch/when"));
                    }
                    Head::Switch(None) => return Some(Guard::Unread(None, "when")),
                    Head::Else(None) => {
                        return Some(Guard::Unread(None, "else without a braced if"));
                    }
                    _ => {}
                }
            }
            None
        }

        /// The condition of the nearest preceding sibling `if` on the field
        /// whose block exits.
        fn preceding_exit(&self, start: usize, stmt: usize) -> Option<Span> {
            let b = self.s.as_bytes();
            let mut found = None;
            let mut depth = 0;
            let mut k = start;
            while k < stmt {
                if depth == 0
                    && let Some((cond, block)) = if_at(self.s, k).filter(|x| x.1.1 < stmt)
                {
                    if mentions(&self.s[cond.0..cond.1], self.field)
                        && exits(self.s, block, self.helpers)
                    {
                        found = Some(cond);
                    }
                    k = block.1 + 1;
                    continue;
                }
                match b[k] {
                    b'{' | b'(' => depth += 1,
                    b'}' | b')' => depth -= 1,
                    _ => {}
                }
                k += 1;
            }
            found
        }

        /// `negate` for an exit or `else` guard: the text then reads as the
        /// allowed condition, the literal `!(…)` of the source, never simplified.
        fn read(&self, cond: Span, negate: bool) -> Guard {
            let text = &self.s[cond.0..cond.1];
            let shown = if negate {
                format!("!({})", collapse(text))
            } else {
                collapse(text)
            };
            match self.cond_set(text) {
                Some(set) if negate => {
                    Guard::From(self.all.difference(&set).cloned().collect(), shown)
                }
                Some(set) => Guard::From(set, shown),
                None => Guard::Unread(Some(shown), "condition shape"),
            }
        }

        /// The states for which `cond` holds, when it is one of the read shapes.
        fn cond_set(&self, cond: &str) -> Option<BTreeSet<String>> {
            let c = unparen(cond);
            let ors = split_top(c, "||");
            if ors.len() > 1 {
                return ors
                    .into_iter()
                    .map(|p| self.cmp(p).filter(|x| x.0).map(|x| x.1))
                    .collect();
            }
            let ands = split_top(c, "&&");
            if ands.len() > 1 {
                let removed: BTreeSet<String> = ands
                    .into_iter()
                    .map(|p| self.cmp(p).filter(|x| !x.0).map(|x| x.1))
                    .collect::<Option<_>>()?;
                return Some(self.all.difference(&removed).cloned().collect());
            }
            let (eq, v) = self.cmp(c)?;
            let one = BTreeSet::from([v]);
            Some(if eq {
                one
            } else {
                self.all.difference(&one).cloned().collect()
            })
        }

        /// `field == E.A` → (true, A); `field != E.A` → (false, A).
        fn cmp(&self, part: &str) -> Option<(bool, String)> {
            let part = unparen(part);
            let (op, eq) = if part.contains("==") {
                ("==", true)
            } else {
                ("!=", false)
            };
            let (l, r) = part.split_once(op)?;
            let (l, r) = (l.trim(), r.trim());
            let lit = if self.is_field(l) {
                r
            } else if self.is_field(r) {
                l
            } else {
                return None;
            };
            self.literal(lit).map(|v| (eq, v))
        }

        fn is_field(&self, x: &str) -> bool {
            x == self.field || x.strip_prefix("this.") == Some(self.field)
        }
    }

    /// Offsets of the `{` still open at `to`, scanning from `from`.
    fn open_blocks(b: &[u8], from: usize, to: usize) -> Vec<usize> {
        let mut stack = vec![];
        for (k, &c) in b.iter().enumerate().take(to).skip(from) {
            match c {
                b'{' => stack.push(k),
                b'}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
        stack
    }

    /// What precedes the block or statement starting at `at`.
    fn head(s: &str, at: usize) -> Head {
        let b = s.as_bytes();
        let Some(k) = prev_nonws(b, at) else {
            return Head::Other;
        };
        if b[k] == b')'
            && let Some(open) = matching_back(b, k)
        {
            let c = (open + 1, k);
            return match prev_word(s, open) {
                "if" => Head::If(c),
                "switch" | "when" => Head::Switch(Some(c)),
                _ => Head::Other,
            };
        }
        match prev_word(s, at) {
            "else" => Head::Else(else_cond(s, k.saturating_sub(3))),
            "when" => Head::Switch(None),
            _ => Head::Other,
        }
    }

    /// The condition of the braced `if` an `else` at `else_start` belongs to.
    fn else_cond(s: &str, else_start: usize) -> Option<Span> {
        let b = s.as_bytes();
        let close = prev_nonws(b, else_start).filter(|&c| b[c] == b'}')?;
        match head(s, matching_back(b, close)?) {
            Head::If(c) => Some(c),
            _ => None,
        }
    }

    /// An `if` keyword at `k`: (condition, block or single statement).
    fn if_at(s: &str, k: usize) -> Option<(Span, Span)> {
        let b = s.as_bytes();
        let keyword = b[k..].starts_with(b"if")
            && (k == 0 || !is_id(b[k - 1]))
            && !b.get(k + 2).is_some_and(|&c| is_id(c));
        if !keyword {
            return None;
        }
        let open = next_nonws(b, k + 2).filter(|&o| b[o] == b'(')?;
        let close = matching(b, open)?;
        let first = next_nonws(b, close + 1)?;
        let end = if b[first] == b'{' {
            matching(b, first)?
        } else {
            statement_end(b, first)?
        };
        Some(((open + 1, close), (first, end)))
    }

    /// `;` or (Kotlin) a line break at depth 0 from `from`.
    fn statement_end(b: &[u8], from: usize) -> Option<usize> {
        let mut depth = 0i32;
        for (k, &c) in b.iter().enumerate().skip(from) {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b';' | b'\n' if depth <= 0 => return Some(k),
                _ => {}
            }
        }
        None
    }

    /// True when `range` throws, returns or calls a method that only throws.
    fn exits(s: &str, range: Span, helpers: &BTreeSet<&str>) -> bool {
        idents(s, range.0, range.1)
            .iter()
            .any(|(_, w)| matches!(*w, "throw" | "return") || helpers.contains(w))
    }

    /// Assignments `field = …` / `this.field = …` at statement start in `body`:
    /// (statement start, value).
    pub(super) fn assignments(
        s: &str,
        body: Span,
        field: &str,
        kotlin: bool,
    ) -> Vec<(usize, Span)> {
        idents(s, body.0, body.1)
            .into_iter()
            .filter(|x| x.1 == field)
            .filter_map(|(pos, _)| {
                let b = s.as_bytes();
                let stmt = statement_start(s, body, pos)?;
                let eq = next_nonws(b, pos + field.len())
                    .filter(|&e| b[e] == b'=' && b.get(e + 1) != Some(&b'='))?;
                Some((stmt, (eq + 1, value_end(b, eq + 1, body.1, kotlin))))
            })
            .collect()
    }

    /// Start of the statement when the identifier at `pos` (or `this.` before
    /// it) begins one directly inside a block, not inside call arguments.
    fn statement_start(s: &str, body: Span, pos: usize) -> Option<usize> {
        let b = s.as_bytes();
        let mut stmt = pos;
        if let Some(dot) = prev_nonws(b, pos).filter(|&k| b[k] == b'.') {
            if prev_word(s, dot) != "this" {
                return None;
            }
            stmt = prev_nonws(b, dot)? - 3;
        }
        if innermost_open(b, body.0, stmt).is_some_and(|o| b[o] != b'{') {
            return None;
        }
        let line_start = s[..stmt].rfind('\n').map_or(0, |n| n + 1);
        let starts = match prev_nonws(b, stmt) {
            None => true,
            Some(k) => {
                k < body.0
                    || k < line_start
                    || matches!(b[k], b';' | b'{' | b'}' | b')' | b':')
                    || (b[k] == b'>' && k > 0 && b[k - 1] == b'-')
                    || prev_word(s, stmt) == "else"
            }
        };
        starts.then_some(stmt)
    }

    /// The innermost bracket still open at `to`, scanning from `from`.
    fn innermost_open(b: &[u8], from: usize, to: usize) -> Option<usize> {
        let mut stack = vec![];
        for (k, &c) in b.iter().enumerate().take(to).skip(from) {
            match c {
                b'(' | b'[' | b'{' => stack.push(k),
                b')' | b']' | b'}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
        stack.pop()
    }

    /// End of an assigned value: `;`, `,`, a closing bracket or (Kotlin) a
    /// line break at depth 0.
    pub(super) fn value_end(b: &[u8], from: usize, to: usize, kotlin: bool) -> usize {
        let from = next_nonws(b, from).unwrap_or(from).min(to);
        let mut depth = 0i32;
        for (k, &c) in b.iter().enumerate().take(to).skip(from) {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => return k,
                b')' | b']' | b'}' => depth -= 1,
                b';' | b',' if depth == 0 => return k,
                b'\n' if depth == 0 && kotlin => return k,
                _ => {}
            }
        }
        to
    }

    /// Raise calls in `body`: (offset, function, argument text).
    pub(super) fn raise_calls(s: &str, body: Span) -> Vec<(usize, &str, &str)> {
        let b = s.as_bytes();
        idents(s, body.0, body.1)
            .into_iter()
            .filter(|x| RAISE_FUNCTIONS.contains(&x.1))
            .filter_map(|(pos, w)| {
                let dot = prev_nonws(b, pos).filter(|&k| b[k] == b'.');
                if w == "apply" && dot.is_some_and(|d| prev_word(s, d) != APPLY_QUALIFIER) {
                    return None;
                }
                let open = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'(')?;
                let close = matching(b, open).unwrap_or(body.1);
                Some((pos, w, s[open + 1..close].trim()))
            })
            .collect()
    }

    /// The type created for a raise argument: `new X(…)`, Kotlin `X(…)`, or a
    /// variable assigned one in `before` (the method text up to the call).
    pub(super) fn created(s: &str, before: Span, arg: &str) -> Option<String> {
        if let Some(t) = new_type(arg) {
            return Some(t);
        }
        if arg.is_empty() || !arg.bytes().all(is_id) {
            return None;
        }
        let b = s.as_bytes();
        idents(s, before.0, before.1)
            .into_iter()
            .rev()
            .filter(|x| x.1 == arg)
            .find_map(|(p, w)| {
                let eq = next_nonws(b, p + w.len())
                    .filter(|&k| b[k] == b'=' && b.get(k + 1) != Some(&b'='))?;
                new_type(&s[eq + 1..before.1])
            })
    }

    /// `new a.X<…>(` or a constructor call `a.X(` with an upper-case `X`.
    fn new_type(x: &str) -> Option<String> {
        let x = x.trim_start();
        let (rest, new) = match x.strip_prefix("new") {
            Some(r) if r.starts_with(char::is_whitespace) => (r.trim_start(), true),
            _ => (x, false),
        };
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        let t = simple(&rest[..end]);
        let call = rest[end..].trim_start().starts_with(['(', '<']);
        (call && !t.is_empty() && (new || t.starts_with(char::is_uppercase))).then_some(t)
    }
}

mod rich {
    //! Rules, Bean Validation and broker publish sites of the Java extractor
    //! (ADR 0022, Verlauf 2026-11-20). Rules and validations read Java files
    //! only; Kotlin `require`/`check`, `throw X(…)` and `@field:` targets are not
    //! read.
    //!
    //! - Rules: in methods of found entities, `throw new X(…)` with `X` ending in
    //!   `Exception`; `Objects.requireNonNull(v, …)` (condition `v == null`);
    //!   `Assert.m(…)` and `Preconditions.m(…)` whose last argument is a string
    //!   literal: condition `!(c)` for `isTrue`/`state`/`checkArgument`/
    //!   `checkState`, `v == null` for `notNull`/`checkNotNull`, else
    //!   `!Assert.m(v)`. Condition of a throw and message as in C#
    //!   ([`crate::domain_csharp::code`]).
    //! - Validations: the Bean Validation annotations of [`BEAN_VALIDATION`] on
    //!   the fields of any Java class (entities, DTOs, view models).
    //! - Integration events: `r.send(…)` and `r.convertAndSend(…)` where `r`'s
    //!   name contains `kafka`, `rabbit`, `jms`, `template` or `bridge` (any case);
    //!   the payload is the last argument, `new X(…)` or a local variable assigned
    //!   one. A computed payload or a Kotlin constructor call adds nothing.
    use super::*;
    use crate::domain_csharp::code::{
        Code, assigned_news, condition, created_at, message, raw, split_args, top_news,
    };
    use archguard_core::domain_model::{DomainRule, IntegrationEvent, Publisher, Validation};

    const BEAN_VALIDATION: [&str; 22] = [
        "NotNull",
        "NotBlank",
        "NotEmpty",
        "Null",
        "Size",
        "Pattern",
        "Min",
        "Max",
        "DecimalMin",
        "DecimalMax",
        "Digits",
        "Email",
        "Positive",
        "PositiveOrZero",
        "Negative",
        "NegativeOrZero",
        "Past",
        "PastOrPresent",
        "Future",
        "FutureOrPresent",
        "AssertTrue",
        "AssertFalse",
    ];
    const CHECKERS: [&str; 2] = ["Assert", "Preconditions"];
    const PUBLISH: [&str; 2] = ["send", "convertAndSend"];
    const BROKER_RECEIVERS: [&str; 5] = ["kafka", "rabbit", "jms", "template", "bridge"];

    /// One rule site: offset, exception or check, condition, message.
    type Site = (usize, String, Option<String>, Option<String>);

    fn code(src: &Src) -> Code<'_> {
        Code {
            text: &src.text,
            raw: &src.raw,
        }
    }

    /// The span of `part`, a slice of `s`.
    fn span_of(s: &str, part: &str) -> Span {
        let at = part.as_ptr() as usize - s.as_ptr() as usize;
        let trimmed = part.trim_start();
        let at = at + (part.len() - trimmed.len());
        (at, at + trimmed.trim_end().len())
    }

    pub(super) fn rules(x: &Extractor) -> Vec<DomainRule> {
        let mut out = vec![];
        for ent in &x.ents {
            let d = &x.decls[ent.decl];
            let src = &x.files[d.file];
            if src.kotlin {
                continue;
            }
            for (_, name, _, body, _) in d.methods() {
                for (pos, exception, condition, message) in sites(src, body) {
                    out.push(DomainRule {
                        entity: ent.id.clone(),
                        method: format!("{}.{name}", ent.id),
                        condition,
                        message,
                        exception,
                        file: src.rel.clone(),
                        line: line(&src.text, pos),
                        source: SOURCE.into(),
                    });
                }
            }
        }
        out
    }

    fn sites(src: &Src, body: Span) -> Vec<Site> {
        let (s, b, c) = (src.text.as_str(), src.text.as_bytes(), code(src));
        let mut out = vec![];
        for (pos, w) in idents(s, body.0, body.1) {
            if w == "throw" {
                let thrown = created_at(s, pos + w.len()).filter(|t| t.0.ends_with("Exception"));
                if let Some((ex, args)) = thrown {
                    out.push((pos, ex, condition(c, body, pos), message(c, args)));
                }
                continue;
            }
            let Some(dot) = prev_nonws(b, pos).filter(|&k| b[k] == b'.') else {
                continue;
            };
            let receiver = prev_word(s, dot);
            let open = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'(');
            let Some(close) = open.and_then(|o| matching(b, o)) else {
                continue;
            };
            let args: Vec<Span> = (split_args(&s[open.unwrap_or(pos) + 1..close]))
                .into_iter()
                .map(|a| span_of(s, a))
                .collect();
            let first = raw(c, args[0]);
            let last = args.last().filter(|_| args.len() >= 2);
            let literal = last.is_some_and(|&a| raw(c, a).starts_with('"'));
            let checked = match (receiver, w) {
                ("Objects", "requireNonNull") => Some(format!("{first} == null")),
                (r, _) if CHECKERS.contains(&r) && literal => Some(match w {
                    "isTrue" | "state" | "checkArgument" | "checkState" => format!("!({first})"),
                    "notNull" | "checkNotNull" => format!("{first} == null"),
                    _ => format!("!{r}.{w}({first})"),
                }),
                _ => None,
            };
            if let Some(cond) = checked {
                let msg = last.and_then(|&a| message(c, a));
                out.push((pos, format!("{receiver}.{w}"), Some(cond), msg));
            }
        }
        out
    }

    pub(super) fn validations(x: &Extractor) -> Vec<Validation> {
        let mut out = vec![];
        for d in x.decls.iter().filter(|d| d.kw == "class") {
            let (src, Some(body)) = (&x.files[d.file], d.body) else {
                continue;
            };
            if src.kotlin {
                continue;
            }
            for (property, rules, pos) in bean_fields(src, body) {
                out.push(Validation {
                    target_type: d.name.clone(),
                    property,
                    rules,
                    file: src.rel.clone(),
                    line: line(&src.text, pos),
                    source: SOURCE.into(),
                });
            }
        }
        out
    }

    /// Fields of the class body `{…}` with Bean Validation annotations: (field,
    /// annotations as written, offset of the first one).
    fn bean_fields(src: &Src, (open, close): Span) -> Vec<(String, Vec<String>, usize)> {
        let (s, b) = (src.text.as_str(), src.text.as_bytes());
        let mut out = vec![];
        let mut pending: Vec<(usize, String)> = vec![];
        let mut k = open + 1;
        while k < close {
            match b[k] {
                b'@' => {
                    let end = (k + 1..close)
                        .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
                        .unwrap_or(close);
                    let paren = next_nonws(b, end).filter(|&o| b[o] == b'(');
                    let stop = paren.and_then(|o| matching(b, o)).map_or(end, |c| c + 1);
                    if BEAN_VALIDATION.contains(&simple(&s[k + 1..end]).as_str()) {
                        pending.push((k, raw(code(src), (k, stop))));
                    }
                    k = stop;
                    continue;
                }
                // Method parameters and bodies, nested types, initializer blocks.
                b'(' | b'{' => {
                    k = matching(b, k).map_or(close, |c| c + 1);
                    pending.clear();
                    continue;
                }
                b';' | b'=' if !pending.is_empty() => {
                    let at = pending[0].0;
                    let rules = pending.drain(..).map(|p| p.1).collect();
                    out.push((prev_word(s, k).to_string(), rules, at));
                }
                _ => {}
            }
            k += 1;
        }
        out
    }

    pub(super) fn integration_events(x: &Extractor) -> Vec<IntegrationEvent> {
        let mut found: BTreeMap<String, BTreeSet<Publisher>> = BTreeMap::new();
        for d in &x.decls {
            let src = &x.files[d.file];
            for (_, name, _, body, _) in d.methods() {
                for (t, pos) in publishes(&src.text, body) {
                    found.entry(t).or_default().insert(Publisher {
                        by: format!("{}.{name}", d.name),
                        service: src.project.clone(),
                        file: src.rel.clone(),
                        line: line(&src.text, pos),
                    });
                }
            }
        }
        (found.into_iter())
            .map(|(type_name, by)| IntegrationEvent {
                type_name,
                published_by: by.into_iter().collect(),
                handled_by: vec![],
                source: SOURCE.into(),
            })
            .collect()
    }

    /// Payload types sent to a broker in `body`: (type, offset of the call).
    fn publishes(s: &str, body: Span) -> Vec<(String, usize)> {
        let b = s.as_bytes();
        let mut out = vec![];
        for (pos, w) in idents(s, body.0, body.1) {
            let Some(dot) = prev_nonws(b, pos).filter(|&k| PUBLISH.contains(&w) && b[k] == b'.')
            else {
                continue;
            };
            let receiver = prev_word(s, dot).to_lowercase();
            if !BROKER_RECEIVERS.iter().any(|r| receiver.contains(r)) {
                continue;
            }
            let open = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'(');
            let Some((open, close)) = open.and_then(|o| Some((o, matching(b, o)?))) else {
                continue;
            };
            let Some(&last) = split_args(&s[open + 1..close]).last() else {
                continue;
            };
            let span = span_of(s, last);
            let arg = &s[span.0..span.1];
            let types = if !arg.is_empty() && arg.bytes().all(is_id) {
                assigned_news(s, (body.0, pos), arg)
            } else {
                top_news(s, span)
            };
            out.extend(types.into_iter().map(|t| (t, pos)));
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::super::extract;

        const ACCOUNT: &str = r#"package shop.domain;

import jakarta.persistence.*;
import jakarta.validation.constraints.*;

@Entity
public class BankAccount {
    @Id
    private Long id;

    @NotNull
    @Size(max = 50, message = "a, b")
    @Column(name = "name", nullable = false)
    private String name;

    @NotNull
    private BigDecimal balance = BigDecimal.ZERO;

    @OneToMany(mappedBy = "account")
    private Set<Operation> operations = new HashSet<>();

    public void withdraw(BigDecimal amount) {
        Objects.requireNonNull(amount, "amount is required");
        Assert.isTrue(amount.signum() > 0, "amount must be positive");
        Assert.hasText(name, "named");
        Preconditions.checkState(open, computed());
        if (balance.compareTo(amount) < 0) {
            throw new InsufficientFundsException("Balance " + balance + " too low");
        }
        balance = balance.subtract(amount);
        kafkaTemplate.send("accounts", new AccountDebited(id, amount));
    }

    @NotNull
    public String getName(@Valid @NotBlank String x) { return name; }
}
"#;

        const SERVICE: &str = r#"package shop.service;

public class TransferService {
    public void transfer(Long id) {
        var event = new TransferRequested(id);
        rabbitTemplate.convertAndSend("x", "y", event);
        streamBridge.send("out", payloadFactory.create());
        mailSender.send(new Mail());
    }
    public void fail() { throw new IllegalStateException("not an entity"); }
}
"#;

        const DTO: &str = r#"package shop.service.dto;

public class AdminUserDTO {
    @NotBlank
    @Pattern(regexp = Constants.LOGIN_REGEX)
    @Size(min = 1, max = 50)
    private String login;

    private String plain;
}
"#;

        fn run() -> archguard_core::domain_model::DomainModel {
            let dir = tempfile::tempdir().unwrap();
            for (path, text) in [
                ("bank/pom.xml", "<project/>"),
                ("bank/src/main/java/shop/domain/BankAccount.java", ACCOUNT),
                (
                    "bank/src/main/java/shop/service/TransferService.java",
                    SERVICE,
                ),
                ("bank/src/main/java/shop/service/dto/AdminUserDTO.java", DTO),
            ] {
                let path = dir.path().join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            extract(dir.path())
        }

        #[test]
        fn java_rules_in_entity_methods_with_condition_and_literal_message() {
            let model = run();
            type Row<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str, u32);
            let rows: Vec<Row> = (model.rules.iter())
                .map(|r| {
                    (
                        r.method.as_str(),
                        r.condition.as_deref(),
                        r.message.as_deref(),
                        r.exception.as_str(),
                        r.line,
                    )
                })
                .collect();
            let mut rows = rows;
            rows.sort_by_key(|r| r.4);
            assert_eq!(
                rows,
                [
                    (
                        "BankAccount.withdraw",
                        Some("amount == null"),
                        Some("amount is required"),
                        "Objects.requireNonNull",
                        23
                    ),
                    (
                        "BankAccount.withdraw",
                        Some("!(amount.signum() > 0)"),
                        Some("amount must be positive"),
                        "Assert.isTrue",
                        24
                    ),
                    (
                        "BankAccount.withdraw",
                        Some("!Assert.hasText(name)"),
                        Some("named"),
                        "Assert.hasText",
                        25
                    ),
                    (
                        "BankAccount.withdraw",
                        Some("balance.compareTo(amount) < 0"),
                        Some(r#""Balance " + balance + " too low""#),
                        "InsufficientFundsException",
                        28
                    ),
                ]
            );
        }

        #[test]
        fn bean_validation_on_entity_and_dto_fields() {
            let model = run();
            let rows: Vec<(&str, &str, Vec<&str>, u32)> = (model.validations.iter())
                .map(|v| {
                    let rules = v.rules.iter().map(String::as_str).collect();
                    (v.target_type.as_str(), v.property.as_str(), rules, v.line)
                })
                .collect();
            assert_eq!(
                rows,
                [
                    (
                        "AdminUserDTO",
                        "login",
                        vec![
                            "@NotBlank",
                            "@Pattern(regexp = Constants.LOGIN_REGEX)",
                            "@Size(min = 1, max = 50)"
                        ],
                        4
                    ),
                    ("BankAccount", "balance", vec!["@NotNull"], 16),
                    (
                        "BankAccount",
                        "name",
                        vec!["@NotNull", r#"@Size(max = 50, message = "a, b")"#],
                        11
                    ),
                ]
            );
        }

        #[test]
        fn broker_sends_with_a_literal_payload_type_are_integration_events() {
            let model = run();
            let rows: Vec<(&str, Vec<String>)> = (model.integration_events.iter())
                .map(|e| {
                    let by = (e.published_by.iter())
                        .map(|p| format!("{} [{}] :{}", p.by, p.service, p.line))
                        .collect();
                    (e.type_name.as_str(), by)
                })
                .collect();
            assert_eq!(
                rows,
                [
                    (
                        "AccountDebited",
                        vec!["BankAccount.withdraw [bank] :31".to_string()]
                    ),
                    (
                        "TransferRequested",
                        vec!["TransferService.transfer [bank] :6".to_string()]
                    ),
                ]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: &str = r#"package shop.sales;

import static shop.sales.OrderStatus.*;
import org.jmolecules.ddd.annotation.AggregateRoot;

/* An order. */
@Entity
@Table(name = "orders")
public class Order extends AbstractAggregateRoot<Order> {
    @Id @GeneratedValue private Long id;
    @OneToMany(mappedBy = "order", cascade = {CascadeType.ALL})
    private Set<OrderLine> lines = new HashSet<>();
    @Embedded private Money total;
    private Discount discount;
    private Long customerId;
    @Enumerated(EnumType.STRING)
    private OrderStatus status = OrderStatus.NEW;

    protected Order() { }

    public void submit() {
        if (status == OrderStatus.NEW) {
            registerEvent(new OrderSubmitted(id, "}"));
            this.status = OrderStatus.SUBMITTED;
        }
    }

    public void approve() {
        if (status == SUBMITTED ||
            status == OrderStatus.ON_HOLD) {
            status = APPROVED;
        }
    }

    public void ship() {
        if (status != OrderStatus.APPROVED) {
            fail();
        }
        OrderShipped shipped = new OrderShipped(id);
        registerEvent(shipped);
        status = OrderStatus.SHIPPED;
    }

    public void cancel() {
        if (status == OrderStatus.SHIPPED || status == OrderStatus.CANCELLED)
            throw new IllegalStateException("not { possible");
        this.status = OrderStatus.CANCELLED;
    }

    public void hold() { status = OrderStatus.ON_HOLD; }

    public void reopen() {
        if (status == OrderStatus.CANCELLED) { return; }
        else { status = OrderStatus.SUBMITTED; }
    }

    public void restore(OrderStatus previous) { this.status = previous; }

    public void archive() {
        switch (status) {
            case SHIPPED -> status = OrderStatus.CANCELLED;
            default -> { }
        }
    }

    public OrderStatus getStatus() { return status; }

    public void close() { }

    private void fail() { throw new IllegalStateException("no"); }
}
"#;

    const INVOICE: &str = r#"package shop.billing;

@Aggregate
public class Invoice {
    @AggregateIdentifier private String id;
    private InvoiceState state;

    @CommandHandler
    public Invoice(CreateInvoiceCommand cmd) {
        apply(new InvoiceCreated(cmd.id()));
        this.state = InvoiceState.OPEN;
    }

    @CommandHandler
    public void handle(final PayInvoiceCommand cmd) {
        if (state != InvoiceState.OPEN) throw new IllegalStateException();
        AggregateLifecycle.apply(new InvoicePaid(id));
        state = InvoiceState.PAID;
    }

    public void map(Function<String, String> f) { f.apply(id); }
}
"#;

    fn java_files() -> Vec<(&'static str, &'static str)> {
        vec![
            ("src/main/java/shop/sales/Order.java", ORDER),
            (
                "src/main/java/shop/sales/OrderStatus.java",
                "package shop.sales;\npublic enum OrderStatus {\n    NEW(\"n\"), SUBMITTED(\"s\"), ON_HOLD(\"h\"),\n    APPROVED(\"a\"), SHIPPED(\"x\"), CANCELLED(\"c\");\n    private final String code;\n    OrderStatus(String code) { this.code = code; }\n}\n",
            ),
            (
                "src/main/java/shop/sales/OrderLine.java",
                "package shop.sales;\n@Entity\npublic class OrderLine {\n    @Id private Long id;\n    private String sku;\n    @ManyToOne(fetch = FetchType.LAZY) private Order order;\n}\n",
            ),
            (
                "src/main/java/shop/sales/Money.java",
                "package shop.sales;\n@Embeddable\npublic class Money {\n    private BigDecimal amount;\n    private String currency;\n}\n",
            ),
            (
                "src/main/java/shop/sales/Discount.java",
                "package shop.sales;\npublic record Discount(@NotNull BigDecimal rate, String code) { }\n",
            ),
            (
                "src/main/java/shop/customers/Customer.java",
                "package shop.customers;\n@Entity\n@org.jmolecules.ddd.annotation.AggregateRoot\npublic class Customer {\n    @Id private UUID id;\n    @Embedded private Address address;\n    public void rename(String name) { }\n    public void close() { }\n}\n",
            ),
            (
                "src/main/java/shop/customers/Address.java",
                "package shop.customers;\n@Embeddable\npublic class Address { private String street; }\n",
            ),
            (
                "src/main/java/shop/app/Note.java",
                "package shop.app;\npublic record Note(String text) { }\n",
            ),
            ("src/main/java/shop/billing/Invoice.java", INVOICE),
            (
                "src/main/java/shop/billing/InvoiceState.java",
                "package shop.billing;\npublic enum InvoiceState { OPEN, PAID }\n",
            ),
            (
                "src/main/java/shop/billing/InvoiceProjection.java",
                "package shop.billing;\n@Component\npublic class InvoiceProjection {\n    @EventHandler\n    public void on(InvoicePaid event) { }\n}\n",
            ),
            (
                "src/main/java/shop/billing/BillingListener.java",
                "package shop.billing;\n@Component\nclass BillingListener {\n    @ApplicationModuleListener\n    void created(InvoiceCreated event) { }\n}\n",
            ),
            (
                "src/main/java/shop/app/OrderEvents.java",
                "package shop.app;\n@Component\npublic class OrderEvents {\n    @EventListener\n    public void on(OrderSubmitted event) { }\n    @TransactionalEventListener(phase = TransactionPhase.AFTER_COMMIT)\n    void shipped(final OrderShipped e) { }\n}\n",
            ),
            (
                "src/main/java/shop/app/SubmitOrderUseCase.java",
                "package shop.app;\n@Component\npublic class SubmitOrderUseCase {\n    public void run(Long id) {\n        Order order = orders.findById(id).orElseThrow();\n        order.submit();\n        order.getStatus();\n        order.close();\n        customer.rename(\"x\");\n        publisher.publishEvent(new OrderAudited(id));\n    }\n}\n",
            ),
            (
                "src/main/java/shop/app/OrderService.java",
                "package shop.app;\n@Service\npublic class OrderService {\n    public void approve(Order order) { order.approve(); }\n}\n",
            ),
            (
                "src/main/java/shop/app/OrderQueries.java",
                "package shop.app;\n@Component\npublic class OrderQueries {\n    @QueryHandler\n    public Order find(FindOrderQuery query) { return null; }\n}\n",
            ),
            (
                "target/generated/Ghost.java",
                "package gen;\n@Entity public class Ghost { }\n",
            ),
            ("build/gen/Gen.kt", "package gen\n@Entity class Gen\n"),
            (
                "src/test/java/shop/FakeOrder.java",
                "package shop;\n@Entity public class FakeOrder { }\n",
            ),
        ]
    }

    const TRIP: &str = r#"package kshop.trips

import kshop.trips.TripStatus.*

@Entity
class Trip(
    @Id val id: UUID,
    @OneToMany(cascade = [CascadeType.ALL]) val stops: MutableList<Stop> = mutableListOf(),
    var status: TripStatus = TripStatus.PLANNED,
) : AbstractAggregateRoot<Trip>() {
    @Embedded
    var route: Route? = null
    val driverId: Long? = null

    fun start() {
        if (status == TripStatus.PLANNED) {
            registerEvent(TripStarted(id))
            status = TripStatus.RUNNING
        }
    }

    fun finish() {
        if (status != RUNNING) throw IllegalStateException("not running")
        status = FINISHED
    }

    fun reset() {
        when (status) {
            FINISHED -> status = TripStatus.PLANNED
            else -> {}
        }
    }

    fun renamed(name: String) = copy(status = TripStatus.PLANNED)
}

enum class TripStatus(val label: String) { PLANNED("p"), RUNNING("r"), FINISHED("f") }

@Entity
class Stop(@Id val id: Long, val name: String)

data class Route(val from: String, val to: String)
"#;

    const DRIVER: &str = r#"package kshop.drivers

class Driver : AggregateRoot<Driver, Long> {
    @Id var id: Long = 0
    @Enumerated(EnumType.STRING)
    var state: DriverState
        private set

    init {
        state = DriverState.IDLE
    }

    fun drive() { state = DriverState.BUSY }
}

enum class DriverState { IDLE, BUSY }
"#;

    fn kotlin_files() -> Vec<(&'static str, &'static str)> {
        vec![
            ("src/main/kotlin/kshop/trips/Trip.kt", TRIP),
            ("src/main/kotlin/kshop/drivers/Driver.kt", DRIVER),
            (
                "src/main/kotlin/kshop/app/StartTripCommandHandler.kt",
                "package kshop.app\n\nclass StartTripCommandHandler(private val trips: TripRepository) {\n    fun handle(cmd: StartTripCommand) {\n        trips.find(cmd.id)?.start()\n    }\n}\n",
            ),
            (
                "src/main/kotlin/kshop/app/TripListener.kt",
                "package kshop.app\n\n@Component\nclass TripListener {\n    @EventListener\n    fun on(event: TripStarted) {}\n}\n",
            ),
        ]
    }

    fn run(files: &[(&str, &str)]) -> DomainModel {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn has_diag(model: &DomainModel, needle: &str) -> bool {
        model.diagnostics.iter().any(|d| d.message.contains(needle))
    }

    fn kinds(model: &DomainModel) -> Vec<(&str, EntityKind)> {
        model
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.kind))
            .collect()
    }

    fn relations(model: &DomainModel) -> Vec<(&str, &str, RelationKind, &str)> {
        (model.relations.iter())
            .map(|r| (r.from.as_str(), r.to.as_str(), r.kind, r.via.as_str()))
            .collect()
    }

    fn machine<'a>(model: &'a DomainModel, entity: &str) -> &'a StateMachine {
        (model.state_machines.iter())
            .find(|m| m.entity == entity)
            .unwrap_or_else(|| panic!("no machine for {entity}"))
    }

    fn transition<'a>(m: &'a StateMachine, method: &str) -> &'a StateTransition {
        (m.transitions.iter())
            .find(|t| t.method == method)
            .unwrap_or_else(|| panic!("no transition in {method}"))
    }

    fn events(model: &DomainModel) -> Vec<(&str, Vec<String>, Vec<String>)> {
        (model.events.iter())
            .map(|e| {
                assert_eq!(e.source, "java");
                (
                    e.type_name.as_str(),
                    e.raised_by.clone(),
                    e.handled_by.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn java_entities_by_annotation_base_and_record_package() {
        let model = run(&java_files());
        assert_eq!(
            kinds(&model),
            vec![
                ("Address", EntityKind::ValueObject),
                ("Customer", EntityKind::AggregateRoot),
                ("Discount", EntityKind::ValueObject),
                ("Invoice", EntityKind::AggregateRoot),
                ("Money", EntityKind::ValueObject),
                ("Order", EntityKind::AggregateRoot),
                ("OrderLine", EntityKind::Entity),
            ]
        );
        assert!(model.entities.iter().all(|e| e.source == "java"));
        let order = model.entities.iter().find(|e| e.id == "Order").unwrap();
        assert_eq!(
            (order.file.as_str(), order.line),
            ("src/main/java/shop/sales/Order.java", 9)
        );
        let field = |name: &str| order.fields.iter().find(|f| f.name == name).unwrap();
        assert_eq!(
            (field("lines").type_name.as_str(), field("lines").collection),
            ("OrderLine", true)
        );
        assert_eq!(field("customerId").type_name, "Long");
        assert_eq!(field("status").type_name, "OrderStatus");
        let discount = model.entities.iter().find(|e| e.id == "Discount").unwrap();
        let names: Vec<&str> = discount.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["rate", "code"]);
    }

    #[test]
    fn java_relations_jpa_and_cross_aggregate_reference() {
        let model = run(&java_files());
        assert_eq!(
            relations(&model),
            vec![
                ("Customer", "Address", RelationKind::HasOne, "address"),
                ("Order", "Customer", RelationKind::References, "customerId"),
                ("Order", "Discount", RelationKind::HasOne, "discount"),
                ("Order", "Money", RelationKind::HasOne, "total"),
                ("Order", "OrderLine", RelationKind::HasMany, "lines"),
                ("OrderLine", "Order", RelationKind::HasOne, "order"),
            ]
        );
    }

    #[test]
    fn java_state_machine_guards_initial_and_events() {
        let model = run(&java_files());
        let m = machine(&model, "Order");
        assert_eq!(
            (m.field.as_str(), m.enum_type.as_str()),
            ("status", "OrderStatus")
        );
        let all = [
            "NEW",
            "SUBMITTED",
            "ON_HOLD",
            "APPROVED",
            "SHIPPED",
            "CANCELLED",
        ];
        assert_eq!(m.states, s(&all));
        // Field initializer: the constructor assigns nothing.
        assert_eq!(m.initial.as_deref(), Some("NEW"));
        let submit = transition(m, "Order.submit");
        assert_eq!(
            (submit.from.clone(), submit.to.as_str()),
            (s(&["NEW"]), "SUBMITTED")
        );
        assert_eq!(submit.guard.as_deref(), Some("status == OrderStatus.NEW"));
        assert_eq!(submit.events, s(&["OrderSubmitted"]));
        assert_eq!(
            (submit.file.as_str(), submit.line),
            ("src/main/java/shop/sales/Order.java", 24)
        );
        // Static import: bare constants.
        let approve = transition(m, "Order.approve");
        assert_eq!(
            (approve.from.clone(), approve.to.as_str()),
            (s(&["SUBMITTED", "ON_HOLD"]), "APPROVED")
        );
        assert_eq!(
            approve.guard.as_deref(),
            Some("status == SUBMITTED || status == OrderStatus.ON_HOLD")
        );
        // Exit forms read as the allowed condition: `!(source)`.
        let ship = transition(m, "Order.ship");
        assert_eq!(ship.from, s(&["APPROVED"]));
        assert_eq!(
            ship.guard.as_deref(),
            Some("!(status != OrderStatus.APPROVED)")
        );
        assert_eq!(ship.events, s(&["OrderShipped"]));
        let cancel = transition(m, "Order.cancel");
        assert_eq!(cancel.from, s(&["NEW", "SUBMITTED", "ON_HOLD", "APPROVED"]));
        assert_eq!(
            cancel.guard.as_deref(),
            Some("!(status == OrderStatus.SHIPPED || status == OrderStatus.CANCELLED)")
        );
        let hold = transition(m, "Order.hold");
        assert_eq!((hold.from.clone(), hold.guard.clone()), (vec![], None));
        let reopen = transition(m, "Order.reopen");
        assert_eq!(reopen.from, s(&all[..5]));
        assert_eq!(
            reopen.guard.as_deref(),
            Some("!(status == OrderStatus.CANCELLED)")
        );
        let archive = transition(m, "Order.archive");
        assert_eq!(
            (archive.from.clone(), archive.to.as_str()),
            (vec![], "CANCELLED")
        );
        assert!(has_diag(
            &model,
            "Order.archive: guard of status not read (switch/when)"
        ));
        assert!(m.transitions.iter().all(|t| t.method != "Order.restore"));
        assert!(has_diag(
            &model,
            "Order.restore assigns status the non-literal `previous`"
        ));
        assert_eq!(m.transitions.len(), 7);
        assert!(!model.complete);
    }

    #[test]
    fn java_axon_constructor_initial_and_exit_guard() {
        let model = run(&java_files());
        let m = machine(&model, "Invoice");
        assert_eq!(m.initial.as_deref(), Some("OPEN"));
        let pay = transition(m, "Invoice.handle");
        assert_eq!((pay.from.clone(), pay.to.as_str()), (s(&["OPEN"]), "PAID"));
        assert_eq!(pay.guard.as_deref(), Some("!(state != InvoiceState.OPEN)"));
        assert_eq!(pay.events, s(&["InvoicePaid"]));
        assert_eq!(m.transitions.len(), 1);
    }

    #[test]
    fn java_events_spring_and_axon_chains() {
        let model = run(&java_files());
        assert_eq!(
            events(&model),
            vec![
                (
                    "InvoiceCreated",
                    s(&["Invoice.Invoice"]),
                    s(&["BillingListener"])
                ),
                (
                    "InvoicePaid",
                    s(&["Invoice.handle"]),
                    s(&["InvoiceProjection"])
                ),
                ("OrderAudited", s(&["SubmitOrderUseCase.run"]), vec![]),
                ("OrderShipped", s(&["Order.ship"]), s(&["OrderEvents"])),
                ("OrderSubmitted", s(&["Order.submit"]), s(&["OrderEvents"])),
            ]
        );
        // `f.apply(id)` is not Axon's apply and is no diagnostic.
        assert!(!has_diag(&model, "Invoice.map"));
        // The listener method, not the class, is where the handler sits.
        let site = |event: &str| {
            (model.event_handler_sites(event))
                .map(|x| (x.handler.clone(), x.file.clone(), x.line))
                .collect::<Vec<_>>()
        };
        let events = "src/main/java/shop/app/OrderEvents.java".to_string();
        assert_eq!(
            site("OrderShipped"),
            [("OrderEvents".to_string(), events.clone(), 7)]
        );
        assert_eq!(
            site("OrderSubmitted"),
            [("OrderEvents".to_string(), events, 5)]
        );
    }

    #[test]
    fn java_use_cases_axon_handlers_named_classes_and_calls() {
        let model = run(&java_files());
        let cases: Vec<(&str, UseCaseKind, Option<&str>, Vec<String>)> = (model.use_cases.iter())
            .map(|u| (u.id.as_str(), u.kind, u.request.as_deref(), u.calls.clone()))
            .collect();
        assert_eq!(
            cases,
            vec![
                (
                    "Invoice.Invoice",
                    UseCaseKind::Command,
                    Some("CreateInvoiceCommand"),
                    vec![]
                ),
                (
                    "Invoice.handle",
                    UseCaseKind::Command,
                    Some("PayInvoiceCommand"),
                    vec![]
                ),
                (
                    "OrderQueries.find",
                    UseCaseKind::Query,
                    Some("FindOrderQuery"),
                    vec![]
                ),
                (
                    "SubmitOrderUseCase",
                    UseCaseKind::Handler,
                    None,
                    s(&["Customer.rename", "Order.submit"])
                ),
            ]
        );
        // `@Service` is off by default (ADR 0022 open point).
        assert!(model.use_cases.iter().all(|u| u.id != "OrderService"));
        // `close` is on Order and Customer; the getter is no domain call.
        let ambiguous = (model.diagnostics.iter())
            .find(|d| d.message.contains(".close("))
            .expect("ambiguous close");
        assert_eq!(
            (ambiguous.file.as_str(), ambiguous.line),
            ("src/main/java/shop/app/SubmitOrderUseCase.java", 8)
        );
        assert!(!has_diag(&model, "getStatus"));
    }

    #[test]
    fn java_broker_listeners_are_message_handlers() {
        let model = run(&[(
            "src/main/java/shop/app/PaymentListener.java",
            r#"package shop.app;
public class PaymentListener {
    private final CommandGateway gateway;
    private final MarkPaidUseCase markPaid;
    @KafkaListener(topics = "payments")
    public void onPaid(@Payload PaymentSucceeded event) {
        gateway.send(new MarkOrderPaidCommand(event.orderId()));
    }
    @RabbitListener(queues = "refunds")
    void onRefund(RefundIssued event) { markPaid.undo(event.orderId()); }
    @JmsListener(destination = "x") void onJms(JmsThing t) { }
    @StreamListener("in") void onStream(StreamThing t) { }
    public void notAListener(Other o) { gateway.send(new Ignored()); }
}
"#,
        )]);
        let found: Vec<(&str, &str, Vec<String>, u32)> = (model.message_handlers.iter())
            .map(|h| {
                assert_eq!(h.source, "java");
                assert_eq!(h.file, "src/main/java/shop/app/PaymentListener.java");
                (h.id.as_str(), h.message.as_str(), h.sends.clone(), h.line)
            })
            .collect();
        assert_eq!(
            found,
            [
                ("PaymentListener.onJms", "JmsThing", vec![], 11),
                (
                    "PaymentListener.onPaid",
                    "PaymentSucceeded",
                    s(&["MarkOrderPaidCommand"]),
                    6
                ),
                (
                    "PaymentListener.onRefund",
                    "RefundIssued",
                    s(&["MarkPaidUseCase"]),
                    10
                ),
                ("PaymentListener.onStream", "StreamThing", vec![], 12),
            ]
        );
    }

    #[test]
    fn target_build_and_tests_are_skipped() {
        let model = run(&java_files());
        for name in ["Ghost", "Gen", "FakeOrder"] {
            assert!(model.entities.iter().all(|e| e.name != name), "{name}");
        }
    }

    #[test]
    fn kotlin_entities_relations_and_fields() {
        let model = run(&kotlin_files());
        assert_eq!(
            kinds(&model),
            vec![
                ("Driver", EntityKind::AggregateRoot),
                ("Route", EntityKind::ValueObject),
                ("Stop", EntityKind::Entity),
                ("Trip", EntityKind::AggregateRoot),
            ]
        );
        assert_eq!(
            relations(&model),
            vec![
                ("Trip", "Driver", RelationKind::References, "driverId"),
                ("Trip", "Route", RelationKind::HasOne, "route"),
                ("Trip", "Stop", RelationKind::HasMany, "stops"),
            ]
        );
        let route = model.entities.iter().find(|e| e.id == "Route").unwrap();
        let names: Vec<&str> = route.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["from", "to"]);
    }

    #[test]
    fn kotlin_state_machines_guards_init_and_when() {
        let model = run(&kotlin_files());
        let trip = machine(&model, "Trip");
        assert_eq!(trip.states, s(&["PLANNED", "RUNNING", "FINISHED"]));
        // Primary-constructor default.
        assert_eq!(trip.initial.as_deref(), Some("PLANNED"));
        let start = transition(trip, "Trip.start");
        assert_eq!(
            (start.from.clone(), start.to.as_str()),
            (s(&["PLANNED"]), "RUNNING")
        );
        assert_eq!(start.guard.as_deref(), Some("status == TripStatus.PLANNED"));
        assert_eq!(start.events, s(&["TripStarted"]));
        let finish = transition(trip, "Trip.finish");
        assert_eq!(
            (finish.from.clone(), finish.to.as_str()),
            (s(&["RUNNING"]), "FINISHED")
        );
        assert_eq!(finish.guard.as_deref(), Some("!(status != RUNNING)"));
        let reset = transition(trip, "Trip.reset");
        assert_eq!((reset.from.clone(), reset.to.as_str()), (vec![], "PLANNED"));
        assert!(has_diag(
            &model,
            "Trip.reset: guard of status not read (switch/when)"
        ));
        // A named argument of `copy(…)` is no assignment.
        assert!(trip.transitions.iter().all(|t| t.method != "Trip.renamed"));
        assert_eq!(trip.transitions.len(), 3);

        let driver = machine(&model, "Driver");
        assert_eq!(driver.initial.as_deref(), Some("IDLE"));
        let drive = transition(driver, "Driver.drive");
        assert_eq!((drive.from.clone(), drive.to.as_str()), (vec![], "BUSY"));
        assert!(!model.complete);
    }

    #[test]
    fn kotlin_events_and_use_cases() {
        let model = run(&kotlin_files());
        assert_eq!(
            events(&model),
            vec![("TripStarted", s(&["Trip.start"]), s(&["TripListener"]))]
        );
        let cases: Vec<(&str, UseCaseKind, Vec<String>)> = (model.use_cases.iter())
            .map(|u| (u.id.as_str(), u.kind, u.calls.clone()))
            .collect();
        assert_eq!(
            cases,
            vec![(
                "StartTripCommandHandler",
                UseCaseKind::Command,
                s(&["Trip.start"])
            )]
        );
    }

    #[test]
    fn module_is_the_package() {
        let model = run(&java_files());
        let m = |n: &str| {
            model
                .entities
                .iter()
                .find(|e| e.name == n)
                .unwrap()
                .module
                .clone()
        };
        assert_eq!(
            (m("Order"), m("Customer")),
            ("shop.sales".into(), "shop.customers".into())
        );
    }

    #[test]
    fn duplicate_names_are_qualified_and_reported() {
        let model = run(&[
            (
                "a/Item.java",
                "\u{FEFF}package shop.a;\n@Entity class Item { }\n",
            ),
            ("b/Item.kt", "package shop.b\n@Entity\nclass Item\n"),
        ]);
        let ids: Vec<&str> = model.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["shop.a.Item", "shop.b.Item"]);
        assert!(has_diag(&model, "Several entities are named Item"));
        assert!(model.complete);
    }

    #[test]
    fn unreadable_file_is_a_diagnostic() {
        let big = format!("@Entity class Big {{ }}\n{}", " ".repeat(1 << 20));
        let model = run(&[("Big.java", big.as_str())]);
        assert!(model.entities.is_empty());
        assert_eq!(model.diagnostics[0].file, "Big.java");
        assert!(!model.complete);
    }
}
