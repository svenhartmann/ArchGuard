use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn shop() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "compose.yaml",
        "services:\n  orders:\n    build: ./orders\n    environment: {DB_HOST: db, DB_NAME: shop}\n  billing:\n    build: ./billing\n    environment: {DB_HOST: db, DB_NAME: shop}\n  db:\n    image: postgres:16\n",
    );
    put(
        dir.path(),
        "orders/schema.sql",
        "CREATE TABLE orders (id int);\n",
    );
    put(
        dir.path(),
        "billing/Invoice.java",
        "@Entity\n@Table(name = \"orders\")\nclass Invoice {}\n",
    );
    dir
}

fn services(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .arg("services")
        .args(args)
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn services_report_shared_tables_in_text_and_json() {
    let dir = shop();
    let text = services(dir.path(), &[]);
    assert!(
        text.contains(
            "shared table: compose.yaml::db database shop, tables orders: compose.yaml::billing, compose.yaml::orders"
        ),
        "{text}"
    );
    assert!(
        text.contains("tables of compose.yaml::billing: orders"),
        "{text}"
    );
    let json: Value = serde_json::from_str(&services(dir.path(), &["--format", "json"])).unwrap();
    let shared = &json["datastores"]["shared"][0];
    assert_eq!(shared["scope"], "table");
    assert_eq!(shared["tables"], serde_json::json!(["orders"]));
    assert_eq!(json["datastores"]["tables"][1]["evidence"][0]["line"], 1);
}
