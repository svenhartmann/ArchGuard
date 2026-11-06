//! PHP call sites and declarations from `scan_php_structure`: names
//! resolved per file as PHP does at compile time.

use archguard_extract::{PhpClassRef, PhpSymbols, PhpTarget, scan_php_structure};

fn symbols(source: &str) -> PhpSymbols {
    scan_php_structure("src/A.php", "App/A.php", source).symbols
}
/// The target of the single site whose code is `code`.
fn target(found: &PhpSymbols, code: &str) -> PhpTarget {
    let sites: Vec<_> = found.sites.iter().filter(|s| s.code == code).collect();
    assert_eq!(sites.len(), 1, "site {code}: {:#?}", found.sites);
    sites[0].target.clone()
}
fn named(name: &str) -> PhpClassRef {
    PhpClassRef::Named(name.into())
}

#[test]
fn class_names_follow_namespace_and_imports() {
    let found = symbols(
        r#"<?php
namespace App\Http;

use App\Domain\User;
use App\Domain\Order as Purchase, Lib\Mail;
use App\Model\{Cart, Item as Line, Sub\Price};
use function App\Support\{format, render as draw};
use const App\LIMIT;

function run() {
    new User(1);
    new Purchase;
    Mail::send();
    new Cart();
    Line::make();
    new Price();
    new Local();
    new \Other\Thing();
    new Mail\Transport();
    new namespace\Kernel();
    Purchase::class;
    LIMIT;
}
"#,
    );
    assert!(found.measured);
    assert_eq!(
        target(&found, "new User(1)"),
        PhpTarget::New(named("App\\Domain\\User"))
    );
    assert_eq!(
        target(&found, "new Purchase"),
        PhpTarget::New(named("App\\Domain\\Order"))
    );
    assert_eq!(
        target(&found, "Mail::send()"),
        PhpTarget::Static(named("Lib\\Mail"), "send".into())
    );
    assert_eq!(
        target(&found, "new Cart()"),
        PhpTarget::New(named("App\\Model\\Cart"))
    );
    assert_eq!(
        target(&found, "Line::make()"),
        PhpTarget::Static(named("App\\Model\\Item"), "make".into())
    );
    assert_eq!(
        target(&found, "new Price()"),
        PhpTarget::New(named("App\\Model\\Sub\\Price"))
    );
    // No import: the current namespace, never a global fallback.
    assert_eq!(
        target(&found, "new Local()"),
        PhpTarget::New(named("App\\Http\\Local"))
    );
    assert_eq!(
        target(&found, "new \\Other\\Thing()"),
        PhpTarget::New(named("Other\\Thing"))
    );
    // An imported name also prefixes qualified names.
    assert_eq!(
        target(&found, "new Mail\\Transport()"),
        PhpTarget::New(named("Lib\\Mail\\Transport"))
    );
    assert_eq!(
        target(&found, "new namespace\\Kernel()"),
        PhpTarget::New(named("App\\Http\\Kernel"))
    );
    // `C::class` and constants are no calls.
    assert_eq!(found.sites.len(), 10, "{:#?}", found.sites);
    let site = found
        .sites
        .iter()
        .find(|s| s.code == "new User(1)")
        .unwrap();
    assert_eq!((site.line, site.column), (11, 5));
    assert_eq!(site.label, "new User");
    assert_eq!(site.from_class, "src/A.php::(module)");
    assert!(site.from_method.as_deref().unwrap().contains("#run@"));
}

#[test]
fn function_names_follow_php_fallback_rules() {
    let found = symbols(
        r#"<?php
namespace App;

use function Lib\helper;
use function Lib\{format as fmt};
use Lib\Tools;

function main() {
    helper();
    fmt(1);
    local();
    \strlen('x');
    Tools\clean();
    namespace\inner();
}
"#,
    );
    let function = |name: &str, fallback: Option<&str>| PhpTarget::Function {
        name: name.into(),
        fallback: fallback.map(Into::into),
    };
    assert_eq!(target(&found, "helper()"), function("Lib\\helper", None));
    assert_eq!(target(&found, "fmt(1)"), function("Lib\\format", None));
    assert_eq!(
        target(&found, "local()"),
        function("App\\local", Some("local"))
    );
    assert_eq!(target(&found, "\\strlen('x')"), function("strlen", None));
    assert_eq!(
        target(&found, "Tools\\clean()"),
        function("Lib\\Tools\\clean", None)
    );
    assert_eq!(
        target(&found, "namespace\\inner()"),
        function("App\\inner", None)
    );
    assert_eq!(found.functions.len(), 1);
    assert_eq!(found.functions[0].name, "App\\main");
    assert_eq!(found.functions[0].class_id, "src/A.php::(module)");
    // Without a namespace an unqualified name is already global.
    let global = symbols("<?php\nfunction a() { b(); }\n");
    assert_eq!(target(&global, "b()"), function("b", None));
}

#[test]
fn this_self_static_and_parent_name_the_enclosing_class() {
    let found = symbols(
        r#"<?php
namespace App;

class Child extends Base\Model implements Port {
    use Logs, \Lib\Timestamps;
    abstract protected function load();

    public function run($other, $name) {
        $this->save();
        $this?->check();
        self::make();
        static::create();
        parent::__construct();
        new static();
        new self;
        $this->repo->find();
        $other->send();
        $this->$name();
        $name();
        Child::$name();
        $other->{"do"}();
        Child::{"go"}();
        $f = function () { return $this->close(); };
        $g = fn() => self::build();
        $list[0]();
        call_user_func([$this, 'save']);
    }
}

trait Logs {
    use Inner { Inner::a insteadof Other; }
    public function log() {}
}
"#,
    );
    let child = &found.classes[0];
    assert_eq!(
        (
            child.name.as_str(),
            child.kind.as_str(),
            child.parent.as_deref()
        ),
        ("App\\Child", "class", Some("App\\Base\\Model"))
    );
    assert_eq!(child.traits, ["App\\Logs", "Lib\\Timestamps"]);
    assert!(!child.trait_rules);
    assert_eq!(child.bodiless, ["load"]);
    let logs = &found.classes[1];
    assert_eq!((logs.kind.as_str(), logs.trait_rules), ("trait", true));
    assert_eq!(logs.traits, ["App\\Inner"]);

    assert_eq!(
        target(&found, "$this->save()"),
        PhpTarget::This("save".into())
    );
    assert_eq!(
        target(&found, "$this?->check()"),
        PhpTarget::This("check".into())
    );
    let current = |m: &str| PhpTarget::Static(PhpClassRef::Current, m.into());
    assert_eq!(target(&found, "self::make()"), current("make"));
    assert_eq!(target(&found, "static::create()"), current("create"));
    assert_eq!(
        target(&found, "parent::__construct()"),
        PhpTarget::Static(PhpClassRef::Parent, "__construct".into())
    );
    assert_eq!(
        target(&found, "new static()"),
        PhpTarget::New(PhpClassRef::Current)
    );
    assert_eq!(
        target(&found, "new self"),
        PhpTarget::New(PhpClassRef::Current)
    );
    for code in [
        "$this->repo->find()",
        "$other->send()",
        "$this->$name()",
        "$name()",
        "Child::$name()",
        "$other->{\"do\"}()",
        "Child::{\"go\"}()",
        "$list[0]()",
    ] {
        assert_eq!(target(&found, code), PhpTarget::Unresolved, "{code}");
    }
    // Closures can be rebound: `$this` and `self` inside stay unknown.
    assert_eq!(target(&found, "$this->close()"), PhpTarget::Unresolved);
    assert_eq!(target(&found, "self::build()"), PhpTarget::Unresolved);
    // `call_user_func` is a plain function call; its callable is not followed.
    assert_eq!(
        target(&found, "call_user_func([$this, 'save'])"),
        PhpTarget::Function {
            name: "App\\call_user_func".into(),
            fallback: Some("call_user_func".into())
        }
    );
}

#[test]
fn declarations_keywords_and_attributes_are_not_calls() {
    let found = symbols(
        r#"<?php
declare(strict_types=1);

#[Route('/x')]
function outer(array $a): void {
    if (isset($a[0]) && !empty($a)) { echo count($a); }
    foreach ($a as $x) { list($p) = $x; }
    $s = (string)($a[1]);
    function nested() { hidden(); }
    $o = new class($a) { public function m() { gone(); } };
    exit(0);
}

top();
"#,
    );
    let codes: Vec<_> = found.sites.iter().map(|s| s.code.as_str()).collect();
    assert_eq!(codes, ["count($a)", "top()"], "{:#?}", found.sites);
    assert_eq!(found.hidden_functions, ["nested"]);
    let top = found.sites.iter().find(|s| s.code == "top()").unwrap();
    assert_eq!(
        (top.from_class.as_str(), top.from_method.as_ref()),
        ("src/A.php::(module)", None)
    );
}

#[test]
fn unbalanced_files_are_not_measured() {
    assert!(!symbols("<?php\nfunction a() { b(\n").measured);
}
