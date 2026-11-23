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

#[test]
fn name_path_rejects_spreads_and_empty_segments() {
    for ok in ["a", "a.b", "Guards.admin", "_x.y1"] {
        assert!(name_path(ok), "{ok}");
    }
    // `is_name` accepts these; a name path does not.
    for bad in ["...x", "a..b", ".a", "a.", ".", "", "1a", "a b"] {
        assert!(!name_path(bad), "{bad}");
    }
    assert!(is_name("...x") && is_name("a..b"));
}

#[test]
fn declared_value_reads_constants_and_function_bodies() {
    let text = "const a = 1;\nconst b = { k: 'v' };\nfunction f(x) { return x; }";
    assert!(matches!(
        declared_value(text, "b").unwrap().value,
        Value::Obj(_)
    ));
    assert_eq!(declared_value(text, "a").unwrap().source(text), "1");
    assert_eq!(
        declared_value(text, "f").unwrap().source(text),
        "{ return x; }"
    );
    assert!(declared_value(text, "missing").is_none());
}

#[test]
fn generated_calls_read_the_first_fetch_with_method_base_and_query() {
    use crate::lang_ts_http::generated_calls;
    use archguard_core::Call;
    let sdk = r#"export const defaults: Oazapfts.Defaults = { headers: {}, baseUrl: "/api/" };
export function getAll({ shared }: { shared?: boolean }, opts?: Opts) {
    return oazapfts.ok(oazapfts.fetchJson<{ status: 200 }>(`/albums${QS.query(QS.explode({ shared }))}`, { ...opts }));
}
export function remove({ id }: { id: string }, opts?: Opts) {
    return oazapfts.ok(oazapfts.fetchText(`/albums/${encodeURIComponent(id)}`, { ...opts, method: "delete" }));
}
export const dyn = ({ path }) => oazapfts.fetchJson(path, {});
export const verb = (m) => fetch('/x', { method: m });
export const baseUrl = () => defaults.baseUrl;
function notExported() { return fetch('/hidden'); }
"#;
    let calls = generated_calls(&[("fetch-client.ts".into(), sdk.into())]);
    let http = |m: &str, p: &str| {
        Ok(Call::Http {
            method: m.into(),
            path: p.into(),
        })
    };
    assert_eq!(calls["getAll"], http("GET", "/api/albums"));
    assert_eq!(calls["remove"], http("DELETE", "/api/albums/:param"));
    assert!(
        calls["dyn"]
            .as_ref()
            .unwrap_err()
            .starts_with("URL is not a literal")
    );
    assert!(
        calls["verb"]
            .as_ref()
            .unwrap_err()
            .starts_with("method is not a literal")
    );
    // No fetch call, or not exported: no entry.
    assert!(!calls.contains_key("baseUrl") && !calls.contains_key("notExported"));
}

fn aliases(rel: &str, files: &[(&str, &str)]) -> Vec<(String, Vec<String>)> {
    let read = |p: &str| {
        files
            .iter()
            .find(|(f, _)| *f == p)
            .map(|(_, t)| t.to_string())
    };
    tsconfig_aliases(rel, &read)
        .into_iter()
        .map(|a| (a.pattern, a.targets))
        .collect()
}

fn alias(pattern: &str, targets: &[&str]) -> (String, Vec<String>) {
    (
        pattern.into(),
        targets.iter().map(|t| t.to_string()).collect(),
    )
}

#[test]
fn tsconfig_paths_resolve_below_baseurl_or_the_config_directory() {
    let own =
        r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"], "lib": ["../lib/index"] } } }"#;
    assert_eq!(
        aliases("fe/tsconfig.json", &[("fe/tsconfig.json", own)]),
        vec![alias("@/*", &["fe/src/*"]), alias("lib", &["lib/index"])]
    );
    let based = r#"{ "compilerOptions": { "baseUrl": "src", "paths": { "@/*": ["*"] } } }"#;
    assert_eq!(
        aliases("fe/tsconfig.json", &[("fe/tsconfig.json", based)]),
        vec![alias("@/*", &["fe/src/*"])]
    );
    assert!(aliases("fe/tsconfig.json", &[]).is_empty());
}

#[test]
fn tsconfig_tolerates_comments_and_trailing_commas() {
    let text = r#"{
  // "paths": { "no/*": ["x/*"] }
  "compilerOptions": {
    /* block */ "paths": { "@/*": ["./src/*",], },
  },
}"#;
    assert_eq!(
        aliases("tsconfig.json", &[("tsconfig.json", text)]),
        vec![alias("@/*", &["src/*"])]
    );
}

#[test]
fn tsconfig_extends_is_followed_up_to_five_hops_and_the_nearest_wins() {
    let files = [
        ("fe/tsconfig.json", r#"{ "extends": "./tsconfig.app" }"#),
        (
            "fe/tsconfig.app.json",
            r#"{ "extends": "../base/tsconfig.base.json", "compilerOptions": { "baseUrl": "src" } }"#,
        ),
        (
            "base/tsconfig.base.json",
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["*"] } } }"#,
        ),
    ];
    // `baseUrl` from the nearest config, `paths` from the one that has them.
    assert_eq!(
        aliases("fe/tsconfig.json", &files),
        vec![alias("@/*", &["fe/src/*"])]
    );
    // Seven configs in a chain: the paths sit past the fifth hop of c0.
    let chain: Vec<(String, String)> = (0..7)
        .map(|i| {
            let text = if i == 6 {
                r#"{ "compilerOptions": { "paths": { "@/*": ["*"] } } }"#.to_string()
            } else {
                format!(r#"{{ "extends": "./c{}.json" }}"#, i + 1)
            };
            (format!("c{i}.json"), text)
        })
        .collect();
    let refs: Vec<(&str, &str)> = chain
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert!(aliases("c0.json", &refs).is_empty());
    assert_eq!(aliases("c1.json", &refs), vec![alias("@/*", &["*"])]);
}
