//! Tables a service's own source names (ADR 0012): EF Core (`DbSet<T>`, `[Table]`,
//! `ToTable`, `CreateTable`), JPA and TypeORM entities, SQLAlchemy `__tablename__`
//! and `CREATE/ALTER TABLE` in SQL files. Names that are not literals stay unknown
//! and are reported as gaps; nothing is guessed from configuration.
use super::support::{diagnostic, excluded, relative};
use archguard_core::{
    ServiceDatastores, ServiceEvidence, ServiceTable, ServiceTopology, SharedDatastoreScope,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const GAP: &str =
    "Table name is not a literal (constant, variable or placeholder); the table stays unknown.";
const SEVERAL: &str =
    "Service uses several relational datastores; its tables are not attributed to one of them.";
/// Engines without tables; their usages never carry a table.
const NO_TABLES: [&str; 6] = [
    "mongodb",
    "redis",
    "cassandra",
    "elasticsearch",
    "neo4j",
    "couchdb",
];

/// One table mention in a file; `name` is `None` when it is not a literal.
struct Mention {
    name: Option<String>,
    line: u32,
    pointer: &'static str,
    description: &'static str,
}

/// What the files of one service name.
#[derive(Default)]
struct Scan {
    mentions: Vec<(String, Mention)>,
    /// EF Core entity class → its table from `[Table]`/`ToTable`; `None` when not literal.
    overrides: BTreeMap<String, Option<String>>,
    /// EF Core `DbSet<T> Name`: (file, line, T, Name).
    sets: Vec<(String, u32, String, String)>,
}

pub(super) fn augment(root: &Path, report: &mut ServiceTopology) {
    let dirs: Vec<(String, PathBuf)> = report
        .services
        .iter()
        .filter_map(|s| {
            let project = s
                .project
                .as_ref()
                .and_then(|id| report.projects.iter().find(|p| &p.id == id))
                .map(|p| &p.path);
            let dir = s.build_context.as_ref().or(project)?;
            Some((s.id.clone(), root.join(dir)))
        })
        .collect();
    let mut tables: BTreeMap<(String, String), Vec<ServiceEvidence>> = BTreeMap::new();
    let mut gaps = BTreeSet::new();
    for (service, dir) in &dirs {
        // A service's directory never includes another service nested below it.
        let nested: Vec<&PathBuf> = dirs
            .iter()
            .map(|(_, d)| d)
            .filter(|d| *d != dir && d.starts_with(dir))
            .collect();
        let mentions = scan_service(root, dir, &nested);
        let short = service.rsplit(':').next().unwrap_or(service);
        for (file, m) in mentions {
            let evidence = ServiceEvidence {
                file: file.clone(),
                pointer: m.pointer.into(),
                line: Some(m.line),
                description: m.description.into(),
            };
            match m.name {
                Some(name) => tables
                    .entry((service.clone(), name))
                    .or_default()
                    .push(evidence),
                None => {
                    gaps.insert(service.clone());
                    diagnostic(report, &file, Some(short), GAP);
                }
            }
        }
    }
    report.datastores.tables = tables
        .into_iter()
        .map(|((service, name), mut evidence)| {
            evidence.sort();
            evidence.dedup();
            ServiceTable {
                service,
                name,
                evidence,
            }
        })
        .collect();
    attribute(report);
    refine(report, &gaps);
}

fn scan_service(root: &Path, dir: &Path, nested: &[&PathBuf]) -> Vec<(String, Mention)> {
    let mut scan = Scan::default();
    for path in files(root, dir) {
        if nested.iter().any(|d| path.starts_with(d)) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            scan.file(&relative(root, &path), &text);
        }
    }
    // A DbSet names the table only when no [Table] or ToTable overrides its entity.
    for (file, line, entity, name) in scan.sets {
        if !scan.overrides.contains_key(&entity) {
            let mention = Mention {
                name: table(&name),
                line,
                pointer: "DbSet",
                description: "EF Core DbSet property; the table is named after it by convention.",
            };
            scan.mentions.push((file, mention));
        }
    }
    scan.mentions
}

/// Gives each service's tables to its datastore usage when that is the service's only
/// datastore with tables; with several, which table lives where stays unknown.
fn attribute(report: &mut ServiceTopology) {
    let d = &report.datastores;
    let mut by_service: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for t in &d.tables {
        by_service
            .entry(&t.service)
            .or_default()
            .push(t.name.clone());
    }
    let relational: Vec<bool> = d
        .usages
        .iter()
        .map(|u| has_tables(d, &u.datastore))
        .collect();
    let mut assigned = Vec::new();
    let mut several = BTreeSet::new();
    for (i, usage) in d.usages.iter().enumerate() {
        let Some(names) = by_service.get(usage.service.as_str()) else {
            continue;
        };
        let count = (0..d.usages.len())
            .filter(|&j| relational[j] && d.usages[j].service == usage.service)
            .count();
        match count {
            1 if relational[i] => assigned.push((i, names.clone())),
            1 | 0 => {}
            _ => {
                several.insert(usage.service.clone());
            }
        }
    }
    for (i, names) in assigned {
        report.datastores.usages[i].tables = names;
    }
    for service in several {
        let short = service.rsplit(':').next().unwrap_or(&service).to_owned();
        diagnostic(report, ".", Some(&short), SEVERAL);
    }
}

/// Shared databases become shared tables when two services name the same table, or
/// are marked disjoint when every service's tables are known and none repeats.
fn refine(report: &mut ServiceTopology, gaps: &BTreeSet<String>) {
    let usages = &report.datastores.usages;
    for shared in &mut report.datastores.shared {
        if shared.scope != SharedDatastoreScope::Database {
            continue;
        }
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut all_known = true;
        for service in &shared.services {
            let tables = usages
                .iter()
                .find(|u| {
                    &u.service == service
                        && u.datastore == shared.datastore
                        && u.database == shared.database
                })
                .map_or(&[][..], |u| &u.tables[..]);
            all_known &= !tables.is_empty() && !gaps.contains(service);
            for name in tables {
                *counts.entry(name).or_default() += 1;
            }
        }
        let common: Vec<String> = counts
            .into_iter()
            .filter(|(_, n)| *n >= 2)
            .map(|(t, _)| t.to_owned())
            .collect();
        shared.disjoint_tables = all_known && common.is_empty();
        if !common.is_empty() {
            shared.scope = SharedDatastoreScope::Table;
            shared.tables = common;
        }
    }
}

fn has_tables(d: &ServiceDatastores, id: &str) -> bool {
    d.nodes
        .iter()
        .find(|n| n.id == id)
        .and_then(|n| n.engine.as_deref())
        .is_none_or(|e| !NO_TABLES.contains(&e))
}

/// Source and SQL files below `dir`, without tests, build output and vendored code.
fn files(root: &Path, dir: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(dir)
        .filter_entry(|e| !excluded(&e.file_name().to_string_lossy()))
        .build()
        .flatten()
        // The walk does not follow links, so it never leaves the root.
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES))
        .map(|e| e.into_path())
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            matches!(ext, "cs" | "java" | "kt" | "py" | "ts" | "sql")
                && !relative(root, p).to_ascii_lowercase().contains("test")
        })
        .take(MAX_FILES)
        .collect()
}

impl Scan {
    fn file(&mut self, file: &str, text: &str) {
        let ext = file.rsplit('.').next().unwrap_or("");
        let mut found = Vec::new();
        match ext {
            "sql" => sql(text, &mut found),
            "py" => python(text, &mut found),
            "cs" => {
                self.ef_core(file, text, &mut found);
                ef_tables(text, &mut found, &mut self.overrides);
            }
            _ => entities(text, &mut found),
        }
        self.mentions
            .extend(found.into_iter().map(|m| (file.to_owned(), m)));
    }

    fn ef_core(&mut self, file: &str, text: &str, found: &mut Vec<Mention>) {
        for (at, _) in text.match_indices("DbSet<") {
            let rest = &text[at + 6..];
            let Some(close) = rest.find('>') else {
                continue;
            };
            let entity = rest[..close].trim().to_owned();
            let after = rest[close + 1..].trim_start();
            let name = ident(after);
            let next = after[name.len()..].trim_start();
            if !name.is_empty() && (next.starts_with('{') || next.starts_with('=')) {
                self.sets
                    .push((file.into(), line(text, at), entity, name.to_owned()));
            }
        }
        for (at, _) in text.match_indices(".ToTable(") {
            let name = literal(&text[at + 9..]).and_then(|n| table(&n));
            // The entity is the generic argument of the nearest `Entity<T>` or
            // `IEntityTypeConfiguration<T>` before the call.
            let owner = ["Entity<", "Configuration<"]
                .iter()
                .filter_map(|p| text[..at].rfind(p).map(|i| i + p.len()))
                .max()
                .and_then(|i| text[i..].split('>').next())
                .map(|t| t.trim().to_owned());
            if let Some(owner) = owner {
                self.overrides.insert(owner, name.clone());
            }
            found.push(Mention {
                name,
                line: line(text, at),
                pointer: "ToTable",
                description: "EF Core ToTable mapping.",
            });
        }
        for (at, _) in text.match_indices("CreateTable(") {
            let window = &text[at + 12..text.len().min(at + 300)];
            found.push(Mention {
                name: name_arg(window).ok().flatten().and_then(|n| table(&n)),
                line: line(text, at),
                pointer: "CreateTable",
                description: "EF Core migration creates the table.",
            });
        }
    }
}

/// Each class declaration with the text since the previous one, which holds its
/// annotations: (offset of `class`, class name, preceding block).
fn classes(text: &str) -> Vec<(usize, &str, &str)> {
    let mut out = Vec::new();
    let mut previous = 0;
    for (at, _) in text.match_indices("class ") {
        let word = at == 0 || !text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_');
        let class = ident(&text[at + 6..]);
        if word && !class.is_empty() {
            out.push((at, class, &text[previous..at]));
            previous = at + 6 + class.len();
        }
    }
    out
}

fn starts_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_alphanumeric() || c == '_')
}

/// EF Core `[Table("x")]` classes; they override the DbSet name of their entity.
fn ef_tables(
    text: &str,
    found: &mut Vec<Mention>,
    overrides: &mut BTreeMap<String, Option<String>>,
) {
    for (at, class, block) in classes(text) {
        let Some(mark) = block.rfind("[Table(") else {
            continue;
        };
        let name = literal(&block[mark + 7..]).and_then(|n| table(&n));
        overrides.insert(class.to_owned(), name.clone());
        found.push(Mention {
            name,
            line: line(text, at),
            pointer: "[Table]",
            description: "EF Core [Table] attribute.",
        });
    }
}

/// `@Entity` classes of JPA (Java, Kotlin) and TypeORM, named by `@Table`, by the
/// entity's own name argument or by convention after the class.
fn entities(text: &str, found: &mut Vec<Mention>) {
    for (at, class, block) in classes(text) {
        let Some(mark) = block.rfind("@Entity") else {
            continue;
        };
        let entity = &block[mark + 7..];
        if starts_ident(entity) {
            continue; // @EntityListeners and the like
        }
        let table_at = block
            .rfind("@Table")
            .filter(|t| !starts_ident(&block[t + 6..]));
        let (pointer, args) = match table_at {
            Some(t) => ("@Table", args(&block[t + 6..])),
            None => ("@Entity", args(entity)),
        };
        let (name, description) = match name_arg(args) {
            Ok(Some(n)) => (table(&n), "Entity mapped to a literally named table."),
            Ok(None) => (
                Some(snake(class)),
                "Entity without a table name; the table is named after the class by convention (snake_case).",
            ),
            Err(()) => (None, ""),
        };
        found.push(Mention {
            name,
            line: line(text, at),
            pointer,
            description,
        });
    }
}

fn python(text: &str, found: &mut Vec<Mention>) {
    for (at, _) in text.match_indices("__tablename__") {
        let rest = text[at + 13..].trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        found.push(Mention {
            name: literal(value).and_then(|n| table(&n)),
            line: line(text, at),
            pointer: "__tablename__",
            description: "SQLAlchemy table name.",
        });
    }
}

/// `CREATE TABLE` and `ALTER TABLE` statements, e.g. Flyway or Liquibase SQL migrations.
fn sql(text: &str, found: &mut Vec<Mention>) {
    let words: Vec<(usize, &str)> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ';' | ','))
        .filter(|w| !w.is_empty())
        .map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w))
        .collect();
    for (i, pair) in words.windows(2).enumerate() {
        let verb = pair[0].1.to_ascii_lowercase();
        if !(matches!(verb.as_str(), "create" | "alter") && pair[1].1.eq_ignore_ascii_case("table"))
        {
            continue;
        }
        let name = words[i + 2..].iter().map(|(_, w)| *w).find(|w| {
            !["if", "not", "exists", "only"]
                .iter()
                .any(|k| w.eq_ignore_ascii_case(k))
        });
        found.push(Mention {
            name: name.and_then(table),
            line: line(text, pair[0].0),
            pointer: if verb == "create" {
                "CREATE TABLE"
            } else {
                "ALTER TABLE"
            },
            description: "SQL statement names the table.",
        });
    }
}

/// The text inside the parentheses that open `s`, or "" without any.
fn args(s: &str) -> &str {
    let Some(rest) = s.trim_start().strip_prefix('(') else {
        return "";
    };
    let mut depth = 1;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..i];
                }
            }
            _ => {}
        }
    }
    rest
}

/// The `name` argument: `Ok(Some)` for a literal, `Ok(None)` when none is given,
/// `Err` when it is a constant or expression.
fn name_arg(args: &str) -> Result<Option<String>, ()> {
    let key = args.match_indices("name").find_map(|(i, _)| {
        let word_start = i == 0 || !args[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
        let rest = args[i + 4..].trim_start();
        (word_start).then_some(())?;
        rest.strip_prefix('=').or_else(|| rest.strip_prefix(':'))
    });
    match key {
        Some(value) => literal(value).map(Some).ok_or(()),
        None if literal(args).is_some() => Ok(literal(args)),
        None if args.trim().is_empty() || args.contains('=') || args.contains(':') => Ok(None),
        None => Err(()),
    }
}

/// The string literal that starts `s` (after whitespace), without quotes.
fn literal(s: &str) -> Option<String> {
    let s = s.trim_start();
    let quote = s.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let rest = &s[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

/// A table name without schema, quotes or brackets, lowercased; `None` unless it is
/// a plain identifier (placeholders such as `${table}` stay unknown).
// shortcut: schemas are dropped, so `a.orders` and `b.orders` match; keep them once a user needs schema-separated tables.
fn table(raw: &str) -> Option<String> {
    let last = raw.rsplit('.').next()?;
    let name = last.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']' | '\''));
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| name.to_ascii_lowercase())
}

fn ident(s: &str) -> &str {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    &s[..end]
}

/// Spring Boot's default physical naming: `LineItem` → `line_item`.
// shortcut: TypeORM and plain Hibernate name tables slightly differently; upgrade per framework if a mismatch shows up.
fn snake(class: &str) -> String {
    let chars: Vec<char> = class.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let lower_before = i > 0 && chars[i - 1].is_lowercase();
        let lower_after = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
        if c.is_uppercase() && lower_before && lower_after {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn line(text: &str, at: usize) -> u32 {
    text[..at].matches('\n').count() as u32 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(snake("LineItem"), "line_item");
        assert_eq!(snake("HTTPLog"), "httplog");
        assert_eq!(table("public.\"Payments\""), Some("payments".into()));
        assert_eq!(table("${prefix}_orders"), None);
        assert_eq!(
            name_arg(r#"name = "x", schema = "s""#),
            Ok(Some("x".into()))
        );
        assert_eq!(name_arg(r#"schema = "s""#), Ok(None));
        assert_eq!(name_arg(r#""x""#), Ok(Some("x".into())));
        assert_eq!(name_arg("TABLE"), Err(()));
        assert_eq!(name_arg("tableName = \"x\""), Ok(None));
    }
}
