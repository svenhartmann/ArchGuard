//! C# extractor for domain-model/v1 (ADR 0022, phase 2): a text-level scanner
//! (comments, preprocessor lines and string contents blanked, brackets
//! matched), no Roslyn. Every entry has `source: "csharp"`.
//!
//! Recognized:
//! - Files: `*.cs` below the root, skipping `bin`, `obj`, the directories the
//!   service scan excludes and test paths; files over 1 MiB, unreadable files
//!   and files past the first 5 000 are diagnostics.
//! - Entities: a class or record whose base list (also over several lines)
//!   names `IAggregateRoot`/`AggregateRoot` → `aggregate_root`, `Entity` →
//!   `entity`, `ValueObject` → `value_object`; a `record` in the folder of an
//!   aggregate root → `value_object`; the `T` of an EF `DbSet<T>` not
//!   classified otherwise → `unknown` (the class with the longest common
//!   namespace prefix; a tie is a diagnostic). Classes named like a marker are
//!   the markers and skipped. `id` is the class name, namespace-qualified when
//!   two entities share a name (with a diagnostic).
//! - Fields: properties and fields of an entity class plus positional record
//!   parameters; `List`, `IList`, `ICollection`, `IReadOnlyCollection`,
//!   `IReadOnlyList`, `IEnumerable`, `HashSet` and arrays are `collection`
//!   with the element type.
//! - Relations: a field whose type is a found entity → `has_one`, a collection
//!   of one → `has_many`; a `_field` exposed by a public property of the same
//!   name and element type gets the property name as `via`. An `int`/`Guid`
//!   (nullable too) `XId` with `X` an aggregate root in another namespace
//!   (folder without one) → `references`.
//! - State machines: an enum-typed field assigned inside a method of its entity
//!   (`Field = Enum.Value;`, also `this.Field`). From states come from the
//!   nearest enclosing `if` comparing the field with enum literals (`==` joined
//!   by `||`, `!=` joined by `&&`, one `==`/`!=`), from the `else` of such an
//!   `if` (negated), or from the nearest preceding `if` on the field whose
//!   block throws, returns or calls an entity method that only throws
//!   (negated). The guard text is the allowed condition: a negated guard is
//!   the literal `!(source condition)`, never simplified. Without such an
//!   `if`, from = [] (any). A constructor
//!   assignment is `initial`. A non-literal value, a `switch` on the field or a
//!   guard of another shape is a diagnostic and makes the model incomplete.
//! - Events: `AddDomainEvent(new X(…))`, or of a variable assigned `new X(…)`
//!   earlier in the method → `X` on that method's transitions and `raised_by:
//!   "Entity.Method"`; handlers: classes implementing `INotificationHandler<X>`.
//! - Use cases: classes implementing `IRequestHandler<TReq, …>` (request
//!   `TReq`) or named `*CommandHandler`, `*QueryHandler`, `*UseCase`; kind
//!   `command`/`query` when the name contains `Command`/`Query`, else
//!   `handler`. `calls`: `.M(` in the class where `M` is a public method of
//!   exactly one entity. This unique-name rule stands in for the
//!   compiler-backed call model of ADR 0022 decision 2; a name on several
//!   entities is a diagnostic and not recorded.
//! - Message handlers (chain starts without an endpoint): classes implementing
//!   `IIntegrationEventHandler<T>` (eShop), MassTransit `IConsumer<T>` or
//!   NServiceBus `IHandleMessages<T>`, one entry per `T`; `sends` by the
//!   backend extractor's send rule over the class body (`mediator.Send(…)`
//!   and the like; `context.Send`/`Publish` are not read).
//! - Rules, validations, integration events and plain models: see `rich`.
//!
//! Not recognized: partial classes across files, nested-type qualification,
//! state changes outside the entity's own methods, `switch`/`is`/pattern
//! guards, `else if` chains as negations, guards on other fields, the State
//! pattern or Stateless, and constructor events as part of `initial`. The
//! marker names are the constants below with the ADR 0022 defaults; reading
//! them from the configuration comes later.
use crate::extract_ctx::{ExtractContext, domain_sources, with_root_config};
use crate::ui_flow_backend::sends_in;
use archguard_core::domain_model::{
    DomainDiagnostic, DomainEvent, DomainModel, Entity, EntityKind, EventHandlerSite, Field,
    MessageHandler, Relation, RelationKind, StateMachine, StateTransition, UseCase, UseCaseKind,
};
use members::{field, generic_args, members};
use states::{Rules, Step, assignments, created, raise_calls};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use text::*;

const SOURCE: &str = "csharp";
const AGGREGATE_MARKERS: [&str; 2] = ["IAggregateRoot", "AggregateRoot"];
const ENTITY_BASE: &str = "Entity";
const VALUE_OBJECT_BASE: &str = "ValueObject";
const RAISE_FUNCTION: &str = "AddDomainEvent";
const EVENT_HANDLER: &str = "INotificationHandler";
const REQUEST_HANDLER: &str = "IRequestHandler";
const USE_CASE_SUFFIXES: [&str; 3] = ["CommandHandler", "QueryHandler", "UseCase"];
const MESSAGE_HANDLERS: [&str; 3] = ["IIntegrationEventHandler", "IConsumer", "IHandleMessages"];
const COLLECTIONS: [&str; 7] = [
    "List",
    "IList",
    "ICollection",
    "IReadOnlyCollection",
    "IReadOnlyList",
    "IEnumerable",
    "HashSet",
];
const ID_TYPES: [&str; 2] = ["int", "Guid"];
const MODIFIERS: [&str; 16] = [
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "readonly",
    "const",
    "virtual",
    "override",
    "abstract",
    "sealed",
    "new",
    "required",
    "volatile",
    "unsafe",
    "partial",
];

/// Byte range in a file's text.
pub(crate) type Span = (usize, usize);
/// Diagnostics found on the way: file index, offset, message, drops an element.
type Pending = Vec<(usize, usize, String, bool)>;

/// The domain model of the C# sources below `root`.
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
    /// The source as read, for messages and rule text.
    raw: String,
    /// The folder of the nearest `*.csproj`, the service of a publish site.
    project: String,
}

struct Decl {
    file: usize,
    kw: String,
    name: String,
    ns: String,
    dir: String,
    pos: usize,
    bases: Vec<String>,
    params: Option<Span>,
    body: Option<Span>,
}

enum Kind {
    Data {
        ty: String,
        name: String,
    },
    Method {
        name: String,
        body: Span,
        ctor: bool,
    },
}

struct Member {
    kind: Kind,
    public: bool,
    /// Declaration text after the attributes, up to the body.
    head: Span,
    /// A property, or a public mutable field: state a class carries. Not a
    /// `const`, a private or `readonly` field (an injected dependency, a
    /// singleton).
    state: bool,
}

struct Ent {
    id: String,
    name: String,
    kind: EntityKind,
    file: usize,
    pos: usize,
    ns: String,
    dir: String,
    members: Vec<Member>,
    fields: Vec<Field>,
}
impl Ent {
    /// An entity known only from `DbSet<T>`, without a class.
    fn bare(file: usize, pos: usize, name: &str) -> Self {
        Self {
            id: name.into(),
            name: name.into(),
            kind: EntityKind::Unknown,
            file,
            pos,
            ns: String::new(),
            dir: String::new(),
            members: vec![],
            fields: vec![],
        }
    }
    fn scope(&self) -> &str {
        if self.ns.is_empty() {
            &self.dir
        } else {
            &self.ns
        }
    }
    fn methods(&self) -> impl Iterator<Item = (&str, Span, bool, bool)> {
        self.members.iter().filter_map(|m| match &m.kind {
            Kind::Method { name, body, ctor } => Some((name.as_str(), *body, *ctor, m.public)),
            Kind::Data { .. } => None,
        })
    }
    /// The relation name of `f`: a `_field` exposed by a public property of
    /// the same name and type is named after the property.
    fn via(&self, f: &Field) -> String {
        let public = |name: &str| {
            self.members
                .iter()
                .any(|m| m.public && matches!(&m.kind, Kind::Data { name: n, .. } if n == name))
        };
        f.name
            .strip_prefix('_')
            .and_then(|bare| {
                self.fields.iter().find(|p| {
                    p.name.eq_ignore_ascii_case(bare)
                        && p.collection == f.collection
                        && p.type_name == f.type_name
                        && public(&p.name)
                })
            })
            .map_or_else(|| f.name.clone(), |p| p.name.clone())
    }
    /// Methods whose body only throws, so calling them exits like `throw`.
    fn throwing<'a>(&'a self, s: &str) -> BTreeSet<&'a str> {
        self.methods()
            .filter(|(_, body, ..)| s[body.0..body.1].trim_start().starts_with("throw"))
            .map(|(n, ..)| n)
            .collect()
    }
}

#[derive(Default)]
struct Extractor {
    model: DomainModel,
    files: Vec<Src>,
    decls: Vec<Decl>,
    /// `DbSet<T>`: file, offset, `T`.
    sets: Vec<(usize, usize, String)>,
    ents: Vec<Ent>,
    pending: Pending,
}

impl Extractor {
    fn read(&mut self, root: &Path) {
        let (sources, gaps) =
            domain_sources(root, &["bin", "obj"], |rel| rel.ends_with(".cs"), "C#");
        for (file, message) in gaps {
            self.push_diag(&file, message);
        }
        let mut projects = BTreeMap::new();
        for (rel, raw) in sources {
            let text = crate::lang_csharp::mask(&raw);
            let project = code::project_of(root, &rel, &[".csproj"], &mut projects);
            self.files.push(Src {
                rel,
                text,
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

    /// Type declarations and `DbSet<T>` mentions of one file.
    fn declarations(&mut self, f: usize) {
        let src = &self.files[f];
        let dir = src.rel.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let (found, sets) = decls::parse(&src.text);
        self.sets
            .extend(sets.into_iter().map(|(pos, t)| (f, pos, t)));
        self.decls.extend(found.into_iter().map(|d| Decl {
            file: f,
            dir: dir.clone(),
            ..d
        }));
    }

    fn entities(&mut self) {
        let kinds = self.classify();
        let unknown = self.dbset_targets(&kinds);
        let mut ents: Vec<Ent> = kinds.into_iter().map(|(d, k)| self.ent(d, k)).collect();
        ents.extend(unknown.into_iter().map(|(d, f, pos, t)| match d {
            Some(d) => self.ent(d, EntityKind::Unknown),
            None => Ent::bare(f, pos, &t),
        }));
        self.pending.extend(qualify(&mut ents));
        self.model.entities = ents
            .iter()
            .map(|e| Entity {
                id: e.id.clone(),
                name: e.name.clone(),
                kind: e.kind,
                fields: e.fields.clone(),
                file: self.files[e.file].rel.clone(),
                line: line(&self.files[e.file].text, e.pos),
                source: SOURCE.into(),
            })
            .collect();
        self.ents = ents;
    }

    /// Declarations with a marker, then records in an aggregate root's folder.
    fn classify(&self) -> Vec<(usize, EntityKind)> {
        let mut kinds: Vec<(usize, EntityKind)> = self
            .decls
            .iter()
            .enumerate()
            .filter_map(|(d, decl)| marker_kind(decl).map(|k| (d, k)))
            .collect();
        let roots: BTreeSet<&str> = kinds
            .iter()
            .filter(|(_, k)| *k == EntityKind::AggregateRoot)
            .map(|&(d, _)| self.decls[d].dir.as_str())
            .collect();
        let records: Vec<(usize, EntityKind)> = self
            .decls
            .iter()
            .enumerate()
            .filter(|(d, decl)| {
                decl.kw == "record"
                    && !kinds.iter().any(|k| k.0 == *d)
                    && roots.contains(decl.dir.as_str())
            })
            .map(|(d, _)| (d, EntityKind::ValueObject))
            .collect();
        kinds.extend(records);
        let plain = rich::plain_models(self, &kinds);
        kinds.extend(plain);
        kinds
    }

    /// `DbSet<T>` targets not classified otherwise: (class, file, offset, `T`).
    fn dbset_targets(
        &mut self,
        kinds: &[(usize, EntityKind)],
    ) -> Vec<(Option<usize>, usize, usize, String)> {
        let mut out: Vec<(Option<usize>, usize, usize, String)> = vec![];
        for (f, pos, t) in std::mem::take(&mut self.sets) {
            if kinds.iter().any(|&(d, _)| self.decls[d].name == t) || out.iter().any(|u| u.3 == t) {
                continue;
            }
            match self.resolve_set(f, &t) {
                Some(d) => out.push((d, f, pos, t)),
                None => self.pending.push((
                    f,
                    pos,
                    format!(
                        "DbSet<{t}> matches several classes named {t}; the entity is not read."
                    ),
                    true,
                )),
            }
        }
        out
    }

    /// The class `T` of a `DbSet<T>` in file `f` means, by the longest common
    /// namespace prefix: `Some(None)` without a class, `None` on a tie.
    fn resolve_set(&self, f: usize, t: &str) -> Option<Option<usize>> {
        let ns = self
            .decls
            .iter()
            .find(|d| d.file == f)
            .map_or("", |d| d.ns.as_str());
        let candidates: Vec<usize> = (0..self.decls.len())
            .filter(|&d| {
                self.decls[d].name == t && matches!(self.decls[d].kw.as_str(), "class" | "record")
            })
            .collect();
        let score = |d: usize| common_prefix(&self.decls[d].ns, ns);
        let best = candidates.iter().map(|&d| score(d)).max();
        let top: Vec<usize> = candidates
            .into_iter()
            .filter(|&d| Some(score(d)) == best)
            .collect();
        match top.as_slice() {
            [] => Some(None),
            [d] => Some(Some(*d)),
            _ => None,
        }
    }

    fn ent(&self, d: usize, kind: EntityKind) -> Ent {
        let decl = &self.decls[d];
        let s = &self.files[decl.file].text;
        let members = decl
            .body
            .map_or(vec![], |body| members(s, body, &decl.name));
        let params = decl.params.filter(|_| decl.kw == "record");
        let mut fields: Vec<Field> = params
            .map_or(vec![], |(a, z)| split_top(&s[a..z], ","))
            .into_iter()
            .filter_map(|p| members::data(skip_attributes(p).split('=').next().unwrap_or("")))
            .map(|(ty, n)| field(&ty, &n))
            .collect();
        fields.extend(members.iter().filter_map(|m| match &m.kind {
            Kind::Data { ty, name } => Some(field(ty, name)),
            Kind::Method { .. } => None,
        }));
        Ent {
            id: decl.name.clone(),
            name: decl.name.clone(),
            kind,
            file: decl.file,
            pos: decl.pos,
            ns: decl.ns.clone(),
            dir: decl.dir.clone(),
            members,
            fields,
        }
    }

    /// The entity a type name means as seen from entity `from`; `None` when no
    /// entity or several outside `from`'s namespace have that name.
    fn resolve(&self, name: &str, from: usize) -> Option<usize> {
        let all: Vec<usize> = (0..self.ents.len())
            .filter(|&e| self.ents[e].name == name)
            .collect();
        match all.as_slice() {
            [one] => Some(*one),
            _ => all
                .into_iter()
                .find(|&e| self.ents[e].ns == self.ents[from].ns),
        }
    }

    fn relations(&mut self) {
        let mut found = vec![];
        for (e, ent) in self.ents.iter().enumerate() {
            for f in &ent.fields {
                if let Some((to, kind, via)) = self.relation_of(e, f) {
                    found.push(Relation {
                        from: ent.id.clone(),
                        to: self.ents[to].id.clone(),
                        kind,
                        via,
                        source: SOURCE.into(),
                    });
                }
            }
        }
        self.model.relations = found;
    }

    fn relation_of(&self, e: usize, f: &Field) -> Option<(usize, RelationKind, String)> {
        let ent = &self.ents[e];
        if let Some(to) = self.resolve(&simple(&f.type_name), e) {
            let kind = if f.collection {
                RelationKind::HasMany
            } else {
                RelationKind::HasOne
            };
            return Some((to, kind, ent.via(f)));
        }
        let x = f
            .name
            .strip_suffix("Id")
            .filter(|_| !f.collection && ID_TYPES.contains(&f.type_name.as_str()))?;
        let to = self.resolve(x, e).filter(|&to| {
            self.ents[to].kind == EntityKind::AggregateRoot && self.ents[to].scope() != ent.scope()
        })?;
        Some((to, RelationKind::References, f.name.clone()))
    }

    /// The members of the enum `name` as seen from entity `e`.
    fn enum_states(&self, name: &str, e: usize) -> Option<Vec<String>> {
        let all: Vec<&Decl> = self
            .decls
            .iter()
            .filter(|d| d.kw == "enum" && d.name == name)
            .collect();
        let decl = match all.as_slice() {
            [one] => *one,
            _ => *all.iter().find(|d| d.ns == self.ents[e].ns)?,
        };
        let s = &self.files[decl.file].text;
        let (a, z) = decl.body?;
        Some(
            split_top(&s[a + 1..z], ",")
                .into_iter()
                .filter_map(|m| {
                    let m = skip_attributes(m);
                    idents(m, 0, m.len()).first().map(|x| x.1.to_string())
                })
                .collect(),
        )
    }

    fn machines_and_events(&mut self) {
        let mut raised: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut pending = Pending::new();
        let mut machines = vec![];
        for e in 0..self.ents.len() {
            let events = self.raised_events(e, &mut raised, &mut pending);
            for m in &self.ents[e].members {
                if let Kind::Data { ty, name } = &m.kind
                    && let Some(machine) =
                        self.machine_of(e, (ty.as_str(), name.as_str()), &events, &mut pending)
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

    /// Event types raised per method of entity `e`; adds the raisers to `raised`.
    fn raised_events(
        &self,
        e: usize,
        raised: &mut BTreeMap<String, BTreeSet<String>>,
        pending: &mut Pending,
    ) -> BTreeMap<String, Vec<String>> {
        let ent = &self.ents[e];
        let s = &self.files[ent.file].text;
        let mut events: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, body, ..) in ent.methods() {
            for (pos, arg) in raise_calls(s, body) {
                let Some(t) = created(s, (body.0, pos), arg) else {
                    let message = format!(
                        "{}.{name}: {RAISE_FUNCTION} argument is neither `new X(…)` nor a variable assigned one; the event is not read.",
                        ent.id
                    );
                    pending.push((ent.file, pos, message, true));
                    continue;
                };
                raised
                    .entry(t.clone())
                    .or_default()
                    .insert(format!("{}.{name}", ent.id));
                events.entry(name.to_string()).or_default().push(t);
            }
        }
        events
    }

    /// The state machine of field `(type, name)` of entity `e`, when the type is
    /// an enum and some method of the entity assigns it a literal member.
    fn machine_of(
        &self,
        e: usize,
        (ty, field_name): (&str, &str),
        events: &BTreeMap<String, Vec<String>>,
        pending: &mut Pending,
    ) -> Option<StateMachine> {
        let ent = &self.ents[e];
        let enum_type = simple(ty);
        if field(ty, "").collection {
            return None;
        }
        let states = self.enum_states(&enum_type, e)?;
        let s = &self.files[ent.file].text;
        let helpers = ent.throwing(s);
        let rules = Rules::new(s, field_name, &enum_type, &states, &helpers);
        let mut machine = StateMachine {
            entity: ent.id.clone(),
            field: field_name.into(),
            enum_type: enum_type.clone(),
            states: states.clone(),
            initial: None,
            transitions: vec![],
        };
        let mut literal = false;
        let mut gaps = Pending::new();
        for (method, body, ctor, _) in ent.methods() {
            for (stmt, rhs) in assignments(s, body, field_name) {
                let at = format!("{}.{method}", ent.id);
                match rules.step(body, stmt, rhs, ctor) {
                    Step::NonLiteral(v) => gaps.push((ent.file, stmt, format!("{at} assigns {field_name} the non-literal `{v}`; the transition is not read."), true)),
                    Step::Initial(to) => {
                        literal = true;
                        machine.initial.get_or_insert(to);
                    }
                    Step::Transition { from, to, guard, unread } => {
                        literal = true;
                        if let Some(why) = unread {
                            pending.push((ent.file, stmt, format!("{at}: guard of {field_name} not read ({why}); from states unknown."), true));
                        }
                        machine.transitions.push(StateTransition {
                            from,
                            to,
                            method: at,
                            guard,
                            events: events.get(method).cloned().unwrap_or_default(),
                            file: self.files[ent.file].rel.clone(),
                            line: line(s, stmt),
                        });
                    }
                }
            }
        }
        machine.transitions.sort();
        // A machine needs a literal state; mappings alone are no transitions, no gap.
        pending.extend(gaps.into_iter().filter(|_| literal));
        literal.then_some(machine)
    }

    fn use_cases(&mut self) {
        let mut owners: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for e in &self.ents {
            for (name, _, ctor, public) in e.methods() {
                if public && !ctor {
                    owners.entry(name).or_default().insert(&e.id);
                }
            }
        }
        let mut pending = Pending::new();
        let cases = self
            .decls
            .iter()
            .filter_map(|d| self.use_case_of(d, &owners, &mut pending))
            .collect();
        self.model.use_cases = cases;
        self.pending.extend(pending);
    }

    fn use_case_of(
        &self,
        d: &Decl,
        owners: &BTreeMap<&str, BTreeSet<&str>>,
        pending: &mut Pending,
    ) -> Option<UseCase> {
        let request = d
            .bases
            .iter()
            .find_map(|x| generic_args(x, REQUEST_HANDLER))
            .map(|a| a[0].clone());
        let named = USE_CASE_SUFFIXES.iter().any(|x| d.name.ends_with(x));
        if d.kw != "class" || (request.is_none() && !named) {
            return None;
        }
        let s = &self.files[d.file].text;
        let (calls, ambiguous) = d
            .body
            .map_or_else(Default::default, |body| calls_in(s, body, owners));
        for (pos, message) in ambiguous {
            pending.push((d.file, pos, format!("{}: {message}", d.name), true));
        }
        Some(UseCase {
            id: d.name.clone(),
            name: d.name.clone(),
            kind: use_case_kind(&d.name),
            request,
            calls,
            file: self.files[d.file].rel.clone(),
            line: line(s, d.pos),
            source: SOURCE.into(),
        })
    }

    /// One entry per class and handled message type, sorted.
    fn message_handlers(&mut self) {
        let mut found = vec![];
        for d in self.decls.iter().filter(|d| d.kw == "class") {
            let src = &self.files[d.file];
            let messages = (d.bases.iter())
                .filter_map(|b| MESSAGE_HANDLERS.iter().find_map(|h| generic_args(b, h)));
            for args in messages {
                found.push(MessageHandler {
                    id: d.name.clone(),
                    message: simple(&args[0]),
                    sends: d
                        .body
                        .map_or_else(Vec::new, |body| sends_in(&src.text, body)),
                    file: src.rel.clone(),
                    line: line(&src.text, d.pos),
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
    if !matches!(decl.kw.as_str(), "class" | "record" | "struct")
        || AGGREGATE_MARKERS.contains(&name)
        || [ENTITY_BASE, VALUE_OBJECT_BASE].contains(&name)
    {
        return None;
    }
    let bases: Vec<String> = decl.bases.iter().map(|x| simple(x)).collect();
    let has = |n: &str| bases.iter().any(|b| b == n);
    if AGGREGATE_MARKERS.iter().any(|m| has(m)) {
        Some(EntityKind::AggregateRoot)
    } else if has(ENTITY_BASE) {
        Some(EntityKind::Entity)
    } else if has(VALUE_OBJECT_BASE) {
        Some(EntityKind::ValueObject)
    } else {
        None
    }
}

/// Qualifies the ids of entities sharing a name; one diagnostic each.
fn qualify(ents: &mut [Ent]) -> Pending {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for e in ents.iter() {
        *count.entry(e.name.clone()).or_default() += 1;
    }
    let mut out = Pending::new();
    for e in ents.iter_mut().filter(|e| count[&e.name] > 1) {
        e.id = if e.ns.is_empty() {
            format!("{}/{}", e.dir, e.name)
        } else {
            format!("{}.{}", e.ns, e.name)
        };
        let message = format!(
            "Several entities are named {}; ids are namespace-qualified ({}).",
            e.name, e.id
        );
        out.push((e.file, e.pos, message, false));
    }
    out
}

/// The events, and where each handler class is declared (sorted).
fn domain_events(
    decls: &[Decl],
    files: &[Src],
    raised: BTreeMap<String, BTreeSet<String>>,
) -> (Vec<DomainEvent>, Vec<EventHandlerSite>) {
    let mut handled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut sites = vec![];
    for d in decls {
        for arg in d
            .bases
            .iter()
            .filter_map(|b| generic_args(b, EVENT_HANDLER))
        {
            let event = simple(&arg[0]);
            let src = &files[d.file];
            sites.push(EventHandlerSite {
                event: event.clone(),
                handler: d.name.clone(),
                file: src.rel.clone(),
                line: line(&src.text, d.pos),
            });
            handled.entry(event).or_default().insert(d.name.clone());
        }
    }
    sites.sort();
    let types: BTreeSet<&String> = raised.keys().chain(handled.keys()).collect();
    let events = types
        .into_iter()
        .map(|t| DomainEvent {
            type_name: t.clone(),
            raised_by: raised
                .get(t)
                .map_or(vec![], |x| x.iter().cloned().collect()),
            handled_by: handled
                .get(t)
                .map_or(vec![], |x| x.iter().cloned().collect()),
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

/// `Entity.M` for each `.M(` in `body` whose `M` one entity owns, plus
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

/// Type declarations.
mod decls {
    use super::*;

    const TYPE_KEYWORDS: [&str; 5] = ["class", "record", "struct", "interface", "enum"];

    /// Declarations (file and folder left empty) and `DbSet<T>`: (offset, `T`).
    pub(super) fn parse(s: &str) -> (Vec<Decl>, Vec<(usize, String)>) {
        let toks = idents(s, 0, s.len());
        let mut out = vec![];
        let mut sets = vec![];
        for t in 0..toks.len() {
            if let Some(ty) = dbset(s, toks[t]) {
                sets.push((toks[t].0, ty));
            }
            out.extend(decl_at(s, &toks, t));
        }
        (out, sets)
    }

    fn dbset(s: &str, (pos, word): (usize, &str)) -> Option<String> {
        let b = s.as_bytes();
        if word != "DbSet" {
            return None;
        }
        let lt = next_nonws(b, pos + word.len()).filter(|&k| b[k] == b'<')?;
        Some(simple(&s[lt + 1..matching(b, lt)?]))
    }

    fn decl_at(s: &str, toks: &[(usize, &str)], t: usize) -> Option<Decl> {
        let (pos, word) = toks[t];
        let after_record = t > 0 && toks[t - 1].1 == "record" && gap(s, toks[t - 1], pos);
        if !TYPE_KEYWORDS.contains(&word) || after_record {
            return None;
        }
        let record_kind = toks
            .get(t + 1)
            .is_some_and(|x| matches!(x.1, "class" | "struct"));
        let k = if word == "record" && record_kind {
            t + 2
        } else {
            t + 1
        };
        let &(npos, name) = toks.get(k).filter(|x| gap(s, toks[k - 1], x.0))?;
        let (params, bases, body) = header(s, npos + name.len());
        Some(Decl {
            file: 0,
            kw: word.into(),
            name: name.into(),
            ns: namespace(s, &toks[..t]),
            dir: String::new(),
            pos,
            bases,
            params,
            body,
        })
    }

    /// Generic parameters, record parameters, base list and body after a type name.
    fn header(s: &str, mut i: usize) -> (Option<Span>, Vec<String>, Option<Span>) {
        let b = s.as_bytes();
        if let Some(k) = next_nonws(b, i).filter(|&k| b[k] == b'<') {
            i = matching(b, k).map_or(i, |e| e + 1);
        }
        let mut params = None;
        if let Some(k) = next_nonws(b, i).filter(|&k| b[k] == b'(')
            && let Some(e) = matching(b, k)
        {
            params = Some((k + 1, e));
            i = e + 1;
        }
        let (bases, i) = base_list(s, i);
        let body = (i..s.len())
            .find(|&j| matches!(b[j], b'{' | b';'))
            .filter(|&o| b[o] == b'{')
            .and_then(|o| matching(b, o).map(|c| (o, c)));
        (params, bases, body)
    }

    fn base_list(s: &str, i: usize) -> (Vec<String>, usize) {
        let b = s.as_bytes();
        let Some(k) = next_nonws(b, i).filter(|&k| b[k] == b':') else {
            return (vec![], i);
        };
        let stop = (k + 1..s.len())
            .find(|&j| matches!(b[j], b'{' | b';') || is_where(b, j))
            .unwrap_or(s.len());
        let bases = split_top(&s[k + 1..stop], ",")
            .into_iter()
            .map(|x| x.trim().to_string())
            .collect();
        (bases, stop)
    }

    fn is_where(b: &[u8], j: usize) -> bool {
        b[j..].starts_with(b"where") && !is_id(b[j - 1]) && !b.get(j + 5).is_some_and(|&c| is_id(c))
    }

    /// The last `namespace` declared before the type.
    fn namespace(s: &str, before: &[(usize, &str)]) -> String {
        before
            .iter()
            .rev()
            .find(|x| x.1 == "namespace")
            .map(|&(p, w)| {
                let rest = &s[p + w.len()..];
                rest[..rest.find([';', '{']).unwrap_or(rest.len())]
                    .trim()
                    .to_string()
            })
            .unwrap_or_default()
    }
}

/// Members of a type body.
mod members {
    use super::*;

    const NOT_MEMBERS: [&str; 8] = [
        "class",
        "struct",
        "interface",
        "enum",
        "record",
        "delegate",
        "event",
        "operator",
    ];

    /// Fields/properties and methods/constructors of the body `{…}` at `open..close`.
    pub(super) fn members(s: &str, (open, close): Span, class: &str) -> Vec<Member> {
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
                    out.extend(member(s, (start, i), Some((i, end)), class));
                    start = after_block(s, end, close);
                    i = start;
                    continue;
                }
                b';' if depth == 0 => {
                    out.extend(member(s, (start, i), None, class));
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    /// Where the next member starts after the block ending at `end`; a property
    /// initializer `{ get; } = …;` belongs to the block's member.
    fn after_block(s: &str, end: usize, close: usize) -> usize {
        let b = s.as_bytes();
        match next_nonws(b, end + 1).filter(|&k| k < close && b[k] == b'=') {
            Some(k) => s[k..close].find(';').map_or(close, |e| k + e + 1),
            None => end + 1,
        }
    }

    fn member(s: &str, (a, z): Span, block: Option<Span>, class: &str) -> Option<Member> {
        let header = skip_attributes(&s[a..z]);
        let p = z - header.len();
        let arrow = find_top(header, "=>");
        let decl_end = arrow.unwrap_or(header.len());
        let decl = &header[..first_assign(&header[..decl_end]).unwrap_or(decl_end)];
        // `@event` is a verbatim identifier, not the keyword.
        let words: Vec<&str> = idents(decl, 0, decl.len())
            .into_iter()
            .filter(|x| x.0 == 0 || decl.as_bytes()[x.0 - 1] != b'@')
            .map(|x| x.1)
            .collect();
        if words.is_empty() || words.iter().any(|w| NOT_MEMBERS.contains(w)) {
            return None;
        }
        let kind = match find_top(decl, "(") {
            Some(paren) => {
                let body = match (block, arrow) {
                    (Some((o, c)), _) => (o + 1, c),
                    (None, Some(ar)) => (p + ar + 2, z),
                    _ => return None,
                };
                method(&decl[..paren], &words, class, body)?
            }
            None => data(decl).map(|(ty, name)| Kind::Data { ty, name })?,
        };
        let data = matches!(kind, Kind::Data { .. });
        let public = words.contains(&"public");
        let state = data
            && !words.contains(&"const")
            && (block.is_some() || arrow.is_some() || (public && !words.contains(&"readonly")));
        Some(Member {
            kind,
            public,
            head: (p, z),
            state,
        })
    }

    /// A method from the declaration text before its `(`.
    fn method(before_paren: &str, words: &[&str], class: &str, body: Span) -> Option<Kind> {
        let mut name_end = before_paren.trim_end();
        if name_end.ends_with('>')
            && let Some(lt) = matching_back(name_end.as_bytes(), name_end.len() - 1)
        {
            name_end = name_end[..lt].trim_end();
        }
        let name = idents(name_end, 0, name_end.len()).last()?.1.to_string();
        if name == class && words.contains(&"static") {
            return None;
        }
        Some(Kind::Method {
            ctor: name == class,
            name,
            body,
        })
    }

    /// `[modifiers] Type Name` → (type without whitespace, name).
    pub(super) fn data(decl: &str) -> Option<(String, String)> {
        let decl = decl.trim();
        let name_start = decl
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |k| k + 1);
        let name = &decl[name_start..];
        let mut ty = decl[..name_start].trim();
        while let Some((first, rest)) = ty.split_once(char::is_whitespace)
            && MODIFIERS.contains(&first)
        {
            ty = rest.trim_start();
        }
        let ty: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        let valid =
            !name.is_empty() && !ty.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit());
        valid.then(|| (ty, name.to_string()))
    }

    pub(super) fn field(ty: &str, name: &str) -> Field {
        let t = ty.trim_end_matches('?');
        let inner = t
            .find('<')
            .filter(|&lt| t.ends_with('>') && COLLECTIONS.contains(&simple(&t[..lt]).as_str()))
            .map(|lt| &t[lt + 1..t.len() - 1])
            .filter(|inner| !inner.contains(','))
            .or_else(|| t.strip_suffix("[]"));
        Field {
            name: name.into(),
            type_name: inner.unwrap_or(t).trim_end_matches('?').to_string(),
            collection: inner.is_some(),
        }
    }

    /// `IRequestHandler<A, B>` → `[A, B]` when `base`'s simple name is `name`.
    pub(super) fn generic_args(base: &str, name: &str) -> Option<Vec<String>> {
        let lt = base.find('<')?;
        if simple(&base[..lt]) != name || !base.ends_with('>') {
            return None;
        }
        Some(
            split_top(&base[lt + 1..base.len() - 1], ",")
                .into_iter()
                .map(|x| x.trim().to_string())
                .collect(),
        )
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
        s: &'a str,
        field: &'a str,
        enum_type: &'a str,
        states: &'a [String],
        all: BTreeSet<String>,
        helpers: &'a BTreeSet<&'a str>,
    }

    impl<'a> Rules<'a> {
        pub(super) fn new(
            s: &'a str,
            field: &'a str,
            enum_type: &'a str,
            states: &'a [String],
            helpers: &'a BTreeSet<&'a str>,
        ) -> Self {
            let all = states.iter().cloned().collect();
            Self {
                s,
                field,
                enum_type,
                states,
                all,
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

        fn literal(&self, value: &str) -> Option<String> {
            value
                .rsplit_once('.')
                .filter(|(t, v)| simple(t) == self.enum_type && self.all.contains(*v))
                .map(|(_, v)| v.to_string())
        }

        fn ordered(&self, set: &BTreeSet<String>) -> Vec<String> {
            self.states
                .iter()
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

        /// The guard from the nearest enclosing `if`, `else` or `switch` on the field.
        fn enclosing(&self, stmt: usize, stack: &[usize]) -> Option<Guard> {
            let on_field = |c: Span| mentions(&self.s[c.0..c.1], self.field);
            for h in std::iter::once(stmt).chain(stack.iter().rev().copied()) {
                match head(self.s, h) {
                    Head::If(c) if on_field(c) => return Some(self.read(c, false)),
                    Head::Else(Some(c)) if on_field(c) => return Some(self.read(c, true)),
                    Head::Switch(c) if on_field(c) => {
                        return Some(Guard::Unread(Some(collapse(&self.s[c.0..c.1])), "switch"));
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

        /// `Field == E.A` → (true, A); `Field != E.A` → (false, A).
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
                "switch" => Head::Switch(c),
                _ => Head::Other,
            };
        }
        if prev_word(s, at) == "else" {
            return Head::Else(else_cond(s, k.saturating_sub(3)));
        }
        Head::Other
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
            first + s[first..].find(';')?
        };
        Some(((open + 1, close), (first, end)))
    }

    /// True when `range` throws, returns or calls a method that only throws.
    fn exits(s: &str, range: Span, helpers: &BTreeSet<&str>) -> bool {
        idents(s, range.0, range.1)
            .iter()
            .any(|(_, w)| matches!(*w, "throw" | "return") || helpers.contains(w))
    }

    /// Assignments `field = …` / `this.field = …` at statement start in `body`:
    /// (statement start, value).
    pub(super) fn assignments(s: &str, body: Span, field: &str) -> Vec<(usize, Span)> {
        idents(s, body.0, body.1)
            .into_iter()
            .filter(|x| x.1 == field)
            .filter_map(|(pos, _)| assignment_at(s, body, pos, field))
            .collect()
    }

    fn assignment_at(s: &str, body: Span, pos: usize, field: &str) -> Option<(usize, Span)> {
        let b = s.as_bytes();
        let stmt = statement_start(s, body, pos)?;
        let eq = next_nonws(b, pos + field.len())
            .filter(|&e| b[e] == b'=' && !matches!(b.get(e + 1), Some(b'=' | b'>')))?;
        Some((stmt, (eq + 1, value_end(b, eq + 1, body.1))))
    }

    /// Start of the statement when the identifier at `pos` (or `this.` before
    /// it) begins one.
    fn statement_start(s: &str, body: Span, pos: usize) -> Option<usize> {
        let b = s.as_bytes();
        let mut stmt = pos;
        if let Some(dot) = prev_nonws(b, pos).filter(|&k| b[k] == b'.') {
            if prev_word(s, dot) != "this" {
                return None;
            }
            stmt = prev_nonws(b, dot)? - 3;
        }
        let starts = match prev_nonws(b, stmt) {
            None => true,
            Some(k) => {
                k < body.0
                    || matches!(b[k], b';' | b'{' | b'}' | b')')
                    || (b[k] == b'>' && k > 0 && b[k - 1] == b'=')
                    || prev_word(s, stmt) == "else"
            }
        };
        starts.then_some(stmt)
    }

    /// End of an assigned value: `;`, `,` or a closing bracket at depth 0.
    fn value_end(b: &[u8], from: usize, to: usize) -> usize {
        let mut depth = 0i32;
        for (k, &c) in b.iter().enumerate().take(to).skip(from) {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => return k,
                b')' | b']' | b'}' => depth -= 1,
                b';' | b',' if depth == 0 => return k,
                _ => {}
            }
        }
        to
    }

    /// Raise calls in `body`: (offset, argument text).
    pub(super) fn raise_calls(s: &str, body: Span) -> Vec<(usize, &str)> {
        let b = s.as_bytes();
        idents(s, body.0, body.1)
            .into_iter()
            .filter(|x| x.1 == RAISE_FUNCTION)
            .filter_map(|(pos, w)| {
                let open = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'(')?;
                let close = matching(b, open).unwrap_or(body.1);
                Some((pos, s[open + 1..close].trim()))
            })
            .collect()
    }

    /// The type created for a raise argument: `new X(…)`, or a variable
    /// assigned `new X(…)` in `before` (the method text up to the call).
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

    fn new_type(x: &str) -> Option<String> {
        let rest = x.trim_start().strip_prefix("new")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = rest.trim_start();
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        (end > 0).then(|| simple(&rest[..end]))
    }
}

/// Navigation in blanked source text, plus what only C# needs.
pub(crate) mod text {
    pub(crate) use crate::lang_text::*;

    pub(super) fn skip_attributes(s: &str) -> &str {
        let mut s = s.trim_start();
        while s.starts_with('[') {
            let Some(e) = matching(s.as_bytes(), 0) else {
                break;
            };
            s = s[e + 1..].trim_start();
        }
        s
    }

    pub(super) fn common_prefix(a: &str, b: &str) -> usize {
        a.split('.')
            .zip(b.split('.'))
            .take_while(|(x, y)| x == y)
            .count()
    }
}

pub(crate) mod code {
    //! Source-text patterns shared by the C# and Java rule readers (ADR 0022,
    //! Verlauf 2026-11-20): the condition guarding a throw, a message argument,
    //! `new X(…)` and chained calls. Works on text with comments and literal
    //! contents blanked (`text`) and the source as read (`raw`), same offsets;
    //! and the service (project folder) of a file.
    use super::Span;
    use super::text::*;
    use std::path::Path;

    /// The folder name of the nearest build file above `rel` (a file name in that
    /// folder ending in one of `build`, e.g. `.csproj` or `pom.xml`): the service
    /// of a publish site. Without one, the first path segment of `rel`.
    pub(crate) fn project_of(
        root: &Path,
        rel: &str,
        build: &[&str],
        cache: &mut std::collections::BTreeMap<String, String>,
    ) -> String {
        let start = rel.rsplit_once('/').map_or("", |(d, _)| d);
        if let Some(known) = cache.get(start) {
            return known.clone();
        }
        let has_build = |dir: &str| {
            std::fs::read_dir(root.join(dir)).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    build.iter().any(|b| name.ends_with(b))
                })
            })
        };
        let mut dir = start;
        let found = loop {
            if !dir.is_empty() && has_build(dir) {
                break dir.rsplit('/').next().unwrap_or(dir).to_string();
            }
            match dir.rsplit_once('/') {
                Some((up, _)) => dir = up,
                None => {
                    break rel
                        .split_once('/')
                        .map_or("", |(first, _)| first)
                        .to_string();
                }
            }
        };
        cache.insert(start.to_string(), found.clone());
        found
    }

    #[derive(Clone, Copy)]
    pub(crate) struct Code<'a> {
        pub(crate) text: &'a str,
        pub(crate) raw: &'a str,
    }

    /// `new X(…)` starting at or after `from`: (simple `X`, argument span).
    pub(crate) fn created_at(s: &str, from: usize) -> Option<(String, Span)> {
        let b = s.as_bytes();
        let k = next_nonws(b, from)?;
        if !s[k..].starts_with("new") || b.get(k + 3).is_some_and(|&c| is_id(c)) {
            return None;
        }
        let t = next_nonws(b, k + 3)?;
        let end = (t..b.len())
            .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
            .unwrap_or(b.len());
        let open = next_nonws(b, end).filter(|&o| b[o] == b'(' && end > t)?;
        Some((simple(&s[t..end]), (open + 1, matching(b, open)?)))
    }

    pub(crate) fn raw(c: Code, (a, z): Span) -> String {
        collapse(c.raw.get(a..z).unwrap_or(&c.text[a..z]))
    }

    /// The call from `start` through the parenthesis opening at `open`, as written.
    pub(crate) fn call_text(c: Code, start: usize, open: usize) -> Option<String> {
        let close = matching(c.text.as_bytes(), open)?;
        Some(raw(c, (start, close + 1)))
    }

    /// Arguments split at `,` outside (), [] and {}; unlike `split_top`, a `<`
    /// or `>` is a comparison here, not a generic bracket.
    pub(crate) fn split_args(s: &str) -> Vec<&str> {
        let mut out = vec![];
        let (mut depth, mut start) = (0i32, 0);
        for (k, c) in s.bytes().enumerate() {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    out.push(&s[start..k]);
                    start = k + 1;
                }
                _ => {}
            }
        }
        out.push(&s[start..]);
        out
    }

    /// The first argument: a string literal's content (interpolations kept), else
    /// its source text.
    pub(crate) fn message(c: Code, (a, z): Span) -> Option<String> {
        let first = split_args(&c.text[a..z]).into_iter().next()?;
        let at = a + (first.as_ptr() as usize - c.text[a..].as_ptr() as usize);
        let text = raw(c, (at, at + first.len()));
        // One literal: in the blanked text only its own two quotes are left.
        let blanked = first.trim().trim_start_matches(['$', '@']);
        let literal = blanked.len() >= 2
            && blanked.starts_with('"')
            && blanked.ends_with('"')
            && !blanked[1..blanked.len() - 1].contains('"');
        match (text.is_empty(), literal) {
            (true, _) => None,
            (_, true) => Some(
                text.trim_start_matches(['$', '@'])
                    .trim_matches('"')
                    .to_string(),
            ),
            _ => Some(text),
        }
    }

    /// The condition under which the throw (or `new`) at `pos` runs.
    pub(crate) fn condition(c: Code, body: Span, pos: usize) -> Option<String> {
        let (s, b) = (c.text, c.text.as_bytes());
        let k = prev_nonws(b, pos)?;
        if b[k] == b':' {
            let q = scan_back(b, body.0, k, |j| {
                b[j] == b'?' && b[j - 1] != b'?' && !matches!(b[j + 1], b'?' | b'.' | b'[')
            })?;
            let start = expression_start(s, body.0, q);
            return Some(format!("!({})", raw(c, (start, q))));
        }
        if b[k] == b'?' && k > 0 && b[k - 1] == b'?' {
            let start = expression_start(s, body.0, k - 1);
            return Some(format!("{} == null", raw(c, (start, k - 1))));
        }
        let stmt = statement_start(b, body.0, pos);
        if s[stmt..].starts_with("if") && !b.get(stmt + 2).is_some_and(|&c| is_id(c)) {
            let open = next_nonws(b, stmt + 2).filter(|&o| b[o] == b'(')?;
            return Some(raw(c, (open + 1, matching(b, open)?)));
        }
        let brace = prev_nonws(b, stmt).filter(|&o| b[o] == b'{')?;
        if_condition(c, brace)
    }

    /// The condition of the `if` (or, negated, of the `if` before the `else`)
    /// heading the block that opens at `brace`.
    fn if_condition(c: Code, brace: usize) -> Option<String> {
        let (s, b) = (c.text, c.text.as_bytes());
        let k = prev_nonws(b, brace)?;
        if b[k] == b')' {
            let open = matching_back(b, k)?;
            return (prev_word(s, open) == "if").then(|| raw(c, (open + 1, k)));
        }
        if prev_word(s, brace) == "else" {
            let close = prev_nonws(b, k.saturating_sub(3)).filter(|&c| b[c] == b'}')?;
            let inner = if_condition(c, matching_back(b, close)?)?;
            return (!inner.starts_with("!(")).then(|| format!("!({inner})"));
        }
        None
    }

    /// The nearest `j < to` (not before `from`) at bracket depth 0 where `hit` holds.
    fn scan_back(b: &[u8], from: usize, to: usize, hit: impl Fn(usize) -> bool) -> Option<usize> {
        let mut depth = 0i32;
        for j in (from + 1..to).rev() {
            match b[j] {
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' if depth == 0 => return None,
                b'(' | b'[' | b'{' => depth -= 1,
                b';' if depth == 0 => return None,
                _ if depth == 0 && hit(j) => return Some(j),
                _ => {}
            }
        }
        None
    }

    /// Start of the expression ending at `end`: after a plain `=`, `return`, an
    /// opening bracket, `,` or a statement boundary at depth 0.
    fn expression_start(s: &str, from: usize, end: usize) -> usize {
        let b = s.as_bytes();
        let assign = |j: usize| {
            b[j] == b'='
                && !matches!(b[j + 1], b'=' | b'>')
                && !matches!(b[j - 1], b'=' | b'!' | b'<' | b'>')
        };
        let stop = scan_back(b, from, end, |j| assign(j) || b[j] == b',');
        let mut start = stop.map_or_else(
            || {
                let mut depth = 0i32;
                (from..end)
                    .rev()
                    .find(|&j| match b[j] {
                        b')' | b']' | b'}' => {
                            depth += 1;
                            false
                        }
                        b'(' | b'[' | b'{' if depth == 0 => true,
                        b'(' | b'[' | b'{' => {
                            depth -= 1;
                            false
                        }
                        b';' => depth == 0,
                        _ => false,
                    })
                    .map_or(from, |j| j + 1)
            },
            |j| j + 1,
        );
        start = next_nonws(b, start).unwrap_or(start);
        if s[start..].starts_with("return") && !b.get(start + 6).is_some_and(|&c| is_id(c)) {
            start = next_nonws(b, start + 6).unwrap_or(start);
        }
        start
    }

    /// Start of the statement containing `pos`: after the nearest `;`, `{` or `}`
    /// at depth 0, crossing the brackets `pos` is inside.
    fn statement_start(b: &[u8], from: usize, pos: usize) -> usize {
        let mut depth = 0i32;
        for j in (from..pos).rev() {
            match b[j] {
                b')' | b']' => depth += 1,
                b'(' | b'[' if depth > 0 => depth -= 1,
                b';' | b'{' | b'}' if depth == 0 => return next_nonws(b, j + 1).unwrap_or(pos),
                _ => {}
            }
        }
        next_nonws(b, from).unwrap_or(pos)
    }

    /// The types of the `new X(…)` at bracket depth 0 of `span`.
    pub(crate) fn top_news(s: &str, span: Span) -> Vec<String> {
        let b = s.as_bytes();
        let depth = |p: usize| {
            (b[span.0..p].iter()).fold(0i32, |d, c| match c {
                b'(' | b'[' | b'{' => d + 1,
                b')' | b']' | b'}' => d - 1,
                _ => d,
            })
        };
        (idents(s, span.0, span.1).into_iter())
            .filter(|&(p, w)| w == "new" && depth(p) == 0)
            .filter_map(|(p, _)| created_at(s, p).map(|c| c.0))
            .collect()
    }

    /// The `new` types of every assignment `var = …;` in `before`.
    pub(crate) fn assigned_news(s: &str, before: Span, var: &str) -> Vec<String> {
        let b = s.as_bytes();
        let mut out = vec![];
        for (p, w) in idents(s, before.0, before.1)
            .into_iter()
            .filter(|x| x.1 == var)
        {
            let Some(eq) = next_nonws(b, p + w.len())
                .filter(|&e| b[e] == b'=' && !matches!(b.get(e + 1), Some(b'=' | b'>')))
            else {
                continue;
            };
            let end = (eq + 1..before.1)
                .scan(0i32, |d, j| {
                    *d += match b[j] {
                        b'(' | b'[' | b'{' => 1,
                        b')' | b']' | b'}' => -1,
                        _ => 0,
                    };
                    Some((j, *d))
                })
                .find(|&(j, d)| (d == 0 && b[j] == b';') || d < 0)
                .map_or(before.1, |(j, _)| j);
            out.extend(top_news(s, (eq + 1, end)));
        }
        out
    }
}

/// API DTOs among the plain models (ADR 0022, Verlauf 2026-11-20).
///
/// A controller is a class with a base ending in `Controller`/`ControllerBase`.
/// A plain model whose type is the return type or a body-bound parameter
/// (`[FromBody]`, or any parameter of an `[ApiController]`) of a public action
/// of a controller is the wire contract of that action, not
/// a domain model; so is a plain model only reachable through the properties
/// of such a DTO (`Items` of a list response). Persistence outranks it: a type
/// in `DbSet<T>`, `IEntityTypeConfiguration<T>` or `.Entity<T>(…)`, a marker
/// entity, and every type a kept model has as a property or base class stay
/// models (the domain uses what the API also exposes). Names are never read for this.
mod api {
    use super::*;

    pub(super) fn dtos(
        x: &Extractor,
        cands: &[usize],
        kinds: &[(usize, EntityKind)],
    ) -> BTreeSet<usize> {
        let seeds = action_types(x);
        if seeds.is_empty() {
            return BTreeSet::new();
        }
        let mapped = mapped_types(x);
        let free: Vec<usize> = (cands.iter().copied())
            .filter(|&d| !mapped.contains(&x.decls[d].name))
            .collect();
        // The free classes a class names as property type or base class.
        let next = |d: usize| -> Vec<usize> {
            let bases = x.decls[d]
                .bases
                .iter()
                .map(|b| b.split('<').next().unwrap_or(b));
            let fields = x.ent(d, EntityKind::Entity).fields;
            let names: Vec<String> = (fields.iter().map(|f| f.type_name.as_str()))
                .chain(bases)
                .map(simple)
                .collect();
            (free.iter().copied())
                .filter(|&n| names.contains(&x.decls[n].name))
                .collect()
        };
        let roots = (free.iter().copied()).filter(|&d| seeds.contains(&x.decls[d].name));
        let dto = reach(roots.collect(), &next);
        let mut kept: Vec<usize> = kinds.iter().map(|k| k.0).collect();
        kept.extend(cands.iter().copied().filter(|d| !dto.contains(d)));
        let rescued = reach(kept, &next);
        dto.difference(&rescued).copied().collect()
    }

    /// `from` and every declaration reachable from it through `next`.
    fn reach(from: Vec<usize>, next: &dyn Fn(usize) -> Vec<usize>) -> BTreeSet<usize> {
        let mut seen: BTreeSet<usize> = from.iter().copied().collect();
        let mut todo = from;
        while let Some(d) = todo.pop() {
            for n in next(d) {
                if seen.insert(n) {
                    todo.push(n);
                }
            }
        }
        seen
    }

    /// Simple names in the return type of public actions of controllers and in
    /// their body-bound parameters (`[FromBody]`, or any parameter of an
    /// `[ApiController]`); an MVC form model bound without either is not read.
    fn action_types(x: &Extractor) -> BTreeSet<String> {
        let controllers = x.decls.iter().filter(|d| {
            d.kw == "class"
                && (d.bases.iter()).any(|b| {
                    let b = simple(b);
                    b.ends_with("Controller") || b.ends_with("ControllerBase")
                })
        });
        let mut out = BTreeSet::new();
        for decl in controllers {
            let (Some(body), s) = (decl.body, &x.files[decl.file].text) else {
                continue;
            };
            let before = &s[..decl.pos];
            let attrs = &before[before.rfind(['{', '}', ';']).map_or(0, |k| k + 1)..];
            let api_controller = attrs.contains("ApiController");
            for m in members(s, body, &decl.name).iter().filter(|m| m.public) {
                if let Kind::Method {
                    name, ctor: false, ..
                } = &m.kind
                {
                    out.extend(head_types(&s[m.head.0..m.head.1], name, api_controller));
                }
            }
        }
        out
    }

    /// Identifiers of an action head `Ret Name(params)`: the return type and the
    /// body-bound parameters, without the action's own name.
    fn head_types(head: &str, name: &str, all_bound: bool) -> Vec<String> {
        let Some(open) = head.find('(') else {
            return vec![];
        };
        let close = head.rfind(')').unwrap_or(head.len()).max(open + 1);
        let params = split_top(&head[open + 1..close], ",");
        let bound = (params.iter().copied()).filter(|p| all_bound || p.contains("FromBody"));
        (std::iter::once(&head[..open]).chain(bound))
            .flat_map(|text| idents(text, 0, text.len()))
            .map(|w| w.1)
            .filter(|w| *w != name)
            .map(String::from)
            .collect()
    }

    /// Types the persistence mapping names.
    fn mapped_types(x: &Extractor) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = x.sets.iter().map(|s| s.2.clone()).collect();
        for decl in &x.decls {
            for b in &decl.bases {
                out.extend(
                    generic_args(b, "IEntityTypeConfiguration")
                        .into_iter()
                        .flatten(),
                );
            }
        }
        for f in &x.files {
            for (at, _) in f.text.match_indices(".Entity<") {
                let rest = &f.text[at + ".Entity<".len()..];
                out.extend(rest.find('>').map(|e| simple(rest[..e].trim())));
            }
        }
        out
    }
}

mod rich {
    //! Rules, validators, integration events and plain models of the C#
    //! extractor (ADR 0022, Verlauf 2026-11-20); text-level like the parent.
    //!
    //! - Plain models: a `class` (→ `entity`) or `record`/`struct` (→
    //!   `value_object`) without a marker in a folder with a path segment ending
    //!   in one of [`MODEL_FOLDERS`], when it has a field or property, is neither
    //!   `abstract` nor `static`, its name does not end in one of
    //!   [`NOT_MODELS`], it carries state (a property, or a public mutable
    //!   field; a private/`readonly` field is an injected dependency, a `const`
    //!   a constant, so repositories, services and visitors are no models) and
    //!   it is no API DTO (see [`api`]).
    //! - Rules: in methods of found entities, `throw new X(…)` where `X` ends in
    //!   `DomainException`, is an `ArgumentException` family type or an
    //!   `*Exception` class declared in a model folder; `new ValidationResult(…)`
    //!   (`IValidatableObject`); a call of a method of the entity whose body only
    //!   throws (the rule sits at the call, the helper itself is skipped);
    //!   `XException.ThrowIf…(…)` and `Guard.Against.…(…)`. The condition is the
    //!   enclosing `if` (single statement or block; an `else` block gives the
    //!   literal `!(…)`), `!(c)` for `c ? v : throw`, `v == null` for `v ??
    //!   throw`, the helper call for `ThrowIf…`/`Guard`; else none. Throws outside
    //!   entity classes (pipeline behaviours, repositories) are no domain rules.
    //! - Validations: per `RuleFor(x => x.P)` / `RuleForEach` in a class deriving
    //!   from `AbstractValidator<T>`, every chained call as source text.
    //! - Integration events: classes and records deriving from `IntegrationEvent`,
    //!   and publish sites `r.PublishAsync(…)`, `r.Publish(…)`/`Publish<X>(…)`,
    //!   `r.AddAndSaveEventAsync(…)` and `r.SaveEventAnd…Async(…)` with `new X(…)`
    //!   or a local variable assigned `new X(…)` (every top-level `new` of each
    //!   assignment, so a ternary or if/else yields both types). A receiver whose
    //!   name contains `mediator` publishes in-process notifications and is
    //!   skipped; a parameter or member argument (the generic forwarding
    //!   wrappers) adds nothing. `Send` is not read.
    use super::code::{
        Code, assigned_news, call_text, condition, created_at, message, raw, top_news,
    };
    use super::*;
    use archguard_core::domain_model::{DomainRule, IntegrationEvent, Publisher, Validation};

    pub(super) const MODEL_FOLDERS: [&str; 4] = ["Domain", "Model", "Models", "Entities"];
    pub(super) const NOT_MODELS: [&str; 9] = [
        "Dto",
        "DTO",
        "Request",
        "Response",
        "Command",
        "Query",
        "ViewModel",
        "Event",
        "Exception",
    ];
    const ARGUMENT_EXCEPTIONS: [&str; 3] = [
        "ArgumentException",
        "ArgumentNullException",
        "ArgumentOutOfRangeException",
    ];
    const VALIDATION_RESULT: &str = "ValidationResult";
    const GUARD: &str = "Guard";
    const VALIDATOR_BASE: &str = "AbstractValidator";
    const INTEGRATION_EVENT_BASE: &str = "IntegrationEvent";
    const PUBLISH_FUNCTIONS: [&str; 3] = ["PublishAsync", "Publish", "AddAndSaveEventAsync"];

    fn code(src: &Src) -> Code<'_> {
        Code {
            text: &src.text,
            raw: &src.raw,
        }
    }

    fn in_model_folder(dir: &str) -> bool {
        (dir.split('/')).any(|seg| MODEL_FOLDERS.iter().any(|f| seg.ends_with(f)))
    }

    /// Plain models among the declarations not in `kinds`. A name an entity in
    /// `kinds` has, or a `DbSet<T>` resolving to another class, keeps the plain
    /// class out (client copies of a model must not qualify the domain's ids).
    pub(super) fn plain_models(
        x: &Extractor,
        kinds: &[(usize, EntityKind)],
    ) -> Vec<(usize, EntityKind)> {
        let marker = |n: &str| {
            AGGREGATE_MARKERS.contains(&n) || [ENTITY_BASE, VALUE_OBJECT_BASE].contains(&n)
        };
        let taken: BTreeSet<&str> = kinds.iter().map(|k| x.decls[k.0].name.as_str()).collect();
        let set_owner = |d: usize, name: &str| {
            let sets: Vec<_> = x.sets.iter().filter(|s| s.2 == name).collect();
            sets.is_empty()
                || sets
                    .iter()
                    .any(|s| x.resolve_set(s.0, name) == Some(Some(d)))
        };
        let cands: Vec<usize> = (x.decls.iter().enumerate())
            .filter(|(d, decl)| {
                matches!(decl.kw.as_str(), "class" | "record" | "struct")
                    && !kinds.iter().any(|k| k.0 == *d)
                    && !taken.contains(decl.name.as_str())
                    && set_owner(*d, &decl.name)
                    && in_model_folder(&decl.dir)
                    && !NOT_MODELS.iter().any(|n| decl.name.ends_with(n))
                    && !marker(&decl.name)
                    && !modified(&x.files[decl.file].text, decl.pos, &["abstract", "static"])
                    && has_fields(&x.files[decl.file].text, decl)
            })
            .map(|(d, _)| d)
            .collect();
        let dtos = api::dtos(x, &cands, kinds);
        (cands.into_iter().filter(|d| !dtos.contains(d)))
            .map(|d| match x.decls[d].kw.as_str() {
                "class" => (d, EntityKind::Entity),
                _ => (d, EntityKind::ValueObject),
            })
            .collect()
    }

    /// One of `words` among the modifiers before the type keyword at `pos`.
    fn modified(s: &str, pos: usize, words: &[&str]) -> bool {
        let start = s[..pos].rfind(['\n', ';', '{', '}']).map_or(0, |k| k + 1);
        idents(s, start, pos).iter().any(|w| words.contains(&w.1))
    }

    fn has_fields(s: &str, decl: &Decl) -> bool {
        let params = (decl.params.filter(|_| decl.kw == "record"))
            .is_some_and(|(a, z)| !s[a..z].trim().is_empty());
        let data = decl
            .body
            .is_some_and(|body| (members(s, body, &decl.name).iter()).any(|m| m.state));
        params || data
    }

    /// One rule site: offset, exception, condition, message.
    type Site = (usize, String, Option<String>, Option<String>);

    pub(super) fn rules(x: &Extractor) -> Vec<DomainRule> {
        let declared: BTreeSet<&str> = (x.decls.iter())
            .filter(|d| d.kw == "class" && d.name.ends_with("Exception") && in_model_folder(&d.dir))
            .map(|d| d.name.as_str())
            .collect();
        let family = |t: &str| {
            t.ends_with("DomainException")
                || ARGUMENT_EXCEPTIONS.contains(&t)
                || declared.contains(t)
                || [VALIDATION_RESULT, GUARD].contains(&t)
        };
        let mut out = vec![];
        for ent in &x.ents {
            let src = &x.files[ent.file];
            let helpers = helper_throws(src, ent);
            for (name, body, ..) in ent.methods() {
                if helpers.contains_key(name) {
                    continue;
                }
                for (pos, exception, condition, message) in sites(src, body, &helpers) {
                    if family(&exception) {
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
        }
        out
    }

    /// Exception and message of each entity method whose body only throws.
    fn helper_throws<'a>(src: &Src, ent: &'a Ent) -> BTreeMap<&'a str, (String, Option<String>)> {
        let names = ent.throwing(&src.text);
        (ent.methods())
            .filter(|m| names.contains(m.0))
            .filter_map(|(name, body, ..)| {
                let at = body.0 + src.text[body.0..body.1].find("throw")?;
                let (ex, args) = created_at(&src.text, at + "throw".len())?;
                Some((name, (ex, message(code(src), args))))
            })
            .collect()
    }

    fn sites(
        src: &Src,
        body: Span,
        helpers: &BTreeMap<&str, (String, Option<String>)>,
    ) -> Vec<Site> {
        let (s, b) = (&src.text, src.text.as_bytes());
        let mut out = vec![];
        for (pos, w) in idents(s, body.0, body.1) {
            let dotted = prev_nonws(b, pos).is_some_and(|k| b[k] == b'.');
            let called = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'(');
            match w {
                "throw" => {
                    if let Some((ex, args)) = created_at(s, pos + w.len()) {
                        out.push((
                            pos,
                            ex,
                            condition(code(src), body, pos),
                            message(code(src), args),
                        ));
                    }
                }
                "new" => {
                    if let Some((ex, args)) =
                        created_at(s, pos).filter(|c| c.0 == VALIDATION_RESULT)
                    {
                        out.push((
                            pos,
                            ex,
                            condition(code(src), body, pos),
                            message(code(src), args),
                        ));
                    }
                }
                _ if !dotted && called.is_some() && helpers.contains_key(w) => {
                    let (ex, msg) = helpers[w].clone();
                    out.push((pos, ex, condition(code(src), body, pos), msg));
                }
                _ if dotted => out.extend(guard_helper(src, pos, w)),
                _ => {}
            }
        }
        out
    }

    /// `XException.ThrowIf…(…)` or `Guard.Against.…(…)` at the dotted `w`.
    fn guard_helper(src: &Src, pos: usize, w: &str) -> Option<Site> {
        let (s, b) = (&src.text, src.text.as_bytes());
        let dot = prev_nonws(b, pos)?;
        let receiver = prev_word(s, dot);
        let start = dot - receiver.len();
        let open = if w.starts_with("ThrowIf") && receiver.ends_with("Exception") {
            next_nonws(b, pos + w.len())
        } else if w == "Against" && receiver == GUARD {
            let k = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'.')?;
            let name_end = (k + 1..b.len()).find(|&j| !is_id(b[j]))?;
            next_nonws(b, name_end)
        } else {
            None
        };
        let open = open.filter(|&o| b[o] == b'(')?;
        Some((
            pos,
            receiver.into(),
            call_text(code(src), start, open),
            None,
        ))
    }

    pub(super) fn validations(x: &Extractor) -> Vec<Validation> {
        let mut out = vec![];
        for d in x.decls.iter().filter(|d| d.kw == "class") {
            let Some(target) = (d.bases.iter()).find_map(|b| generic_args(b, VALIDATOR_BASE))
            else {
                continue;
            };
            let (src, Some(body)) = (&x.files[d.file], d.body) else {
                continue;
            };
            let (s, b) = (&src.text, src.text.as_bytes());
            for (pos, w) in idents(s, body.0, body.1) {
                let open = next_nonws(b, pos + w.len()).filter(|&k| b[k] == b'(');
                let (true, Some(open)) = (matches!(w, "RuleFor" | "RuleForEach"), open) else {
                    continue;
                };
                let Some(close) = matching(b, open) else {
                    continue;
                };
                let lambda = &s[open + 1..close];
                let Some((_, path)) = lambda.split_once("=>") else {
                    continue;
                };
                let path = path.trim();
                let property = path.split_once('.').map_or(path, |(_, p)| p).trim();
                out.push(Validation {
                    target_type: simple(&target[0]),
                    property: property.to_string(),
                    rules: chained(src, close + 1),
                    file: src.rel.clone(),
                    line: line(s, pos),
                    source: SOURCE.into(),
                });
            }
        }
        out
    }

    /// The calls chained with `.` after `from`, each as written.
    fn chained(src: &Src, mut from: usize) -> Vec<String> {
        let b = src.text.as_bytes();
        let mut out = vec![];
        while let Some(dot) = next_nonws(b, from).filter(|&k| b[k] == b'.') {
            let Some(start) = next_nonws(b, dot + 1) else {
                break;
            };
            let end = (start..b.len()).find(|&j| !is_id(b[j])).unwrap_or(b.len());
            let mut open = next_nonws(b, end);
            if let Some(lt) = open.filter(|&k| b[k] == b'<') {
                open = matching(b, lt).and_then(|g| next_nonws(b, g + 1));
            }
            let Some(close) = open.filter(|&o| b[o] == b'(').and_then(|o| matching(b, o)) else {
                break;
            };
            out.push(raw(code(src), (start, close + 1)));
            from = close + 1;
        }
        out
    }

    pub(super) fn integration_events(x: &Extractor) -> Vec<IntegrationEvent> {
        let mut found: BTreeMap<String, BTreeSet<Publisher>> = BTreeMap::new();
        for d in &x.decls {
            if matches!(d.kw.as_str(), "class" | "record")
                && d.bases.iter().any(|b| simple(b) == INTEGRATION_EVENT_BASE)
            {
                found.entry(d.name.clone()).or_default();
            }
            let (src, Some(body)) = (&x.files[d.file], d.body.filter(|_| d.kw == "class")) else {
                continue;
            };
            for m in members(&src.text, body, &d.name) {
                let Kind::Method { name, body, .. } = m.kind else {
                    continue;
                };
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

    /// Published types in `body`: (type, offset of the call).
    fn publishes(s: &str, body: Span) -> Vec<(String, usize)> {
        let b = s.as_bytes();
        let mut out = vec![];
        for (pos, w) in idents(s, body.0, body.1) {
            let named = PUBLISH_FUNCTIONS.contains(&w)
                || (w.starts_with("SaveEventAnd") && w.ends_with("Async"));
            let Some(dot) = prev_nonws(b, pos).filter(|&k| named && b[k] == b'.') else {
                continue;
            };
            if prev_word(s, dot).to_lowercase().contains("mediator") {
                continue;
            }
            out.extend(
                payload(s, body, pos + w.len())
                    .into_iter()
                    .map(|t| (t, pos)),
            );
        }
        out
    }

    /// The published types of the call whose name ends at `after`: a generic
    /// argument, or the first argument's `new` types (directly or through a
    /// local variable assigned before the call).
    fn payload(s: &str, body: Span, after: usize) -> Vec<String> {
        let b = s.as_bytes();
        let Some(k) = next_nonws(b, after) else {
            return vec![];
        };
        if b[k] == b'<' {
            return matching(b, k).map_or(vec![], |close| vec![simple(&s[k + 1..close])]);
        }
        let Some(close) = (b[k] == b'(').then(|| matching(b, k)).flatten() else {
            return vec![];
        };
        let arg = split_top(&s[k + 1..close], ",")[0];
        let at = k + 1 + (arg.len() - arg.trim_start().len());
        let arg = arg.trim();
        if !arg.is_empty() && arg.bytes().all(is_id) {
            assigned_news(s, (body.0, after), arg)
        } else {
            top_news(s, (at, at + arg.len()))
        }
    }

    #[cfg(test)]
    mod tests {
        //! Fixtures shaped like eShop (Ordering, Catalog, Basket, PaymentProcessor)
        //! and CleanArchitecture.
        use super::super::extract;
        use archguard_core::domain_model::{DomainModel, EntityKind};

        const SEED: &str = "namespace Shop.Domain.SeedWork;
public abstract class Entity { public void AddDomainEvent(object e) { } }
public interface IAggregateRoot { }
";

        const ORDER_ITEM: &str = r#"namespace Shop.Domain.AggregatesModel.OrderAggregate;
public class OrderItem : Entity
{
    public decimal Discount { get; private set; }
    public int Units { get; private set; }
    public OrderItem(decimal unitPrice, decimal discount, int units = 1)
    {
        if (units <= 0)
        {
            throw new OrderingDomainException("Invalid number of units");
        }
        if ((unitPrice * units) < discount)
            throw new OrderingDomainException("The total, of order item is lower than applied discount");
        Units = units;
    }
    public void SetNewDiscount(decimal discount)
    {
        if (discount < 0) { throw new OrderingDomainException("Discount is not valid"); }
        Discount = discount;
    }
    public void Log() { throw new InvalidOperationException("not a domain rule"); }
}
"#;

        const PAYMENT: &str = r#"namespace Shop.Domain.AggregatesModel.BuyerAggregate;
public class PaymentMethod : Entity
{
    private string _cardNumber;
    private string _alias;
    public PaymentMethod(string cardNumber, string alias, DateTime expiration)
    {
        _cardNumber = !string.IsNullOrWhiteSpace(cardNumber) ? cardNumber : throw new OrderingDomainException(nameof(cardNumber));
        _alias = alias ?? throw new ArgumentNullException(nameof(alias));
        ArgumentOutOfRangeException.ThrowIfNegative(expiration.Year);
        Guard.Against.NullOrEmpty(alias, message: "Alias, please");
        if (expiration < DateTime.UtcNow)
        {
            throw new OrderingDomainException(nameof(expiration));
        }
        else
        {
            throw new ArgumentException($"never {alias}");
        }
    }
}
"#;

        const ORDER: &str = r#"namespace Shop.Domain.AggregatesModel.OrderAggregate;
public class Order : Entity, IAggregateRoot
{
    public OrderStatus OrderStatus { get; private set; }
    public void SetShippedStatus()
    {
        if (OrderStatus != OrderStatus.Paid)
        {
            StatusChangeException(OrderStatus.Shipped);
        }
        OrderStatus = OrderStatus.Shipped;
    }
    private void StatusChangeException(OrderStatus orderStatusToChange)
    {
        throw new OrderingDomainException($"Is not possible to change the order status from {OrderStatus} to {orderStatusToChange}.");
    }
}
public enum OrderStatus { Submitted, Paid, Shipped }
"#;

        const CATALOG_ITEM: &str = r#"namespace Shop.Catalog.API.Model;
public class CatalogItem
{
    public string Name { get; set; }
    public int AvailableStock { get; set; }
    public CatalogItem(string name) { Name = name; }
    public int RemoveStock(int quantityDesired)
    {
        if (AvailableStock == 0)
        {
            throw new CatalogDomainException($"Empty stock, product item {Name} is sold out");
        }
        this.AvailableStock -= quantityDesired;
        return quantityDesired;
    }
}
public record CatalogItemDto(string Name);
public abstract class ModelBase { public int Id { get; set; } }
public static class ModelExtensions { public static int Count = 0; }
public class Empty { }
"#;

        const BASKET: &str = r#"namespace Shop.Basket.API.Model;
public class CustomerBasket
{
    public string BuyerId { get; set; }
    public List<BasketItem> Items { get; set; } = [];
}
public class BasketItem : IValidatableObject
{
    public int Quantity { get; set; }
    public IEnumerable<ValidationResult> Validate(ValidationContext validationContext)
    {
        var results = new List<ValidationResult>();
        if (Quantity < 1)
        {
            results.Add(new ValidationResult("Invalid number of units", new[] { "Quantity" }));
        }
        return results;
    }
}
"#;

        const BEHAVIOR: &str = r#"namespace Shop.Ordering.API.Behaviors;
public class ValidatorBehavior
{
    public async Task Handle()
    {
        if (failures.Any())
        {
            throw new OrderingDomainException("Command Validation Errors");
        }
    }
}
"#;

        const VALIDATOR: &str = r#"namespace Shop.Ordering.API.Validations;
public class CreateOrderCommandValidator : AbstractValidator<CreateOrderCommand>
{
    public CreateOrderCommandValidator()
    {
        RuleFor(command => command.City).NotEmpty();
        RuleFor(command => command.CardNumber).NotEmpty().Length(12, 19);
        RuleFor(command => command.CardExpiration).NotEmpty().Must(BeValidExpirationDate).WithMessage("Please specify a valid, card expiration date");
        RuleForEach(v => v.Items)
            .MaximumLength(200)
            .MustAsync(BeUnique)
                .WithMessage("'{PropertyName}' must be unique.");
    }
}
"#;

        const EVENTS: &str = r#"namespace Shop.Ordering.API.IntegrationEvents;
public record OrderStatusChangedToPaidIntegrationEvent(int OrderId) : IntegrationEvent;
public record OrderStartedIntegrationEvent(string UserId) : IntegrationEvent;
public record UnusedIntegrationEvent : IntegrationEvent;
public class OrderStatusChangedToPaidDomainEventHandler
{
    public async Task Handle(OrderStatusChangedToPaidDomainEvent domainEvent, CancellationToken cancellationToken)
    {
        var integrationEvent = new OrderStatusChangedToPaidIntegrationEvent(
            domainEvent.OrderId);
        await _orderingIntegrationEventService.AddAndSaveEventAsync(integrationEvent);
        await _mediator.Publish(new LocalNotification());
    }
}
public class CreateOrderCommandHandler
{
    public async Task<bool> Handle(CreateOrderCommand message, CancellationToken cancellationToken)
    {
        await _orderingIntegrationEventService.AddAndSaveEventAsync(new OrderStartedIntegrationEvent(message.UserId));
        return true;
    }
}
public class OrderingIntegrationEventService
{
    public async Task PublishEventsThroughEventBusAsync(Guid transactionId)
    {
        foreach (var logEvt in pending) { await _eventBus.PublishAsync(logEvt.IntegrationEvent); }
    }
    public async Task AddAndSaveEventAsync(IntegrationEvent evt) { await _eventBus.PublishAsync(evt); }
}
"#;

        const PAYMENT_HANDLER: &str = r#"namespace Shop.PaymentProcessor;
public class OrderStatusChangedToStockConfirmedIntegrationEventHandler : IIntegrationEventHandler<OrderStatusChangedToStockConfirmedIntegrationEvent>
{
    public async Task Handle(OrderStatusChangedToStockConfirmedIntegrationEvent @event)
    {
        IntegrationEvent orderPaymentIntegrationEvent;
        if (options.CurrentValue.PaymentSucceeded)
        {
            orderPaymentIntegrationEvent = new OrderPaymentSucceededIntegrationEvent(@event.OrderId);
        }
        else
        {
            orderPaymentIntegrationEvent = new OrderPaymentFailedIntegrationEvent(@event.OrderId);
        }
        await eventBus.PublishAsync(orderPaymentIntegrationEvent);
    }
}
public class CatalogPaidHandler : IIntegrationEventHandler<OrderStatusChangedToPaidIntegrationEvent>
{
    public async Task Handle(OrderStatusChangedToPaidIntegrationEvent @event)
    {
        var confirmed = rejected
            ? (IntegrationEvent)new OrderStockRejectedIntegrationEvent(@event.OrderId, items)
            : new OrderStockConfirmedIntegrationEvent(@event.OrderId);
        await catalogIntegrationEventService.SaveEventAndCatalogContextChangesAsync(confirmed);
        await bus.Publish<StockChecked>(new { Id = 1 });
    }
}
"#;

        const COLOUR: &str = r#"namespace Clean.Domain.ValueObjects;
public class Colour(string code) : ValueObject
{
    public static Colour From(string code)
    {
        var colour = new Colour(code);
        if (!SupportedColours.Contains(colour))
        {
            throw new UnsupportedColourException(code);
        }
        return colour;
    }
}
"#;

        fn run() -> DomainModel {
            let files: &[(&str, &str)] = &[
                ("Ordering.Domain/Ordering.Domain.csproj", "<Project />"),
                ("Ordering.Domain/SeedWork/Entity.cs", SEED),
                (
                    "Ordering.Domain/AggregatesModel/OrderAggregate/OrderItem.cs",
                    ORDER_ITEM,
                ),
                (
                    "Ordering.Domain/AggregatesModel/OrderAggregate/Order.cs",
                    ORDER,
                ),
                (
                    "Ordering.Domain/AggregatesModel/BuyerAggregate/PaymentMethod.cs",
                    PAYMENT,
                ),
                (
                    "Ordering.Domain/Exceptions/OrderingDomainException.cs",
                    "namespace Shop.Domain.Exceptions;\npublic class OrderingDomainException : Exception { public OrderingDomainException(string m) : base(m) { } }\n",
                ),
                (
                    "Ordering.Domain/Events/OrderStartedDomainEvent.cs",
                    "namespace Shop.Domain.Events;\npublic record OrderStartedDomainEvent(string UserId);\n",
                ),
                ("Catalog.API/Catalog.API.csproj", "<Project />"),
                ("Catalog.API/Model/CatalogItem.cs", CATALOG_ITEM),
                ("Basket.API/Model/Basket.cs", BASKET),
                ("Ordering.API/Ordering.API.csproj", "<Project />"),
                ("Ordering.API/Behaviors/ValidatorBehavior.cs", BEHAVIOR),
                (
                    "Ordering.API/Validations/CreateOrderCommandValidator.cs",
                    VALIDATOR,
                ),
                ("Ordering.API/IntegrationEvents/Events.cs", EVENTS),
                ("PaymentProcessor/PaymentProcessor.csproj", "<Project />"),
                ("PaymentProcessor/Handlers.cs", PAYMENT_HANDLER),
                ("Clean/Domain/ValueObjects/Colour.cs", COLOUR),
                (
                    "Clean/Domain/Exceptions/UnsupportedColourException.cs",
                    "namespace Clean.Domain.Exceptions;\npublic class UnsupportedColourException(string code) : Exception($\"Colour {code} is unsupported.\") { }\n",
                ),
                (
                    "Clean/Domain/Common/ValueObject.cs",
                    "namespace Clean.Domain.Common;\npublic abstract class ValueObject { }\n",
                ),
                // Client copies of domain models: they must not qualify the domain's ids.
                (
                    "ClientApp/Models/Order.cs",
                    "namespace Shop.ClientApp.Models;\npublic class Order { public int Id { get; set; } }\n",
                ),
                (
                    "ClientApp/Models/CatalogItem.cs",
                    "namespace Shop.ClientApp.Models;\npublic class CatalogItem { public int Id { get; set; } }\n",
                ),
                (
                    "Catalog.API/CatalogContext.cs",
                    "namespace Shop.Catalog.API;\npublic class CatalogContext : DbContext { public DbSet<CatalogItem> CatalogItems { get; set; } }\n",
                ),
            ];
            let dir = tempfile::tempdir().unwrap();
            for (path, text) in files {
                let path = dir.path().join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            extract(dir.path())
        }

        type Row<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str, u32);

        /// The rules of `entity` in source order.
        fn rules_of<'a>(model: &'a DomainModel, entity: &str) -> Vec<Row<'a>> {
            let mut rows: Vec<Row> = (model.rules.iter())
                .filter(|r| r.entity == entity)
                .map(|r| {
                    let method = r.method.split_once('.').unwrap().1;
                    (
                        method,
                        r.condition.as_deref(),
                        r.message.as_deref(),
                        r.exception.as_str(),
                        r.line,
                    )
                })
                .collect();
            rows.sort_by_key(|r| r.4);
            rows
        }

        #[test]
        fn guarding_throws_in_entity_methods_are_rules_with_condition_and_message() {
            let model = run();
            assert_eq!(
                rules_of(&model, "OrderItem"),
                [
                    (
                        "OrderItem",
                        Some("units <= 0"),
                        Some("Invalid number of units"),
                        "OrderingDomainException",
                        10
                    ),
                    (
                        "OrderItem",
                        Some("(unitPrice * units) < discount"),
                        Some("The total, of order item is lower than applied discount"),
                        "OrderingDomainException",
                        13
                    ),
                    (
                        "SetNewDiscount",
                        Some("discount < 0"),
                        Some("Discount is not valid"),
                        "OrderingDomainException",
                        18
                    ),
                ]
            );
            let item = model.rules.iter().find(|r| r.line == 18).unwrap();
            assert_eq!(item.method, "OrderItem.SetNewDiscount");
            assert_eq!(
                item.file,
                "Ordering.Domain/AggregatesModel/OrderAggregate/OrderItem.cs"
            );
        }

        #[test]
        fn ternary_coalesce_guard_helpers_and_else_give_the_throwing_condition() {
            let model = run();
            assert_eq!(
                rules_of(&model, "PaymentMethod"),
                [
                    (
                        "PaymentMethod",
                        Some("!(!string.IsNullOrWhiteSpace(cardNumber))"),
                        Some("nameof(cardNumber)"),
                        "OrderingDomainException",
                        8
                    ),
                    (
                        "PaymentMethod",
                        Some("alias == null"),
                        Some("nameof(alias)"),
                        "ArgumentNullException",
                        9
                    ),
                    (
                        "PaymentMethod",
                        Some("ArgumentOutOfRangeException.ThrowIfNegative(expiration.Year)"),
                        None,
                        "ArgumentOutOfRangeException",
                        10
                    ),
                    (
                        "PaymentMethod",
                        Some(r#"Guard.Against.NullOrEmpty(alias, message: "Alias, please")"#),
                        None,
                        "Guard",
                        11
                    ),
                    (
                        "PaymentMethod",
                        Some("expiration < DateTime.UtcNow"),
                        Some("nameof(expiration)"),
                        "OrderingDomainException",
                        14
                    ),
                    (
                        "PaymentMethod",
                        Some("!(expiration < DateTime.UtcNow)"),
                        Some("never {alias}"),
                        "ArgumentException",
                        18
                    ),
                ]
            );
        }

        #[test]
        fn a_throwing_helper_puts_the_rule_at_its_guarded_call() {
            let model = run();
            assert_eq!(
                rules_of(&model, "Order"),
                [(
                    "SetShippedStatus",
                    Some("OrderStatus != OrderStatus.Paid"),
                    Some(
                        "Is not possible to change the order status from {OrderStatus} to {orderStatusToChange}."
                    ),
                    "OrderingDomainException",
                    9
                )]
            );
        }

        #[test]
        fn throws_outside_entities_and_other_exceptions_are_no_rules() {
            let model = run();
            assert!(model.rules.iter().all(|r| !r.file.contains("Behaviors")));
            assert!(
                (model.rules.iter()).all(|r| r.exception != "InvalidOperationException"),
                "{:#?}",
                model.rules
            );
            // An exception class declared in a domain folder counts.
            assert_eq!(
                rules_of(&model, "Colour"),
                [(
                    "From",
                    Some("!SupportedColours.Contains(colour)"),
                    Some("code"),
                    "UnsupportedColourException",
                    9
                )]
            );
        }

        #[test]
        fn plain_models_in_model_folders_become_entities_with_their_rules() {
            let model = run();
            let kind =
                |name: &str| (model.entities.iter().find(|e| e.name == name)).map(|e| e.kind);
            assert_eq!(kind("CatalogItem"), Some(EntityKind::Entity));
            let files = |name: &str| -> Vec<(&str, &str)> {
                (model.entities.iter().filter(|e| e.name == name))
                    .map(|e| (e.id.as_str(), e.file.as_str()))
                    .collect()
            };
            assert_eq!(
                files("Order"),
                [(
                    "Order",
                    "Ordering.Domain/AggregatesModel/OrderAggregate/Order.cs"
                )]
            );
            assert_eq!(
                files("CatalogItem"),
                [("CatalogItem", "Catalog.API/Model/CatalogItem.cs")]
            );
            assert_eq!(kind("CustomerBasket"), Some(EntityKind::Entity));
            assert_eq!(kind("BasketItem"), Some(EntityKind::Entity));
            for skipped in [
                "CatalogItemDto",
                "ModelBase",
                "ModelExtensions",
                "Empty",
                "OrderingDomainException",
                "OrderStartedDomainEvent",
                "ValidatorBehavior",
            ] {
                assert_eq!(kind(skipped), None, "{skipped}");
            }
            assert_eq!(
                rules_of(&model, "CatalogItem"),
                [(
                    "RemoveStock",
                    Some("AvailableStock == 0"),
                    Some("Empty stock, product item {Name} is sold out"),
                    "CatalogDomainException",
                    11
                )]
            );
            assert_eq!(
                rules_of(&model, "BasketItem"),
                [(
                    "Validate",
                    Some("Quantity < 1"),
                    Some("Invalid number of units"),
                    "ValidationResult",
                    15
                )]
            );
            assert!(
                (model.relations.iter())
                    .any(|r| r.from == "CustomerBasket" && r.to == "BasketItem"),
                "{:#?}",
                model.relations
            );
        }

        #[test]
        fn fluent_validation_rules_per_property() {
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
                        "CreateOrderCommand",
                        "CardExpiration",
                        vec![
                            "NotEmpty()",
                            "Must(BeValidExpirationDate)",
                            r#"WithMessage("Please specify a valid, card expiration date")"#
                        ],
                        8
                    ),
                    (
                        "CreateOrderCommand",
                        "CardNumber",
                        vec!["NotEmpty()", "Length(12, 19)"],
                        7
                    ),
                    ("CreateOrderCommand", "City", vec!["NotEmpty()"], 6),
                    (
                        "CreateOrderCommand",
                        "Items",
                        vec![
                            "MaximumLength(200)",
                            "MustAsync(BeUnique)",
                            r#"WithMessage("'{PropertyName}' must be unique.")"#
                        ],
                        9
                    ),
                ]
            );
        }

        #[test]
        fn integration_events_with_publish_sites_services_and_handlers() {
            let model = run();
            let rows: Vec<(&str, Vec<String>, Vec<&str>)> = (model.integration_events.iter())
                .map(|e| {
                    let by = (e.published_by.iter())
                        .map(|p| format!("{} [{}] :{}", p.by, p.service, p.line))
                        .collect();
                    let handled = e.handled_by.iter().map(String::as_str).collect();
                    (e.type_name.as_str(), by, handled)
                })
                .collect();
            let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
            assert_eq!(
                rows,
                [
                    (
                        "OrderPaymentFailedIntegrationEvent",
                        s(&[
                            "OrderStatusChangedToStockConfirmedIntegrationEventHandler.Handle [PaymentProcessor] :15"
                        ]),
                        vec![]
                    ),
                    (
                        "OrderPaymentSucceededIntegrationEvent",
                        s(&[
                            "OrderStatusChangedToStockConfirmedIntegrationEventHandler.Handle [PaymentProcessor] :15"
                        ]),
                        vec![]
                    ),
                    (
                        "OrderStartedIntegrationEvent",
                        s(&["CreateOrderCommandHandler.Handle [Ordering.API] :19"]),
                        vec![]
                    ),
                    (
                        "OrderStatusChangedToPaidIntegrationEvent",
                        s(&[
                            "OrderStatusChangedToPaidDomainEventHandler.Handle [Ordering.API] :11"
                        ]),
                        vec!["CatalogPaidHandler"]
                    ),
                    (
                        "OrderStockConfirmedIntegrationEvent",
                        s(&["CatalogPaidHandler.Handle [PaymentProcessor] :25"]),
                        vec![]
                    ),
                    (
                        "OrderStockRejectedIntegrationEvent",
                        s(&["CatalogPaidHandler.Handle [PaymentProcessor] :25"]),
                        vec![]
                    ),
                    (
                        "StockChecked",
                        s(&["CatalogPaidHandler.Handle [PaymentProcessor] :26"]),
                        vec![]
                    ),
                    ("UnusedIntegrationEvent", vec![], vec![]),
                ]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "namespace Shop.Seed;
public abstract class Entity { public void AddDomainEvent(object e) { } }
public interface IAggregateRoot { }
public abstract class AggregateRoot<T> : Entity, IAggregateRoot { }
public abstract class ValueObject { }
";

    const ORDER: &str = r#"using System;
namespace Shop.Domain.Sales;

/* An order. */
public class Order
    : Entity, IAggregateRoot
{
    private readonly List<OrderLine> _lines;
    private readonly List<OrderLine> _archive;
    public IReadOnlyCollection<OrderLine> Lines => _lines.AsReadOnly();
    [Required]
    public Money Total { get; private set; }
    public int? CustomerId { get; private set; }
    public OrderState State { get; private set; }

    public Order()
    {
        _lines = new List<OrderLine>();
        State = OrderState.New;
    }

    public void Submit()
    {
        if (State == OrderState.New)
        {
            AddDomainEvent(new OrderSubmitted(this, "}"));
            State = OrderState.Submitted;
        }
    }

    public void Approve()
    {
        if (State == OrderState.Submitted ||
            State == OrderState.OnHold)
        {
            State = OrderState.Approved;
        }
    }

    public void Ship()
    {
        if (State != OrderState.Approved)
        {
            Fail(OrderState.Shipped);
        }

        var shipped = new OrderShipped(Id);
        this.AddDomainEvent(shipped);
        State = OrderState.Shipped;
    }

    public void Cancel()
    {
        if (State == OrderState.Shipped || State == OrderState.Cancelled)
        {
            throw new InvalidOperationException("not { possible");
        }
        State = OrderState.Cancelled;
    }

    public void Hold() { State = OrderState.OnHold; }

    public void Reopen()
    {
        if (State == OrderState.Cancelled) { return; }
        else { State = OrderState.Submitted; }
    }

    public void Restore(OrderState previous) { State = previous; }

    public void Close() { }

    private void Fail(OrderState to)
    {
        throw new InvalidOperationException($"cannot go to {to}");
    }
}
"#;

    fn files() -> Vec<(&'static str, &'static str)> {
        vec![
            ("Domain/Seed/Entity.cs", SEED),
            ("Domain/Sales/Order.cs", ORDER),
            (
                "Domain/Sales/OrderState.cs",
                "namespace Shop.Domain.Sales;\n[Flags]\npublic enum OrderState\n{\n    New = 1,\n    Submitted,\n    OnHold,\n    Approved,\n    Shipped,\n    Cancelled\n}\n",
            ),
            (
                "Domain/Sales/OrderLine.cs",
                "namespace Shop.Domain.Sales;\npublic class OrderLine : Entity\n{\n    public string Sku { get; set; }\n    public int Units { get; private set; }\n}\n",
            ),
            (
                "Domain/Sales/Money.cs",
                "namespace Shop.Domain.Sales;\npublic record Money(decimal Amount, string Currency);\n",
            ),
            (
                "Domain/Customers/Customer.cs",
                "namespace Shop.Domain.Customers;\npublic class Customer : AggregateRoot<Guid>\n{\n    public Address Address { get; set; }\n    public void Close() { }\n    public void Rename(string name) { }\n}\n",
            ),
            (
                "Domain/Customers/Address.cs",
                "namespace Shop.Domain.Customers;\npublic class Address : ValueObject\n{\n    public string Street { get; set; }\n}\n",
            ),
            (
                "Data/ShopContext.cs",
                "namespace Shop.Data;\npublic class ShopContext : DbContext\n{\n    public DbSet<Order> Orders { get; set; }\n    public required DbSet<AuditEntry> Audit { get; set; }\n}\n",
            ),
            (
                "Data/AuditEntry.cs",
                "namespace Shop.Data;\npublic class AuditEntry\n{\n    public string Text { get; set; }\n}\n",
            ),
            (
                "App/SubmitOrderCommandHandler.cs",
                "namespace Shop.App;\npublic class SubmitOrderCommandHandler\n    : IRequestHandler<SubmitOrderCommand, bool>\n{\n    public async Task<bool> Handle(SubmitOrderCommand c, CancellationToken t)\n    {\n        var order = await _repo.GetAsync(c.Id);\n        order.Submit();\n        order.Close();\n        _customer.Rename(\"x\");\n        return true;\n    }\n}\n",
            ),
            (
                "App/GetOrderQueryHandler.cs",
                "namespace Shop.App;\npublic class GetOrderQueryHandler : IRequestHandler<GetOrderQuery>\n{\n}\n",
            ),
            (
                "App/ImportUseCase.cs",
                "namespace Shop.App;\npublic class ImportUseCase\n{\n    public void Run() { _order.Approve(); }\n}\n",
            ),
            (
                "App/OrderSubmittedHandler.cs",
                "namespace Shop.App;\npublic class OrderSubmittedHandler : INotificationHandler<OrderSubmitted>\n{\n}\n",
            ),
            (
                "Domain/bin/Debug/Ghost.cs",
                "public class Ghost : Entity, IAggregateRoot { }\n",
            ),
            ("Domain/obj/Gen.cs", "public class Gen : Entity { }\n"),
            (
                "Shop.Tests/FakeOrder.cs",
                "public class FakeOrder : Entity, IAggregateRoot { }\n",
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

    #[test]
    fn entities_by_marker_record_folder_and_dbset() {
        let model = run(&files());
        let kinds: Vec<(&str, EntityKind)> = model
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("Address", EntityKind::ValueObject),
                ("AuditEntry", EntityKind::Unknown),
                ("Customer", EntityKind::AggregateRoot),
                ("Money", EntityKind::ValueObject),
                ("Order", EntityKind::AggregateRoot),
                ("OrderLine", EntityKind::Entity),
            ]
        );
        assert!(model.entities.iter().all(|e| e.source == "csharp"));
        let order = model.entities.iter().find(|e| e.id == "Order").unwrap();
        assert_eq!(
            (order.file.as_str(), order.line),
            ("Domain/Sales/Order.cs", 5)
        );
        let field = |name: &str| order.fields.iter().find(|f| f.name == name).unwrap();
        assert_eq!(
            (
                field("_lines").type_name.as_str(),
                field("_lines").collection
            ),
            ("OrderLine", true)
        );
        assert!(field("Lines").collection);
        assert_eq!(field("CustomerId").type_name, "int");
        assert!(!field("State").collection);
        let money = model.entities.iter().find(|e| e.id == "Money").unwrap();
        assert_eq!(
            money
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["Amount", "Currency"]
        );
    }

    #[test]
    fn relations_has_one_has_many_references() {
        let model = run(&files());
        let rels: Vec<(&str, &str, RelationKind, &str)> = model
            .relations
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str(), r.kind, r.via.as_str()))
            .collect();
        assert_eq!(
            rels,
            vec![
                ("Customer", "Address", RelationKind::HasOne, "Address"),
                ("Order", "Customer", RelationKind::References, "CustomerId"),
                ("Order", "Money", RelationKind::HasOne, "Total"),
                ("Order", "OrderLine", RelationKind::HasMany, "Lines"),
                ("Order", "OrderLine", RelationKind::HasMany, "_archive"),
            ]
        );
    }

    #[test]
    fn state_machine_guards_initial_and_events() {
        let model = run(&files());
        assert_eq!(model.state_machines.len(), 1);
        let m = &model.state_machines[0];
        assert_eq!((m.entity.as_str(), m.field.as_str()), ("Order", "State"));
        assert_eq!(m.enum_type, "OrderState");
        assert_eq!(
            m.states,
            s(&[
                "New",
                "Submitted",
                "OnHold",
                "Approved",
                "Shipped",
                "Cancelled"
            ])
        );
        assert_eq!(m.initial.as_deref(), Some("New"));
        let t = |method: &str| {
            m.transitions
                .iter()
                .find(|t| t.method == format!("Order.{method}"))
                .unwrap_or_else(|| panic!("no transition in {method}"))
        };
        let submit = t("Submit");
        assert_eq!(
            (submit.from.clone(), submit.to.as_str()),
            (s(&["New"]), "Submitted")
        );
        assert_eq!(submit.guard.as_deref(), Some("State == OrderState.New"));
        assert_eq!(submit.events, s(&["OrderSubmitted"]));
        assert_eq!(submit.file, "Domain/Sales/Order.cs");
        assert_eq!(t("Approve").from, s(&["Submitted", "OnHold"]));
        assert_eq!(
            t("Approve").guard.as_deref(),
            Some("State == OrderState.Submitted || State == OrderState.OnHold")
        );
        let ship = t("Ship");
        assert_eq!(ship.from, s(&["Approved"]));
        // An exit guard (throw, return, throwing helper) reads as the allowed
        // condition: the literal negation of the source text.
        assert_eq!(
            ship.guard.as_deref(),
            Some("!(State != OrderState.Approved)")
        );
        assert_eq!(ship.events, s(&["OrderShipped"]));
        assert_eq!(
            t("Cancel").from,
            s(&["New", "Submitted", "OnHold", "Approved"])
        );
        assert_eq!(
            t("Cancel").guard.as_deref(),
            Some("!(State == OrderState.Shipped || State == OrderState.Cancelled)")
        );
        assert_eq!(
            (t("Hold").from.clone(), t("Hold").guard.clone()),
            (vec![], None)
        );
        assert_eq!(
            t("Reopen").from,
            s(&["New", "Submitted", "OnHold", "Approved", "Shipped"])
        );
        assert_eq!(
            t("Reopen").guard.as_deref(),
            Some("!(State == OrderState.Cancelled)")
        );
        assert!(m.transitions.iter().all(|t| t.method != "Order.Restore"));
        assert!(has_diag(&model, "previous"));
        assert!(!model.complete);
    }

    #[test]
    fn events_with_raisers_and_handlers() {
        let model = run(&files());
        let events: Vec<(&str, Vec<String>, Vec<String>)> = model
            .events
            .iter()
            .map(|e| {
                assert_eq!(e.source, "csharp");
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
                ("OrderShipped", s(&["Order.Ship"]), vec![]),
                (
                    "OrderSubmitted",
                    s(&["Order.Submit"]),
                    s(&["OrderSubmittedHandler"])
                ),
            ]
        );
        let sites: Vec<_> = (model.event_handler_sites("OrderSubmitted"))
            .map(|x| (x.handler.as_str(), x.file.as_str(), x.line))
            .collect();
        assert_eq!(
            sites,
            [("OrderSubmittedHandler", "App/OrderSubmittedHandler.cs", 2)]
        );
    }

    #[test]
    fn use_cases_with_request_and_unique_calls() {
        let model = run(&files());
        let cases: Vec<(&str, UseCaseKind, Option<&str>, Vec<String>)> = model
            .use_cases
            .iter()
            .map(|u| (u.id.as_str(), u.kind, u.request.as_deref(), u.calls.clone()))
            .collect();
        assert_eq!(
            cases,
            vec![
                (
                    "GetOrderQueryHandler",
                    UseCaseKind::Query,
                    Some("GetOrderQuery"),
                    vec![]
                ),
                (
                    "ImportUseCase",
                    UseCaseKind::Handler,
                    None,
                    s(&["Order.Approve"])
                ),
                (
                    "SubmitOrderCommandHandler",
                    UseCaseKind::Command,
                    Some("SubmitOrderCommand"),
                    s(&["Customer.Rename", "Order.Submit"])
                ),
            ]
        );
        let ambiguous = model
            .diagnostics
            .iter()
            .find(|d| d.message.contains("Close"))
            .expect("ambiguous Close");
        assert_eq!(ambiguous.file, "App/SubmitOrderCommandHandler.cs");
        assert_eq!(ambiguous.line, 9);
    }

    #[test]
    fn message_handlers_with_the_requests_they_send() {
        let model = run(&[
            (
                "Ordering/OrderPaymentSucceededIntegrationEventHandler.cs",
                r#"namespace Ordering;
public class OrderPaymentSucceededIntegrationEventHandler(
    IMediator mediator,
    ILogger<OrderPaymentSucceededIntegrationEventHandler> logger) :
    IIntegrationEventHandler<OrderPaymentSucceededIntegrationEvent>
{
    public async Task Handle(OrderPaymentSucceededIntegrationEvent @event)
    {
        var command = new SetPaidOrderStatusCommand(@event.OrderId);
        logger.LogInformation("Sending {Name}", "mediator.Send(new Fake())");
        await mediator.Send(command);
    }
}
"#,
            ),
            (
                "Billing/Consumers.cs",
                r#"public class InvoiceConsumer : IConsumer<Bus.OrderShipped>, IConsumer<OrderCancelled>
{
    public Task Consume(ConsumeContext<OrderShipped> context) => _sender.Send(new CreateInvoiceCommand());
}
public class AuditHandler : IHandleMessages<AuditMessage> { }
public class NotAHandler : INotificationHandler<OrderShipped> { }
"#,
            ),
        ]);
        let found: Vec<(&str, &str, Vec<String>, &str, u32)> = (model.message_handlers.iter())
            .map(|h| {
                assert_eq!(h.source, "csharp");
                (
                    h.id.as_str(),
                    h.message.as_str(),
                    h.sends.clone(),
                    h.file.as_str(),
                    h.line,
                )
            })
            .collect();
        let consumers = "Billing/Consumers.cs";
        assert_eq!(
            found,
            [
                ("AuditHandler", "AuditMessage", vec![], consumers, 5),
                (
                    "InvoiceConsumer",
                    "OrderCancelled",
                    s(&["CreateInvoiceCommand"]),
                    consumers,
                    1
                ),
                (
                    "InvoiceConsumer",
                    "OrderShipped",
                    s(&["CreateInvoiceCommand"]),
                    consumers,
                    1
                ),
                (
                    "OrderPaymentSucceededIntegrationEventHandler",
                    "OrderPaymentSucceededIntegrationEvent",
                    s(&["SetPaidOrderStatusCommand"]),
                    "Ordering/OrderPaymentSucceededIntegrationEventHandler.cs",
                    2
                ),
            ]
        );
    }

    const STATUS: &str = "namespace S;\npublic enum Status { New, Paid }\n";

    fn order(body: &str) -> Vec<(&'static str, String)> {
        let src = format!(
            "namespace S;\npublic class Order : Entity\n{{\n    public Status Status {{ get; private set; }}\n{body}}}\n"
        );
        vec![
            ("Domain/Status.cs", STATUS.into()),
            ("Domain/Order.cs", src),
        ]
    }

    fn run_owned(files: &[(&str, String)]) -> DomainModel {
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        run(&files)
    }

    #[test]
    fn a_mapping_assignment_creates_no_machine_and_no_gap() {
        let model = run_owned(&order(
            "    public Order(Dto m) { Status = m.Status; }\n    public void Load(Dto m) { Status = m.Status; }\n",
        ));
        assert!(
            model.state_machines.is_empty(),
            "{:#?}",
            model.state_machines
        );
        assert!(!has_diag(&model, "non-literal"), "{:#?}", model.diagnostics);
    }

    #[test]
    fn one_literal_assignment_creates_the_machine_and_keeps_the_gap() {
        let model = run_owned(&order(
            "    public Order(Dto m) { Status = m.Status; }\n    public void Pay() { Status = Status.Paid; }\n",
        ));
        let [m] = model.state_machines.as_slice() else {
            panic!("{:#?}", model.state_machines)
        };
        assert_eq!(m.transitions.len(), 1);
        assert_eq!(m.transitions[0].to, "Paid");
        assert!(has_diag(&model, "non-literal"));
    }

    fn api(extra: &[(&'static str, &'static str)]) -> DomainModel {
        let mut files = vec![
            (
                "Api/ThingsController.cs",
                "namespace S;\npublic class ThingsController : Controller\n{\n    public ThingView Get([FromBody] CreateThing c) { return null; }\n    private Hidden Own() { return null; }\n}\n",
            ),
            (
                "Models/CreateThing.cs",
                "namespace S;\npublic class CreateThing { public string Name { get; set; } }\n",
            ),
            (
                "Models/ThingView.cs",
                "namespace S;\npublic class ThingView { public List<Line> Lines { get; set; } }\n",
            ),
            (
                "Models/Line.cs",
                "namespace S;\npublic class Line { public int Units { get; set; } }\n",
            ),
            (
                "Models/Hidden.cs",
                "namespace S;\npublic class Hidden { public int Units { get; set; } }\n",
            ),
            (
                "Models/Cart.cs",
                "namespace S;\npublic class Cart { public Line Top { get; set; } }\n",
            ),
        ];
        files.extend_from_slice(extra);
        run(&files)
    }

    fn names(model: &DomainModel) -> Vec<&str> {
        let mut n: Vec<&str> = model.entities.iter().map(|e| e.name.as_str()).collect();
        n.sort();
        n
    }

    #[test]
    fn a_class_only_used_by_controller_actions_is_an_api_dto_not_a_model() {
        // CreateThing/ThingView are action parameter/return; Line is reached
        // through ThingView but also a property of the kept Cart, so it stays.
        // Hidden is only in a private method: no action.
        assert_eq!(names(&api(&[])), ["Cart", "Hidden", "Line"]);
    }

    #[test]
    fn a_dbset_class_stays_a_model_though_an_action_returns_it() {
        let model = api(&[(
            "Data/Ctx.cs",
            "namespace S;\npublic class Ctx : DbContext { public DbSet<ThingView> Views { get; set; } }\n",
        )]);
        assert_eq!(names(&model), ["Cart", "Hidden", "Line", "ThingView"]);
    }

    #[test]
    fn a_class_with_only_injected_fields_is_no_model() {
        let model = run(&[(
            "Domain/Repo.cs",
            "namespace S;\npublic class Repo { private readonly Db _db; private static readonly Repo Instance = new(); }\n",
        )]);
        assert!(model.entities.is_empty(), "{:#?}", model.entities);
    }

    #[test]
    fn bin_obj_and_tests_are_skipped() {
        let model = run(&files());
        assert!(model.entities.iter().any(|e| e.name == "Order"));
        for name in ["Ghost", "Gen", "FakeOrder"] {
            assert!(model.entities.iter().all(|e| e.name != name), "{name}");
        }
    }

    #[test]
    fn duplicate_names_are_qualified_and_reported() {
        let model = run(&[
            (
                "A/Item.cs",
                "\u{FEFF}namespace Shop.A;\npublic class Item : Entity { }\n",
            ),
            (
                "B/Item.cs",
                "namespace Shop.B;\npublic class Item : Entity { }\n",
            ),
        ]);
        let ids: Vec<&str> = model.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["Shop.A.Item", "Shop.B.Item"]);
        assert!(has_diag(&model, "Item"));
        assert!(model.complete);
    }

    #[test]
    fn unreadable_file_is_a_diagnostic() {
        let big = format!("public class Big : Entity {{ }}\n{}", " ".repeat(1 << 20));
        let model = run(&[("Big.cs", big.as_str())]);
        assert!(model.entities.is_empty());
        assert_eq!(model.diagnostics[0].file, "Big.cs");
        assert!(!model.complete);
    }
}
