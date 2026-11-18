//! Table evidence from a service's own source (ADR 0012).
use archguard_engine::{
    analyze_services,
    core::{ServiceTopology, SharedDatastoreScope},
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
