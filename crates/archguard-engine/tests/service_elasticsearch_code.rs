//! Index names a service's Java and C# source names literally, with access (ADR 0023).
use archguard_engine::{analyze_services, core::ServiceTopology};
use std::{fs, path::Path};

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// (index, access) of one service, sorted as reported.
fn usages(report: &ServiceTopology, service: &str) -> Vec<(String, String)> {
    report
        .elasticsearch
        .usages
        .iter()
        .filter(|u| u.service.ends_with(service))
        .map(|u| (u.index.clone(), u.access.clone()))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn gaps(report: &ServiceTopology) -> usize {
    report
        .diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Index name is not a literal"))
        .count()
}

#[test]
fn fixture_lists_java_and_csharp_indices_with_access() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/elasticsearch-services");
    let report = analyze_services(&root).unwrap();
    assert_eq!(
        usages(&report, "java-writer"),
        pairs(&[
            ("audit-log", "write"),
            ("hdr", "read"),
            ("legacy-1", "write"),
            ("misc", "unknown"),
            ("orders-archive", "read"),
            ("orders-v1", "read"),
            ("orders-v1", "write"),
        ])
    );
    assert_eq!(
        usages(&report, "dotnet-reader"),
        pairs(&[
            ("audit-log", "write"),
            ("default-ix", "unknown"),
            ("named", "read"),
            ("orders-v1", "read"),
            ("reader-out", "write"),
            ("stats", "read"),
        ])
    );
    assert_eq!(gaps(&report), 0);
    // Terraform files in the shared fixture have intended gaps; the code reader has none.
    let own = |f: &str| f.starts_with("java-writer/") || f.starts_with("dotnet-reader/");
    assert!(!report.diagnostics.iter().any(|d| own(&d.file)));
    let first = report
        .elasticsearch
        .usages
        .iter()
        .find(|u| u.index == "legacy-1")
        .unwrap();
    assert!(first.evidence[0].file.ends_with("shop/Writer.java"));
    assert_eq!(first.evidence[0].line, Some(11));
}

#[test]
fn non_literals_are_gaps_and_tests_and_foreign_files_do_not_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "api/src/Main.java",
        "import org.elasticsearch.client.Client;\nclass M {\n  void f() {\n    var r = new SearchRequest.Builder().index(Names.ORDERS);\n    var s = new SearchRequest.Builder().index(\"fine\");\n  }\n}\n",
    );
    // The same gap text in one file is reported once, so the second gap sits in its own file.
    put(
        root,
        "api/src/Second.java",
        "import org.elasticsearch.client.Client;\nclass S { var r = new SearchRequest.Builder().index(\"a\" + suffix); }\n",
    );
    put(
        root,
        "api/src/test/java/MainTest.java",
        "import org.elasticsearch.client.Client;\nclass T { var r = new SearchRequest.Builder().index(\"in-test\"); }\n",
    );
    put(
        root,
        "api/src/Other.java",
        "class O { var r = list.index(\"no-import\"); }\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(usages(&report, "api"), pairs(&[("fine", "read")]));
    assert_eq!(gaps(&report), 2);
    assert!(!report.complete);
}

#[test]
fn operation_arguments_are_neither_usages_nor_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "api/Es.cs",
        "using Nest;\nclass E { void f() {\n  _es.Index(doc, i => i.Index(\"c3\"));\n} }\n",
    );
    put(
        root,
        "api/Es.java",
        "import org.elasticsearch.client.Client;\nclass J { void f() {\n  elastic.index(req);\n  ops.index(query, IndexCoordinates.of(\"p2\"));\n} }\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(
        usages(&report, "api"),
        pairs(&[("c3", "write"), ("p2", "unknown")])
    );
    assert_eq!(gaps(&report), 0);
    assert!(report.complete);
}

#[test]
fn builder_variables_name_indices_in_a_separate_statement() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "api/B.java",
        "import co.elastic.clients.elasticsearch.core.*;\nclass B { void f() {\n  var b = new IndexRequest.Builder<Doc>();\n  b.index(Names.ORDERS);\n  var c = new SearchRequest.Builder();\n  c.index(\"lit\");\n  elastic.index(req);\n} }\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(usages(&report, "api"), pairs(&[("lit", "unknown")]));
    assert_eq!(gaps(&report), 1);
    assert!(!report.complete);
}

#[test]
fn fixture_lists_python_and_typescript_indices_with_access() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/elasticsearch-services");
    let report = analyze_services(&root).unwrap();
    assert_eq!(
        usages(&report, "py-indexer"),
        pairs(&[
            ("bulk-target", "write"),
            ("events-count", "read"),
            ("events-old", "write"),
            ("events-v1", "read"),
            ("events-v1", "write"),
            ("in-list", "write"),
        ])
    );
    assert_eq!(
        usages(&report, "ts-search"),
        pairs(&[
            ("products-v2", "read"),
            ("products-v2", "write"),
            ("single-doc", "read"),
            ("tpl-index", "read"),
        ])
    );
    let own = |f: &str| f.starts_with("py-indexer/") || f.starts_with("ts-search/");
    assert!(!report.diagnostics.iter().any(|d| own(&d.file)));
}

#[test]
fn script_non_literals_are_gaps_and_only_imports_and_name_positions_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "api/a.py",
        "import elasticsearch\ndef f(es, name, doc):\n    es.index(index=name, document=doc)\n    es.search(index=f\"x-{name}\")\n    es.get(index=\"ok\", id=doc)\n    es.index(document={\"index\": \"body\"})\n    helpers.bulk(es, [{\"_index\": name}])\n",
    );
    put(
        root,
        "api/b.ts",
        "import { Client } from '@elastic/elasticsearch'\nasync function f(c: Client, cfg: any, n: string) {\n  await c.index({ index: cfg.idx, document: n })\n  await c.search({ index: `x-${n}` })\n  await c.count({ index: 'ok-ts' })\n  await c.index(params)\n}\n",
    );
    put(
        root,
        "api/c.py",
        "# from elasticsearch import Elasticsearch\nes.index(index=\"comment-only\")\n",
    );
    put(
        root,
        "api/d.ts",
        "// import x from '@elastic/elasticsearch'\nc.index({ index: 'comment-only' })\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(
        usages(&report, "api"),
        pairs(&[("ok", "read"), ("ok-ts", "read")])
    );
    assert_eq!(gaps(&report), 2);
    assert!(!report.complete);
}

#[test]
fn many_call_matches_do_not_scan_the_rest_of_the_file_each() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    let body = "d.get(\"k\")\n".repeat(20_000);
    put(root, "api/big.py", &format!("import elasticsearch\n{body}"));
    // Per-match cost must stop at the closing paren; quadratic scanning needs minutes here.
    // A wall-clock bound is used because no work counter exists; 10 s is far above the linear cost.
    let start = std::time::Instant::now();
    let report = analyze_services(root).unwrap();
    assert!(start.elapsed().as_secs() < 10, "took {:?}", start.elapsed());
    assert!(usages(&report, "api").is_empty());
}

const TWO_SERVICES: &str = "services:\n  api:\n    build: ./api\n  web:\n    build: ./web\n";

#[test]
fn constants_of_the_same_service_name_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", TWO_SERVICES);
    put(
        root,
        "api/Doc.java",
        "class Doc {\n  public static final String INDEX_NAME = \"orders\";\n}\n",
    );
    put(
        root,
        "api/W.java",
        "import org.elasticsearch.client.Client;\nclass W { void f() {\n  var r = new SearchRequest.Builder().index(Doc.INDEX_NAME);\n  var q = new SearchRequest.Builder().index(INDEX_NAME);\n} }\n",
    );
    put(
        root,
        "api/K.cs",
        "class K { const string Index = \"b\"; }\n",
    );
    put(
        root,
        "api/R.cs",
        "using Nest;\nclass R { void f() { client.Search<D>(s => s.Index(K.Index)); } }\n",
    );
    put(root, "web/consts.ts", "export const INDEX = 'c'\n");
    put(
        root,
        "web/s.ts",
        "import { Client } from '@elastic/elasticsearch'\nc.search({ index: INDEX })\n",
    );
    put(root, "web/consts.py", "INDEX = \"d\"\n");
    put(
        root,
        "web/i.py",
        "import elasticsearch\nes.get(index=INDEX, id=1)\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(
        usages(&report, "api"),
        pairs(&[("b", "read"), ("orders", "read")])
    );
    assert_eq!(
        usages(&report, "web"),
        pairs(&[("c", "read"), ("d", "read")])
    );
    assert_eq!(gaps(&report), 0);
    let orders = report
        .elasticsearch
        .usages
        .iter()
        .find(|u| u.index == "orders")
        .unwrap();
    let files: Vec<_> = orders.evidence.iter().map(|e| e.file.as_str()).collect();
    assert_eq!(files, ["api/Doc.java", "api/W.java", "api/W.java"]); // declaration, two call sites
    assert_eq!(orders.evidence[0].line, Some(2));
}

#[test]
fn unresolvable_constants_stay_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "compose.yaml", TWO_SERVICES);
    // Same constant name in two services: neither resolves the other's.
    put(
        root,
        "api/Doc.java",
        "class Doc {\n  static final String X = \"api-x\";\n  static final String TWICE = \"t1\";\n  static final String EXPR = \"a\" + X;\n}\nclass Other { static final String TWICE = \"t2\"; }\n",
    );
    put(
        root,
        "web/Doc.java",
        "class Doc {\n  static final String X = \"web-x\";\n  static final String ONLY_WEB = \"w\";\n}\n",
    );
    for (file, arg) in [
        ("A", "Doc.TWICE"),
        ("B", "Doc.EXPR"),
        ("C", "Doc.MISSING"),
        ("D", "Doc.ONLY_WEB"),
        ("E", "Other.X"),
    ] {
        put(
            root,
            &format!("api/{file}.java"),
            &format!(
                "import org.elasticsearch.client.Client;\nclass {file} {{ var r = new SearchRequest.Builder().index({arg}); }}\n"
            ),
        );
    }
    put(
        root,
        "web/F.java",
        "import org.elasticsearch.client.Client;\nclass F { var r = new SearchRequest.Builder().index(Doc.X); }\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(usages(&report, "api"), pairs(&[]));
    assert_eq!(usages(&report, "web"), pairs(&[("web-x", "read")]));
    assert_eq!(gaps(&report), 5);
}

/// Usages of service `api` for one source file calling `.index(arg)` (Java) or the language's call.
fn api_usages(files: &[(&str, &str)]) -> (Vec<(String, String)>, usize) {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    for (path, text) in files {
        put(dir.path(), &format!("api/{path}"), text);
    }
    let report = analyze_services(dir.path()).unwrap();
    (usages(&report, "api"), gaps(&report))
}

const JAVA_USE: &str = "import org.elasticsearch.client.Client;\nclass U { var r = new SearchRequest.Builder().index(Doc.IDX); }\n";

#[test]
fn class_literals_and_logger_fields_do_not_hide_the_owner() {
    let doc = "class Doc {\n  private static final Logger log = LoggerFactory.getLogger(Doc.class);\n  static final String IDX = \"orders\";\n}\n";
    let (u, g) = api_usages(&[("Doc.java", doc), ("U.java", JAVA_USE)]);
    assert_eq!((u, g), (pairs(&[("orders", "read")]), 0));
    // A nested class closes: the constant after it belongs to the outer class again.
    let nested = "class Doc {\n  static class In { }\n  static final String IDX = \"outer\";\n}\n";
    let (u, g) = api_usages(&[("Doc.java", nested), ("U.java", JAVA_USE)]);
    assert_eq!((u, g), (pairs(&[("outer", "read")]), 0));
}

#[test]
fn interface_constants_resolve_but_method_locals_do_not() {
    let doc = "interface Doc {\n  String IDX = \"if-idx\";\n}\n";
    let (u, g) = api_usages(&[("Doc.java", doc), ("U.java", JAVA_USE)]);
    assert_eq!((u, g), (pairs(&[("if-idx", "read")]), 0));
    let local = "interface Doc {\n  default void f() {\n    String IDX = \"local\";\n  }\n}\n";
    let (u, g) = api_usages(&[("Doc.java", local), ("U.java", JAVA_USE)]);
    assert_eq!((u, g), (pairs(&[]), 1));
}

#[test]
fn only_module_level_python_and_static_csharp_constants_count() {
    let py = "import elasticsearch\ndef f(es):\n    IDX = \"local\"\n    es.get(index=IDX, id=1)\n";
    assert_eq!(api_usages(&[("a.py", py)]), (pairs(&[]), 1));
    let cs = |decl: &str| {
        format!(
            "using Nest;\nclass K {{\n  {decl}\n  void f() {{ client.Search<D>(s => s.Index(K.IDX)); }}\n}}\n"
        )
    };
    let inst = cs("readonly string IDX = \"inst\";");
    assert_eq!(api_usages(&[("K.cs", &inst)]), (pairs(&[]), 1));
    let stat = cs("static readonly string IDX = \"stat\";");
    assert_eq!(
        api_usages(&[("K.cs", &stat)]),
        (pairs(&[("stat", "read")]), 0)
    );
}

#[test]
fn braces_in_literals_and_comments_do_not_move_the_owner() {
    let use_outer = JAVA_USE.replace("Doc.IDX", "Outer.IDX");
    let probes = [
        "static class In { char c = '{'; }",
        "static class In { char c = '}'; }",
        "static class In { char c = '\\''; }\n  static class Jn { }",
        "String t = \"\"\"\n    { } }\n    \"\"\";",
        "String s = \"a\\\"}\";",
        "static class In { } // {",
        "static class In { } // }",
        "/* } */ int y;",
        "/*\n   }\n */ int y;",
    ];
    let mut broken = Vec::new();
    for probe in probes {
        let doc = format!("class Outer {{\n  {probe}\n  static final String IDX = \"v\";\n}}\n");
        let (u, g) = api_usages(&[("Other.java", &doc), ("U.java", &use_outer)]);
        if (u, g) != (pairs(&[("v", "read")]), 0) {
            broken.push(probe);
        }
    }
    assert!(broken.is_empty(), "owner lost for {broken:?}");
}
