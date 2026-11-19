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
//!   (negated). Without such an `if`, from = [] (any). A constructor
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
//!
//! Not recognized: partial classes across files, nested-type qualification,
//! state changes outside the entity's own methods, `switch`/`is`/pattern
//! guards, `else if` chains as negations, guards on other fields, the State
//! pattern or Stateless, integration-event handlers, and constructor events as
//! part of `initial`. The marker names are the constants below with the ADR
//! 0022 defaults; reading them from the configuration comes later.
use crate::project_files::read_repo_file;
use crate::services::support::{excluded, relative, test_path};
use archguard_core::domain_model::{
    DomainDiagnostic, DomainEvent, DomainModel, Entity, EntityKind, Field, Relation, RelationKind,
    StateMachine, StateTransition, UseCase, UseCaseKind,
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
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
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
type Span = (usize, usize);
/// Diagnostics found on the way: file index, offset, message, drops an element.
type Pending = Vec<(usize, usize, String, bool)>;

/// The domain model of the C# sources below `root`.
pub fn extract(root: &Path) -> DomainModel {
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
        let mut paths = vec![];
        for entry in walker(root) {
            match entry {
                Ok(e) if e.file_type().is_some_and(|t| t.is_file()) => {
                    let rel = relative(root, e.path());
                    if rel.ends_with(".cs") && !test_path(&rel) {
                        paths.push(rel);
                    }
                }
                Ok(_) => {}
                Err(e) => self.push_diag("", format!("Directory not read: {e}")),
            }
        }
        paths.sort();
        if paths.len() > MAX_FILES {
            let message =
                format!("Domain scan stopped after {MAX_FILES} C# files; later files are unknown.");
            self.push_diag(&paths[MAX_FILES], message);
            paths.truncate(MAX_FILES);
        }
        for rel in paths {
            match read_repo_file(root, Path::new(&rel), MAX_BYTES) {
                Ok(bytes) => {
                    let text = lex::strip(&String::from_utf8_lossy(&bytes));
                    self.files.push(Src { rel, text });
                }
                Err(_) => self.push_diag(
                    &rel,
                    "Source file not read: larger than 1 MiB, not a regular file or not readable; its domain elements are unknown.".into(),
                ),
            }
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
        self.model.events = domain_events(&self.decls, raised);
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
    /// an enum and a method of the entity assigns the field.
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
        let mut assigned = false;
        for (method, body, ctor, _) in ent.methods() {
            for (stmt, rhs) in assignments(s, body, field_name) {
                assigned = true;
                let at = format!("{}.{method}", ent.id);
                match rules.step(body, stmt, rhs, ctor) {
                    Step::NonLiteral(v) => pending.push((ent.file, stmt, format!("{at} assigns {field_name} the non-literal `{v}`; the transition is not read."), true)),
                    Step::Initial(to) => {
                        machine.initial.get_or_insert(to);
                    }
                    Step::Transition { from, to, guard, unread } => {
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
        assigned.then_some(machine)
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

fn walker(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .git_global(false)
        .max_depth(Some(64))
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !(excluded(&name) || matches!(name.as_ref(), "bin" | "obj"))
        })
        .build()
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

fn domain_events(decls: &[Decl], raised: BTreeMap<String, BTreeSet<String>>) -> Vec<DomainEvent> {
    let mut handled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in decls {
        for arg in d
            .bases
            .iter()
            .filter_map(|b| generic_args(b, EVENT_HANDLER))
        {
            handled
                .entry(simple(&arg[0]))
                .or_default()
                .insert(d.name.clone());
        }
    }
    let types: BTreeSet<&String> = raised.keys().chain(handled.keys()).collect();
    types
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
        let words: Vec<&str> = idents(decl, 0, decl.len())
            .into_iter()
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
        Some(Member {
            kind,
            public: words.contains(&"public"),
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

        fn read(&self, cond: Span, negate: bool) -> Guard {
            let text = &self.s[cond.0..cond.1];
            match self.cond_set(text) {
                Some(set) if negate => {
                    Guard::From(self.all.difference(&set).cloned().collect(), collapse(text))
                }
                Some(set) => Guard::From(set, collapse(text)),
                None => Guard::Unread(Some(collapse(text)), "condition shape"),
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

/// Navigation in blanked source text.
mod text {
    pub(super) fn is_id(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
    }

    /// Identifiers in `s[from..to]` with their offsets.
    pub(super) fn idents(s: &str, from: usize, to: usize) -> Vec<(usize, &str)> {
        let b = s.as_bytes();
        let mut out = vec![];
        let mut k = from;
        while k < to {
            if !is_id(b[k]) {
                k += 1;
                continue;
            }
            let start = k;
            while k < to && is_id(b[k]) {
                k += 1;
            }
            if !b[start].is_ascii_digit() {
                out.push((start, &s[start..k]));
            }
        }
        out
    }

    /// Only whitespace between token `a` and offset `b`.
    pub(super) fn gap(s: &str, a: (usize, &str), b: usize) -> bool {
        s[a.0 + a.1.len()..b].trim().is_empty()
    }

    pub(super) fn next_nonws(b: &[u8], from: usize) -> Option<usize> {
        (from..b.len()).find(|&k| !b[k].is_ascii_whitespace())
    }

    pub(super) fn prev_nonws(b: &[u8], before: usize) -> Option<usize> {
        (0..before).rev().find(|&k| !b[k].is_ascii_whitespace())
    }

    /// The identifier ending right before `before` (whitespace skipped).
    pub(super) fn prev_word(s: &str, before: usize) -> &str {
        let b = s.as_bytes();
        let Some(end) = prev_nonws(b, before).map(|k| k + 1) else {
            return "";
        };
        let start = (0..end).rev().find(|&k| !is_id(b[k])).map_or(0, |k| k + 1);
        &s[start..end]
    }

    fn pair(c: u8) -> Option<(u8, u8)> {
        [(b'(', b')'), (b'{', b'}'), (b'[', b']'), (b'<', b'>')]
            .into_iter()
            .find(|&(o, z)| c == o || c == z)
    }

    /// The bracket closing the one at `open`.
    pub(super) fn matching(b: &[u8], open: usize) -> Option<usize> {
        let (o, z) = pair(b[open]).filter(|p| p.0 == b[open])?;
        let mut depth = 0;
        for (k, &c) in b.iter().enumerate().skip(open) {
            depth += i32::from(c == o) - i32::from(c == z);
            if depth == 0 {
                return Some(k);
            }
        }
        None
    }

    /// The bracket opening the one at `close`.
    pub(super) fn matching_back(b: &[u8], close: usize) -> Option<usize> {
        let (o, z) = pair(b[close]).filter(|p| p.1 == b[close])?;
        let mut depth = 0;
        for k in (0..=close).rev() {
            depth += i32::from(b[k] == z) - i32::from(b[k] == o);
            if depth == 0 {
                return Some(k);
            }
        }
        None
    }

    /// Splits at `sep` outside (), <>, [] and {}.
    pub(super) fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
        let b = s.as_bytes();
        let mut out = vec![];
        let (mut depth, mut start, mut k) = (0i32, 0, 0);
        while k < b.len() {
            match b[k] {
                b'(' | b'<' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'>' if k == 0 || b[k - 1] != b'=' => depth -= 1,
                _ if depth == 0 && b[k..].starts_with(sep.as_bytes()) => {
                    out.push(&s[start..k]);
                    k += sep.len();
                    start = k;
                    continue;
                }
                _ => {}
            }
            k += 1;
        }
        out.push(&s[start..]);
        out
    }

    /// First `pat` outside parentheses and brackets.
    pub(super) fn find_top(s: &str, pat: &str) -> Option<usize> {
        let b = s.as_bytes();
        let mut depth = 0i32;
        for k in 0..b.len() {
            if depth == 0 && b[k..].starts_with(pat.as_bytes()) {
                return Some(k);
            }
            match b[k] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                _ => {}
            }
        }
        None
    }

    /// First plain `=` (not `==`, `=>`, `!=`, `<=`, `>=`) outside parentheses.
    pub(super) fn first_assign(s: &str) -> Option<usize> {
        let b = s.as_bytes();
        let mut depth = 0i32;
        for k in 0..b.len() {
            match b[k] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'=' if depth == 0
                    && !matches!(b.get(k + 1), Some(b'=' | b'>'))
                    && !(k > 0 && matches!(b[k - 1], b'=' | b'!' | b'<')) =>
                {
                    return Some(k);
                }
                _ => {}
            }
        }
        None
    }

    pub(super) fn unparen(s: &str) -> &str {
        let mut s = s.trim();
        while s.starts_with('(') && matching(s.as_bytes(), 0) == Some(s.len() - 1) {
            s = s[1..s.len() - 1].trim();
        }
        s
    }

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

    /// `Ns.Type<T>?` → `Type`.
    pub(super) fn simple(t: &str) -> String {
        let t = t.trim();
        let t = &t[..t.find('<').unwrap_or(t.len())];
        let t = t.trim_end_matches('?').trim_end_matches("[]");
        t.rsplit('.').next().unwrap_or(t).trim().to_string()
    }

    pub(super) fn mentions(cond: &str, field: &str) -> bool {
        idents(cond, 0, cond.len()).iter().any(|x| x.1 == field)
    }

    pub(super) fn common_prefix(a: &str, b: &str) -> usize {
        a.split('.')
            .zip(b.split('.'))
            .take_while(|(x, y)| x == y)
            .count()
    }

    pub(super) fn line(s: &str, pos: usize) -> u32 {
        let newlines = s.as_bytes()[..pos.min(s.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count();
        u32::try_from(newlines + 1).unwrap_or(u32::MAX)
    }

    pub(super) fn collapse(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// Blanking of comments, preprocessor lines and literal contents.
mod lex {
    /// `src` with comments, preprocessor lines and the contents of string and
    /// char literals replaced by spaces (newlines kept), so offsets and lines
    /// stay those of the source.
    pub(super) fn strip(src: &str) -> String {
        let mut b = src.as_bytes().to_vec();
        // A byte order mark would glue onto the first identifier (`namespace`).
        if b.starts_with("\u{FEFF}".as_bytes()) {
            b[..3].fill(b' ');
        }
        let mut i = 0;
        while i < b.len() {
            match literal_at(&b, i) {
                Some((from, to, next)) => {
                    blank(&mut b, from, to);
                    i = next;
                }
                None => i += 1,
            }
        }
        String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }

    /// A comment, preprocessor line or literal at `i`: (blank from, blank to,
    /// continue at).
    fn literal_at(b: &[u8], i: usize) -> Option<(usize, usize, usize)> {
        let next = b.get(i + 1).copied();
        match b[i] {
            b'/' if next == Some(b'/') => Some((i, line_end(b, i), line_end(b, i))),
            b'/' if next == Some(b'*') => {
                let end = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
                Some((i, end, end))
            }
            b'#' if at_line_start(b, i) => Some((i, line_end(b, i), line_end(b, i))),
            b'\'' => Some(char_literal(b, i)),
            b'"' => Some(string_literal(b, i)),
            _ => None,
        }
    }

    fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
        b.get(from..)?
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|p| from + p)
    }

    fn line_end(b: &[u8], i: usize) -> usize {
        b[i..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(b.len(), |p| i + p)
    }

    fn at_line_start(b: &[u8], i: usize) -> bool {
        b[..i]
            .iter()
            .rev()
            .take_while(|&&c| c != b'\n')
            .all(|c| c.is_ascii_whitespace())
    }

    fn char_literal(b: &[u8], i: usize) -> (usize, usize, usize) {
        let mut j = i + 1;
        while j < b.len() && b[j] != b'\'' && b[j] != b'\n' {
            j += if b[j] == b'\\' { 2 } else { 1 };
        }
        (i + 1, j, j + 1)
    }

    /// Regular, verbatim (`@"…"`), interpolated and raw (`"""…"""`) strings.
    fn string_literal(b: &[u8], i: usize) -> (usize, usize, usize) {
        let quotes = b[i..].iter().take_while(|&&c| c == b'"').count();
        if quotes >= 3 {
            let end = find(b, i + quotes, &b[i..i + quotes]).unwrap_or(b.len());
            return (i + quotes, end, end + quotes);
        }
        let verbatim = b[..i]
            .iter()
            .rev()
            .take_while(|&&c| matches!(c, b'@' | b'$'))
            .any(|&c| c == b'@');
        let mut j = i + 1;
        while j < b.len() {
            match (b[j], verbatim) {
                (b'"', true) if b.get(j + 1) == Some(&b'"') => j += 2,
                (b'"', _) | (b'\n', false) => break,
                (b'\\', false) => j += 2,
                _ => j += 1,
            }
        }
        (i + 1, j, j + 1)
    }

    fn blank(b: &mut [u8], from: usize, to: usize) {
        let to = to.min(b.len());
        for c in &mut b[from.min(to)..to] {
            if *c != b'\n' {
                *c = b' ';
            }
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
        assert_eq!(ship.guard.as_deref(), Some("State != OrderState.Approved"));
        assert_eq!(ship.events, s(&["OrderShipped"]));
        assert_eq!(
            t("Cancel").from,
            s(&["New", "Submitted", "OnHold", "Approved"])
        );
        assert_eq!(
            (t("Hold").from.clone(), t("Hold").guard.clone()),
            (vec![], None)
        );
        assert_eq!(
            t("Reopen").from,
            s(&["New", "Submitted", "OnHold", "Approved", "Shipped"])
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
