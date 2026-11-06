//! Kotlin call sites and declarations from `scan_kotlin_structure`: what the
//! source says each call targets, before the engine resolves names across
//! the project.

use archguard_extract::kotlin::{KotlinImport, KotlinSymbols, KotlinTarget, scan_kotlin_structure};

fn symbols(source: &str) -> KotlinSymbols {
    scan_kotlin_structure("src/Shop.kt", "com/shop/Shop.kt", source).symbols
}
/// The target of the single site whose code is `code`.
fn target(found: &KotlinSymbols, code: &str) -> KotlinTarget {
    let sites: Vec<_> = found.sites.iter().filter(|s| s.code == code).collect();
    assert_eq!(sites.len(), 1, "site {code}: {:#?}", found.sites);
    sites[0].target.clone()
}
fn name(name: &str) -> KotlinTarget {
    KotlinTarget::Name(name.into())
}
fn path(segments: &[&str]) -> KotlinTarget {
    KotlinTarget::Path(segments.iter().map(|s| s.to_string()).collect())
}

#[test]
fn package_and_imports_keep_aliases_and_stars() {
    let found = symbols(
        "package com.shop.order\n\nimport com.shop.pay.Payment\nimport com.shop.pay.Money as Cash\nimport com.shop.util.*\nimport com.shop.pay.Payment.Status;\n\nfun f() = 1\n",
    );
    assert!(found.measured);
    assert_eq!(found.package, "com.shop.order");
    let import = |path: &str, name: Option<&str>| KotlinImport {
        path: path.into(),
        name: name.map(Into::into),
    };
    assert_eq!(
        found.imports,
        [
            import("com.shop.pay.Payment", Some("Payment")),
            import("com.shop.pay.Money", Some("Cash")),
            import("com.shop.util", None),
            import("com.shop.pay.Payment.Status", Some("Status")),
        ]
    );
}

#[test]
fn declarations_record_what_member_lookup_needs() {
    let found = symbols(
        r#"package com.shop

abstract class Repo(val name: String, private var size: Int, plain: Int) : Base(name), Port {
    abstract fun load(): Int
    val cache = mutableMapOf<String, Int>()
    fun String.tagged() = this + name
    fun save() { load() }
    companion object {
        fun create() = 1
    }
    data class Row(val id: Int)
    object Empty
}

interface Port { fun send() { } fun close() }

fun top() = 1
fun Repo.count() = 2
val VERSION = 3
typealias Rows = List<Repo.Row>
"#,
    );
    let class = |name: &str| {
        found
            .classes
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no class {name}: {:#?}", found.classes))
    };
    let repo = class("Repo");
    assert_eq!(repo.id, "src/Shop.kt::Repo");
    assert_eq!(repo.kind, "class");
    assert_eq!(repo.outer, None);
    assert_eq!(repo.supertypes, ["Base", "Port"]);
    assert_eq!(repo.modifiers, ["abstract"]);
    assert_eq!(repo.bodiless, ["load"]);
    // Constructor properties, body properties and member extensions.
    assert_eq!(repo.values, ["name", "size", "cache", "tagged"]);
    let companion = class("Companion");
    assert!(companion.companion);
    assert_eq!(companion.outer.as_deref(), Some("src/Shop.kt::Repo"));
    assert_eq!(class("Row").modifiers, ["data"]);
    assert_eq!(class("Row").values, ["id"]);
    assert_eq!(class("Empty").kind, "object");
    assert_eq!(class("Port").bodiless, ["close"]);
    let functions: Vec<_> = found
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.extension))
        .collect();
    assert_eq!(functions, [("top", false), ("count", true)]);
    assert_eq!(found.values, ["VERSION", "Rows"]);
}

const CALLS: &str = r#"package com.shop

class Cart(private val repo: Repo) : Base(), Port {
    fun total(items: List<Item>, flag: Boolean): Int {
        val local = Item(1)
        Price.of(2)
        com.shop.money.Money.zero()
        Money.Companion.zero()
        this.check()
        super.save()
        super<Port>.send()
        check()
        listOf<Int>(3)
        repo.find()
        local.price()
        items.size()
        load().first()
        items?.first()
        if (flag) { check() } else { audit() }
        when (flag) {
            true -> { check() }
            else -> audit()
        }
        items.forEach { check() }
        run { audit() }
        build { }
        this@Cart.check()
        @Suppress("x") val y = 1
        val o = object : Runnable { override fun run() { hidden() } }
        val s = "${template()}"
        fun helper() = audit()
        helper()
        return 0
    }
    fun check() = audit()
    fun audit() {}
    fun String.shout() = check()
}
"#;

#[test]
fn call_shapes_follow_the_source() {
    let found = symbols(CALLS);
    assert_eq!(target(&found, "Item(1)"), name("Item"));
    assert_eq!(target(&found, "Price.of(2)"), path(&["Price", "of"]));
    assert_eq!(
        target(&found, "com.shop.money.Money.zero()"),
        path(&["com", "shop", "money", "Money", "zero"])
    );
    assert_eq!(
        target(&found, "Money.Companion.zero()"),
        path(&["Money", "Companion", "zero"])
    );
    assert_eq!(
        target(&found, "this.check()"),
        KotlinTarget::This("check".into())
    );
    assert_eq!(
        target(&found, "super.save()"),
        KotlinTarget::Super {
            qualifier: None,
            name: "save".into()
        }
    );
    assert_eq!(
        target(&found, "super<Port>.send()"),
        KotlinTarget::Super {
            qualifier: Some("Port".into()),
            name: "send".into()
        }
    );
    assert_eq!(target(&found, "listOf<Int>(3)"), name("listOf"));
    // Braced control bodies and `when` branches are no lambdas.
    let checks: Vec<_> = found
        .sites
        .iter()
        .filter(|s| s.code == "check()")
        .map(|s| s.target.clone())
        .collect();
    assert_eq!(
        checks,
        [
            name("check"),
            name("check"),
            name("check"),
            // Inside `forEach { … }`: the lambda may have a receiver.
            KotlinTarget::Unresolved,
            // `fun String.shout()`: `this` is a String, not a Cart.
            KotlinTarget::Unresolved,
        ]
    );
    let site = found
        .sites
        .iter()
        .find(|s| s.code == "Price.of(2)")
        .unwrap();
    assert_eq!(site.from_class, "src/Shop.kt::Cart");
    assert!(
        site.from_method
            .as_deref()
            .unwrap()
            .starts_with("src/Shop.kt::Cart#total@")
    );
    assert_eq!((site.line, site.column), (6, 9));
    assert_eq!(site.label, "Price.of");
}

#[test]
fn unknown_receivers_lambdas_and_locals_stay_unresolved() {
    let found = symbols(CALLS);
    for code in [
        // A local and a parameter as receivers.
        "local.price()",
        "items.size()",
        // A call result and a safe call.
        "load().first()",
        "items?.first()",
        "items.forEach { check() }",
        // Scope functions and every call inside their lambdas.
        "run { audit() }",
        "this@Cart.check()",
        // A local function: its body and its calls.
        "helper()",
    ] {
        assert_eq!(target(&found, code), KotlinTarget::Unresolved, "{code}");
    }
    assert_eq!(target(&found, "load()"), name("load"));
    // A property is no local: the engine knows the class's properties.
    assert_eq!(target(&found, "repo.find()"), path(&["repo", "find"]));
    // Trailing lambda without parentheses.
    assert_eq!(target(&found, "build { }"), name("build"));
    let audits: Vec<_> = found
        .sites
        .iter()
        .filter(|s| s.code == "audit()")
        .map(|s| s.target.clone())
        .collect();
    // In `else { … }`, a `when` branch, `run { … }`, the local function
    // `helper` and `check`'s expression body.
    assert_eq!(
        audits,
        [
            name("audit"),
            name("audit"),
            KotlinTarget::Unresolved,
            KotlinTarget::Unresolved,
            name("audit"),
        ]
    );
    // Annotations, object expressions and string templates are no sites.
    for code in ["Suppress(\"x\")", "hidden()", "template()", "Runnable"] {
        assert!(
            found.sites.iter().all(|s| !s.code.contains(code)),
            "{code}: {:#?}",
            found.sites
        );
    }
}

#[test]
fn unbalanced_files_are_not_measured() {
    let found = symbols("fun broken() { if (\n");
    assert!(!found.measured);
    assert!(found.sites.is_empty());
}
