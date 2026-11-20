//! Python extractor for domain-model/v1 (ADR 0022, phase 6): a text-level
//! reader on the shared Python lexing of `lang_python` (mask, string values,
//! call arguments, `class`/`def` blocks, imports and module resolution), no
//! interpreter. Every entry has `source: "python"`. Test files, `venv`,
//! `.venv` and `migrations` are not read; unreadable or oversized files and
//! files past the first 5 000 are diagnostics.
//!
//! Rules, fixed (a class or statement none of them reads is not guessed):
//! - Frameworks. A class belongs to a framework through a base named
//!   `models.Model` / `django.db.models.Model` / `Model` imported from
//!   `django.db.models` (Django); `SQLModel` (SQLModel); `db.Model` with
//!   `db = SQLAlchemy(…)` in a scanned file (Flask-SQLAlchemy, table names
//!   snake_case); `Base` with `Base = declarative_base()` or a scanned class
//!   deriving `DeclarativeBase` (SQLAlchemy). Bases are looked up through the
//!   imports of the file, then the same file, then a unique class name in the
//!   scanned files; the framework is inherited along scanned bases.
//! - Entities (`entity`, id = class name, `module.Class` when two share a
//!   name): framework classes at module level that are not abstract
//!   (Django `Meta.abstract = True`, SQLAlchemy `__abstract__ = True`) and,
//!   for SQLModel, have `table=True`. The fields and relations of every scanned
//!   base that is not itself an entity (abstract models, mixins, SQLModel
//!   bases) are inherited. A model class nested in another block is a gap.
//! - API models: a SQLModel class without `table=True` goes to `api_models`
//!   with the reason "SQLModel without table=True".
//! - Fields: Django `x = models.CharField(…)` (a call whose last name part ends
//!   in `Field`, or `ForeignKey`); SQLModel annotated members; SQLAlchemy
//!   `Column(…)` / `mapped_column(…)`. The type is the callee, resp. the
//!   annotation without `Optional`, `Mapped`, `X | None`; `list[X]`, `List[X]`,
//!   `set[X]`, `WriteOnlyMapped[X]`, `ManyToManyField` are `collection`.
//! - Relations: Django `ForeignKey`/`OneToOneField` → `has_one`,
//!   `ManyToManyField` → `has_many` (target a class or the string `"Model"`,
//!   `"app.Model"`, `"self"`; a string is matched by class name, an `app`
//!   prefix by a path segment). SQLModel `Relationship()` and SQLAlchemy
//!   `relationship()`: `has_one`/`has_many` by the annotation (`list[X]` or
//!   `Mapped[list[X]]` is many), target from the annotation or the first
//!   argument; without an annotation `has_one` when the class has a foreign
//!   key to the target or `uselist=False`, else `has_many`. `Field(foreign_key=
//!   "table.col")` and `ForeignKey("table.col" | Class.col)` → `references`
//!   (table = `__tablename__`, SQLModel class name in lower case, Flask
//!   snake_case). `backref` does not create the reverse relation. A target no
//!   scanned entity matches (`settings.AUTH_USER_MODEL`, `"auth.User"`) is a
//!   gap.
//! - State machines (Django): a `models.TextChoices`/`IntegerChoices` class
//!   (nested or at module level) used as `choices=E.choices` (or `choices=E`)
//!   of a field, with `self.<field> = E.X` in a method of the entity. That
//!   assignment is a transition (not in `__init__`, where it is the `initial`
//!   state, as is a literal `default=E.X`). From: the nearest enclosing
//!   `if`/`elif` comparing `self.<field>` with `E.…` (`==`, `in`, joined by
//!   `or`; `!=`, `not in` joined by `and` = complement), else an exit `if` of
//!   the block (a body of one `raise`/`return`) as the literal negation, else
//!   any state. The guard is the condition text ("allowed when"), `!(…)` for
//!   an `else` or an exit. A non-literal assignment to the field outside
//!   `__init__` is a gap. Assignments outside the entity and `update(…)` are
//!   not read.
//! - Rules: `raise ValidationError(…)` / `raise ValueError(…)` in a method of
//!   an entity: the nearest enclosing `if`/`elif` as condition (`else` as
//!   `!(…)`, none when unconditional), the first argument's literal as
//!   message (else its source text).
//!
//! Not read: events, use cases, signals, `__init__` parameters as fields,
//! `GenericForeignKey`, `Table(…)` association tables, `backref`, proxy and
//! multi-table inherited fields of a concrete base.
use crate::extract_ctx::{ExtractContext, domain_sources, with_root_config};
use crate::lang_pyscan::dotted;
use crate::lang_python::{
    Arg, Def, DefKind, Import, Stmt, call_args, defs, imports, mask, module_name, resolve_import,
    statements, string_value,
};
use crate::lang_text::{collapse, is_id, line, matching, unparen};
use archguard_core::domain_model::{
    ApiModel, DomainDiagnostic, DomainModel, DomainRule, Entity, EntityKind, Field, Relation,
    RelationKind, StateMachine, StateTransition,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SOURCE: &str = "python";
const SKIP: [&str; 1] = ["migrations"];
const CHOICES: [&str; 2] = ["TextChoices", "IntegerChoices"];
const RULE_EXCEPTIONS: [&str; 2] = ["ValidationError", "ValueError"];

/// The domain model of the Python sources below `root`.
pub fn extract(root: &Path) -> DomainModel {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> DomainModel {
    let (sources, gaps) = domain_sources(ctx.root, &SKIP, |rel| rel.ends_with(".py"), "Python");
    let mut x = Extractor::default();
    for (file, message) in gaps {
        x.gap(&file, 0, message);
    }
    x.load(sources);
    x.run();
    x.finish()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fw {
    Django,
    SqlModel,
    SqlAlchemy,
    Flask,
}

struct File {
    rel: String,
    src: String,
    masked: String,
    defs: Vec<Def>,
    stmts: Vec<Stmt>,
    imports: Vec<Import>,
}

struct Class {
    f: usize,
    d: usize,
    name: String,
    bases: Vec<String>,
    /// `table=True` among the class keywords.
    table: bool,
}

#[derive(Clone)]
enum Target {
    Lit(String),
    Ref(String),
}

#[derive(Clone)]
struct RawRel {
    /// None: a legacy `relationship()` whose side is decided by the foreign keys.
    kind: Option<RelationKind>,
    target: Target,
    /// The target is `table.col` (foreign key), not a class.
    by_table: bool,
}

#[derive(Clone)]
struct Mem {
    name: String,
    line: u32,
    type_name: String,
    collection: bool,
    rel: Option<RawRel>,
}

struct Machine {
    field: String,
    enum_name: String,
    states: Vec<String>,
    default: Option<String>,
}

#[derive(Default)]
struct Extractor {
    model: DomainModel,
    files: Vec<File>,
    paths: BTreeSet<String>,
    classes: Vec<Class>,
    by_name: BTreeMap<String, Vec<usize>>,
    sa_bases: BTreeSet<String>,
    sa_dbs: BTreeSet<String>,
    fw: Vec<Option<Fw>>,
    /// Class index → entity id.
    ent: BTreeMap<usize, String>,
    gaps: BTreeSet<(String, u32, String)>,
}

fn last(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

impl Extractor {
    fn gap(&mut self, file: &str, line: u32, message: String) {
        self.gaps.insert((file.into(), line, message));
    }

    fn finish(mut self) -> DomainModel {
        for (file, line, message) in std::mem::take(&mut self.gaps) {
            self.model.diagnostics.push(DomainDiagnostic {
                file,
                line,
                message,
            });
            self.model.complete = false;
        }
        let m = &mut self.model;
        m.entities.sort();
        m.relations.sort();
        m.relations.dedup();
        m.state_machines.sort();
        m.rules.sort();
        m.api_models.sort();
        self.model
    }

    fn load(&mut self, sources: Vec<(String, String)>) {
        self.paths = sources.iter().map(|(r, _)| r.clone()).collect();
        for (rel, src) in sources {
            let masked = mask(&src);
            let d = defs(&masked);
            let f = self.files.len();
            for (di, def) in d.iter().enumerate() {
                if def.kind != DefKind::Class {
                    continue;
                }
                let (bases, table) = bases_of(&src, &masked, def);
                self.by_name
                    .entry(def.name.clone())
                    .or_default()
                    .push(self.classes.len());
                self.classes.push(Class {
                    f,
                    d: di,
                    name: def.name.clone(),
                    bases,
                    table,
                });
            }
            self.files.push(File {
                rel,
                stmts: statements(&masked),
                imports: imports(&masked),
                defs: d,
                masked,
                src,
            });
        }
    }

    fn def(&self, c: usize) -> &Def {
        &self.files[self.classes[c].f].defs[self.classes[c].d]
    }

    /// Statements directly in the body of class/def `d` of file `f`.
    fn direct(&self, f: usize, d: &Def) -> Vec<Stmt> {
        let file = &self.files[f];
        let lo = file.stmts.partition_point(|s| s.at < d.body.0);
        let hi = file.stmts.partition_point(|s| s.at < d.body.1);
        let body = &file.stmts[lo..hi];
        let Some(first) = body.first() else {
            return vec![];
        };
        (body.iter())
            .filter(|s| s.indent == first.indent)
            .copied()
            .collect()
    }

    fn attr_true(&self, c: usize, name: &str) -> bool {
        let f = self.classes[c].f;
        let file = &self.files[f];
        (self.direct(f, self.def(c)).iter()).any(|s| {
            member(file, s)
                .is_some_and(|m| m.name == name && m.value.is_some_and(|(_, v)| v == "True"))
        })
    }

    fn is_abstract(&self, c: usize) -> bool {
        if self.attr_true(c, "__abstract__") {
            return true;
        }
        // Django: nested `class Meta` with `abstract = True`.
        let (f, me) = (self.classes[c].f, self.classes[c].d);
        (self.classes.iter().enumerate()).any(|(k, class)| {
            class.f == f
                && class.name == "Meta"
                && self.files[f].defs[class.d].parent == Some(me)
                && self.attr_true(k, "abstract")
        })
    }

    /// The class `name` as seen from file `f`: imports, same file, unique name.
    fn lookup(&self, f: usize, name: &str) -> Option<usize> {
        let simple = last(name);
        let first = name.split('.').next().unwrap_or(name);
        let top = |c: &usize| self.def(*c).parent.is_none();
        let in_file = |file: &str, sym: &str| {
            let cands = self.by_name.get(sym)?;
            (cands.iter().copied()).find(|c| self.files[self.classes[*c].f].rel == file && top(c))
        };
        for imp in self.files[f].imports.iter().filter(|i| i.local == first) {
            if let Some((file, sym)) = resolve_import(&self.files[f].rel, imp, &self.paths) {
                let sym = sym.unwrap_or_else(|| simple.to_string());
                if let Some(c) = in_file(&file, &sym) {
                    return Some(c);
                }
            }
        }
        let tops: Vec<usize> = (self.by_name.get(simple)?.iter().copied())
            .filter(top)
            .collect();
        match tops.iter().copied().find(|&c| self.classes[c].f == f) {
            Some(c) => Some(c),
            None if tops.len() == 1 => Some(tops[0]),
            None => None,
        }
    }

    fn root_fw(&self, f: usize, base: &str) -> Option<Fw> {
        let imported = |name: &str, module_end: &str| {
            (self.files[f].imports.iter())
                .any(|i| i.local == name && i.module.ends_with(module_end))
        };
        match base {
            "models.Model" | "django.db.models.Model" => Some(Fw::Django),
            "Model" if imported("Model", "django.db.models") => Some(Fw::Django),
            "SQLModel" | "sqlmodel.SQLModel" => Some(Fw::SqlModel),
            b if b
                .strip_suffix(".Model")
                .is_some_and(|p| self.sa_dbs.contains(p)) =>
            {
                Some(Fw::Flask)
            }
            b if self.sa_bases.contains(b) => Some(Fw::SqlAlchemy),
            _ => None,
        }
    }

    fn find_sa_bases(&mut self) {
        for file in &self.files {
            for s in file.stmts.iter().filter(|s| s.indent == 0) {
                let Some(m) = member(file, s) else { continue };
                let Some((callee, _)) = m.value.and_then(|(at, _)| call(file, at)) else {
                    continue;
                };
                match last(&callee) {
                    "declarative_base" => self.sa_bases.insert(m.name.to_string()),
                    "SQLAlchemy" => self.sa_dbs.insert(m.name.to_string()),
                    _ => false,
                };
            }
        }
        for c in &self.classes {
            if c.bases.iter().any(|b| last(b) == "DeclarativeBase") {
                self.sa_bases.insert(c.name.clone());
            }
        }
    }

    /// The framework of class `c` through its bases, as far as known so far.
    fn framework_of(&self, c: usize) -> Option<Fw> {
        let f = self.classes[c].f;
        self.classes[c].bases.iter().find_map(|b| {
            self.root_fw(f, b).or_else(|| {
                let k = self.lookup(f, b).filter(|&k| k != c)?;
                self.fw[k]
            })
        })
    }

    fn find_frameworks(&mut self) {
        self.fw = vec![None; self.classes.len()];
        for _ in 0..=self.classes.len().min(16) {
            let mut changed = false;
            for c in 0..self.classes.len() {
                if self.fw[c].is_none() {
                    self.fw[c] = self.framework_of(c);
                    changed |= self.fw[c].is_some();
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn run(&mut self) {
        self.find_sa_bases();
        self.find_frameworks();
        let mut entities = vec![];
        for c in 0..self.classes.len() {
            let Some(fw) = self.fw[c] else { continue };
            let d = self.def(c);
            let (file, line) = (self.files[self.classes[c].f].rel.clone(), d.line);
            // `class Meta(Base.Meta)` inherits a model's options; it is no model.
            if d.parent.is_some() && d.name == "Meta" {
                continue;
            }
            if d.parent.is_some() {
                let message = format!(
                    "Model class {} is nested in another block and is not read.",
                    d.name
                );
                self.gap(&file, line, message);
                continue;
            }
            if fw == Fw::SqlModel && !self.classes[c].table {
                self.model.api_models.push(ApiModel {
                    name: self.classes[c].name.clone(),
                    file,
                    line,
                    reason: "SQLModel without table=True".into(),
                });
                continue;
            }
            if !self.is_abstract(c) {
                entities.push(c);
            }
        }
        let mut names: BTreeMap<&str, usize> = BTreeMap::new();
        for &c in &entities {
            *names.entry(&self.classes[c].name).or_default() += 1;
        }
        let ids: Vec<(usize, String)> = (entities.iter())
            .map(|&c| {
                let k = &self.classes[c];
                let id = match names[k.name.as_str()] > 1 {
                    true => format!(
                        "{}.{}",
                        module_name(&self.files[k.f].rel, &self.paths),
                        k.name
                    ),
                    false => k.name.clone(),
                };
                (c, id)
            })
            .collect();
        self.ent = ids.into_iter().collect();
        for &c in &entities {
            self.entity(c);
        }
    }

    fn tablename(&self, c: usize, fw: Fw) -> Option<String> {
        let f = self.classes[c].f;
        let name = &self.classes[c].name;
        for s in self.direct(f, self.def(c)) {
            if let Some(m) = member(&self.files[f], &s).filter(|m| m.name == "__tablename__") {
                return m.value.and_then(|(_, v)| string_value(v));
            }
        }
        match fw {
            Fw::SqlModel => Some(name.to_lowercase()),
            Fw::Flask => Some(snake(name)),
            _ => None,
        }
    }

    /// Own and inherited members: bases that are not entities come first.
    fn members(&self, c: usize, fw: Fw, seen: &mut BTreeSet<usize>) -> Vec<(usize, Mem)> {
        if !seen.insert(c) {
            return vec![];
        }
        let f = self.classes[c].f;
        let mut out = vec![];
        for b in &self.classes[c].bases {
            if let Some(k) = self.lookup(f, b).filter(|k| !self.ent.contains_key(k)) {
                out.extend(self.members(k, fw, seen));
            }
        }
        let file = &self.files[f];
        for s in self.direct(f, self.def(c)) {
            let mem = member(file, &s).and_then(|m| read_member(file, &m, fw));
            out.extend(mem.map(|m| (c, m)));
        }
        out
    }

    fn entity(&mut self, c: usize) {
        let fw = self.fw[c].unwrap_or(Fw::Django);
        let (f, id) = (self.classes[c].f, self.ent[&c].clone());
        let (file, line) = (self.files[f].rel.clone(), self.def(c).line);
        let members = self.members(c, fw, &mut BTreeSet::new());
        let mut resolved = vec![];
        // Foreign-key targets of this class decide the side of a legacy `relationship()`.
        let mut fk_targets = BTreeSet::new();
        for (decl, m) in &members {
            let df = self.classes[*decl].f;
            let r = (m.rel.as_ref()).map(|r| Resolver(self).target(df, &id, &r.target, r.by_table));
            if let (Some(rel), Some(Ok(t))) = (&m.rel, &r)
                && rel.by_table
            {
                fk_targets.insert(t.clone());
            }
            resolved.push((*decl, m, r));
        }
        let mut fields = vec![];
        let (mut rels, mut gaps) = (vec![], vec![]);
        for (decl, m, r) in resolved {
            fields.push(Field {
                name: m.name.clone(),
                type_name: m.type_name.clone(),
                collection: m.collection,
            });
            let (Some(rel), Some(r)) = (&m.rel, r) else {
                continue;
            };
            match r {
                Ok(to) => {
                    let kind = match (rel.by_table, rel.kind) {
                        (true, _) => RelationKind::References,
                        (_, Some(k)) => k,
                        (_, None) if fk_targets.contains(&to) => RelationKind::HasOne,
                        _ => RelationKind::HasMany,
                    };
                    rels.push(Relation {
                        from: id.clone(),
                        to,
                        kind,
                        via: m.name.clone(),
                        source: SOURCE.into(),
                    });
                }
                Err(why) => gaps.push((
                    self.files[self.classes[decl].f].rel.clone(),
                    m.line,
                    format!("Relation {id}.{} not read: {why}.", m.name),
                )),
            }
        }
        for (file, line, message) in gaps {
            self.gap(&file, line, message);
        }
        self.model.relations.extend(rels);
        self.model.entities.push(Entity {
            id: id.clone(),
            name: self.classes[c].name.clone(),
            kind: EntityKind::Entity,
            fields,
            file,
            line,
            source: SOURCE.into(),
        });
        self.methods(c, &id, fw);
    }

    /// Rules, and for Django the state machines, from the methods of `c`.
    fn methods(&mut self, c: usize, id: &str, fw: Fw) {
        let f = self.classes[c].f;
        let me = self.classes[c].d;
        let methods: Vec<usize> = (self.files[f].defs.iter().enumerate())
            .filter(|(_, d)| d.parent == Some(me) && d.kind == DefKind::Function)
            .map(|(i, _)| i)
            .collect();
        let machines = if fw == Fw::Django {
            self.enum_fields(c)
        } else {
            vec![]
        };
        let mut trans: Vec<Vec<StateTransition>> = vec![vec![]; machines.len()];
        let mut initial: Vec<Option<String>> = machines.iter().map(|m| m.default.clone()).collect();
        let mut found = vec![];
        for di in methods {
            let d = &self.files[f].defs[di];
            let file = &self.files[f];
            let lo = file.stmts.partition_point(|s| s.at < d.body.0);
            let hi = file.stmts.partition_point(|s| s.at < d.body.1);
            let method = Method {
                id,
                name: format!("{}.{}", self.classes[c].name, d.name),
                init: d.name == "__init__",
            };
            for k in lo..hi {
                found.extend(read_stmt(file, lo, k, &method, &machines));
            }
        }
        for item in found {
            match item {
                Found::Rule(r) => self.model.rules.push(r),
                Found::Initial(mi, to) => initial[mi] = Some(to),
                Found::Transition(mi, t) => trans[mi].push(t),
                Found::Gap(file, line, message) => self.gap(&file, line, message),
            }
        }
        for ((m, transitions), initial) in machines.into_iter().zip(trans).zip(initial) {
            if !transitions.is_empty() {
                self.model.state_machines.push(StateMachine {
                    entity: id.into(),
                    field: m.field,
                    enum_type: m.enum_name,
                    states: m.states,
                    initial,
                    transitions,
                });
            }
        }
    }

    /// The fields of `c` with `choices=E.choices` of a Choices class.
    fn enum_fields(&self, c: usize) -> Vec<Machine> {
        let f = self.classes[c].f;
        let file = &self.files[f];
        let mut out = vec![];
        for s in self.direct(f, self.def(c)) {
            let Some(m) = member(file, &s) else { continue };
            let Some((callee, args)) = m.value.and_then(|(at, _)| call(file, at)) else {
                continue;
            };
            if !last(&callee).ends_with("Field") {
                continue;
            }
            let Some(choices) = args.iter().find(|a| a.name == Some("choices")) else {
                continue;
            };
            let enum_name = last(choices.value.trim_end_matches(".choices"));
            let Some(states) = self.choice_states(c, enum_name) else {
                continue;
            };
            let default = (args.iter().find(|a| a.name == Some("default")))
                .and_then(|a| literal_state(a.value, enum_name, &states));
            out.push(Machine {
                field: m.name.to_string(),
                enum_name: enum_name.into(),
                states,
                default,
            });
        }
        out
    }

    /// Members of the Choices class `name`: nested in the file first, then visible from it.
    fn choice_states(&self, c: usize, name: &str) -> Option<Vec<String>> {
        let f = self.classes[c].f;
        let is_choices =
            |k: usize| (self.classes[k].bases.iter()).any(|b| CHOICES.contains(&last(b)));
        let nested = (self.by_name.get(name)?.iter().copied())
            .find(|&k| is_choices(k) && self.classes[k].f == f && self.def(k).parent.is_some());
        let k = nested.or_else(|| self.lookup(f, name).filter(|&k| is_choices(k)))?;
        let kf = self.classes[k].f;
        let file = &self.files[kf];
        Some(
            (self.direct(kf, self.def(k)).iter())
                .filter_map(|s| member(file, s))
                .filter(|m| m.ann.is_none() && m.value.is_some() && !m.name.starts_with('_'))
                .map(|m| m.name.to_string())
                .collect(),
        )
    }
}

/// `if`/`elif`/`else` blocks around a statement: conditions and state guards.
mod blocks {
    use super::*;

    /// The kind and condition text of the `if`/`elif`/`else` header at statement `j`.
    fn header(f: &File, j: usize) -> Option<(&'static str, String)> {
        let st = f.stmts[j];
        let m = &f.masked[st.at..st.end];
        let kw = ["if", "elif", "else"].into_iter().find(|k| {
            m.starts_with(k)
                && m[k.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })?;
        let b = m.as_bytes();
        let mut depth = 0i32;
        let colon = (0..b.len()).find(|&i| {
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            depth == 0 && b[i] == b':'
        })?;
        Some((kw, collapse(f.src[st.at + kw.len()..st.at + colon].trim())))
    }

    /// Statements before `k` (from `lo`) that open a block around it, nearest first.
    fn openers(f: &File, lo: usize, k: usize) -> Vec<usize> {
        let mut cur = f.stmts[k].indent;
        let mut out = vec![];
        for j in (lo..k).rev() {
            if f.stmts[j].indent < cur {
                out.push(j);
                cur = f.stmts[j].indent;
            }
        }
        out
    }

    /// The condition text of the header at `j`: `else` is the negation of the
    /// nearest `if`/`elif` of its chain.
    fn guard_of(f: &File, lo: usize, j: usize) -> Option<String> {
        match header(f, j)? {
            ("else", _) => {
                let ind = f.stmts[j].indent;
                let prev = (lo..j)
                    .rev()
                    .take_while(|&p| f.stmts[p].indent >= ind)
                    .find(|&p| f.stmts[p].indent == ind)?;
                header(f, prev)
                    .filter(|h| h.0 != "else")
                    .map(|h| format!("!({})", h.1))
            }
            (_, c) => Some(c),
        }
    }

    pub(super) fn enclosing_condition(f: &File, lo: usize, k: usize) -> Option<String> {
        openers(f, lo, k)
            .into_iter()
            .find_map(|j| guard_of(f, lo, j))
    }

    /// (allowed source states, guard text) of the assignment at statement `k`.
    pub(super) fn guards(
        f: &File,
        lo: usize,
        k: usize,
        m: &Machine,
    ) -> (Vec<String>, Option<String>) {
        let me = format!("self.{}", m.field);
        let allowed = |set: Vec<String>, neg: bool| match neg {
            false => set,
            true => (m.states.iter())
                .filter(|s| !set.contains(s))
                .cloned()
                .collect(),
        };
        for j in openers(f, lo, k) {
            let Some(text) = guard_of(f, lo, j) else {
                continue;
            };
            let negated = text.starts_with("!(");
            let inner = text
                .strip_prefix("!(")
                .and_then(|t| t.strip_suffix(')'))
                .unwrap_or(&text);
            if !inner.contains(&me) {
                continue;
            }
            return match compare(inner, &me, &m.enum_name, &m.states) {
                Some((set, neg)) => (allowed(set, neg != negated), Some(text)),
                None => (vec![], Some(text)),
            };
        }
        // An exit `if` before the assignment in its block.
        let ind = f.stmts[k].indent;
        for j in (lo..k).rev().take_while(|&j| f.stmts[j].indent >= ind) {
            if f.stmts[j].indent != ind {
                continue;
            }
            let Some(("if", cond)) = header(f, j) else {
                continue;
            };
            let exits = f.stmts.get(j + 1).is_some_and(|b| {
                b.indent > ind
                    && ["raise", "return"]
                        .iter()
                        .any(|w| f.masked[b.at..].starts_with(w))
            }) && f.stmts.get(j + 2).is_none_or(|n| n.indent <= ind);
            if !exits {
                continue;
            }
            if let Some((set, neg)) = compare(&cond, &me, &m.enum_name, &m.states) {
                return (allowed(set, !neg), Some(format!("!({cond})")));
            }
        }
        (vec![], None)
    }

    /// States a condition on `me` compares with: (set, negated).
    fn compare(cond: &str, me: &str, en: &str, states: &[String]) -> Option<(Vec<String>, bool)> {
        let lits = |r: &str| -> Option<Vec<String>> {
            let r = r
                .trim()
                .trim_start_matches(['(', '['])
                .trim_end_matches([')', ']']);
            r.split(',').map(|x| literal_state(x, en, states)).collect()
        };
        let one = |p: &str| -> Option<(Vec<String>, bool)> {
            let p = unparen(p);
            for (op, neg, many) in [
                (" not in ", true, true),
                (" in ", false, true),
                ("!=", true, false),
                ("==", false, false),
            ] {
                if let Some((l, r)) = p.split_once(op) {
                    if l.trim() != me {
                        return None;
                    }
                    let set = if many {
                        lits(r)?
                    } else {
                        vec![literal_state(r, en, states)?]
                    };
                    return Some((set, neg));
                }
            }
            None
        };
        if let Some(r) = one(cond) {
            return Some(r);
        }
        let all = |sep: &str, neg: bool| -> Option<(Vec<String>, bool)> {
            let mut set = vec![];
            for p in cond.split(sep) {
                let (s, n) = one(p)?;
                if n != neg {
                    return None;
                }
                set.extend(s);
            }
            Some((set, neg))
        };
        all(" or ", false).or_else(|| all(" and ", true))
    }
}
use blocks::{enclosing_condition, guards};

/// Resolution of relation targets to entity ids.
mod resolving {
    use super::*;

    pub(super) struct Resolver<'a>(pub(super) &'a Extractor);

    impl Resolver<'_> {
        /// The entity named `name` (`Model` or `app.Model`) as seen from file `f`.
        fn name(&self, f: usize, name: &str) -> Result<String, String> {
            let (app, name) = (name.rsplit_once('.')).map_or(("", name), |(a, n)| (last(a), n));
            let mut cands: Vec<usize> = (self.0.by_name.get(name).into_iter().flatten().copied())
                .filter(|c| self.0.ent.contains_key(c))
                .collect();
            if !app.is_empty() {
                cands.retain(|&c| {
                    (self.0.files[self.0.classes[c].f].rel.split('/'))
                        .any(|p| p.trim_end_matches(".py") == app)
                });
            }
            let same: Vec<usize> = (cands.iter().copied())
                .filter(|&c| self.0.classes[c].f == f)
                .collect();
            match (cands.as_slice(), same.as_slice()) {
                ([one], _) | (_, [one]) => Ok(self.0.ent[one].clone()),
                ([], _) => Err(format!("no scanned entity named {name}")),
                _ => Err(format!("{name} matches several scanned entities")),
            }
        }

        /// The entity whose table is the one of `table.col`.
        fn table(&self, s: &str) -> Result<String, String> {
            let table = s.split('.').next().unwrap_or(s);
            let hits: Vec<&String> = (self.0.ent.iter())
                .filter(|&(&c, _)| {
                    self.0.fw[c]
                        .and_then(|fw| self.0.tablename(c, fw))
                        .as_deref()
                        == Some(table)
                })
                .map(|(_, id)| id)
                .collect();
            match hits.as_slice() {
                [one] => Ok((*one).clone()),
                [] => Err(format!("no scanned entity has table {table}")),
                _ => Err(format!("table {table} belongs to several entities")),
            }
        }

        pub(super) fn target(
            &self,
            f: usize,
            me: &str,
            t: &Target,
            by_table: bool,
        ) -> Result<String, String> {
            match (t, by_table) {
                (Target::Lit(s), true) => self.table(s),
                (Target::Lit(s), false) if s == "self" => Ok(me.to_string()),
                (Target::Lit(s), false) => self.name(f, s),
                (Target::Ref(r), by_table) => {
                    let mut parts: Vec<&str> = r.split('.').collect();
                    if by_table && parts.len() > 1 {
                        parts.pop();
                    }
                    let name = parts.last().copied().unwrap_or("");
                    if name.is_empty() || !name.bytes().all(is_id) {
                        return Err(format!("`{r}` is not a class name"));
                    }
                    match self.0.lookup(f, name).and_then(|k| self.0.ent.get(&k)) {
                        Some(id) => Ok(id.clone()),
                        None => Err(format!("`{name}` is not a scanned entity")),
                    }
                }
            }
        }
    }
}
use resolving::Resolver;

struct Method<'a> {
    /// Entity id.
    id: &'a str,
    /// `Class.method`.
    name: String,
    init: bool,
}

enum Found {
    Rule(DomainRule),
    Initial(usize, String),
    Transition(usize, StateTransition),
    Gap(String, u32, String),
}

/// `raise ValidationError(…)` / `raise ValueError(…)` at statement `k`.
fn rule_at(file: &File, lo: usize, k: usize, rest: &str, m: &Method) -> Option<DomainRule> {
    let st = file.stmts[k];
    let at = st.at + "raise ".len() + (rest.len() - rest.trim_start().len());
    let (callee, args) = call(file, at)?;
    RULE_EXCEPTIONS
        .contains(&last(&callee))
        .then(|| DomainRule {
            entity: m.id.into(),
            method: m.name.clone(),
            condition: enclosing_condition(file, lo, k),
            message: (args.first())
                .map(|a| string_value(a.value).unwrap_or_else(|| collapse(a.value))),
            exception: last(&callee).into(),
            file: file.rel.clone(),
            line: line(&file.masked, st.at),
            source: SOURCE.into(),
        })
}

/// The rule or state assignment of statement `k`; `lo` starts the method body.
fn read_stmt(file: &File, lo: usize, k: usize, m: &Method, machines: &[Machine]) -> Vec<Found> {
    let st = file.stmts[k];
    if let Some(rest) = file.masked[st.at..st.end].strip_prefix("raise ") {
        return rule_at(file, lo, k, rest, m)
            .map(Found::Rule)
            .into_iter()
            .collect();
    }
    let Some(a) = member(file, &st).filter(|a| a.ann.is_none()) else {
        return vec![];
    };
    let Some((vat, value)) = a.value else {
        return vec![];
    };
    let at_line = line(&file.masked, vat);
    let mut out = vec![];
    for (mi, mach) in machines.iter().enumerate() {
        if a.name != format!("self.{}", mach.field) {
            continue;
        }
        match literal_state(value, &mach.enum_name, &mach.states) {
            Some(to) if m.init => out.push(Found::Initial(mi, to)),
            Some(to) => {
                let (from, guard) = guards(file, lo, k, mach);
                out.push(Found::Transition(
                    mi,
                    StateTransition {
                        from,
                        to,
                        method: m.name.clone(),
                        guard,
                        events: vec![],
                        file: file.rel.clone(),
                        line: at_line,
                    },
                ));
            }
            None if !m.init => out.push(Found::Gap(
                file.rel.clone(),
                at_line,
                format!(
                    "{} assigns a non-literal value to state field {}; the transition is not read.",
                    m.name, mach.field
                ),
            )),
            None => {}
        }
    }
    out
}

/// `Name(Base, kw=…)` of a class: the bases as written and `table=True`.
fn bases_of(src: &str, masked: &str, def: &Def) -> (Vec<String>, bool) {
    let Some((open, _)) = def.head else {
        return (vec![], false);
    };
    let Some((args, _)) = call_args(src, masked, open - 1) else {
        return (vec![], false);
    };
    let table = args
        .iter()
        .any(|a| a.name == Some("table") && a.value == "True");
    let bases = (args.iter().filter(|a| a.name.is_none()))
        .map(|a| {
            a.value
                .split('[')
                .next()
                .unwrap_or(a.value)
                .trim()
                .to_string()
        })
        .collect();
    (bases, table)
}

struct Member<'a> {
    name: &'a str,
    ann: Option<&'a str>,
    /// Offset in the source and text of the assigned value.
    value: Option<(usize, &'a str)>,
}

/// The first `=` of an annotated member at depth 0 from `from`, no `==`, `!=`, `<=`, `>=`.
fn find_eq(b: &[u8], from: usize) -> Option<usize> {
    let mut depth = 0i32;
    (from..b.len()).find(|&i| {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        depth == 0
            && b[i] == b'='
            && b.get(i + 1) != Some(&b'=')
            && (i == 0 || !b"=!<>".contains(&b[i - 1]))
    })
}

/// `name = value`, `name: ann = value` or `name: ann` (a dotted name allowed).
fn member<'a>(f: &'a File, st: &Stmt) -> Option<Member<'a>> {
    let m = &f.masked[st.at..st.end];
    let n = m.bytes().take_while(|&c| is_id(c) || c == b'.').count();
    if n == 0 || m.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let name = &f.src[st.at..st.at + n];
    let rest = m[n..].trim_start();
    let off = st.end - rest.len();
    let b = rest.as_bytes();
    let value = |i: usize| {
        let raw = &f.src[off + i + 1..st.end];
        let at = off + i + 1 + (raw.len() - raw.trim_start().len());
        Some((at, raw.trim()))
    };
    match b.first()? {
        b':' => {
            let eq = find_eq(b, 1);
            let ann = &f.src[off + 1..eq.map_or(st.end, |i| off + i)];
            Some(Member {
                name,
                ann: Some(ann.trim()),
                value: eq.and_then(value),
            })
        }
        b'=' if b.get(1) != Some(&b'=') => Some(Member {
            name,
            ann: None,
            value: value(0),
        }),
        _ => None,
    }
}

/// The call whose callee starts at `at`: dotted name and arguments.
fn call(f: &File, at: usize) -> Option<(String, Vec<Arg<'_>>)> {
    let (name, end) = dotted(f.masked.as_bytes(), at);
    if name.is_empty() || f.masked.as_bytes().get(end) != Some(&b'(') {
        return None;
    }
    call_args(&f.src, &f.masked, end).map(|(a, _)| (name, a))
}

fn target(v: &str) -> Target {
    string_value(v).map_or_else(|| Target::Ref(v.trim().to_string()), Target::Lit)
}

/// (type, collection) of an annotation without `Mapped`, `Optional`, `| None`.
fn ann_type(ann: &str) -> (String, bool) {
    let mut t = ann.replace(['"', '\''], "");
    let mut coll = false;
    loop {
        t = (t
            .trim()
            .trim_end_matches("| None")
            .trim_end_matches("|None"))
        .to_string();
        t = t.trim().trim_start_matches("None |").trim().to_string();
        let Some(open) = t.find('[').filter(|_| t.ends_with(']')) else {
            break;
        };
        match last(&t[..open]) {
            "Mapped" | "Optional" => {}
            "WriteOnlyMapped" | "DynamicMapped" | "list" | "List" | "set" | "Set" | "Sequence"
            | "Collection" | "Iterable" => coll = true,
            _ => break,
        }
        t = t[open + 1..t.len() - 1].to_string();
    }
    (
        last(t.split('[').next().unwrap_or(&t)).trim().to_string(),
        coll,
    )
}

/// A member with its call (`callee`, `args`), as read by one framework rule.
struct Read<'a> {
    m: &'a Member<'a>,
    line: u32,
    callee: &'a str,
    args: &'a [Arg<'a>],
}

impl Read<'_> {
    fn mem(&self, type_name: String, collection: bool, rel: Option<RawRel>) -> Option<Mem> {
        Some(Mem {
            name: self.m.name.to_string(),
            line: self.line,
            type_name,
            collection,
            rel,
        })
    }

    fn arg(&self, name: &str) -> Option<&Arg<'_>> {
        self.args.iter().find(|a| a.name == Some(name))
    }
}

fn read_member(f: &File, m: &Member, fw: Fw) -> Option<Mem> {
    if m.name.starts_with('_') || m.name.contains('.') {
        return None;
    }
    let line = line(&f.masked, m.value.map_or(0, |v| v.0));
    let c = m.value.and_then(|(at, _)| call(f, at));
    let callee = c.as_ref().map_or("", |c| last(&c.0));
    let args: &[Arg] = c.as_ref().map_or(&[], |c| &c.1);
    let r = Read {
        m,
        line,
        callee,
        args,
    };
    match fw {
        Fw::Django => django_member(&r),
        Fw::SqlModel => sqlmodel_member(&r),
        Fw::SqlAlchemy | Fw::Flask => sqlalchemy_member(&r),
    }
}

fn django_member(r: &Read) -> Option<Mem> {
    if !(r.callee.ends_with("Field") || r.callee == "ForeignKey") {
        return None;
    }
    let kind = match r.callee {
        "ForeignKey" | "OneToOneField" => Some(RelationKind::HasOne),
        "ManyToManyField" => Some(RelationKind::HasMany),
        _ => None,
    };
    let first = (r.args.iter()).find(|a| a.name.is_none() || a.name == Some("to"));
    let t = first.map_or(Target::Ref(String::new()), |a| target(a.value));
    let tn = match &t {
        Target::Lit(s) | Target::Ref(s) if kind.is_some() => last(s).to_string(),
        _ => r.callee.to_string(),
    };
    let rel = kind.map(|kind| RawRel {
        kind: Some(kind),
        target: t,
        by_table: false,
    });
    r.mem(tn, kind == Some(RelationKind::HasMany), rel)
}

fn sqlmodel_member(r: &Read) -> Option<Mem> {
    let ann = r.m.ann?;
    if ann.starts_with("ClassVar") || r.m.name == "model_config" {
        return None;
    }
    let (tn, coll) = ann_type(ann);
    if r.callee == "Relationship" {
        let kind = if coll {
            RelationKind::HasMany
        } else {
            RelationKind::HasOne
        };
        let rel = RawRel {
            kind: Some(kind),
            target: Target::Lit(tn.clone()),
            by_table: false,
        };
        return r.mem(tn, coll, Some(rel));
    }
    let rel = r.arg("foreign_key").map(|a| RawRel {
        kind: None,
        target: target(a.value),
        by_table: true,
    });
    r.mem(tn, coll, rel)
}

fn sqlalchemy_member(r: &Read) -> Option<Mem> {
    match r.callee {
        "Column" | "mapped_column" => {
            let tn = match r.m.ann {
                Some(a) => ann_type(a).0,
                None => (r.args.iter())
                    .find(|a| a.name.is_none() && string_value(a.value).is_none())
                    .map(|a| last(&dotted(a.value.as_bytes(), 0).0).to_string())
                    .unwrap_or_default(),
            };
            let fk = r.args.iter().find_map(|a| call_in(a.value, "ForeignKey"));
            let rel = fk.map(|v| RawRel {
                kind: None,
                target: target(&v),
                by_table: true,
            });
            r.mem(tn, false, rel)
        }
        "relationship" => {
            let ann = r.m.ann.map(ann_type);
            let first = (r.args.iter()).find(|a| a.name.is_none() || a.name == Some("argument"));
            let t = match (first, &ann) {
                (Some(a), _) => target(a.value),
                (None, Some((tn, _))) if !tn.is_empty() => Target::Lit(tn.clone()),
                _ => Target::Ref(String::new()),
            };
            let one = r.arg("uselist").is_some_and(|a| a.value == "False");
            let kind = match (&ann, one) {
                (_, true) => Some(RelationKind::HasOne),
                (Some((_, many)), _) => Some(if *many {
                    RelationKind::HasMany
                } else {
                    RelationKind::HasOne
                }),
                (None, _) => None,
            };
            let (Target::Lit(s) | Target::Ref(s)) = &t;
            let tn = last(s).to_string();
            let rel = RawRel {
                kind,
                target: t,
                by_table: false,
            };
            r.mem(tn, ann.is_some_and(|a| a.1), Some(rel))
        }
        _ => None,
    }
}

/// The first argument of `name(…)` inside the expression `v`, as written.
fn call_in(v: &str, name: &str) -> Option<String> {
    let at = v.find(name)?;
    let open = at + name.len();
    let b = v.as_bytes();
    if b.get(open) != Some(&b'(') || (at > 0 && is_id(b[at - 1])) {
        return None;
    }
    let close = matching(b, open)?;
    let first = v[open + 1..close].split(',').next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// `E.X` or `Entity.E.X` for a member X of enum E.
fn literal_state(v: &str, en: &str, states: &[String]) -> Option<String> {
    let (pre, x) = v.trim().rsplit_once('.')?;
    (last(pre) == en && states.iter().any(|s| s == x)).then(|| x.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(files: &[(&str, &str)]) -> DomainModel {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        extract(dir.path())
    }

    fn rels(m: &DomainModel) -> Vec<String> {
        (m.relations.iter())
            .map(|r| format!("{}-{:?}->{} via {}", r.from, r.kind, r.to, r.via))
            .collect()
    }

    const DJANGO: &str = r#"from django.db import models
class Base(models.Model):
    created = models.DateTimeField()
    class Meta:
        abstract = True
class Author(Base):
    name = models.CharField(max_length=10)
class Book(Base):
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    tags = models.ManyToManyField('Tag')
    editor = models.ForeignKey('shop.Author', null=True, on_delete=models.SET_NULL)
    owner = models.ForeignKey(settings.AUTH_USER_MODEL, on_delete=models.CASCADE)
class Tag(models.Model):
    label = models.CharField(max_length=3)
"#;

    #[test]
    fn django_abstract_base_string_and_unresolvable_relations() {
        let m = model(&[("shop/models.py", DJANGO)]);
        let ids: Vec<&str> = m.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["Author", "Book", "Tag"]);
        let book = m.entities.iter().find(|e| e.id == "Book").unwrap();
        let names: Vec<&str> = book.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["created", "author", "tags", "editor", "owner"]);
        assert_eq!(
            rels(&m),
            [
                "Book-HasOne->Author via author",
                "Book-HasOne->Author via editor",
                "Book-HasMany->Tag via tags",
            ]
        );
        assert!(!m.complete);
        assert_eq!(m.diagnostics.len(), 1);
        let d = &m.diagnostics[0];
        assert!(d.message.contains("Book.owner") && d.line == 12, "{d:?}");
    }

    const ORDER: &str = r#"from django.db import models
class Order(models.Model):
    class Status(models.TextChoices):
        NEW = 'n', 'New'
        PAID = 'p', 'Paid'
        SHIPPED = 's', 'Shipped'
    status = models.CharField(max_length=1, choices=Status.choices, default=Status.NEW)
    def pay(self):
        if self.status == Order.Status.NEW:
            self.status = Order.Status.PAID
    def ship(self):
        if self.status != self.Status.PAID:
            raise ValueError('not paid')
        self.status = Order.Status.SHIPPED
    def reset(self):
        self.status = Order.Status.NEW
    def set_to(self, value):
        self.status = value
    def clean(self):
        if self.status == Order.Status.SHIPPED and not self.paid_at:
            raise ValidationError(f'shipped {self.id} unpaid')
        elif self.total < 0:
            raise ValidationError('negative')
        else:
            raise ValueError('other')
"#;

    #[test]
    fn textchoices_state_machine_with_literal_and_non_literal_assignment() {
        let m = model(&[("shop/models.py", ORDER)]);
        assert_eq!(m.state_machines.len(), 1);
        let sm = &m.state_machines[0];
        assert_eq!(
            (sm.field.as_str(), sm.enum_type.as_str()),
            ("status", "Status")
        );
        assert_eq!(sm.states, ["NEW", "PAID", "SHIPPED"]);
        assert_eq!(sm.initial.as_deref(), Some("NEW"));
        let mut t: Vec<(&str, Vec<&str>, &str, Option<&str>)> = (sm.transitions.iter())
            .map(|t| {
                let from = t.from.iter().map(String::as_str).collect();
                (t.method.as_str(), from, t.to.as_str(), t.guard.as_deref())
            })
            .collect();
        t.sort();
        assert_eq!(
            t,
            [
                (
                    "Order.pay",
                    vec!["NEW"],
                    "PAID",
                    Some("self.status == Order.Status.NEW")
                ),
                ("Order.reset", vec![], "NEW", None),
                (
                    "Order.ship",
                    vec!["PAID"],
                    "SHIPPED",
                    Some("!(self.status != self.Status.PAID)")
                ),
            ]
        );
        assert!(!m.complete);
        assert_eq!(m.diagnostics.len(), 1);
        let d = &m.diagnostics[0];
        assert!(d.message.contains("Order.set_to") && d.line == 18, "{d:?}");
    }

    #[test]
    fn rules_come_from_raise_under_if_in_entity_methods() {
        let m = model(&[("shop/models.py", ORDER)]);
        let r: Vec<(&str, Option<&str>, Option<&str>, &str)> = (m.rules.iter())
            .map(|r| {
                let cond = r.condition.as_deref();
                (
                    r.method.as_str(),
                    cond,
                    r.message.as_deref(),
                    r.exception.as_str(),
                )
            })
            .collect();
        assert_eq!(
            r,
            [
                (
                    "Order.clean",
                    Some("!(self.total < 0)"),
                    Some("other"),
                    "ValueError"
                ),
                (
                    "Order.clean",
                    Some("self.status == Order.Status.SHIPPED and not self.paid_at"),
                    Some("f'shipped {self.id} unpaid'"),
                    "ValidationError"
                ),
                (
                    "Order.clean",
                    Some("self.total < 0"),
                    Some("negative"),
                    "ValidationError"
                ),
                (
                    "Order.ship",
                    Some("self.status != self.Status.PAID"),
                    Some("not paid"),
                    "ValueError"
                ),
            ]
        );
    }

    const SQLMODEL: &str = r#"from sqlmodel import Field, Relationship, SQLModel
class ItemBase(SQLModel):
    title: str
    description: str | None = None
class Item(ItemBase, table=True):
    id: int = Field(primary_key=True)
    owner_id: int = Field(foreign_key='user.id')
    owner: 'User | None' = Relationship(back_populates='items')
class User(SQLModel, table=True):
    id: int = Field(primary_key=True)
    items: list[Item] = Relationship(back_populates='owner')
"#;

    #[test]
    fn sqlmodel_table_is_entity_and_plain_model_is_api_model() {
        let m = model(&[("app/models.py", SQLMODEL)]);
        let ids: Vec<&str> = m.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["Item", "User"]);
        let fields: Vec<&str> = (m.entities[0].fields.iter())
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(fields, ["title", "description", "id", "owner_id", "owner"]);
        assert_eq!(
            rels(&m),
            [
                "Item-HasOne->User via owner",
                "Item-References->User via owner_id",
                "User-HasMany->Item via items",
            ]
        );
        let api = &m.api_models;
        assert_eq!(api.len(), 1);
        assert_eq!(
            (api[0].name.as_str(), api[0].reason.as_str()),
            ("ItemBase", "SQLModel without table=True")
        );
        assert!(m.complete, "{:?}", m.diagnostics);
    }

    const SQLA: &str = r#"from sqlalchemy import Column, ForeignKey, Integer
from sqlalchemy.orm import declarative_base, relationship
Base = declarative_base()
class Mixin:
    stamp = Column(Integer)
class Parent(Mixin, Base):
    __tablename__ = 'parent'
    id = Column(Integer, primary_key=True)
    children = relationship('Child', back_populates='parent')
class Child(Base):
    __tablename__ = 'child'
    id = Column(Integer, primary_key=True)
    parent_id = Column(Integer, ForeignKey('parent.id'))
    parent = relationship('Parent', back_populates='children')
    ghost = relationship('Missing')
"#;

    #[test]
    fn sqlalchemy_relationship_foreign_key_mixin_and_gap() {
        let m = model(&[("db.py", SQLA)]);
        let ids: Vec<&str> = m.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["Child", "Parent"]);
        let stamp = |f: &&Field| f.name == "stamp" && f.type_name == "Integer";
        assert!(m.entities[1].fields.iter().any(|f| stamp(&f)));
        assert_eq!(
            rels(&m),
            [
                "Child-HasOne->Parent via parent",
                "Child-References->Parent via parent_id",
                "Parent-HasMany->Child via children",
            ]
        );
        assert!(!m.complete && m.diagnostics.len() == 1);
        assert!(m.diagnostics[0].message.contains("Child.ghost"));
    }

    #[test]
    fn flask_sqlalchemy_db_model_with_mapped_annotations() {
        let src = r#"import sqlalchemy as sa
import sqlalchemy.orm as so
from flask_sqlalchemy import SQLAlchemy
db = SQLAlchemy()
class User(db.Model):
    id: so.Mapped[int] = so.mapped_column(primary_key=True)
    posts: so.WriteOnlyMapped['Post'] = so.relationship(back_populates='author')
class Post(db.Model):
    user_id: so.Mapped[int] = so.mapped_column(sa.ForeignKey(User.id))
    author: so.Mapped[User] = so.relationship(back_populates='posts')
"#;
        let m = model(&[("models.py", src)]);
        assert_eq!(
            rels(&m),
            [
                "Post-HasOne->User via author",
                "Post-References->User via user_id",
                "User-HasMany->Post via posts",
            ]
        );
        assert!(m.complete, "{:?}", m.diagnostics);
    }
}
