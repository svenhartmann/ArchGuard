//! Scala call sites from `scan_scala_structure`: what the scanner records
//! for the engine's resolver, with names as written.

use archguard_extract::scala::{ScalaSymbols, ScalaTarget, scan_scala_structure};

fn symbols(source: &str) -> ScalaSymbols {
    scan_scala_structure("src/Shop.scala", "com/shop/Shop.scala", source).symbols
}

fn targets(found: &ScalaSymbols) -> Vec<(String, ScalaTarget)> {
    found
        .sites
        .iter()
        .map(|s| (s.label.clone(), s.target.clone()))
        .collect()
}

fn path(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn call_shapes_map_to_targets() {
    let found = symbols(
        r#"package com.shop

class Shop(repo: Repo) extends Base {
  def run(items: List[Int]): Int = {
    val order = new Order(1)
    val empty = new com.shop.model.Cart
    val made = Order.create(2)
    helper(items)
    Box[Int](3)
    this.helper(items)
    super.close()
    super[Base].close()
    items.map(x => x + 1).filter(_ > 0)
    repo.load(1)
    items match {
      case Some(x) => total(x)
      case _ => 0
    }
    items.size
  }
  @deprecated("x") def helper(xs: List[Int]): Int = xs.length
}
"#,
    );
    assert!(found.measured);
    let run: Vec<_> = targets(&found)
        .into_iter()
        .filter(|(label, _)| label != "xs.length")
        .collect();
    assert_eq!(
        run,
        vec![
            ("new Order".into(), ScalaTarget::New(path(&["Order"]))),
            (
                "new com.shop.model.Cart".into(),
                ScalaTarget::New(path(&["com", "shop", "model", "Cart"]))
            ),
            (
                "Order.create".into(),
                ScalaTarget::Path(path(&["Order", "create"]))
            ),
            ("helper".into(), ScalaTarget::Name("helper".into())),
            ("Box".into(), ScalaTarget::Name("Box".into())),
            ("this.helper".into(), ScalaTarget::This("helper".into())),
            (
                "super.close".into(),
                ScalaTarget::Super {
                    qualifier: None,
                    name: "close".into()
                }
            ),
            (
                "super[Base].close".into(),
                ScalaTarget::Super {
                    qualifier: Some("Base".into()),
                    name: "close".into()
                }
            ),
            (
                "items.map".into(),
                ScalaTarget::Path(path(&["items", "map"]))
            ),
            ("filter".into(), ScalaTarget::Unresolved),
            (
                "repo.load".into(),
                ScalaTarget::Path(path(&["repo", "load"]))
            ),
            // `Some(x)` in a case pattern is an extractor, not a call.
            ("total".into(), ScalaTarget::Name("total".into())),
        ]
    );
    let order = &found.sites[0];
    assert_eq!((order.line, order.column), (5, 17));
    assert_eq!(order.code, "new Order(1)");
    assert!(
        order
            .from_method
            .as_deref()
            .unwrap()
            .starts_with("src/Shop.scala::Shop#run@4")
    );
}

#[test]
fn calls_in_extension_blocks_stay_unresolved() {
    let found = symbols(
        "extension (s: String)\n  def shout: String = upper(s) + helper(s)\n\ndef plain() = helper(1)\n",
    );
    let found = targets(&found);
    assert_eq!(
        found,
        vec![
            ("upper".into(), ScalaTarget::Unresolved),
            ("helper".into(), ScalaTarget::Unresolved),
            ("helper".into(), ScalaTarget::Name("helper".into())),
        ]
    );
}

#[test]
fn imports_and_classes_reach_the_resolver() {
    let found = symbols(
        "package com.shop\nimport com.shop.model.{Order => O}\nobject Main:\n  def run() = O(1)\n",
    );
    assert_eq!(found.package, "com.shop");
    assert_eq!(found.imports[0].path, "com.shop.model.Order");
    assert_eq!(found.imports[0].name.as_deref(), Some("O"));
    assert_eq!(found.classes[0].kind, "object");
    assert_eq!(found.sites[0].target, ScalaTarget::Name("O".into()));
}
