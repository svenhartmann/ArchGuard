//! Static Kotlin call resolution: one rule per test, over small tempdir
//! projects. Combined reports prefix ids with `kotlin:`.

use std::fs;

use archguard_core::{CallAnalysis, Config, MethodCall};
use archguard_engine::analyze_calls;

const MODEL: &str = r#"package com.shop.model

abstract class Base {
    open fun save() = 1
    abstract fun id(): Int
    fun use() = this.id()
    companion object {
        fun registry() = 0
    }
}

interface Audited {
    fun audit() = 2
    fun tag(): String
}

open class Order : Base(), Audited {
    override fun id() = 1
    override fun tag() = "o"
    fun total() = 3
    companion object {
        fun create() = Order()
    }
    class Line(val qty: Int) { fun make() = create() }
    object Codes { fun next() = 4 }
}

data class Money(val cents: Int) {
    fun twice() = copy(cents * 2)
}

object Registry {
    fun register(order: Order) = order.total()
}

fun format(order: Order) = "x"
fun Order.discount() = 5
"#;

const CHECKOUT: &str = r#"package com.shop.app

import com.shop.model.Order
import com.shop.model.Money as Cash
import com.shop.model.Registry
import com.shop.model.Registry.register
import com.shop.model.format
import com.shop.model.Audited
import com.shop.util.*
import com.shop.other.*
import org.lib.Mailer

class Checkout(private val repo: Repo) : Order(), Audited {
    val handler: () -> Unit = {}
    fun run(items: List<Int>) {
        Order()
        Cash(1)
        com.shop.model.Money(2)
        Order.create()
        Order.Companion.create()
        Order.Line(1)
        Order.Codes.next()
        Registry.register(this)
        Registry()
        register(this)
        format(this)
        slug("a")
        Clock()
        Helper()
        this.total()
        total()
        save()
        audit()
        id()
        super.save()
        super<Audited>.audit()
        registry()
        discount()
        repo.find()
        handler()
        items.forEach { total() }
        println("x")
        listOf(1)
        Mailer()
        Mailer.send()
        Math.max(1, 2)
        Legacy()
    }
}
"#;

const PAGE: &str = r#"package com.shop.app

import org.framework.Controller

class Page : Controller() {
    fun show() {
        render()
        own()
        Helper()
    }
    fun own() = 1
}
"#;

fn shop() -> Vec<(&'static str, &'static str)> {
    vec![
        ("src/main/kotlin/com/shop/model/Order.kt", MODEL),
        ("src/main/kotlin/com/shop/app/Checkout.kt", CHECKOUT),
        ("src/main/kotlin/com/shop/app/Page.kt", PAGE),
        (
            "src/main/kotlin/com/shop/app/Helper.kt",
            "package com.shop.app\n\nclass Helper\n",
        ),
        (
            "src/main/kotlin/com/shop/util/Text.kt",
            "package com.shop.util\n\nfun slug(s: String) = s\nclass Clock\n",
        ),
        (
            "src/main/kotlin/com/shop/other/Clock.kt",
            "package com.shop.other\n\nclass Clock\n",
        ),
        (
            "src/main/java/com/shop/app/Legacy.java",
            "package com.shop.app;\n\npublic class Legacy {}\n",
        ),
        (
            "src/main/kotlin/com/shop/dup/A.kt",
            "package com.shop.dup\n\nclass Twice\nfun twice() = 1\n",
        ),
        (
            "src/main/kotlin/com/shop/dup/B.kt",
            "package com.shop.dup\n\nclass Twice\nfun twice() = 2\nfun use() { Twice(); twice() }\n",
        ),
    ]
}

fn analyze(files: &[(&str, &str)]) -> CallAnalysis {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    analyze_calls(dir.path(), &Config::default())
}

/// The single call whose source text is `code`.
fn call<'a>(report: &'a CallAnalysis, code: &str) -> &'a MethodCall {
    let found: Vec<_> = report.calls.iter().filter(|c| c.code == code).collect();
    assert_eq!(found.len(), 1, "call {code}: {:#?}", report.calls);
    found[0]
}

/// (resolution, dispatch, target class id after the file (or external
/// name), target method name) of `code`.
fn target(report: &CallAnalysis, code: &str) -> (String, String, Option<String>, Option<String>) {
    let call = call(report, code);
    let class = call
        .to_class
        .as_ref()
        .map(|id| match id.split_once(".kt::") {
            Some((_, class)) => class.to_string(),
            // Combined reports prefix external names with the language too.
            None => id.trim_start_matches("kotlin:").to_string(),
        });
    let method = call.to_method.as_ref().map(|id| {
        report
            .methods
            .iter()
            .find(|m| &m.id == id)
            .map_or_else(|| id.clone(), |m| m.name.clone())
    });
    (
        call.resolution.clone(),
        call.dispatch.clone(),
        class,
        method,
    )
}

fn project(
    dispatch: &str,
    class: &str,
    method: Option<&str>,
) -> (String, String, Option<String>, Option<String>) {
    (
        "project".into(),
        dispatch.into(),
        Some(class.into()),
        method.map(Into::into),
    )
}

fn unresolved() -> (String, String, Option<String>, Option<String>) {
    ("unresolved".into(), "unknown".into(), None, None)
}

fn external(class: Option<&str>) -> (String, String, Option<String>, Option<String>) {
    (
        "external".into(),
        "unknown".into(),
        class.map(Into::into),
        None,
    )
}

#[test]
fn constructors_resolve_through_package_imports_aliases_and_qualified_names() {
    let report = analyze(&shop());
    // In Checkout, and inside Order's companion, where it names the outer
    // class.
    let orders: Vec<_> = report
        .calls
        .iter()
        .filter(|c| c.code == "Order()")
        .map(|c| {
            (
                c.resolution.as_str(),
                c.dispatch.as_str(),
                c.to_class.as_deref(),
            )
        })
        .collect();
    assert_eq!(orders.len(), 2, "{orders:?}");
    assert!(
        orders
            .iter()
            .all(|(resolution, dispatch, class)| *resolution == "project"
                && *dispatch == "constructor"
                && class.is_some_and(|c| c.ends_with("model/Order.kt::Order"))),
        "{orders:?}"
    );
    // `import com.shop.model.Money as Cash`.
    assert_eq!(
        target(&report, "Cash(1)"),
        project("constructor", "Money", None)
    );
    assert_eq!(
        target(&report, "com.shop.model.Money(2)"),
        project("constructor", "Money", None)
    );
    assert_eq!(
        target(&report, "Order.Line(1)"),
        project("constructor", "Order.Line", None)
    );
    // Same package, another file: see the Page test for `Helper()`.
}

#[test]
fn star_imports_resolve_only_a_single_candidate() {
    let report = analyze(&shop());
    // Only `com.shop.util` declares `slug`.
    assert_eq!(
        target(&report, "slug(\"a\")"),
        project("static", "(module)", Some("slug"))
    );
    // `com.shop.util.Clock` and `com.shop.other.Clock`.
    assert_eq!(target(&report, "Clock()"), unresolved());
}

#[test]
fn companions_objects_and_top_level_functions_are_static_targets() {
    let report = analyze(&shop());
    for code in ["Order.create()", "Order.Companion.create()"] {
        assert_eq!(
            target(&report, code),
            project("static", "Order.Companion", Some("create")),
            "{code}"
        );
    }
    assert_eq!(
        target(&report, "Order.Codes.next()"),
        project("static", "Order.Codes", Some("next"))
    );
    assert_eq!(
        target(&report, "Registry.register(this)"),
        project("static", "Registry", Some("register"))
    );
    // `import com.shop.model.Registry.register`: an object member.
    assert_eq!(
        target(&report, "register(this)"),
        project("static", "Registry", Some("register"))
    );
    // An object cannot be constructed: `Registry()` would be `invoke`.
    assert_eq!(target(&report, "Registry()"), unresolved());
    assert_eq!(
        target(&report, "format(this)"),
        project("static", "(module)", Some("format"))
    );
    // A companion of a supertype is in scope unqualified.
    assert_eq!(
        target(&report, "registry()"),
        project("static", "Base.Companion", Some("registry"))
    );
}

#[test]
fn members_follow_the_class_hierarchy_and_super_calls() {
    let report = analyze(&shop());
    assert_eq!(
        target(&report, "this.total()"),
        project("instance", "Order", Some("total"))
    );
    assert_eq!(
        target(&report, "save()"),
        project("instance", "Base", Some("save"))
    );
    // An interface default method, reached through two paths.
    assert_eq!(
        target(&report, "audit()"),
        project("instance", "Audited", Some("audit"))
    );
    // The override in Order, not the abstract declaration in Base.
    assert_eq!(
        target(&report, "id()"),
        project("instance", "Order", Some("id"))
    );
    assert_eq!(
        target(&report, "super.save()"),
        project("instance", "Base", Some("save"))
    );
    assert_eq!(
        target(&report, "super<Audited>.audit()"),
        project("instance", "Audited", Some("audit"))
    );
    // Abstract here: the implementation is not known statically.
    assert_eq!(target(&report, "this.id()"), unresolved());
    // A nested class sees the companion of its outer class.
    assert_eq!(
        target(&report, "create()"),
        project("static", "Order.Companion", Some("create"))
    );
    let edge = call(&report, "this.total()");
    assert!(
        edge.from_class.ends_with("app/Checkout.kt::Checkout"),
        "{edge:?}"
    );
    assert_eq!(edge.target_label, "this.total");
}

#[test]
fn unknown_receivers_extensions_and_lambdas_stay_unresolved() {
    let report = analyze(&shop());
    for code in [
        // An extension function: its receiver type is not checked.
        "discount()",
        // A constructor property and a function-typed property.
        "repo.find()",
        "handler()",
        "items.forEach { total() }",
    ] {
        assert_eq!(target(&report, code), unresolved(), "{code}");
    }
    // Inside the lambda: `forEach` might pass a receiver.
    let in_lambda: Vec<_> = report
        .calls
        .iter()
        .filter(|c| c.code == "total()")
        .map(|c| c.resolution.as_str())
        .collect();
    assert_eq!(in_lambda, ["project", "unresolved"]);
    // A data class's generated `copy`.
    assert_eq!(target(&report, "copy(cents * 2)"), unresolved());
    // Declared twice in the project.
    assert_eq!(target(&report, "Twice()"), unresolved());
    assert_eq!(target(&report, "twice()"), unresolved());
}

#[test]
fn an_unknown_supertype_hides_everything_but_own_members() {
    let report = analyze(&shop());
    assert_eq!(
        target(&report, "own()"),
        project("instance", "Page", Some("own"))
    );
    // `render` may be inherited from the framework class; its nested
    // classes would shadow `Helper` too.
    assert_eq!(target(&report, "render()"), unresolved());
    let helpers: Vec<_> = report
        .calls
        .iter()
        .filter(|c| c.code == "Helper()")
        .map(|c| {
            (
                c.from_class.rsplit("::").next().unwrap(),
                c.resolution.as_str(),
            )
        })
        .collect();
    assert_eq!(helpers, [("Checkout", "project"), ("Page", "unresolved")]);
}

#[test]
fn names_declared_nowhere_in_the_project_are_external() {
    let report = analyze(&shop());
    assert_eq!(target(&report, "println(\"x\")"), external(None));
    assert_eq!(target(&report, "listOf(1)"), external(None));
    assert_eq!(target(&report, "Mailer()"), external(None));
    assert_eq!(
        target(&report, "Mailer.send()"),
        external(Some("org.lib.Mailer"))
    );
    assert_eq!(target(&report, "Math.max(1, 2)"), external(Some("Math")));
    // A Java class of the same package: declared, but not resolved here.
    assert_eq!(target(&report, "Legacy()"), unresolved());
    let analyzer = report
        .analyzers
        .iter()
        .find(|a| a.language == "kotlin")
        .unwrap();
    assert!(
        analyzer.diagnostics[0].starts_with("Calls: "),
        "{:?}",
        analyzer.diagnostics
    );
}

#[test]
fn an_unscanned_file_turns_external_into_unresolved() {
    let report = analyze(&[
        ("src/a.kt", "fun a() {\n    println(\"x\")\n    b()\n}\n"),
        ("src/broken.kt", "fun b() { if (\n"),
    ]);
    assert_eq!(target(&report, "b()"), unresolved());
    assert_eq!(target(&report, "println(\"x\")"), unresolved());
}
