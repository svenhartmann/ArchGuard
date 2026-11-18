use archguard_core::{CallMethod, SourceMethodMetrics};

use super::structure::*;

fn scan(source: &str) -> SwiftStructure {
    scan_swift_structure(
        "Sources/Shop/Shop.swift",
        "Sources/Shop/Shop.swift",
        "Shop",
        source,
    )
}
fn method<'a>(found: &'a SwiftStructure, name: &str) -> &'a CallMethod {
    found
        .methods
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("no function {name}: {:?}", found.diagnostics))
}
fn metrics<'a>(found: &'a SwiftStructure, name: &str) -> &'a SourceMethodMetrics {
    let id = &method(found, name).id;
    found.metrics.iter().find(|m| &m.method_id == id).unwrap()
}
/// (cyclomatic, max nesting, cognitive) of one function.
fn complexity(found: &SwiftStructure, name: &str) -> (u32, u32, u32) {
    let measured = method(found, name).complexity.as_ref().unwrap();
    (
        measured.cyclomatic,
        measured.max_nesting,
        metrics(found, name).cognitive,
    )
}

#[test]
fn counts_follow_the_shared_rules() {
    let found = scan(
        r#"import Foundation

func plain() -> Int { 1 }
func branches(a: Bool, b: Bool) -> Int {
    if a {
        if b { return 1 } else { return 2 }
    } else if b {
        return 3
    } else {
        return 4
    }
}
func logic(a: Bool, b: Bool, c: Bool) -> Bool { a && b || c }
func guarded(x: Int?) -> Int {
    guard let x = x else { return 0 }
    if let y = Optional(x), y > 1 { return y }
    return x
}
func arms(x: Int) -> String {
    switch x {
    case 0, 1: return "low"
    case 2:
        if case 3 = x { return "never" }
        return "mid"
    default: return "high"
    }
}
func loops(items: [Int]) -> Int {
    var n = 0
    do {
        for item in items {
            while n < item { n += 1 }
        }
    } catch {
        return -1
    }
    repeat { n -= 1 } while n > 0
    return n
}
func nulls(a: String?) -> Int { a?.count ?? 0 }
func ternary(a: Int) -> Int { a > 0 ? a : -a }
func closures(xs: [Int]) -> [Int] {
    xs.map { x in x > 0 ? x : 0 }.filter { $0 > 1 && $0 < 9 }
}
func labeled(rows: [[Int]]) {
    outer: for row in rows {
        for cell in row {
            if cell < 0 { break outer }
            if cell == 0 { continue }
        }
    }
}
func local(a: Int) -> Int {
    func helper(b: Int) -> Int { if b > 0 { return b }; return 0 }
    struct Box { func open() { if true {} } }
    #if DEBUG
    print(a)
    #else
    print(-a)
    #endif
    return helper(b: a)
}
"#,
    );
    assert_eq!(complexity(&found, "plain"), (1, 0, 0));
    // Same shape and numbers as the Kotlin, Rust, Python and PHP `branches` cases.
    assert_eq!(complexity(&found, "branches"), (4, 2, 6));
    assert_eq!(complexity(&found, "logic"), (3, 0, 2));
    // `guard` and `if let` are decisions; the guard's `else` adds no
    // cognitive load, and the comma of a condition list is no decision.
    assert_eq!(complexity(&found, "guarded"), (3, 1, 2));
    // Each `case` label except `default`; `case 0, 1` is one label and
    // `if case` is an `if`, not a label.
    assert_eq!(complexity(&found, "arms"), (4, 2, 3));
    // `do` does not nest or count, `catch` does; `repeat … while` is one loop.
    assert_eq!(complexity(&found, "loops"), (5, 2, 5));
    // `??` is a decision without cognitive load; `?.` is not counted.
    assert_eq!(complexity(&found, "nulls"), (2, 0, 0));
    assert_eq!(complexity(&found, "ternary"), (2, 1, 1));
    // Closures are one level deeper.
    assert_eq!(complexity(&found, "closures"), (3, 2, 3));
    // A jump to a label adds cognitive load; a plain `continue` does not.
    assert_eq!(complexity(&found, "labeled"), (5, 3, 10));
    // A local function nests and counts toward its enclosing function; a
    // local type is not measured; `#if` is no decision.
    assert_eq!(complexity(&found, "local"), (2, 2, 2));
    assert!(
        found
            .methods
            .iter()
            .all(|m| m.name != "helper" && m.name != "open")
    );
    // `1`: one operand and no operator.
    let plain = metrics(&found, "plain");
    assert_eq!(
        (plain.operators, plain.operands, plain.source_lines),
        (0, 1, 1)
    );
    assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
}

#[test]
fn strings_interpolations_and_comments_hide_their_contents() {
    let found = scan(
        r##"func templates(name: String, n: Int) -> String {
    let raw = #"if x { "\(y)" } && || switch"#
    // if a && b { }
    /* switch { /* nested */ case } */
    let multi = """
        if a { } "quoted"
        """
    return "Hello \(n > 1 ? "all" : name) \(name.count) {"
}
func after(a: Bool) -> Int { if a { return 1 } else { return 2 } }
"##,
    );
    assert_eq!(complexity(&found, "templates"), (1, 0, 0));
    assert_eq!(complexity(&found, "after"), (2, 1, 2));
    assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
}

#[test]
fn types_extensions_and_functions_come_from_declarations() {
    let found = scan(
        r#"import Core

protocol Port: AnyObject { func send(_ msg: String) }

public final class Repo<T: Equatable>: Base, Port where T: Hashable {
    @objc private static func create() -> Repo<Int>? { nil }
    class func make() -> Repo { Repo() }
    init?(name: String) { if name.isEmpty { return nil } }
    convenience init() { self.init(name: "x")! }
    deinit { print("bye") }
    subscript(index: Int) -> T? { nil }
    var size: Int { get { 0 } }
    lazy var cache: [Int] = { [1, 2].map { $0 } }()
    func send(_ msg: String) { print(msg) }
    struct Cursor { mutating func next() -> Int { 1 } }
    enum Mode: String { case a, b; func label() -> String { rawValue } }
}

actor Store { func load() async throws -> Int { 1 } }

extension Repo: CustomStringConvertible {
    var description: String { "repo" }
    func describe() -> String { description }
}

extension Repo.Cursor { func reset() {} }

func ==(lhs: Store, rhs: Store) -> Bool { true }
func `default`() {}
"#,
    );
    let class = |id: &str| {
        found
            .classes
            .iter()
            .find(|c| c.id == format!("Sources/Shop/Shop.swift::{id}"))
            .unwrap_or_else(|| panic!("no class {id}: {:?}", found.classes))
    };
    let kinds: Vec<_> = [
        "Port",
        "Repo",
        "Repo.Cursor",
        "Repo.Mode",
        "Store",
        "extension Repo@21",
        "extension Repo.Cursor@26",
        "(module)",
    ]
    .iter()
    .map(|id| (class(id).kind.as_str(), class(id).is_abstract))
    .collect();
    assert_eq!(
        kinds,
        [
            ("interface", Some(true)),
            ("class", Some(false)),
            ("struct", Some(false)),
            ("enum", Some(false)),
            ("actor", Some(false)),
            ("extension", Some(false)),
            ("extension", Some(false)),
            ("module", Some(false)),
        ]
    );
    assert_eq!(found.classes.len(), 8, "{:?}", found.classes);
    let repo = class("Repo");
    assert_eq!(repo.supertypes, ["Base", "Port"]);
    assert_eq!(repo.package, "Shop");
    assert_eq!(repo.language.as_deref(), Some("swift"));
    assert_eq!(repo.line, 5);
    assert_eq!(class("Repo.Mode").supertypes, ["String"]);
    assert_eq!(class("extension Repo@21").name, "Repo");
    assert_eq!(
        class("extension Repo@21").supertypes,
        ["CustomStringConvertible"]
    );
    // Protocol requirements, accessors and property initializers are not
    // recorded; initializers, deinitializers and subscripts are.
    let names: Vec<_> = found.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "create",
            "make",
            "init",
            "init",
            "deinit",
            "subscript",
            "send",
            "next",
            "label",
            "load",
            "describe",
            "reset",
            "==",
            "default",
        ]
    );
    let create = method(&found, "create");
    assert!(create.is_static);
    assert_eq!(create.class_id, repo.id);
    assert_eq!(create.signature, "func create() -> Repo<Int>?");
    assert!(method(&found, "make").is_static);
    assert!(!method(&found, "send").is_static);
    assert_eq!(complexity(&found, "init"), (2, 1, 1));
    assert_eq!(method(&found, "next").class_id, class("Repo.Cursor").id);
    assert_eq!(
        method(&found, "describe").class_id,
        class("extension Repo@21").id
    );
    let module = method(&found, "==");
    assert_eq!(module.class_id, "Sources/Shop/Shop.swift::(module)");
    assert!(module.is_static);
}

#[test]
fn extensions_attach_to_their_unique_project_type() {
    let base = scan("class Repo: Base {}\nstruct Repo2 {}\n");
    let other = scan_swift_structure(
        "Sources/Shop/Repo+Ext.swift",
        "Sources/Shop/Repo+Ext.swift",
        "Shop",
        "extension Repo: Codable { func save() {} }\nextension String { func shout() -> String { self } }\n",
    );
    let mut classes: Vec<_> = base.classes.into_iter().chain(other.classes).collect();
    let mut methods = other.methods;
    attach_extensions(&mut classes, &mut methods);
    let ids: Vec<_> = classes.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "Sources/Shop/Shop.swift::Repo",
            "Sources/Shop/Shop.swift::Repo2",
            "Sources/Shop/Repo+Ext.swift::extension String@2",
        ]
    );
    assert_eq!(classes[0].supertypes, ["Base", "Codable"]);
    assert_eq!(methods[0].class_id, "Sources/Shop/Shop.swift::Repo");
    assert_eq!(
        methods[1].class_id,
        "Sources/Shop/Repo+Ext.swift::extension String@2"
    );
}

#[test]
fn call_sites_name_types_and_static_members() {
    let found = scan(
        r#"struct Order {
    func total() -> Int {
        let a = Price(cents: 1)
        let b = Price.zero()
        let c = Price.init(cents: 2)
        let d = Money.Currency(code: "EUR")
        helper()
        self.reset()
        a.add(b)
        return list.first?.count() ?? 0
    }
}
"#,
    );
    let sites: Vec<_> = found
        .sites
        .iter()
        .map(|s| (s.label.as_str(), s.target.clone()))
        .collect();
    let stat = |owner: &str, name: &str| SwiftTarget::Static {
        owner: owner.into(),
        name: name.into(),
    };
    assert_eq!(
        sites,
        [
            ("Price", SwiftTarget::Init("Price".into())),
            ("Price.zero", stat("Price", "zero")),
            ("Price.init", SwiftTarget::Init("Price".into())),
            ("Money.Currency", stat("Money", "Currency")),
            ("helper", SwiftTarget::Unresolved),
            ("reset", SwiftTarget::Unresolved),
            ("add", SwiftTarget::Unresolved),
            ("count", SwiftTarget::Unresolved),
        ]
    );
    let first = &found.sites[0];
    assert_eq!((first.line, first.column), (3, 17));
    assert_eq!(first.code, "Price(cents: 1)");
    assert_eq!(first.from_method, method(&found, "total").id);
}

#[test]
fn unbalanced_sources_are_reported_not_measured() {
    let found = scan("func broken() { if (\n");
    assert!(found.methods.is_empty());
    assert_eq!(
        found.diagnostics,
        ["Sources/Shop/Shop.swift: unbalanced brackets; not measured"]
    );
}
