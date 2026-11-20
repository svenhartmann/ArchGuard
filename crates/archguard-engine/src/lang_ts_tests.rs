use crate::lang_ts::*;
use crate::lang_ts_decl::*;

fn parse(text: &str) -> Item {
    Parser { s: text, i: 0 }.value(0)
}

#[test]
fn parser_reads_nested_literals_and_keeps_the_rest_raw() {
    let text = "{ path: 'a/:id', data: { roles: ['x', \"y\"] }, canActivate: [authGuard], \
                load: () => import('./b'), t: `/u/${id}/v` } // tail";
    let Value::Obj(fields) = parse(text).value else {
        panic!("object expected");
    };
    let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["path", "data", "canActivate", "load", "t"]);
    assert_eq!(fields[0].1.value.literal().as_deref(), Some("a/:id"));
    assert!(matches!(&fields[3].1.value, Value::Raw(r) if r == "() => import('./b')"));
    assert_eq!(fields[4].1.value.literal().as_deref(), Some("/u/:param/v"));
    // A template literal starting with a hole is no literal.
    assert_eq!(parse("`${base}/x`").value.literal(), None);
    // A value followed by more than a separator is raw source.
    assert!(matches!(parse("'a' + b").value, Value::Raw(r) if r == "'a' + b"));
}

#[test]
fn strings_skip_escapes_and_comments_are_whitespace() {
    let text = "/* c */ 'it\\'s' // x\n";
    let item = parse(text);
    assert!(matches!(&item.value, Value::Str(s) if s == "it\\'s"));
    assert_eq!(item.source(text), "'it\\'s'");
}

#[test]
fn fill_reads_holes_and_fill_all_needs_every_hole() {
    let known = |h: &str| (h == "id").then(|| "42".to_string());
    assert_eq!(
        fill("/a/${id}/${x.y}", known).as_deref(),
        Some("/a/42/:param")
    );
    assert_eq!(fill("${x}/a", known), None);
    assert_eq!(
        fill("/${ {a: 1}.a }/b", |_| None).as_deref(),
        Some("/:param/b")
    );
    assert_eq!(fill_all("/a/${id}", known).as_deref(), Some("/a/42"));
    assert_eq!(fill_all("/a/${id}/${x}", known), None);
}

#[test]
fn brackets_skip_strings_and_comments() {
    let text = "f(a, ')', /* ) */ g(b), `)`) + 1";
    assert_eq!(block_end(text, 1, b'(', b')'), text.find(" + 1").unwrap());
    assert_eq!(close(text, 1), text.find(" + 1").unwrap());
    // `close` counts every bracket kind together.
    assert_eq!(close("{ a: [1, (2)] } x", 0), 15);
    assert_eq!(block_end("{ open", 0, b'{', b'}'), 6);
    assert_eq!(split_plus("'/a/' + id + `/b`"), ["'/a/'", "id", "`/b`"]);
    assert_eq!(split_plus("f(a + b)"), ["f(a + b)"]);
    assert_eq!(split_plus("i++"), ["i++"]);
}

#[test]
fn words_names_and_paths() {
    assert_eq!(words("from fromage $from from", "from"), [0, 19]);
    assert!(is_name("Guards.admin") && !is_name("1a") && !is_ident("a.b"));
    assert_eq!(trailing_ident("const router").as_deref(), Some("router"));
    assert_eq!(url_path("https://h:1/api/x?q=1#f"), "/api/x");
    assert_eq!(url_path("api/x"), "/api/x");
    assert_eq!(join("src/app", "../lib/./x"), "src/lib/x");
    assert_eq!(join("a", "../../x"), "x");
    assert_eq!(skip_generic(b"get<A<B>>(x)", 3), 9);
    assert_eq!(line_of("a\nb\nc", 4), 3);
    let (name, args) = call_parts("hasRole('a', 'b')").unwrap();
    assert_eq!((name, args.len()), ("hasRole", 2));
    assert_eq!(
        local_array("let x = 1; const routes = [a]", "routes"),
        Some(26)
    );
    assert!(declares("export default function Home() {}", "Home"));
    assert!(has_dep(
        "{ \"dependencies\": { \"vue-router\" : \"4\" } }",
        "vue-router"
    ));
    assert!(!has_dep("{ \"name\": \"vue-router-x\" }", "vue-router"));
}

// The React copy read the module as a whole value, so an import without `;`
// ran into the next line and was lost; the Vue copy read the string alone.
#[test]
fn imports_without_semicolons_are_read() {
    let text = "import axios from 'axios'\nimport Link, { useRouter as r, type T } from \"next/link\"\n\
                import * as api from './api'\nimport type { X } from './x'\nconst a = import('./lazy')\n";
    let found: Vec<(String, Option<String>, String)> = imports(text)
        .into_iter()
        .map(|i| (i.local, i.imported, i.module))
        .collect();
    let own = |l: &str, i: Option<&str>, m: &str| (l.into(), i.map(Into::into), m.into());
    assert_eq!(
        found,
        [
            own("axios", None, "axios"),
            own("Link", None, "next/link"),
            own("r", Some("useRouter"), "next/link"),
            own("T", Some("T"), "next/link"),
            own("api", Some("*"), "./api"),
        ]
    );
}

#[test]
fn bindings_of_hooks() {
    let text = "const router = useRouter();\nconst { push, replace: go } = useRouter();\n\
                if (x == useRouter()) {}\n";
    let found = bindings(text, "useRouter");
    assert_eq!(found.len(), 2);
    assert!(matches!(&found[0], Binding::Name(n) if n == "router"));
    assert!(matches!(&found[1], Binding::Destructure(p)
        if p == &[("push".to_string(), "push".to_string()), ("replace".into(), "go".into())]));
}
