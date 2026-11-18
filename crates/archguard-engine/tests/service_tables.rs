//! Table evidence from a service's own source (ADR 0012).
use archguard_engine::{
    analyze_services,
    core::{ServiceTopology, SharedDatastore, SharedDatastoreScope},
};
use std::{fs, path::Path};

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn tables(report: &ServiceTopology, service: &str) -> Vec<String> {
    let mut out: Vec<String> = report
        .datastores
        .tables
        .iter()
        .filter(|t| t.service == service)
        .map(|t| t.name.clone())
        .collect();
    out.sort();
    out
}

const COMPOSE: &str = "services:
  orders:
    build: ./orders
    environment:
      DATABASE_URL: postgres://u:p@db:5432/shop
      CACHE_URL: redis://cache:6379/0
  billing:
    build: ./billing
    environment: [DB_HOST=db, DB_NAME=shop]
  reporting:
    build: ./reporting
    environment: [DB_HOST=db, DB_NAME=shop]
  db:
    image: postgres:16
  cache:
    image: redis:7
";

fn shop() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", COMPOSE);
    // C#: DbSet names, [Table] and ToTable overrides, a migration.
    put(
        root,
        "orders/Data/ShopContext.cs",
        r#"public class ShopContext : DbContext {
    public DbSet<Order> Orders { get; set; }
    public DbSet<Customer> Customers { get; set; }
    public DbSet<Audit> Audits => Set<Audit>();
    protected override void OnModelCreating(ModelBuilder b) {
        b.Entity<Audit>(e => e.ToTable("audit_log"));
        b.Entity<Order>().HasKey(o => o.Id);
        b.Entity<Secret>().ToTable(tableName);
    }
}
[Table("customer")]
public class Customer { }
"#,
    );
    put(
        root,
        "orders/Migrations/20240101_Init.cs",
        r#"migrationBuilder.CreateTable(
    name: "Payments",
    columns: table => new { });"#,
    );
    // Java: JPA entities, Flyway SQL, an unreadable table name.
    put(
        root,
        "billing/src/main/java/Invoice.java",
        "@Entity\n@Table(name = \"orders\")\npublic class Invoice {}\n",
    );
    put(
        root,
        "billing/src/main/java/LineItem.java",
        "@Entity\npublic class LineItem {}\n",
    );
    put(
        root,
        "billing/src/main/java/Dynamic.java",
        "@Entity\n@Table(name = Names.TABLE)\npublic class Dynamic {}\n",
    );
    put(
        root,
        "billing/src/main/resources/db/migration/V1__init.sql",
        "CREATE TABLE IF NOT EXISTS invoices (id int);\nALTER TABLE public.\"Payments\" ADD COLUMN x int;\n",
    );
    // Tests never count as a service's tables.
    put(
        root,
        "billing/src/test/java/Fake.java",
        "@Entity\n@Table(name = \"reports\")\nclass Fake {}\n",
    );
    // Python SQLAlchemy and Kotlin.
    put(
        root,
        "reporting/models.py",
        "class Report(Base):\n    __tablename__ = \"reports\"\n",
    );
    put(
        root,
        "reporting/Summary.kt",
        "@Entity\n@Table(name = \"summaries\")\nclass Summary(val id: Long)\n",
    );
    dir
}

#[test]
fn source_tables_are_attributed_to_their_service() {
    let dir = shop();
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(
        tables(&report, "compose.yaml::orders"),
        ["audit_log", "customer", "orders", "payments"]
    );
    assert_eq!(
        tables(&report, "compose.yaml::billing"),
        ["invoices", "line_item", "orders", "payments"]
    );
    assert_eq!(
        tables(&report, "compose.yaml::reporting"),
        ["reports", "summaries"]
    );
    let evidence = &report
        .datastores
        .tables
        .iter()
        .find(|t| t.service == "compose.yaml::billing" && t.name == "invoices")
        .unwrap()
        .evidence[0];
    assert_eq!(
        evidence.file,
        "billing/src/main/resources/db/migration/V1__init.sql"
    );
    assert_eq!(evidence.line, Some(1));
    // Names from constants or variables stay unknown and are reported as gaps.
    for (file, service) in [
        ("billing/src/main/java/Dynamic.java", "billing"),
        ("orders/Data/ShopContext.cs", "orders"),
    ] {
        assert!(
            report.diagnostics.iter().any(|d| d.file == file
                && d.service.as_deref() == Some(service)
                && d.message.contains("not a literal")),
            "{file}: {:?}",
            report.diagnostics
        );
    }
    assert!(!report.complete);
}

#[test]
fn services_on_one_database_with_a_common_table_share_that_table() {
    let dir = shop();
    let report = analyze_services(dir.path()).unwrap();
    let shared: Vec<_> = report
        .datastores
        .shared
        .iter()
        .filter(|s| s.database.as_deref() == Some("shop"))
        .collect();
    assert_eq!(shared.len(), 1, "{shared:?}");
    assert_eq!(shared[0].scope, SharedDatastoreScope::Table);
    assert_eq!(shared[0].tables, ["orders", "payments"]);
    assert!(!shared[0].disjoint_tables);
    // The tables go to the one relational datastore usage, never to the cache.
    let usage = |store: &str| {
        report
            .datastores
            .usages
            .iter()
            .find(|u| u.service == "compose.yaml::orders" && u.datastore == store)
            .unwrap()
            .tables
            .clone()
    };
    assert_eq!(
        usage("compose.yaml::db"),
        ["audit_log", "customer", "orders", "payments"]
    );
    assert!(usage("compose.yaml::cache").is_empty());
}

#[test]
fn services_on_one_database_with_disjoint_tables_are_reported_as_disjoint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:
  a:
    build: ./a
    environment: [DB_HOST=db, DB_NAME=shop]
  b:
    build: ./b
    environment: [DB_HOST=db, DB_NAME=shop]
  c:
    build: ./c
    environment: [DB_HOST=db, DB_NAME=other]
  db:
    image: postgres:16
",
    );
    put(root, "a/schema.sql", "create table alpha (id int);\n");
    put(root, "b/schema.sql", "CREATE TABLE beta (id int);\n");
    let report = analyze_services(root).unwrap();
    let shop = report
        .datastores
        .shared
        .iter()
        .find(|s| s.database.as_deref() == Some("shop"))
        .unwrap();
    assert_eq!(shop.scope, SharedDatastoreScope::Database);
    assert!(shop.disjoint_tables);
    assert!(shop.tables.is_empty());
    // c has no known tables: the server-level finding claims nothing about tables.
    let server = report
        .datastores
        .shared
        .iter()
        .find(|s| s.scope == SharedDatastoreScope::Server)
        .unwrap();
    assert!(!server.disjoint_tables);
}

fn two_on_shop(a: &str, b: &str) -> String {
    format!(
        "services:
  a:
    build: {a}
    environment: [DB_HOST=db, DB_NAME=shop]
  b:
    build: {b}
    environment: [DB_HOST=db, DB_NAME=shop]
  db:
    image: postgres:16
"
    )
}

fn shop_share(report: &ServiceTopology) -> &SharedDatastore {
    report
        .datastores
        .shared
        .iter()
        .find(|s| s.database.as_deref() == Some("shop"))
        .unwrap()
}

#[test]
fn a_migration_with_multibyte_text_near_create_table_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", COMPOSE);
    // One of the two puts the 300-byte window end inside a two-byte `ü`.
    let umlauts = "ü".repeat(200);
    for (file, pad) in [("A", ""), ("B", " ")] {
        put(
            root,
            &format!("orders/Migrations/{file}.cs"),
            &format!("migrationBuilder.CreateTable({pad}name: \"Users\", // {umlauts}\n"),
        );
    }
    let report = analyze_services(root).unwrap();
    assert_eq!(tables(&report, "compose.yaml::orders"), ["users"]);
}

#[test]
fn services_with_the_same_build_context_get_no_tables_and_no_false_sharing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", &two_on_shop(".", "."));
    put(root, "schema.sql", "CREATE TABLE orders (id int);\n");
    let report = analyze_services(root).unwrap();
    assert!(tables(&report, "compose.yaml::a").is_empty());
    assert!(tables(&report, "compose.yaml::b").is_empty());
    let shop = shop_share(&report);
    assert_eq!(shop.scope, SharedDatastoreScope::Database, "{shop:?}");
    assert!(!shop.disjoint_tables);
    for service in ["a", "b"] {
        assert!(
            report.diagnostics.iter().any(|d| d.file == "schema.sql"
                && d.service.as_deref() == Some(service)
                && d.message.contains("same source directory")),
            "{service}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn discovery_exclusions_and_ignores_apply_and_only_test_directories_are_tests() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", &two_on_shop("./a", "./b"));
    put(
        root,
        "archguard.toml",
        "[project]\nexclude = [\"a/legacy/**\"]\n",
    );
    put(root, ".gitignore", "generated/\n");
    let entity = |t: &str| format!("@Entity\n@Table(name = \"{t}\")\npublic class X {{}}\n");
    put(root, "a/legacy/Old.java", &entity("legacy"));
    put(root, "a/generated/Gen.java", &entity("generated"));
    put(root, "a/latest-news/News.java", &entity("news"));
    put(root, "a/Contest.java", &entity("contest"));
    put(root, "a/test/Fake.java", &entity("fake"));
    put(root, "a/Shop.Tests/Fake.java", &entity("fake_project"));
    let report = analyze_services(root).unwrap();
    assert_eq!(tables(&report, "compose.yaml::a"), ["contest", "news"]);
}

#[test]
fn skipped_files_are_gaps_and_latin1_files_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", &two_on_shop("./a", "./b"));
    put(root, "a/schema.sql", "create table alpha (id int);\n");
    put(root, "b/schema.sql", "CREATE TABLE beta (id int);\n");
    // Latin-1 `é` in a comment, as older SQL scripts have.
    fs::write(
        root.join("b/latin1.sql"),
        b"-- caf\xe9\nCREATE TABLE gamma (id int);\n",
    )
    .unwrap();
    put(root, "b/huge.sql", &"-- padding\n".repeat(100_000));
    let report = analyze_services(root).unwrap();
    assert_eq!(tables(&report, "compose.yaml::b"), ["beta", "gamma"]);
    assert!(!shop_share(&report).disjoint_tables);
    assert!(
        report.diagnostics.iter().any(|d| d.file == "b/huge.sql"
            && d.service.as_deref() == Some("b")
            && d.message.contains("not read")),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn a_local_dbset_variable_is_no_table() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", COMPOSE);
    put(
        root,
        "orders/Data/Repository.cs",
        "class Repository<TEntity> {\n    public void Add(TEntity e) {\n        DbSet<TEntity> dbSet = _context.Set<TEntity>();\n    }\n    public DbSet<Order> Orders => Set<Order>();\n}\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(tables(&report, "compose.yaml::orders"), ["orders"]);
}
