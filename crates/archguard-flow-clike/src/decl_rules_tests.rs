//! The declaration rules per dialect, as a table: (dialect, file, snippet) -> declarations.
//! `class N[ interface][:Bases][ @Attrs]`, `fn n[ pub][ @Attrs]`, in source order.
use crate::decl::{Decl, Dialect, decls};
use crate::source::Source;

fn show(d: &Decl) -> String {
    let attrs: Vec<&str> = d.attrs.iter().map(|a| a.name.as_str()).collect();
    let attrs = if attrs.is_empty() {
        String::new()
    } else {
        format!(" @{}", attrs.join(","))
    };
    if d.class {
        let interface = if d.flag { " interface" } else { "" };
        let bases = if d.bases.is_empty() {
            String::new()
        } else {
            format!(":{}", d.bases.join(","))
        };
        format!("class {}{interface}{bases}{attrs}", d.name)
    } else {
        let public = if d.flag { " pub" } else { "" };
        format!("fn {}{public}{attrs}", d.name)
    }
}

fn run(dialect: Dialect, file: &str, snippet: &str) -> Vec<String> {
    let src = Source::new(file.into(), snippet);
    decls(&src, dialect).iter().map(show).collect()
}

const CASES: &[(Dialect, &str, &str, &[&str])] = &[
    // Wave 6: a header that goes on below the annotation.
    (
        Dialect::Java,
        "C.java",
        "class C {\n  @PreAuthorize(\"x\") final\n  String run() { return \"\"; }\n}\n",
        &["class C", "fn run @PreAuthorize"],
    ),
    (
        Dialect::Java,
        "C.java",
        "class C {\n  @GetMapping(\"/a\") public\n  String a() { return \"\"; }\n}\n",
        &["class C", "fn a @GetMapping"],
    ),
    (
        Dialect::Java,
        "C.java",
        "class C {\n  @GetMapping(\"/a\")\n  public\n  String a() { return \"\"; }\n}\n",
        &["class C", "fn a @GetMapping"],
    ),
    (
        Dialect::Java,
        "C.java",
        "class Ctl {\n  @GetMapping(\"/a\")\n  @PreAuthorize(\"hasRole('B')\")\n  String\n  a() { return \"\"; }\n  @GetMapping(\"/b\")\n  Map<String, List<X>>\n  b() { return null; }\n}\n",
        &[
            "class Ctl",
            "fn a @GetMapping,PreAuthorize",
            "fn b @GetMapping",
        ],
    ),
    // Kotlin: a property's annotations stop at the property; `object` is a type.
    (
        Dialect::Kotlin,
        "C.kt",
        "class C {\n  @get:PermitAll\n  val x: String = \"\"\n  @Get(\"/b\") fun b(): String = \"\"\n}\n",
        &["class C", "fn b @Get"],
    ),
    (
        Dialect::Kotlin,
        "C.kt",
        "class C {\n  @Inject lateinit var s: S\n  @Get(\"/b\") fun b(): String = \"\"\n}\n",
        &["class C", "fn b @Get"],
    ),
    (
        Dialect::Kotlin,
        "R.kt",
        "@RestController\nobject Routes : Base() {\n  @Get(\"/a\") fun a(): String = \"\"\n}\n",
        &["class Routes:Base @RestController", "fn a @Get"],
    ),
    // Java: `object` is no type keyword; bases come from extends and implements.
    (
        Dialect::Java,
        "A.java",
        "public class A extends Base implements IFoo, Bar<T> {\n}\n",
        &["class A:Base,IFoo,Bar"],
    ),
    (
        Dialect::Java,
        "I.java",
        "public interface I {\n}\n",
        &["class I interface"],
    ),
    (
        Dialect::Java,
        "A.java",
        "class A {\n  public int run(int a) { return a; }\n}\n",
        &["class A"],
    ),
    // C#: attribute lists, bases, bare public methods only in .cs files.
    (
        Dialect::CSharp,
        "C.cs",
        "[ApiController]\n[Route(\"x\")]\npublic class C : ControllerBase\n{\n  [HttpGet(\"a\"), Authorize]\n  public string A() { return \"\"; }\n}\n",
        &[
            "class C:ControllerBase @ApiController,Route",
            "fn A pub @HttpGet,Authorize",
        ],
    ),
    (
        Dialect::CSharp,
        "C.cs",
        "public class C\n{\n  public int Run(int a) { return a; }\n  public static int S() { return 1; }\n  private int P() { return 2; }\n}\n",
        &["class C", "fn Run pub"],
    ),
    (
        Dialect::CSharp,
        "C.cs",
        "public interface IFoo { }\npublic record R(int A);\npublic struct S { }\n",
        &["class IFoo interface", "class R", "class S"],
    ),
    (
        Dialect::CSharp,
        "page.razor",
        "public class C\n{\n  [Parameter]\n  public int Run(int a) { return a; }\n  public int Other() { return 1; }\n}\n",
        &["class C", "fn Run pub @Parameter"],
    ),
];

#[test]
fn declarations_per_dialect() {
    let mut wrong = Vec::new();
    for (dialect, file, snippet, expected) in CASES {
        let got = run(*dialect, file, snippet);
        if &got != expected {
            wrong.push(format!(
                "{dialect:?} {file}: {snippet:?}\n  expected {expected:?}\n  got      {got:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
