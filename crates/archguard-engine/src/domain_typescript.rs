//! TypeScript extractor for domain-model/v1 (ADR 0022, phase 6): a text-level
//! scanner like the Java one (comments and string contents blanked, brackets
//! matched), no compiler. Every entry has `source: "typescript"`.
//!
//! Recognized:
//! - Files: `*.ts`, `*.tsx`, `*.mts`, `*.cts` (not `*.d.ts`) and `*.prisma`
//!   below the root, skipping `node_modules`, `dist`, the other directories the
//!   service scan excludes and test paths (`*.spec.ts`, `*.test.ts`, `test/`,
//!   `__tests__/`); files over 1 MiB, unreadable files and files past the first
//!   5 000 are diagnostics.
//! - Entities: a class decorated `@AggregateRoot()` or extending/implementing
//!   `AggregateRoot` (`@nestjs/cqrs`) → `aggregate_root`; `@Entity()` (TypeORM,
//!   MikroORM) → `entity`; `@Embeddable()` (MikroORM) → `value_object`; a
//!   Prisma `model X { … }` → `entity`. `id` is the class name, qualified with
//!   the folder when two entities share a name (with a diagnostic). A class
//!   decorated `@Table(…)` imported from sequelize-typescript,
//!   `@immich/sql-tools` or typeorm is an `entity` too; a class merely named
//!   `*Entity`, or a `@Table` from another module, is not.
//! - Fields: class properties with a type (`x: T`, `x?: T | null`, `x!: T`) or
//!   an initializer `new T<…>(…)` / `E.V`; Prisma `name Type[?|[]]` lines. `T[]`,
//!   `Array`, `ReadonlyArray`, `Set`, `Collection`, `List`, `Iterable` and a
//!   field decorated `@OneToMany`/`@ManyToMany` are `collection`.
//! - Relations: a field whose type is a found entity → `has_one`, a collection
//!   of one → `has_many` (`@ManyToOne`, `@OneToOne`, `@OneToMany`,
//!   `@ManyToMany` need no extra rule). Prisma: `X[]` → `has_many`, `X` with
//!   `@relation` → `has_one`, a back-relation `X?` without `@relation` → none.
//!   A `string`/`number` `xId` with `X` an aggregate root in another folder →
//!   `references`.
//! - State machines: a field typed by a TS `enum` (or a Prisma enum) or by a
//!   string-literal union alias (`type S = 'a' | 'b'`), assigned inside a
//!   method of its entity (`this.status = OrderStatus.Paid`,
//!   `this.status = 'paid'`). Guards as in the Java extractor: the nearest
//!   enclosing `if` comparing `this.field` with literals (`===`/`==` joined by
//!   `||`, `!==`/`!=` joined by `&&`, one comparison), the `else` of such an
//!   `if` (negated), or the nearest preceding `if` whose block throws,
//!   returns or calls an entity method that only throws (negated). A negated
//!   guard is the literal `!(source condition)`, never simplified. Without
//!   such an `if`, from = [] (any). A constructor assignment is `initial`,
//!   else the property initializer. A non-literal value, a `switch` around the
//!   assignment or a guard of another shape is a diagnostic and makes the
//!   model incomplete.
//! - Events: `this.apply(…)` (`@nestjs/cqrs` `AggregateRoot`) and
//!   `….eventBus.publish(…)` with `new X(…)` or a variable assigned one
//!   earlier in the method; `….eventEmitter.emit('name', …)` with the string
//!   as event type → `raised_by: "Class.method"` for any class, and the type
//!   on that method's transitions. Handlers: a class decorated
//!   `@EventsHandler(X, …)` or implementing `IEventHandler<X>`, and a class
//!   with a method decorated `@OnEvent('name')`.
//! - Use cases: classes decorated `@CommandHandler(X)` / `@QueryHandler(X)`
//!   (request = X), else classes named `*UseCase` or `*Handler`. `calls`:
//!   `.m(` in the class where `m` is a public method of exactly one entity,
//!   constructors, `get`/`set` accessors, `toString` and `equals` left out. A
//!   name on several entities is a diagnostic.
//!
//! - Arrow-function relations: `@ManyToOne/@OneToOne/@OneToMany/@ManyToMany/
//!   @ForeignKeyColumn(() => X)` on a field whose type names no entity gives
//!   a relation to `X` (found by name, not by import); an `X` that is not a
//!   unique entity is a diagnostic and makes the model incomplete.
//!   Docs: <https://typeorm.io/relations>, <https://www.prisma.io/docs/orm/prisma-schema/data-model/relations>.
//!
//! Not read, with one diagnostic per app (first importing file): Kysely
//! `interface DB` row maps (`import … from 'kysely'`) and Drizzle
//! `pgTable/mysqlTable/sqliteTable(…)` (`import … from 'drizzle-orm…'`).
//!
//! Not recognized:
//! constructor parameter properties as fields, properties
//! without a type and with another initializer, inline literal unions as
//! field types, `state = …` without `this.`, `??=`, `switch` guards,
//! regular-expression literals containing quotes or brackets, Prisma
//! `@relation` on lists, request types of named `*Handler` classes, state
//! libraries (XState) and the State pattern. The marker names are the
//! constants below; reading them from the configuration comes later.
use crate::extract_ctx::{ExtractContext, domain_sources, with_root_config};
use crate::lang_text::*;
use archguard_core::domain_model::{
    DomainDiagnostic, DomainEvent, DomainModel, Entity, EntityKind, Field, Relation, RelationKind,
    StateMachine, StateTransition, UseCase, UseCaseKind,
};
use members::{Kind, Member, field};
use states::{Rules, Step, assignments, created, raise_calls};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SOURCE: &str = "typescript";
/// Base class, implemented interface or class decorator of an aggregate root.
const AGGREGATE_MARKERS: [&str; 1] = ["AggregateRoot"];
const ENTITY_DECORATOR: &str = "Entity";
const VALUE_OBJECT_DECORATOR: &str = "Embeddable";
const TO_MANY: [&str; 2] = ["OneToMany", "ManyToMany"];
/// Entity decorator besides `@Entity`; counts only when the file imports it
/// from one of [`TABLE_PACKAGES`], so a UI library's `@Table` is no entity.
const TABLE_DECORATOR: &str = "Table";
/// Packages exporting an entity `@Table`: sequelize-typescript
/// (<https://github.com/sequelize/sequelize-typescript>), `@immich/sql-tools`
/// and TypeORM (<https://typeorm.io/decorator-reference>).
const TABLE_PACKAGES: [&str; 3] = ["sequelize-typescript", "@immich/sql-tools", "typeorm"];
/// Table builders of Drizzle (<https://orm.drizzle.team/docs/sql-schema-declaration>).
const DRIZZLE_BUILDERS: [&str; 3] = ["pgTable", "mysqlTable", "sqliteTable"];
/// Field decorators whose first argument `() => X` names the related entity
/// when the field type does not (`Relation<X>`, `string` foreign keys).
const ARROW_RELATIONS: [&str; 5] = [
    "ManyToOne",
    "OneToOne",
    "OneToMany",
    "ManyToMany",
    "ForeignKeyColumn",
];
/// Prisma field attribute that makes a single model field a `has_one`.
const PRISMA_RELATION: &str = "relation";
/// `this.apply(…)`; `<…eventBus>.publish(…)`; `<…emitter>.emit('name', …)`.
const APPLY: &str = "apply";
const PUBLISH: &str = "publish";
const EMIT: &str = "emit";
/// Lower-case suffixes of the receivers of `publish` and `emit`.
const BUS_SUFFIX: &str = "eventbus";
const EMITTER_SUFFIX: &str = "emitter";
const EVENTS_HANDLER: &str = "EventsHandler";
const EVENT_HANDLER_INTERFACE: &str = "IEventHandler";
const ON_EVENT: &str = "OnEvent";
const COMMAND_HANDLER: &str = "CommandHandler";
const QUERY_HANDLER: &str = "QueryHandler";
const USE_CASE_SUFFIXES: [&str; 2] = ["UseCase", "Handler"];
const NOT_CALLS: [&str; 2] = ["toString", "equals"];
const COLLECTIONS: [&str; 7] = [
    "Array",
    "ReadonlyArray",
    "Set",
    "ReadonlySet",
    "Collection",
    "List",
    "Iterable",
];
const ID_TYPES: [&str; 2] = ["string", "number"];
const EXTENSIONS: [&str; 5] = [".ts", ".tsx", ".mts", ".cts", ".prisma"];
/// Words before `class`.
const CLASS_MODIFIERS: [&str; 4] = ["export", "default", "abstract", "declare"];
/// Words before a member name.
const MODIFIERS: [&str; 10] = [
    "public",
    "private",
    "protected",
    "readonly",
    "static",
    "abstract",
    "declare",
    "override",
    "async",
    "accessor",
];

/// Byte range in a file's text.
type Span = (usize, usize);
/// Diagnostics found on the way: file index, offset, message, drops an element.
type Pending = Vec<(usize, usize, String, bool)>;

/// The domain model of the TypeScript and Prisma sources below `root`.
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
    x.finish()
}

struct Src {
    rel: String,
    /// Source with comments and string contents blanked; offsets unchanged.
    text: String,
    /// Source with comments and template literals blanked, strings kept:
    /// literal values and guard text are read here.
    shown: String,
    prisma: bool,
}

struct Deco {
    name: String,
    /// Inside the parentheses, when called.
    args: Option<Span>,
}

struct Decl {
    file: usize,
    /// `class`, `enum`, `type` (alias) or `model` (Prisma).
    kw: &'static str,
    name: String,
    /// The folder of the file.
    scope: String,
    pos: usize,
    decorators: Vec<Deco>,
    /// `extends` and `implements` entries as written.
    bases: Vec<String>,
    /// `{…}` for classes, enums and models; the value of a type alias.
    body: Option<Span>,
    members: Vec<Member>,
}
impl Decl {
    fn deco(&self, name: &str) -> Option<&Deco> {
        self.decorators.iter().find(|d| d.name == name)
    }
    fn methods(&self) -> impl Iterator<Item = (&Member, &str, Span, bool)> {
        self.members.iter().filter_map(|m| match &m.kind {
            Kind::Method {
                name, body, ctor, ..
            } => Some((m, name.as_str(), *body, *ctor)),
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
        let keep = |rel: &str| {
            let declaration = [".d.ts", ".d.mts", ".d.cts"]
                .iter()
                .any(|x| rel.ends_with(x));
            EXTENSIONS.iter().any(|x| rel.ends_with(x)) && !declaration
        };
        let (sources, gaps) = domain_sources(root, &[], keep, "TypeScript/Prisma");
        for (file, message) in gaps {
            self.push_diag(&file, message);
        }
        for (rel, src) in sources {
            self.files.push(Src {
                text: crate::lang_ts_decl::mask(&src, false),
                shown: crate::lang_ts_decl::mask(&src, true),
                prisma: rel.ends_with(".prisma"),
                rel,
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
        let found = decls::parse(&src.text, src.prisma, dir);
        self.decls
            .extend(found.into_iter().map(|d| Decl { file: f, ..d }));
    }

    /// Kysely and Drizzle schemas are not read: one gap each for the app.
    fn unread_schemas(&mut self) {
        let kysely = (self.files.iter())
            .find(|s| imports(&s.shown, |m| m == "kysely", &[]))
            .map(|s| s.rel.clone());
        let drizzle = (self.files.iter())
            .find(|s| {
                let module = |m: &str| m == "drizzle-orm" || m.starts_with("drizzle-orm/");
                imports(&s.shown, module, &DRIZZLE_BUILDERS)
            })
            .map(|s| s.rel.clone());
        if let Some(rel) = kysely {
            let msg = "Kysely database types (`interface DB`) are not read; first import here";
            self.push_diag(&rel, msg.into());
        }
        if let Some(rel) = drizzle {
            let msg = "Drizzle table definitions are not read; first import here";
            self.push_diag(&rel, msg.into());
        }
    }

    fn entities(&mut self) {
        self.unread_schemas();
        let mut ents: Vec<Ent> = (self.decls.iter().enumerate())
            .filter_map(|(d, decl)| {
                marker_kind(decl, &self.files[decl.file].shown).map(|kind| Ent {
                    id: decl.name.clone(),
                    name: decl.name.clone(),
                    kind,
                    decl: d,
                    fields: (decl.members.iter())
                        .filter_map(|m| match &m.kind {
                            Kind::Data { ty, name, .. } => {
                                Some(field(ty, name, m.has_any(&TO_MANY)))
                            }
                            Kind::Method { .. } => None,
                        })
                        .collect(),
                })
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

    /// The entity a type name means as seen from entity `from`; `None` when no
    /// entity or several outside `from`'s folder have that name.
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
        let mut gaps = vec![];
        for (e, ent) in self.ents.iter().enumerate() {
            let decl = &self.decls[ent.decl];
            let text = &self.files[decl.file].text;
            for f in &ent.fields {
                let mut push = |to: usize, kind| {
                    found.push(Relation {
                        from: ent.id.clone(),
                        to: self.ents[to].id.clone(),
                        kind,
                        via: f.name.clone(),
                        source: SOURCE.into(),
                        external: false,
                    });
                };
                if let Some((to, kind)) = self.relation_of(e, f) {
                    push(to, kind);
                    continue;
                }
                // `@ManyToOne(() => X)`: the target comes from the arrow function.
                let Some(deco) = (decl.members.iter()).find_map(|m| {
                    let named = matches!(&m.kind, Kind::Data { name, .. } if *name == f.name);
                    let d =
                        (m.decorators.iter()).find(|d| ARROW_RELATIONS.contains(&d.name.as_str()));
                    d.filter(|_| named)
                }) else {
                    continue;
                };
                let target = deco.args.and_then(|(a, z)| arrow_target(&text[a..z]));
                match target.and_then(|t| self.resolve(&t, e)) {
                    Some(to) => {
                        let many = TO_MANY.contains(&deco.name.as_str());
                        let kind = if many {
                            RelationKind::HasMany
                        } else {
                            RelationKind::HasOne
                        };
                        push(to, kind);
                    }
                    None => gaps.push(DomainDiagnostic {
                        file: self.files[decl.file].rel.clone(),
                        line: line(text, decl.pos),
                        message: format!(
                            "relation {}.{} (@{}): target is not a unique entity this extractor read",
                            ent.name, f.name, deco.name
                        ),
                    }),
                }
            }
        }
        self.model.relations = found;
        if !gaps.is_empty() {
            self.model.complete = false;
            self.model.diagnostics.extend(gaps);
        }
    }

    fn relation_of(&self, e: usize, f: &Field) -> Option<(usize, RelationKind)> {
        if let Some(to) = self.resolve(&simple(&f.type_name), e) {
            let decl = &self.decls[self.ents[e].decl];
            let back_relation = decl.kw == "model"
                && !f.collection
                && !(decl.members.iter()).any(|m| {
                    matches!(&m.kind, Kind::Data { name, .. } if *name == f.name)
                        && m.has_any(&[PRISMA_RELATION])
                });
            if back_relation {
                return None;
            }
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

    /// The states of the enum or string-union alias `name` as seen from entity
    /// `e`, and whether it is a union.
    fn enum_states(&self, name: &str, e: usize) -> Option<(Vec<String>, bool)> {
        let all: Vec<&Decl> = (self.decls.iter())
            .filter(|d| matches!(d.kw, "enum" | "type") && d.name == name)
            .collect();
        let decl = match all.as_slice() {
            [one] => *one,
            _ => *all.iter().find(|d| d.scope == self.scope(e))?,
        };
        let src = &self.files[decl.file];
        let (a, z) = decl.body?;
        if decl.kw == "type" {
            return union_states(&src.text, &src.shown, (a, z)).map(|v| (v, true));
        }
        let inner = &src.text[a + 1..z];
        let states = if src.prisma {
            (inner.lines())
                .map(str::trim)
                .filter(|l| !l.starts_with('@'))
                .filter_map(|l| idents(l, 0, l.len()).first().map(|x| x.1.to_string()))
                .collect()
        } else {
            (split_top(inner, ",").into_iter())
                .filter_map(|m| idents(m, 0, m.len()).first().map(|x| x.1.to_string()))
                .collect()
        };
        Some((states, false))
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
        self.model.events = domain_events(&self.decls, &self.files, raised);
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
        for (_, name, body, _) in decl.methods() {
            for (pos, call, arg) in raise_calls(s, body) {
                let found = if call == EMIT {
                    let first = split_top(&s[arg.0..arg.1], ",")[0];
                    let at = offset(s, first);
                    string_literal(&src.shown[at..at + first.len()])
                } else {
                    created(s, (body.0, pos), s[arg.0..arg.1].trim())
                };
                let Some(t) = found else {
                    let why = if call == EMIT {
                        "event name is not a string literal"
                    } else {
                        "argument is neither a constructor call nor a variable assigned one"
                    };
                    let message =
                        format!("{}.{name}: {call} {why}; the event is not read.", decl.name);
                    pending.push((decl.file, pos, message, true));
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
    /// when the type is an enum or union and a method of the entity assigns it.
    fn machine_of(
        &self,
        e: usize,
        (ty, field_name, init): (&str, &str, Option<Span>),
        pending: &mut Pending,
    ) -> Option<StateMachine> {
        let ent = &self.ents[e];
        let decl = &self.decls[ent.decl];
        if field(ty, "", false).collection || decl.kw != "class" {
            return None;
        }
        let enum_type = simple(ty);
        let (states, union) = self.enum_states(&enum_type, e)?;
        let src = &self.files[decl.file];
        let s = &src.text;
        let helpers = decl.throwing(s);
        let rules = Rules {
            s,
            shown: &src.shown,
            field: field_name,
            enum_type: &enum_type,
            states: &states,
            all: states.iter().cloned().collect(),
            union,
            helpers: &helpers,
        };
        let mut machine = StateMachine {
            entity: ent.id.clone(),
            field: field_name.into(),
            enum_type: enum_type.clone(),
            states: states.clone(),
            initial: None,
            transitions: vec![],
        };
        let mut assigned = false;
        for (_, method, body, ctor) in decl.methods() {
            for (stmt, rhs) in assignments(s, body, field_name) {
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
            machine.initial = init.and_then(|(a, z)| rules.literal(&collapse(&src.shown[a..z])));
        }
        machine.transitions.sort();
        assigned.then_some(machine)
    }

    fn use_cases(&mut self) {
        let mut owners: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for e in &self.ents {
            for (m, name, _, ctor) in self.decls[e.decl].methods() {
                let accessor = matches!(m.kind, Kind::Method { accessor: true, .. });
                if m.public && !ctor && !accessor && !NOT_CALLS.contains(&name) {
                    owners.entry(name).or_default().insert(&e.id);
                }
            }
        }
        let mut pending = Pending::new();
        let mut cases = vec![];
        for d in &self.decls {
            cases.extend(self.use_case_of(d, &owners, &mut pending));
        }
        self.model.use_cases = cases;
        self.pending.extend(pending);
    }

    /// `d` when decorated as a command/query handler or named like a use case.
    fn use_case_of(
        &self,
        d: &Decl,
        owners: &BTreeMap<&str, BTreeSet<&str>>,
        pending: &mut Pending,
    ) -> Option<UseCase> {
        if d.kw != "class" {
            return None;
        }
        let src = &self.files[d.file];
        let s = &src.text;
        let request = |deco: &Deco| deco.args.and_then(|a| arg_types(s, a).into_iter().next());
        let (kind, request) = if let Some(deco) = d.deco(COMMAND_HANDLER) {
            (UseCaseKind::Command, request(deco))
        } else if let Some(deco) = d.deco(QUERY_HANDLER) {
            (UseCaseKind::Query, request(deco))
        } else if USE_CASE_SUFFIXES.iter().any(|x| d.name.ends_with(x)) {
            (use_case_kind(&d.name), None)
        } else {
            return None;
        };
        let (calls, ambiguous) = calls_in(s, d.body?, owners);
        for (at, message) in ambiguous {
            pending.push((d.file, at, format!("{}: {message}", d.name), true));
        }
        Some(UseCase {
            id: d.name.clone(),
            name: d.name.clone(),
            kind,
            request,
            calls,
            file: src.rel.clone(),
            line: line(s, d.pos),
            source: SOURCE.into(),
        })
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
        let m = &mut self.model;
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

/// `X` of `() => X` at the start of decorator arguments.
fn arrow_target(args: &str) -> Option<String> {
    let rest = args.trim_start().strip_prefix("()")?.trim_start();
    let rest = rest.strip_prefix("=>")?.trim_start();
    let id: String = (rest.chars())
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        .collect();
    let after = rest[id.len()..].trim_start();
    (!id.is_empty() && (after.is_empty() || after.starts_with(','))).then_some(id)
}

/// Whether `shown` has an `import … from 'm'` with `module(m)` that names one
/// of `names` (any name when empty).
fn imports(shown: &str, module: impl Fn(&str) -> bool, names: &[&str]) -> bool {
    let mut from = 0;
    while let Some(i) = shown[from..].find("from") {
        let at = from + i;
        from = at + 4;
        let rest = shown[from..].trim_start();
        let Some(q) = rest.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
            continue;
        };
        let Some(end) = rest[1..].find(q) else {
            continue;
        };
        if !module(&rest[1..1 + end]) {
            continue;
        }
        let Some(start) = shown[..at].rfind("import") else {
            continue;
        };
        let clause = &shown[start + 6..at];
        if names.is_empty()
            || (clause.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')))
                .any(|w| names.contains(&w))
        {
            return true;
        }
    }
    false
}

fn marker_kind(decl: &Decl, shown: &str) -> Option<EntityKind> {
    if decl.kw == "model" {
        return Some(EntityKind::Entity);
    }
    if decl.kw != "class" || AGGREGATE_MARKERS.contains(&decl.name.as_str()) {
        return None;
    }
    let based = (decl.bases.iter()).any(|b| AGGREGATE_MARKERS.contains(&simple(b).as_str()));
    if based || AGGREGATE_MARKERS.iter().any(|m| decl.deco(m).is_some()) {
        Some(EntityKind::AggregateRoot)
    } else if decl.deco(ENTITY_DECORATOR).is_some()
        || (decl.deco(TABLE_DECORATOR).is_some()
            && imports(shown, |m| TABLE_PACKAGES.contains(&m), &[TABLE_DECORATOR]))
    {
        Some(EntityKind::Entity)
    } else if decl.deco(VALUE_OBJECT_DECORATOR).is_some() {
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
            "Several entities are named {}; ids are folder-qualified ({}).",
            e.name, e.id
        );
        out.push((d.file, d.pos, message, false));
    }
    out
}

/// The simple type names in a decorator's arguments, e.g. `@EventsHandler(A, B)`.
fn arg_types(s: &str, (a, z): Span) -> Vec<String> {
    (split_top(&s[a..z], ",").into_iter())
        .map(simple)
        .filter(|t| !t.is_empty() && t.bytes().all(is_id))
        .collect()
}

/// `'x'`, `"x"` or `` `x` `` (in `shown` text) → `x`.
fn string_literal(x: &str) -> Option<String> {
    let x = x.trim();
    let q = x.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let inner = x.strip_prefix(q)?.strip_suffix(q)?;
    (!inner.contains(q)).then(|| inner.to_string())
}

/// The literals of a string-union alias value; `None` when one part is not a
/// string literal.
fn union_states(s: &str, shown: &str, (a, z): Span) -> Option<Vec<String>> {
    (split_top(&s[a..z], "|").into_iter())
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let at = offset(s, p);
            string_literal(&shown[at..at + p.len()])
        })
        .collect()
}

fn domain_events(
    decls: &[Decl],
    files: &[Src],
    raised: BTreeMap<String, BTreeSet<String>>,
) -> Vec<DomainEvent> {
    let mut handled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in decls.iter().filter(|d| d.kw == "class") {
        let src = &files[d.file];
        let mut types: Vec<String> = (d.deco(EVENTS_HANDLER).and_then(|x| x.args))
            .map_or(vec![], |a| arg_types(&src.text, a));
        types.extend(d.bases.iter().filter_map(|b| {
            let (name, rest) = b.split_once('<')?;
            (name.trim() == EVENT_HANDLER_INTERFACE).then(|| simple(rest.trim_end_matches('>')))
        }));
        for m in &d.members {
            for deco in m.decorators.iter().filter(|x| x.name == ON_EVENT) {
                let Some((a, z)) = deco.args else { continue };
                let first = split_top(&src.text[a..z], ",")[0];
                let at = offset(&src.text, first);
                types.extend(string_literal(&src.shown[at..at + first.len()]));
            }
        }
        for t in types {
            handled.entry(t).or_default().insert(d.name.clone());
        }
    }
    let types: BTreeSet<&String> = raised.keys().chain(handled.keys()).collect();
    types
        .into_iter()
        .map(|t| DomainEvent {
            type_name: t.clone(),
            raised_by: (raised.get(t)).map_or(vec![], |x| x.iter().cloned().collect()),
            handled_by: (handled.get(t)).map_or(vec![], |x| x.iter().cloned().collect()),
            source: SOURCE.into(),
        })
        .collect()
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

/// Classes, enums, type aliases and Prisma models with their decorators.
mod decls {
    use super::*;

    /// Declarations of one file (file index left 0).
    pub(super) fn parse(s: &str, prisma: bool, dir: &str) -> Vec<Decl> {
        let toks = idents(s, 0, s.len());
        (0..toks.len())
            .filter_map(|t| {
                if prisma {
                    prisma_at(s, &toks, t)
                } else {
                    decl_at(s, &toks, t)
                }
            })
            .map(|d| Decl {
                scope: dir.to_string(),
                ..d
            })
            .collect()
    }

    fn new_decl(kw: &'static str, name: &str, pos: usize, body: Option<Span>) -> Decl {
        Decl {
            file: 0,
            kw,
            name: name.into(),
            scope: String::new(),
            pos,
            decorators: vec![],
            bases: vec![],
            body,
            members: vec![],
        }
    }

    fn decl_at(s: &str, toks: &[(usize, &str)], t: usize) -> Option<Decl> {
        let (pos, word) = toks[t];
        let b = s.as_bytes();
        if !matches!(word, "class" | "enum" | "type")
            || prev_nonws(b, pos).is_some_and(|k| matches!(b[k], b'.' | b'@'))
        {
            return None;
        }
        let &(npos, name) = toks.get(t + 1).filter(|x| gap(s, toks[t], x.0))?;
        let after = skip_generics(b, npos + name.len());
        let next = next_nonws(b, after)?;
        match word {
            "enum" => {
                let close = (b[next] == b'{').then(|| matching(b, next)).flatten()?;
                Some(new_decl("enum", name, pos, Some((next, close))))
            }
            "type" => {
                let eq = Some(next).filter(|&k| b[k] == b'=' && b.get(k + 1) != Some(&b'='))?;
                Some(new_decl(
                    "type",
                    name,
                    pos,
                    Some((eq + 1, alias_end(b, eq + 1))),
                ))
            }
            _ => {
                let stop = (after..s.len()).find(|&j| b[j] == b'{')?;
                let body = (stop, matching(b, stop)?);
                let text = format!(" {} ", collapse(&s[after..stop]));
                let text = text.replace(" extends ", ",").replace(" implements ", ",");
                let bases = (split_top(&text, ",").into_iter())
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                Some(Decl {
                    decorators: decorators_before(s, pos),
                    bases,
                    members: members::class(s, body),
                    ..new_decl("class", name, pos, Some(body))
                })
            }
        }
    }

    /// `model X { … }` / `enum X { … }` at a line start of a Prisma schema.
    fn prisma_at(s: &str, toks: &[(usize, &str)], t: usize) -> Option<Decl> {
        let (pos, word) = toks[t];
        let line_start = s[..pos].rfind('\n').map_or(0, |n| n + 1);
        if !matches!(word, "model" | "enum") || !s[line_start..pos].trim().is_empty() {
            return None;
        }
        let &(npos, name) = toks.get(t + 1).filter(|x| gap(s, toks[t], x.0))?;
        let b = s.as_bytes();
        let open = next_nonws(b, npos + name.len()).filter(|&k| b[k] == b'{')?;
        let body = (open, matching(b, open)?);
        if word == "enum" {
            return Some(new_decl("enum", name, pos, Some(body)));
        }
        Some(Decl {
            members: members::prisma(s, body),
            ..new_decl("model", name, pos, Some(body))
        })
    }

    pub(super) fn skip_generics(b: &[u8], i: usize) -> usize {
        match next_nonws(b, i).filter(|&k| b[k] == b'<') {
            Some(k) => matching(b, k).map_or(i, |e| e + 1),
            None => i,
        }
    }

    /// End of a type alias value: `;`, or a line break at depth 0 that the
    /// next line does not continue with `|` or `&`.
    fn alias_end(b: &[u8], from: usize) -> usize {
        let mut depth = 0i32;
        for k in from..b.len() {
            match b[k] {
                b'(' | b'[' | b'{' | b'<' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'>' if b[k - 1] != b'=' => depth -= 1,
                b';' if depth <= 0 => return k,
                b'\n' if depth <= 0 => {
                    let prev = prev_nonws(b, k).map(|p| b[p]);
                    let next = next_nonws(b, k).map(|p| b[p]);
                    let open = matches!(prev, Some(b'|' | b'&' | b'='));
                    if !open && !matches!(next, Some(b'|' | b'&')) {
                        return k;
                    }
                }
                _ => {}
            }
        }
        b.len()
    }

    /// Decorators directly before offset `pos`, walking back over `@a.B(…)`
    /// and `export`/`default`/`abstract`/`declare`.
    fn decorators_before(s: &str, pos: usize) -> Vec<Deco> {
        let b = s.as_bytes();
        let mut out = vec![];
        let mut k = pos;
        while let Some(mut p) = prev_nonws(b, k) {
            let mut args = None;
            if b[p] == b')' {
                let Some(open) = matching_back(b, p) else {
                    break;
                };
                let Some(q) = prev_nonws(b, open).filter(|&q| is_id(b[q])) else {
                    break;
                };
                args = Some((open + 1, p));
                p = q;
            }
            if !is_id(b[p]) {
                break;
            }
            let word = prev_word(s, p + 1);
            let start = p + 1 - word.len();
            match decorator_start(s, start) {
                Some(at) => {
                    out.push(Deco {
                        name: word.to_string(),
                        args,
                    });
                    k = at;
                }
                None if args.is_none() && CLASS_MODIFIERS.contains(&word) => k = start,
                None => break,
            }
        }
        out
    }

    /// The `@` of a decorator whose last name segment starts at `start`.
    fn decorator_start(s: &str, start: usize) -> Option<usize> {
        let b = s.as_bytes();
        let mut j = start;
        loop {
            match j.checked_sub(1).map(|p| b[p]) {
                Some(b'@') => return Some(j - 1),
                Some(b'.') => {
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

/// Members of a class body or Prisma model.
mod members {
    use super::*;

    pub(super) enum Kind {
        Data {
            ty: String,
            name: String,
            /// The initializer value.
            init: Option<Span>,
        },
        Method {
            name: String,
            body: Span,
            ctor: bool,
            /// `get x()` / `set x(v)`.
            accessor: bool,
        },
    }

    pub(super) struct Member {
        pub(super) kind: Kind,
        pub(super) public: bool,
        pub(super) decorators: Vec<Deco>,
    }
    impl Member {
        pub(super) fn has_any(&self, names: &[&str]) -> bool {
            self.decorators
                .iter()
                .any(|d| names.contains(&d.name.as_str()))
        }
    }

    /// Properties, methods and the constructor of the class body `{…}` at
    /// `open..close`. A member ends at `;`, after a method block, or at a line
    /// break that closes a property.
    pub(super) fn class(s: &str, (open, close): Span) -> Vec<Member> {
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
                    let header = blank_decorators(&s[start..i]);
                    let paren = find_top(&header, "(");
                    let method = paren.is_some_and(|p| first_assign(&header).is_none_or(|e| e > p));
                    if method {
                        out.extend(member(s, (start, i), Some((i, end))));
                        start = end + 1;
                    }
                    i = end + 1;
                    continue;
                }
                b';' if depth == 0 => {
                    out.extend(member(s, (start, i), None));
                    start = i + 1;
                }
                b'\n' if depth == 0 && property_ends(s, start, i) => {
                    out.extend(member(s, (start, i), None));
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        out.extend(member(s, (start, close), None));
        out
    }

    /// A line break at `i` ends the property begun at `start`: the text so far
    /// (decorators left out) ends like a complete value and the next line
    /// does not continue it.
    fn property_ends(s: &str, start: usize, i: usize) -> bool {
        let b = s.as_bytes();
        let header = blank_decorators(&s[start..i]);
        let Some(&last) = header.trim_end().as_bytes().last() else {
            return false;
        };
        let arrow = header.trim_end().ends_with("=>");
        let complete =
            (is_id(last) || matches!(last, b')' | b']' | b'}' | b'\'' | b'"' | b'>')) && !arrow;
        let next = next_nonws(b, i).map(|k| b[k]);
        complete && !matches!(next, Some(b'.' | b'|' | b'&' | b'=' | b'?' | b':' | b','))
    }

    fn member(s: &str, (a, z): Span, block: Option<Span>) -> Option<Member> {
        let header = blank_decorators(&s[a..z]);
        if header.trim().is_empty() || header.trim_start().starts_with('[') {
            return None;
        }
        let assign = first_assign(&header);
        let paren = find_top(&header, "(").filter(|&p| assign.is_none_or(|e| e > p));
        let end = paren.or(assign).unwrap_or(header.len());
        let decl_part = &header[..end];
        let colon = if paren.is_some() {
            None
        } else {
            find_top(decl_part, ":")
        };
        let (name, accessor, private) = name_of(&header, colon.unwrap_or(end))?;
        let kind = match paren {
            Some(_) => {
                let (o, c) = block?;
                Kind::Method {
                    ctor: name == "constructor",
                    name: name.to_string(),
                    body: (o + 1, c),
                    accessor,
                }
            }
            None => {
                let init = assign.map(|e| (a + e + 1, z));
                let ty = match colon {
                    Some(c) => collapse(&header[c + 1..assign.unwrap_or(header.len())]),
                    None => inferred(&collapse(&s[init?.0..init?.1]))?,
                };
                Kind::Data {
                    ty,
                    name: name.to_string(),
                    init,
                }
            }
        };
        Some(Member {
            kind,
            public: !private,
            decorators: decorators_in(s, (a, z)),
        })
    }

    /// (name, `get`/`set` accessor, private) from the words before `end`.
    fn name_of(header: &str, end: usize) -> Option<(&str, bool, bool)> {
        let words = idents(header, 0, end);
        let mut names = words.iter().filter(|w| !MODIFIERS.contains(&w.1));
        let first = names.next()?.1;
        let (name, accessor) = match names.next() {
            Some(next) if matches!(first, "get" | "set") => (next.1, true),
            _ => (first, false),
        };
        let private = words.iter().any(|w| matches!(w.1, "private" | "protected"))
            || header.trim_start().starts_with('#');
        Some((name, accessor, private))
    }

    /// The type of an untyped property from its initializer: `new T<…>(…)` →
    /// `T<…>`, `E.V` → `E`.
    fn inferred(v: &str) -> Option<String> {
        if let Some(rest) = v.strip_prefix("new ") {
            let end = rest.find('(')?;
            return Some(rest[..end].trim().to_string());
        }
        let (t, c) = v.rsplit_once('.')?;
        let plain = |x: &str| !x.is_empty() && x.bytes().all(is_id);
        (plain(t) && plain(c) && t.starts_with(char::is_uppercase)).then(|| t.to_string())
    }

    /// Decorators `@a.B(…)` in `s[a..z]`.
    fn decorators_in(s: &str, (a, z): Span) -> Vec<Deco> {
        let b = s.as_bytes();
        (a..z)
            .filter(|&k| b[k] == b'@')
            .filter_map(|k| {
                let end = (k + 1..z)
                    .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
                    .unwrap_or(z);
                let name = simple(&s[k + 1..end]);
                let args = next_nonws(b, end)
                    .filter(|&o| o < z && b[o] == b'(')
                    .and_then(|o| matching(b, o).map(|c| (o + 1, c)));
                (!name.is_empty()).then_some(Deco { name, args })
            })
            .collect()
    }

    /// `raw` with every `@a.B(…)` replaced by spaces; offsets unchanged.
    fn blank_decorators(raw: &str) -> String {
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

    /// Fields of a Prisma model: `name Type[?|[]] @attr…` per line, `@@…`
    /// block attributes left out; `@relation` kept as a decorator.
    pub(super) fn prisma(s: &str, (open, close): Span) -> Vec<Member> {
        s[open + 1..close]
            .lines()
            .filter_map(|l| {
                let mut words = l.split_whitespace();
                let (name, ty) = (words.next()?, words.next()?);
                if name.starts_with('@') || !name.bytes().all(is_id) {
                    return None;
                }
                let related = l.contains(&format!("@{PRISMA_RELATION}"));
                Some(Member {
                    kind: Kind::Data {
                        ty: ty.to_string(),
                        name: name.to_string(),
                        init: None,
                    },
                    public: true,
                    decorators: (related.then(|| Deco {
                        name: PRISMA_RELATION.into(),
                        args: None,
                    }))
                    .into_iter()
                    .collect(),
                })
            })
            .collect()
    }

    /// A field; `to_many` (`@OneToMany`/`@ManyToMany`) makes a generic type a
    /// collection of its argument. `| null`, `| undefined` and a Prisma `?`
    /// are dropped.
    pub(super) fn field(ty: &str, name: &str, to_many: bool) -> Field {
        let parts: Vec<&str> = (split_top(ty, "|").into_iter())
            .map(str::trim)
            .filter(|p| !matches!(*p, "" | "null" | "undefined"))
            .collect();
        let t = match parts.as_slice() {
            [one] => one,
            _ => ty.trim(),
        };
        let t = t.trim_end_matches('?');
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
            type_name: inner.unwrap_or(t).trim().to_string(),
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
        Switch(Span),
        /// `else`, with the condition of its braced `if` when found.
        Else(Option<Span>),
        Other,
    }

    /// One state field of one entity.
    pub(super) struct Rules<'a> {
        pub(super) s: &'a str,
        pub(super) shown: &'a str,
        pub(super) field: &'a str,
        pub(super) enum_type: &'a str,
        pub(super) states: &'a [String],
        pub(super) all: BTreeSet<String>,
        /// A string-literal union: literals are `'v'`, not `E.V`.
        pub(super) union: bool,
        pub(super) helpers: &'a BTreeSet<&'a str>,
    }

    impl Rules<'_> {
        /// What the assignment at `stmt` with value `rhs` means.
        pub(super) fn step(&self, body: Span, stmt: usize, rhs: Span, ctor: bool) -> Step {
            let value = collapse(&self.shown[rhs.0..rhs.1]);
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

        /// `E.V`, or `'v'` for a union (text from `shown`).
        pub(super) fn literal(&self, value: &str) -> Option<String> {
            let v = if self.union {
                string_literal(value)?
            } else {
                let (t, v) = value.rsplit_once('.')?;
                (simple(t) == self.enum_type).then(|| v.to_string())?
            };
            self.all.contains(&v).then_some(v)
        }

        fn ordered(&self, set: &BTreeSet<String>) -> Vec<String> {
            (self.states.iter())
                .filter(|x| set.contains(*x))
                .cloned()
                .collect()
        }

        /// The `shown` text of a sub-slice of `s`.
        fn shown_of(&self, part: &str) -> &str {
            let at = offset(self.s, part);
            &self.shown[at..at + part.len()]
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

        /// The guard from the nearest enclosing `if`, `else` or `switch` on the field.
        fn enclosing(&self, stmt: usize, stack: &[usize]) -> Option<Guard> {
            let on_field = |c: Span| mentions(&self.s[c.0..c.1], self.field);
            for h in std::iter::once(stmt).chain(stack.iter().rev().copied()) {
                match head(self.s, h) {
                    Head::If(c) if on_field(c) => return Some(self.read(c, false)),
                    Head::Else(Some(c)) if on_field(c) => return Some(self.read(c, true)),
                    Head::Switch(c) if on_field(c) => {
                        let text = collapse(&self.shown[c.0..c.1]);
                        return Some(Guard::Unread(Some(text), "switch"));
                    }
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
            let text = collapse(&self.shown[cond.0..cond.1]);
            let shown = if negate { format!("!({text})") } else { text };
            match self.cond_set(&self.s[cond.0..cond.1]) {
                Some(set) if negate => {
                    Guard::From(self.all.difference(&set).cloned().collect(), shown)
                }
                Some(set) => Guard::From(set, shown),
                None => Guard::Unread(Some(shown), "condition shape"),
            }
        }

        /// The states for which `cond` (a slice of `s`) holds, when it is one
        /// of the read shapes.
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

        /// `this.f === E.A` → (true, A); `this.f !== 'a'` → (false, a).
        fn cmp(&self, part: &str) -> Option<(bool, String)> {
            let part = unparen(part);
            let ops = [("!==", false), ("===", true), ("!=", false), ("==", true)];
            let (op, eq) = ops.into_iter().find(|(op, _)| part.contains(op))?;
            let (l, r) = part.split_once(op)?;
            let (l, r) = (l.trim(), r.trim());
            let lit = if self.is_field(l) {
                r
            } else if self.is_field(r) {
                l
            } else {
                return None;
            };
            self.literal(self.shown_of(lit)).map(|v| (eq, v))
        }

        fn is_field(&self, x: &str) -> bool {
            x.strip_prefix("this.").map(str::trim) == Some(self.field)
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
                "switch" => Head::Switch(c),
                _ => Head::Other,
            };
        }
        match prev_word(s, at) {
            "else" => Head::Else(else_cond(s, k.saturating_sub(3))),
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

    /// `;` or a line break at depth 0 from `from`.
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

    /// Assignments `this.field = …` at statement start in `body`:
    /// (statement start, value).
    pub(super) fn assignments(s: &str, body: Span, field: &str) -> Vec<(usize, Span)> {
        idents(s, body.0, body.1)
            .into_iter()
            .filter(|x| x.1 == field)
            .filter_map(|(pos, _)| {
                let b = s.as_bytes();
                let stmt = statement_start(s, body, pos)?;
                let eq = next_nonws(b, pos + field.len())
                    .filter(|&e| b[e] == b'=' && !matches!(b.get(e + 1), Some(b'=' | b'>')))?;
                Some((stmt, (eq + 1, value_end(b, eq + 1, body.1))))
            })
            .collect()
    }

    /// Start of the statement when `this.<identifier at pos>` begins one
    /// directly inside a block, not inside call arguments.
    fn statement_start(s: &str, body: Span, pos: usize) -> Option<usize> {
        let b = s.as_bytes();
        let dot = prev_nonws(b, pos).filter(|&k| b[k] == b'.')?;
        if prev_word(s, dot) != "this" {
            return None;
        }
        let stmt = prev_nonws(b, dot)? - 3;
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

    /// End of an assigned value: `;`, `,`, a closing bracket or a line break
    /// at depth 0.
    fn value_end(b: &[u8], from: usize, to: usize) -> usize {
        let from = next_nonws(b, from).unwrap_or(from).min(to);
        let mut depth = 0i32;
        for (k, &c) in b.iter().enumerate().take(to).skip(from) {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => return k,
                b')' | b']' | b'}' => depth -= 1,
                b';' | b',' | b'\n' if depth == 0 => return k,
                _ => {}
            }
        }
        to
    }

    /// Raise calls in `body`: (offset, function, argument span). `apply` on
    /// `this`, `publish` on an event bus, `emit` on an emitter.
    pub(super) fn raise_calls(s: &str, body: Span) -> Vec<(usize, &'static str, Span)> {
        let b = s.as_bytes();
        idents(s, body.0, body.1)
            .into_iter()
            .filter_map(|(pos, w)| {
                let dot = prev_nonws(b, pos).filter(|&k| b[k] == b'.')?;
                let receiver = prev_word(s, dot).to_ascii_lowercase();
                let call = [APPLY, PUBLISH, EMIT].into_iter().find(|c| *c == w)?;
                let ok = match call {
                    APPLY => receiver == "this",
                    PUBLISH => receiver.ends_with(BUS_SUFFIX),
                    _ => receiver.ends_with(EMITTER_SUFFIX),
                };
                let open = next_nonws(b, pos + w.len()).filter(|&k| ok && b[k] == b'(')?;
                let close = matching(b, open).unwrap_or(body.1);
                Some((pos, call, (open + 1, close)))
            })
            .collect()
    }

    /// The type created for a raise argument: `new X(…)`, or a variable
    /// assigned one in `before` (the method text up to the call).
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

    /// `new a.X<…>(` → `X`.
    fn new_type(x: &str) -> Option<String> {
        let rest = x.trim_start().strip_prefix("new")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = rest.trim_start();
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '.'))
            .unwrap_or(rest.len());
        let t = simple(&rest[..end]);
        let call = rest[end..].trim_start().starts_with(['(', '<']);
        (call && !t.is_empty()).then_some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::domain_model::{
        EntityKind, RelationKind, StateMachine, StateTransition, UseCaseKind, event_chain,
    };

    const ORDER: &str = r#"import { AggregateRoot } from '@nestjs/cqrs';
import { Entity, Column, OneToMany } from 'typeorm';

export enum OrderStatus {
  Submitted = 'submitted',
  Paid = 'paid',
  Shipped = 'shipped',
  Cancelled = 'cancelled',
}

/* An order. */
@Entity('orders')
export class Order extends AggregateRoot {
  @PrimaryGeneratedColumn('uuid')
  id: string;

  @OneToMany(() => OrderLine, (line) => line.order, { cascade: true })
  lines: OrderLine[];

  @Column(() => Money)
  total: Money;

  @Column()
  customerId: string;

  @Column({ type: 'enum', enum: OrderStatus })
  status: OrderStatus = OrderStatus.Submitted;

  pay(amount: number): void {
    if (this.status === OrderStatus.Submitted) {
      this.apply(new OrderPaidEvent(this.id, "}"));
      this.status = OrderStatus.Paid;
    }
  }

  ship() {
    if (this.status !== OrderStatus.Paid) {
      this.fail();
    }
    const shipped = new OrderShippedEvent(this.id);
    this.apply(shipped);
    this.status = OrderStatus.Shipped;
  }

  cancel(): void {
    if (this.status === OrderStatus.Shipped || this.status === OrderStatus.Cancelled)
      throw new Error('not { possible');
    this.status = OrderStatus.Cancelled;
  }

  restore(previous: OrderStatus) { this.status = previous; }

  archive() {
    switch (this.status) {
      case OrderStatus.Shipped:
        this.status = OrderStatus.Cancelled;
        break;
    }
  }

  get isPaid(): boolean {
    return this.status === OrderStatus.Paid;
  }

  close() {}

  private fail(): never {
    throw new Error('no');
  }
}
"#;

    const PAYMENT: &str = r#"export type PaymentState = 'pending' | 'authorized'
  | "captured" | 'failed';

@Entity()
export class Payment {
  @PrimaryKey() id!: string;
  @Enum() state: PaymentState;

  constructor(id: string) {
    this.id = id;
    this.state = 'pending';
  }

  authorize() {
    if (this.state !== 'pending') {
      return;
    }
    this.state = 'authorized';
  }

  capture() {
    if (this.state === 'authorized') {
      this.state = "captured";
    }
  }

  retry() {
    if (this.state === 'captured') {
      throw new Error('done');
    } else {
      this.state = 'pending';
    }
  }
}

export class PaymentService {
  constructor(private readonly eventEmitter: EventEmitter2) {}

  capture(payment: Payment) {
    payment.capture();
    this.eventEmitter.emit('payment.captured', { id: payment.id });
    this.eventEmitter.emit(name, payment);
  }
}

@Injectable()
export class PaymentListener {
  @OnEvent('payment.captured', { async: true })
  handleCaptured(payload: unknown) {}
}
"#;

    const SCHEMA: &str = r#"// Prisma schema
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

model Invoice {
  id     String        @id @default(uuid())
  number String        @unique
  lines  InvoiceLine[]
  status InvoiceStatus @default(OPEN)
  @@map("invoices")
}

model InvoiceLine {
  id        Int     @id @default(autoincrement())
  invoice   Invoice @relation(fields: [invoiceId], references: [id])
  invoiceId String
  note      String?
}

enum InvoiceStatus {
  OPEN
  PAID
}
"#;

    fn files() -> Vec<(&'static str, &'static str)> {
        vec![
            ("src/orders/domain/order.entity.ts", ORDER),
            (
                "src/orders/domain/order-line.entity.ts",
                "@Entity()\nexport class OrderLine {\n  @PrimaryGeneratedColumn() id: number;\n  @ManyToOne(() => Order, (o) => o.lines)\n  order: Order;\n  @Column() sku: string;\n}\n",
            ),
            (
                "src/orders/domain/money.ts",
                "@Embeddable()\nexport class Money {\n  @Property() amount: number;\n  @Property() currency: string;\n}\n",
            ),
            (
                "src/customers/customer.entity.ts",
                "@Entity()\nexport class Customer extends AggregateRoot {\n  @PrimaryKey() id!: string;\n  @OneToMany(() => Address, (a) => a.customer)\n  addresses = new Collection<Address>(this);\n  rename(name: string) {}\n  close() {}\n}\n\n@Entity()\nexport class Address {\n  @PrimaryKey() id!: number;\n  @Property() street?: string | null;\n}\n",
            ),
            ("src/payments/payment.ts", PAYMENT),
            ("prisma/schema.prisma", SCHEMA),
            (
                "src/orders/application/pay-order.handler.ts",
                "@CommandHandler(PayOrderCommand)\nexport class PayOrderHandler implements ICommandHandler<PayOrderCommand> {\n  constructor(private readonly repo: OrderRepository, private readonly publisher: EventPublisher) {}\n\n  async execute(command: PayOrderCommand): Promise<void> {\n    const order = this.publisher.mergeObjectContext(await this.repo.findOne(command.id));\n    order.pay(command.amount);\n    order.commit();\n  }\n}\n",
            ),
            (
                "src/orders/application/get-order.handler.ts",
                "@QueryHandler(GetOrderQuery)\nexport class GetOrderHandler implements IQueryHandler<GetOrderQuery> {\n  async execute(query: GetOrderQuery) {\n    return null;\n  }\n}\n",
            ),
            (
                "src/orders/application/ship-order.use-case.ts",
                "export class ShipOrderUseCase {\n  constructor(private readonly eventBus: EventBus) {}\n\n  async run(id: string) {\n    order.ship();\n    order.close();\n    customer.rename('x');\n    this.eventBus.publish(new OrderAuditedEvent(id));\n  }\n}\n",
            ),
            (
                "src/orders/application/order-paid.handler.ts",
                "@EventsHandler(OrderPaidEvent, OrderShippedEvent)\nexport class OrderPaidHandler implements IEventHandler<OrderPaidEvent> {\n  handle(event: OrderPaidEvent) {}\n}\n\nexport class AuditHandler implements IEventHandler<OrderAuditedEvent> {\n  handle(event: OrderAuditedEvent) {}\n}\n",
            ),
            (
                "node_modules/lib/ghost.entity.ts",
                "@Entity() export class Ghost {}\n",
            ),
            ("dist/gen.ts", "@Entity() export class Gen {}\n"),
            (
                "src/orders/order.spec.ts",
                "@Entity() export class FakeOrder {}\n",
            ),
            ("test/fixtures.ts", "@Entity() export class FakeLine {}\n"),
            (
                "src/types.d.ts",
                "@Entity() export declare class Declared {}\n",
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
        (model.entities.iter())
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

    #[test]
    fn entities_by_decorator_base_and_prisma_model() {
        let model = run(&files());
        assert_eq!(
            kinds(&model),
            vec![
                ("Address", EntityKind::Entity),
                ("Customer", EntityKind::AggregateRoot),
                ("Invoice", EntityKind::Entity),
                ("InvoiceLine", EntityKind::Entity),
                ("Money", EntityKind::ValueObject),
                ("Order", EntityKind::AggregateRoot),
                ("OrderLine", EntityKind::Entity),
                ("Payment", EntityKind::Entity),
            ]
        );
        assert!(model.entities.iter().all(|e| e.source == "typescript"));
        let order = model.entities.iter().find(|e| e.id == "Order").unwrap();
        assert_eq!(
            (order.file.as_str(), order.line),
            ("src/orders/domain/order.entity.ts", 13)
        );
        let field = |name: &str| order.fields.iter().find(|f| f.name == name).unwrap();
        assert_eq!(
            (field("lines").type_name.as_str(), field("lines").collection),
            ("OrderLine", true)
        );
        assert_eq!(field("customerId").type_name, "string");
        assert_eq!(field("status").type_name, "OrderStatus");
        let address = model.entities.iter().find(|e| e.id == "Address").unwrap();
        assert_eq!(address.fields[1].type_name, "string");
        let invoice = model.entities.iter().find(|e| e.id == "Invoice").unwrap();
        let fields: Vec<(&str, &str, bool)> = (invoice.fields.iter())
            .map(|f| (f.name.as_str(), f.type_name.as_str(), f.collection))
            .collect();
        assert_eq!(
            fields,
            [
                ("id", "String", false),
                ("number", "String", false),
                ("lines", "InvoiceLine", true),
                ("status", "InvoiceStatus", false),
            ]
        );
        assert_eq!(
            (invoice.file.as_str(), invoice.line),
            ("prisma/schema.prisma", 7)
        );
    }

    #[test]
    fn relations_from_decorators_prisma_and_cross_module_ids() {
        let model = run(&files());
        assert_eq!(
            relations(&model),
            vec![
                ("Customer", "Address", RelationKind::HasMany, "addresses"),
                ("Invoice", "InvoiceLine", RelationKind::HasMany, "lines"),
                ("InvoiceLine", "Invoice", RelationKind::HasOne, "invoice"),
                ("Order", "Customer", RelationKind::References, "customerId"),
                ("Order", "Money", RelationKind::HasOne, "total"),
                ("Order", "OrderLine", RelationKind::HasMany, "lines"),
                ("OrderLine", "Order", RelationKind::HasOne, "order"),
            ]
        );
    }

    #[test]
    fn enum_state_machine_guards_initial_and_events() {
        let model = run(&files());
        let m = machine(&model, "Order");
        assert_eq!(
            (m.field.as_str(), m.enum_type.as_str()),
            ("status", "OrderStatus")
        );
        let all = ["Submitted", "Paid", "Shipped", "Cancelled"];
        assert_eq!(m.states, s(&all));
        assert_eq!(m.initial.as_deref(), Some("Submitted"));
        let pay = transition(m, "Order.pay");
        assert_eq!(
            (pay.from.clone(), pay.to.as_str()),
            (s(&["Submitted"]), "Paid")
        );
        assert_eq!(
            pay.guard.as_deref(),
            Some("this.status === OrderStatus.Submitted")
        );
        assert_eq!(pay.events, s(&["OrderPaidEvent"]));
        assert_eq!(
            (pay.file.as_str(), pay.line),
            ("src/orders/domain/order.entity.ts", 32)
        );
        // Exit forms read as the allowed condition: `!(source)`.
        let ship = transition(m, "Order.ship");
        assert_eq!(
            (ship.from.clone(), ship.to.as_str()),
            (s(&["Paid"]), "Shipped")
        );
        assert_eq!(
            ship.guard.as_deref(),
            Some("!(this.status !== OrderStatus.Paid)")
        );
        assert_eq!(ship.events, s(&["OrderShippedEvent"]));
        let cancel = transition(m, "Order.cancel");
        assert_eq!(cancel.from, s(&["Submitted", "Paid"]));
        assert_eq!(
            cancel.guard.as_deref(),
            Some("!(this.status === OrderStatus.Shipped || this.status === OrderStatus.Cancelled)")
        );
        let archive = transition(m, "Order.archive");
        assert_eq!(
            (archive.from.clone(), archive.to.as_str()),
            (vec![], "Cancelled")
        );
        assert!(has_diag(
            &model,
            "Order.archive: guard of status not read (switch)"
        ));
        assert!(has_diag(
            &model,
            "Order.restore assigns status the non-literal `previous`"
        ));
        assert_eq!(m.transitions.len(), 4);
        assert!(!model.complete);
    }

    #[test]
    fn string_union_state_machine_with_constructor_initial_and_else_guard() {
        let model = run(&files());
        let m = machine(&model, "Payment");
        assert_eq!(
            (m.field.as_str(), m.enum_type.as_str()),
            ("state", "PaymentState")
        );
        assert_eq!(
            m.states,
            s(&["pending", "authorized", "captured", "failed"])
        );
        assert_eq!(m.initial.as_deref(), Some("pending"));
        let authorize = transition(m, "Payment.authorize");
        assert_eq!(
            (authorize.from.clone(), authorize.to.as_str()),
            (s(&["pending"]), "authorized")
        );
        assert_eq!(
            authorize.guard.as_deref(),
            Some("!(this.state !== 'pending')")
        );
        let capture = transition(m, "Payment.capture");
        assert_eq!(
            (capture.from.clone(), capture.to.as_str()),
            (s(&["authorized"]), "captured")
        );
        assert_eq!(
            capture.guard.as_deref(),
            Some("this.state === 'authorized'")
        );
        let retry = transition(m, "Payment.retry");
        assert_eq!(retry.from, s(&["pending", "authorized", "failed"]));
        assert_eq!(retry.to, "pending");
        assert_eq!(retry.guard.as_deref(), Some("!(this.state === 'captured')"));
        assert_eq!(m.transitions.len(), 3);
        // Prisma enums have no methods, so no machine.
        assert!(model.state_machines.iter().all(|m| m.entity != "Invoice"));
    }

    #[test]
    fn events_from_apply_publish_emit_and_their_handlers() {
        let model = run(&files());
        let events: Vec<(&str, Vec<String>, Vec<String>)> = (model.events.iter())
            .map(|e| {
                assert_eq!(e.source, "typescript");
                (
                    e.type_name.as_str(),
                    e.raised_by.clone(),
                    e.handled_by.clone(),
                )
            })
            .collect();
        assert_eq!(
            events,
            vec![
                (
                    "OrderAuditedEvent",
                    s(&["ShipOrderUseCase.run"]),
                    s(&["AuditHandler"])
                ),
                (
                    "OrderPaidEvent",
                    s(&["Order.pay"]),
                    s(&["OrderPaidHandler"])
                ),
                (
                    "OrderShippedEvent",
                    s(&["Order.ship"]),
                    s(&["OrderPaidHandler"])
                ),
                (
                    "payment.captured",
                    s(&["PaymentService.capture"]),
                    s(&["PaymentListener"])
                ),
            ]
        );
        assert!(has_diag(
            &model,
            "PaymentService.capture: emit event name is not a string literal"
        ));
        // Only the four expected diagnostics: ambiguous call, non-literal,
        // switch and non-literal emit name.
        assert_eq!(model.diagnostics.len(), 4, "{:?}", model.diagnostics);
    }

    #[test]
    fn use_cases_by_decorator_and_name_with_calls() {
        let model = run(&files());
        let cases: Vec<(&str, UseCaseKind, Option<&str>, Vec<String>)> = (model.use_cases.iter())
            .map(|u| (u.id.as_str(), u.kind, u.request.as_deref(), u.calls.clone()))
            .collect();
        assert_eq!(
            cases,
            vec![
                ("AuditHandler", UseCaseKind::Handler, None, vec![]),
                (
                    "GetOrderHandler",
                    UseCaseKind::Query,
                    Some("GetOrderQuery"),
                    vec![]
                ),
                ("OrderPaidHandler", UseCaseKind::Handler, None, vec![]),
                (
                    "PayOrderHandler",
                    UseCaseKind::Command,
                    Some("PayOrderCommand"),
                    s(&["Order.pay"])
                ),
                (
                    "ShipOrderUseCase",
                    UseCaseKind::Handler,
                    None,
                    s(&["Customer.rename", "Order.ship"])
                ),
            ]
        );
        assert!(model.use_cases.iter().all(|u| u.source == "typescript"));
        // `close` is on Order and Customer.
        let ambiguous = (model.diagnostics.iter())
            .find(|d| d.message.contains(".close("))
            .expect("ambiguous close");
        assert_eq!(
            (ambiguous.file.as_str(), ambiguous.line),
            ("src/orders/application/ship-order.use-case.ts", 6)
        );
    }

    #[test]
    fn nestjs_cqrs_chain_from_command_handler_to_event_handler() {
        let model = run(&files());
        let handler = (model.use_cases.iter())
            .find(|u| u.request.as_deref() == Some("PayOrderCommand"))
            .unwrap();
        let method = &handler.calls[0];
        let pay = (model.state_machines.iter())
            .flat_map(|m| &m.transitions)
            .find(|t| &t.method == method)
            .unwrap();
        assert_eq!(pay.to, "Paid");
        let chain = event_chain(&model, pay);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0.type_name, "OrderPaidEvent");
        assert_eq!(chain[0].1, ["OrderPaidHandler"]);
    }

    #[test]
    fn node_modules_dist_tests_and_declarations_are_skipped() {
        let model = run(&files());
        for name in ["Ghost", "Gen", "FakeOrder", "FakeLine", "Declared"] {
            assert!(model.entities.iter().all(|e| e.name != name), "{name}");
        }
    }

    #[test]
    fn duplicate_names_are_qualified_and_reported() {
        let model = run(&[
            ("a/item.ts", "\u{FEFF}@Entity()\nexport class Item {}\n"),
            ("b/item.ts", "@Entity()\nexport class Item {}\n"),
        ]);
        let ids: Vec<&str> = model.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a.Item", "b.Item"]);
        assert!(has_diag(&model, "Several entities are named Item"));
        assert!(model.complete);
    }

    #[test]
    fn unreadable_file_is_a_diagnostic() {
        let big = format!("@Entity() class Big {{}}\n{}", " ".repeat(1 << 20));
        let model = run(&[("big.ts", big.as_str())]);
        assert!(model.entities.is_empty());
        assert_eq!(model.diagnostics[0].file, "big.ts");
        assert!(!model.complete);
    }

    const TABLES: [(&str, &str); 3] = [
        (
            "t/user.table.ts",
            "import { Table } from '@immich/sql-tools';\n@Table('user')\nexport class UserTable {\n  @PrimaryGeneratedColumn() id!: string;\n}\n",
        ),
        (
            "t/album.table.ts",
            "import { Table } from '@immich/sql-tools';\n@Table('album')\nexport class AlbumTable {\n  @PrimaryGeneratedColumn() id!: string;\n  @ForeignKeyColumn(() => UserTable, { onDelete: 'CASCADE' })\n  ownerId!: string;\n}\n",
        ),
        (
            "t/fake.ts",
            "export class FakeEntity {\n  @Column() a: string;\n}\n",
        ),
    ];

    #[test]
    fn table_decorator_is_an_entity_and_foreign_key_column_a_relation() {
        let model = run(&TABLES);
        let names: Vec<&str> = model.entities.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["AlbumTable", "UserTable"]);
        let r = &model.relations[0];
        assert_eq!(
            (r.from.as_str(), r.to.as_str()),
            ("AlbumTable", "UserTable")
        );
        assert_eq!((r.kind, r.via.as_str()), (RelationKind::HasOne, "ownerId"));
        assert!(model.complete);
    }

    #[test]
    fn unresolvable_arrow_target_is_a_gap() {
        let model = run(&[(
            "a.ts",
            "@Entity()\nexport class A {\n  @ManyToOne(() => Missing)\n  m: Rel<Missing>;\n}\n",
        )]);
        assert!(model.relations.is_empty());
        assert!(has_diag(&model, "A.m (@ManyToOne)"));
        assert!(!model.complete);
    }

    #[test]
    fn table_from_a_non_orm_module_is_not_an_entity() {
        let model = run(&[(
            "ui.ts",
            "import { Table } from 'antd';\n@Table('x')\nexport class Grid {\n  @Column() a: string;\n}\n",
        )]);
        assert!(model.entities.is_empty());
    }

    #[test]
    fn table_from_an_orm_package_is_an_entity() {
        let model = run(&[(
            "t.ts",
            "import {\n  Column,\n  Table,\n} from '@immich/sql-tools';\n@Table('x')\nexport class XTable {}\n",
        )]);
        assert_eq!(model.entities.len(), 1);
    }

    #[test]
    fn kysely_and_drizzle_imports_are_one_gap_each() {
        let model = run(&[
            (
                "a.ts",
                "import { Kysely } from 'kysely';\nexport interface DB {}\n",
            ),
            ("b.ts", "import { Kysely } from 'kysely';\n"),
            (
                "c.ts",
                "import { pgTable, text } from 'drizzle-orm/pg-core';\nexport const u = pgTable('u', { a: text('a') });\n",
            ),
        ]);
        assert!(has_diag(&model, "Kysely"));
        assert!(has_diag(&model, "Drizzle"));
        assert_eq!(model.diagnostics.len(), 2);
        assert!(!model.complete);
    }
}
