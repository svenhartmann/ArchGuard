//! Kysely and Drizzle schemas for the TypeScript extractor (ADR 0022).
//!
//! Kysely (<https://kysely.dev/docs/getting-started#types>,
//! <https://kysely.dev/docs/recipes/data-types>): an `interface DB { table:
//! Row }` becomes one entity per member, named by the key; `Row` is an
//! `interface`/`type` object (or an inline `{…}`) whose `name: Type` members are
//! the fields; `Generated<T>` and `ColumnType<Select, …>` unwrap to `T` /
//! `Select`. Kysely types carry no foreign keys, so no relations. A row type
//! that is a class already read as an entity is not repeated; any other row
//! type that is not a unique object type of the sources is a gap.
//!
//! Drizzle (<https://orm.drizzle.team/docs/sql-schema-declaration>,
//! <https://orm.drizzle.team/docs/indexes-constraints#foreign-key>):
//! `pgTable|mysqlTable|sqliteTable('name', { col: type('c')… })` is an entity
//! named by the string; each key is a field typed by its builder
//! (`integer`, `text`, …); `.references(() => table.col)` (also
//! `(): AnyPgColumn => table.col`, extra options ignored) is a `has_one`
//! relation to the entity assigned to the variable `table`. A non-literal
//! table name, a spread or key-less column, a column that is not a builder
//! call, a reference to a table that is not read, and table-level
//! `foreignKey(…)` in the third argument are gaps. Drizzle's query-level
//! `relations(…)` helper is not read: it adds no structure beyond the foreign
//! keys above.

use crate::lang_text::*;
use crate::lang_ts_imports::{imports, skip_generics, string_literal};
use archguard_core::domain_model::{Entity, EntityKind, Field, Relation, RelationKind};
use std::collections::BTreeSet;

const SOURCE: &str = "typescript";
/// Table builders of Drizzle (<https://orm.drizzle.team/docs/sql-schema-declaration>).
const DRIZZLE_BUILDERS: [&str; 3] = ["pgTable", "mysqlTable", "sqliteTable"];
/// Byte range in a file's text.
type Span = (usize, usize);
type FieldOf = fn(&str, &str, bool) -> Field;

/// What one app's Kysely and Drizzle sources gave.
#[derive(Default)]
pub(super) struct Found {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
    /// File index, offset, message.
    pub gaps: Vec<(usize, usize, String)>,
}

impl Found {
    fn note(&mut self, file: usize, pos: usize, msg: String) {
        self.gaps.push((file, pos, msg));
    }
}

/// A file as the scanner sees it: `text` blanks literal contents, `shown`
/// keeps strings; offsets are the same.
pub(super) struct File<'a> {
    pub rel: &'a str,
    pub text: &'a str,
    pub shown: &'a str,
}

/// Readers of the files of one app. `known` are the names of entities the
/// class scan already found; `field(type, name, to_many)` builds a [`Field`]
/// as the class scan does.
pub(super) fn read(files: &[File], known: &BTreeSet<&str>, field: FieldOf) -> Found {
    let mut out = Found::default();
    if (files.iter()).any(|f| imports(f.shown, |m| m == "kysely", &[])) {
        kysely(files, known, field, &mut out);
    }
    drizzle(files, &mut out);
    out
}

/// An `interface X {…}` or `type X = {…}` with its member text.
struct Object {
    file: usize,
    name: String,
    /// Offset of the declaration word.
    pos: usize,
    body: Span,
    /// `interface X extends …`.
    extends: bool,
}

fn objects(files: &[File]) -> Vec<Object> {
    let mut out = vec![];
    for (fi, f) in files.iter().enumerate() {
        let toks = idents(f.text, 0, f.text.len());
        out.extend((0..toks.len()).filter_map(|t| object_at(fi, f.text, &toks, t)));
    }
    out
}

fn object_at(fi: usize, s: &str, toks: &[(usize, &str)], t: usize) -> Option<Object> {
    let (pos, word) = toks[t];
    let b = s.as_bytes();
    let before = prev_nonws(b, pos).map(|k| b[k]);
    if !matches!(word, "interface" | "type") || matches!(before, Some(b'.' | b'@')) {
        return None;
    }
    let &(npos, name) = toks.get(t + 1).filter(|x| gap(s, toks[t], x.0))?;
    let after = skip_generics(b, npos + name.len());
    let next = next_nonws(b, after)?;
    let (open, extends) = if word == "type" {
        let eq = Some(next).filter(|&k| b[k] == b'=')?;
        (next_nonws(b, eq + 1).filter(|&k| b[k] == b'{')?, false)
    } else {
        let o = (after..b.len()).find(|&j| b[j] == b'{')?;
        (o, s[after..o].contains("extends"))
    };
    Some(Object {
        file: fi,
        name: name.into(),
        pos,
        body: (open, matching(b, open)?),
        extends,
    })
}

/// The `name: Type` members of the `{…}` at `body` of `src` (strings kept),
/// as (name, type); the message for each part that is none.
fn members(src: &str, (open, close): Span) -> (Vec<(String, String)>, Vec<String>) {
    let (mut ok, mut bad) = (vec![], vec![]);
    let parts = split_top(&src[open + 1..close], ";");
    let parts = parts.into_iter().flat_map(|p| split_top(p, ","));
    for part in parts.flat_map(|p| split_top(p, "\n")) {
        let part = part.trim();
        let named = part
            .split_once(':')
            .map(|(n, t)| (n.trim().trim_end_matches('?'), t));
        match named.filter(|(n, _)| string_literal(n).is_some() || is_name(n)) {
            Some((n, t)) => {
                let n = string_literal(n).unwrap_or_else(|| n.to_string());
                ok.push((n, t.trim().to_string()));
            }
            None if part.is_empty() => {}
            None => bad.push(format!("member `{}` is not `name: Type`", collapse(part))),
        }
    }
    (ok, bad)
}

fn is_name(n: &str) -> bool {
    !n.is_empty() && n.bytes().all(is_id)
}

/// `Generated<T>` / `ColumnType<S, I, U>` → `T` / `S`, repeatedly; `| null`
/// dropped.
fn unwrap_column(t: &str) -> String {
    let parts: Vec<&str> = (split_top(t, "|").into_iter())
        .map(str::trim)
        .filter(|p| !matches!(*p, "" | "null" | "undefined"))
        .collect();
    let [t] = parts.as_slice() else {
        return t.trim().into();
    };
    for w in ["Generated", "GeneratedAlways", "ColumnType"] {
        let inner = (t.strip_prefix(w))
            .and_then(|r| r.strip_prefix('<'))
            .and_then(|r| r.strip_suffix('>'));
        if let Some(inner) = inner {
            return unwrap_column(split_top(inner, ",")[0]);
        }
    }
    (*t).into()
}

fn entity(id: &str, fields: Vec<Field>, f: &File, pos: usize) -> Entity {
    Entity {
        id: id.into(),
        name: id.into(),
        kind: EntityKind::Entity,
        fields,
        file: f.rel.into(),
        line: line(f.text, pos),
        source: SOURCE.into(),
        module: f.rel.rsplit_once('/').map_or("", |(d, _)| d).into(),
    }
}

/// What a `DB` member's type gives.
enum Rows {
    /// A class already read as an entity.
    Modelled,
    Read(Vec<(String, String)>),
    Unreadable,
}

fn kysely(files: &[File], known: &BTreeSet<&str>, field: FieldOf, out: &mut Found) {
    let objs = objects(files);
    let dbs: Vec<&Object> = objs.iter().filter(|o| o.name == "DB").collect();
    if dbs.is_empty() {
        let first = (files.iter()).position(|f| imports(f.shown, |m| m == "kysely", &[]));
        let msg = "Kysely is used but no `interface DB` was found; tables are not read; first import here";
        out.note(first.unwrap_or(0), 0, msg.into());
    }
    for db in dbs {
        db_tables(files, &objs, db, (known, field), out);
    }
}

fn db_tables(
    files: &[File],
    objs: &[Object],
    db: &Object,
    (known, field): (&BTreeSet<&str>, FieldOf),
    out: &mut Found,
) {
    let f = &files[db.file];
    if db.extends {
        let msg = "Kysely `interface DB` extends another type; its tables are not read";
        out.note(db.file, db.pos, msg.into());
    }
    let (tables, bad) = members(f.shown, db.body);
    for msg in bad {
        out.note(db.file, db.pos, format!("Kysely `interface DB`: {msg}"));
    }
    for (table, ty) in tables {
        match table_rows(files, objs, db, ty.trim(), known) {
            Rows::Modelled => {}
            Rows::Read(rows) => {
                let fields = (rows.iter())
                    .map(|(n, t)| field(&unwrap_column(t), n, false))
                    .collect();
                out.entities.push(entity(&table, fields, f, db.pos));
            }
            Rows::Unreadable => {
                let msg = format!(
                    "Kysely table `{table}`: row type `{}` is not a unique object type with `name: Type` members; table not read",
                    collapse(&ty)
                );
                out.note(db.file, db.pos, msg);
            }
        }
    }
}

fn table_rows(
    files: &[File],
    objs: &[Object],
    db: &Object,
    ty: &str,
    known: &BTreeSet<&str>,
) -> Rows {
    let inline = ty.starts_with('{') && matching(ty.as_bytes(), 0) == Some(ty.len() - 1);
    let (rows, bad) = if inline {
        members(ty, (0, ty.len() - 1))
    } else if known.contains(ty) {
        return Rows::Modelled;
    } else {
        // The one object type of that name, else the one in this file.
        let all: Vec<&Object> = objs.iter().filter(|o| o.name == ty).collect();
        let here: Vec<&&Object> = all.iter().filter(|o| o.file == db.file).collect();
        let pick = match (all.as_slice(), here.as_slice()) {
            ([one], _) => Some(*one),
            (_, [one]) => Some(**one),
            _ => None,
        };
        match pick.filter(|o| !o.extends) {
            Some(o) => members(files[o.file].shown, o.body),
            None => return Rows::Unreadable,
        }
    };
    if bad.is_empty() {
        Rows::Read(rows)
    } else {
        Rows::Unreadable
    }
}

/// `X` of `X.col` in the first argument of `.references(…)`: `() => X.col`,
/// `(): T => X.col`, with options after a comma ignored.
fn reference_target(args: &str) -> Option<String> {
    let (head, tail) = split_top(args, ",")[0].split_once("=>")?;
    head.trim_start().starts_with('(').then_some(())?;
    let (table, col) = tail.trim().split_once('.')?;
    (is_name(table.trim()) && is_name(col.trim())).then(|| table.trim().to_string())
}

struct Table {
    file: usize,
    pos: usize,
    var: Option<String>,
    id: String,
    /// (column, referenced variable).
    refs: Vec<(String, String)>,
    fields: Vec<Field>,
}

fn drizzle(files: &[File], out: &mut Found) {
    let module = |m: &str| m == "drizzle-orm" || m.starts_with("drizzle-orm/");
    let mut tables: Vec<Table> = vec![];
    for (fi, f) in files.iter().enumerate() {
        if imports(f.shown, module, &DRIZZLE_BUILDERS) {
            tables.extend(file_tables(fi, f, out));
        }
    }
    for t in &tables {
        for (col, var) in &t.refs {
            link(&tables, t, (col, var), out);
        }
    }
    for t in tables {
        let e = entity(&t.id, t.fields, &files[t.file], t.pos);
        out.entities.push(e);
    }
}

/// The relation of `t.col` to the table assigned to variable `var`.
fn link(tables: &[Table], t: &Table, (col, var): (&str, &str), out: &mut Found) {
    let to: Vec<&Table> = (tables.iter())
        .filter(|o| o.var.as_deref() == Some(var))
        .collect();
    match to.as_slice() {
        [one] => out.relations.push(Relation {
            from: t.id.clone(),
            to: one.id.clone(),
            kind: RelationKind::HasOne,
            via: col.into(),
            source: SOURCE.into(),
            external: false,
        }),
        _ => {
            let msg = format!(
                "Drizzle table `{}`: column `{col}` references `{var}`, which is not one table this extractor read; relation not read",
                t.id
            );
            out.note(t.file, t.pos, msg);
        }
    }
}

fn file_tables(fi: usize, f: &File, out: &mut Found) -> Vec<Table> {
    let b = f.text.as_bytes();
    let mut tables = vec![];
    for (pos, word) in idents(f.text, 0, f.text.len()) {
        let open = pos + word.len();
        let called = DRIZZLE_BUILDERS.contains(&word)
            && b.get(open) == Some(&b'(')
            && prev_nonws(b, pos).map(|k| b[k]) != Some(b'.');
        if let Some(close) = called.then(|| matching(b, open)).flatten() {
            tables.extend(table_of(fi, f, (pos, open, close), out));
        }
    }
    tables
}

/// The table of the builder call at (word, open paren, close paren).
fn table_of(
    fi: usize,
    f: &File,
    (pos, open, close): (usize, usize, usize),
    out: &mut Found,
) -> Option<Table> {
    let args = split_top(&f.shown[open + 1..close], ",");
    let Some(name) = args.first().and_then(|a| string_literal(a)) else {
        let msg = "Drizzle table: the table name is not a string literal; table not read";
        out.note(fi, pos, msg.into());
        return None;
    };
    let cols = args.get(1).map(|a| a.trim());
    let Some(cols) = cols.filter(|c| c.starts_with('{') && c.ends_with('}')) else {
        let msg =
            format!("Drizzle table `{name}`: columns are not an object literal; table not read");
        out.note(fi, pos, msg);
        return None;
    };
    if args.iter().skip(2).any(|a| a.contains("foreignKey")) {
        let msg = format!("Drizzle table `{name}`: table-level `foreignKey(…)` is not read");
        out.note(fi, pos, msg);
    }
    let b = f.text.as_bytes();
    let var = prev_nonws(b, pos)
        .filter(|&k| b[k] == b'=')
        .and_then(|k| prev_word_at(f.text, k))
        .map(str::to_string);
    let mut table = Table {
        file: fi,
        pos,
        var,
        id: name,
        refs: vec![],
        fields: vec![],
    };
    for col in split_top(&cols[1..cols.len() - 1], ",") {
        if !col.trim().is_empty() {
            column(&mut table, col.trim(), out);
        }
    }
    Some(table)
}

/// One `key: type('c')….references(() => t.c)` entry of a table.
fn column(table: &mut Table, col: &str, out: &mut Found) {
    let (fi, pos, name) = (table.file, table.pos, table.id.clone());
    let Some((key, expr)) = col.split_once(':').filter(|(k, _)| !k.starts_with("..")) else {
        let msg = format!(
            "Drizzle table `{name}`: spread or key-less column `{}` is not read",
            collapse(col)
        );
        return out.note(fi, pos, msg);
    };
    let key = key.trim();
    let key = string_literal(key).unwrap_or_else(|| key.to_string());
    let expr = expr.trim();
    let ty = expr.split('(').next().unwrap_or("").trim();
    if !expr.contains('(') || ty.is_empty() || !ty.bytes().all(|c| is_id(c) || c == b'.') {
        let msg = format!(
            "Drizzle table `{name}`: column `{key}` is not a builder call; column not read"
        );
        return out.note(fi, pos, msg);
    }
    table.fields.push(Field {
        name: key.clone(),
        type_name: simple(ty),
        collection: false,
    });
    let Some(i) = expr.find(".references(") else {
        return;
    };
    let a = i + ".references(".len() - 1;
    let target = matching(expr.as_bytes(), a).and_then(|z| reference_target(&expr[a + 1..z]));
    match target {
        Some(t) => table.refs.push((key, t)),
        None => {
            let msg = format!(
                "Drizzle table `{name}`: `.references(…)` of column `{key}` is not `() => table.column`; relation not read"
            );
            out.note(fi, pos, msg);
        }
    }
}

/// The identifier ending right before `end` (spaces skipped), e.g. the
/// variable of `const users =`; `None` after a type annotation.
fn prev_word_at(s: &str, end: usize) -> Option<&str> {
    let b = s.as_bytes();
    let e = prev_nonws(b, end)? + 1;
    let start = (0..e).rev().take_while(|&k| is_id(b[k])).last()?;
    Some(&s[start..e])
}
