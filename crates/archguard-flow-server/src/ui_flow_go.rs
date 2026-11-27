//! Go web-framework endpoint access extractor for ui-flow/v1 (ADR 0021).
//!
//! Text level (comments and string contents are blanked for searching, literals
//! are read from the raw text); the Go compiler helper is not used. An
//! application is a directory with a `go.mod`; its non-test `.go` files are read.
//!
//! Recognised routers, by the import path of their constructor: `gin.New/Default`,
//! `chi.NewRouter`, `echo.New`, `mux.NewRouter` (gorilla), `http.NewServeMux` and
//! the default mux (`http.Handle`). Statements run in source order per router
//! variable (a function that receives a router is run at each call with the
//! caller's state; a function that returns one is run at its call):
//! - gin: `Use` guards routes registered later; `Group` copies the parent's
//!   chain at creation; route arguments before the handler are middleware.
//! - chi: `Use`, `With(..)`, `Group(func)` (inherit, additions stay inside),
//!   `Route(path, func)`, `Mount(path, router)`.
//! - echo: `Group(path, mw..)`, route middleware after the handler; `e.Use`/`e.Pre`
//!   at the root apply to every route whatever the position.
//! - gorilla: `Use` guards every route of the (sub)router; `Methods(..)` after
//!   `HandleFunc`; `PathPrefix(..).Subrouter()`.
//! - net/http: Go 1.22 patterns (`"GET /p/{id}"`), wrappers around the handler
//!   and around the mux handed to `ListenAndServe` / `http.Server{Handler: ..}`.
//!
//! A middleware maps to a condition ONLY through `[views.access]` or a recognised
//! library (gin `BasicAuth`, chi `jwtauth.Authenticator`, echo `BasicAuth`/`KeyAuth`/
//! `echo-jwt` with a literal `Skipper`, casbin authorizers); passive packages
//! (logging, tracing, CORS, ..) add none. Anything else is `Unknown(text)` with a
//! note, never guessed from its name. `Open` only from a middleware listed `open`
//! or a literal echo-jwt skipper. A middleware inside if/for/switch is unknown.
//! Casbin: a CSV policy and a model with the supported matcher are evaluated per
//! route, a database adapter is `Unknown` with a gap. Ory Oathkeeper access-rule
//! files (YAML/JSON) give one endpoint per rule URL and method.
//!
//! Not followed (gap): non-literal route paths, routers hidden in other
//! structures, functions that take a router but are never called.
use archguard_core::UiFlow;
use archguard_flow_common::extract_ctx::{
    ExtractContext, dir_name, finish, gap, read_source, read_text, walk, with_root_config,
};
use archguard_flow_common::support::test_path;
use std::path::Path;

mod casbin;
mod chain;
mod model;
mod mw;
mod oathkeeper;
mod stmt;
mod sweep;
mod text;
mod walk;
use model::{Interp, Module};
use text::Src;

const MAX_FILES: usize = 20_000;

pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let files = match walk(ctx.root) {
        Ok(files) => files,
        Err(e) => {
            gap(
                &mut flow.diagnostics,
                ".",
                0,
                format!("Discovery settings not read: {e}"),
            );
            finish(&mut flow);
            return flow;
        }
    };
    let mut go: Vec<&String> = (files.files.iter())
        .filter(|f| f.ends_with(".go") && !f.ends_with("_test.go") && !test_path(f))
        .collect();
    go.truncate(MAX_FILES);
    let mut srcs = Vec::new();
    for rel in go {
        match read_source(ctx.root, rel) {
            Ok(raw) => srcs.push(Src::new(rel.clone(), raw)),
            Err(m) => gap(&mut flow.diagnostics, rel, 0, m),
        }
    }
    let mods: Vec<Module> = (files.files.iter())
        .filter(|f| *f == "go.mod" || f.ends_with("/go.mod"))
        .filter_map(|f| {
            let text = match read_text(ctx.root, f, 1 << 20) {
                Ok(text) => text,
                Err(e) => {
                    let message =
                        format!("go.mod not read: {e}; its module path and sources are unknown.");
                    gap(&mut flow.diagnostics, f, 0, message);
                    return None;
                }
            };
            let dir = f.strip_suffix("go.mod")?.trim_end_matches('/').to_string();
            let path = archguard_extract::go::module_path(&text)?;
            Some(Module {
                name: dir_name(ctx.root, &dir),
                dir,
                path,
            })
        })
        .collect();
    for m in &mods {
        let mine: Vec<usize> = (0..srcs.len())
            .filter(|&i| owner(&mods, &srcs[i].rel).is_some_and(|o| o.dir == m.dir))
            .collect();
        if mine.is_empty() {
            continue;
        }
        let mut it = Interp::new(ctx, &srcs, &mine, m);
        it.run();
        it.sweep();
        it.finish_into(&mut flow);
    }
    oathkeeper::oathkeeper(ctx, &files.files, &mut flow);
    finish(&mut flow);
    flow
}

/// The nearest `go.mod` module of a file.
fn owner<'a>(mods: &'a [Module], rel: &str) -> Option<&'a Module> {
    (mods.iter())
        .filter(|m| m.dir.is_empty() || rel.starts_with(&format!("{}/", m.dir)))
        .max_by_key(|m| m.dir.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Condition, DiagnosticSeverity, EndpointAccess};

    fn run(files: &[(&str, &str)], access: &str) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        let config = format!("[views.access]\n{access}");
        let config = (!access.is_empty()).then_some((".archguard/archguard.toml", config.as_str()));
        let gomod = ("go.mod", "module example.org/app\n\ngo 1.22\n");
        for (rel, text) in files.iter().copied().chain(config).chain([gomod]) {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    fn ep<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a EndpointAccess {
        (flow.endpoints.iter())
            .find(|e| e.method == method && e.path == path)
            .unwrap_or_else(|| panic!("no {method} {path} in {:#?}", flow.endpoints))
    }

    fn prot(c: Condition) -> AccessState {
        AccessState::Protected(c)
    }

    fn unknown(t: &str) -> Condition {
        Condition::Unknown(t.into())
    }

    fn gaps(flow: &UiFlow) -> Vec<&str> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| d.message.as_str())
            .collect()
    }

    const AUTH: &str = "auth = \"authenticated\"\n";

    fn gin(body: &str) -> String {
        format!(
            "package main\n\nimport \"github.com/gin-gonic/gin\"\n\nfunc main() {{\n{body}\n}}\n"
        )
    }

    #[test]
    fn gf1_gin_use_applies_only_to_later_routes() {
        let src = gin("r := gin.New()\nr.GET(\"/a\", a)\nr.Use(auth)\nr.GET(\"/b\", b)");
        let flow = run(&[("main.go", &src)], AUTH);
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Undeclared);
        assert_eq!(
            ep(&flow, "GET", "/b").access,
            prot(Condition::Authenticated)
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn gf2_gin_group_copies_the_parent_chain_at_creation() {
        let src = gin(
            "r := gin.New()\nv1 := r.Group(\"/api\")\nu := v1.Group(\"/users\")\nv1.Use(auth)\n\
             p := v1.Group(\"/profiles\")\nu.POST(\"\", reg)\np.GET(\"\", get)",
        );
        let flow = run(&[("main.go", &src)], AUTH);
        assert_eq!(
            ep(&flow, "POST", "/api/users").access,
            AccessState::Undeclared
        );
        assert_eq!(
            ep(&flow, "GET", "/api/profiles").access,
            prot(Condition::Authenticated)
        );
    }

    #[test]
    fn gf3_unlisted_middleware_keeps_its_argument_text() {
        let src = gin(
            "r := gin.New()\nv1 := r.Group(\"/api\")\nv1.Use(AuthMiddleware(false))\n\
             v1.Use(AuthMiddleware(true))\nv1.GET(\"/x\", h)",
        );
        let flow = run(&[("main.go", &src)], "");
        assert_eq!(
            ep(&flow, "GET", "/api/x").access,
            prot(Condition::All(vec![
                unknown("AuthMiddleware(false)"),
                unknown("AuthMiddleware(true)")
            ]))
        );
    }

    #[test]
    fn gf4_router_passed_to_a_function_keeps_the_callers_state() {
        let src = format!(
            "{}\nfunc Register(g *gin.RouterGroup) {{\n\tg.GET(\"/x\", h)\n}}\n",
            gin("r := gin.New()\nv1 := r.Group(\"/api\")\nRegister(v1.Group(\"/t\", auth))")
        );
        let flow = run(&[("main.go", &src)], AUTH);
        assert_eq!(
            ep(&flow, "GET", "/api/t/x").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
    }

    fn chi(body: &str) -> String {
        format!(
            "package main\n\nimport (\n\t\"github.com/go-chi/chi/v5\"\n\t\"github.com/go-chi/chi/v5/middleware\"\n\t\
             \"github.com/go-chi/jwtauth/v5\"\n)\n\nfunc main() {{\n{body}\n}}\n"
        )
    }

    #[test]
    fn gf5_chi_group_inherits_and_keeps_additions_inside() {
        let src = chi(
            "r := chi.NewRouter()\nr.Use(middleware.Logger)\nr.Group(func(r chi.Router) {\n\tr.Use(auth)\n\
             \tr.Get(\"/a\", a)\n})\nr.Get(\"/b\", b)",
        );
        let flow = run(&[("main.go", &src)], AUTH);
        assert_eq!(
            ep(&flow, "GET", "/a").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(ep(&flow, "GET", "/b").access, AccessState::Undeclared);
    }

    #[test]
    fn gf6_chi_with_guards_only_the_chained_route() {
        let src = chi("r := chi.NewRouter()\nr.With(auth).Get(\"/a\", a)\nr.Get(\"/b\", b)");
        let flow = run(&[("main.go", &src)], AUTH);
        assert_eq!(
            ep(&flow, "GET", "/a").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(ep(&flow, "GET", "/b").access, AccessState::Undeclared);
    }

    #[test]
    fn gf7_chi_verifier_never_rejects_authenticator_does() {
        let v = chi("r := chi.NewRouter()\nr.Use(jwtauth.Verifier(t))\nr.Get(\"/a\", a)");
        let flow = run(&[("main.go", &v)], "");
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Undeclared);
        let a = chi(
            "r := chi.NewRouter()\nr.Use(jwtauth.Verifier(t))\nr.Use(jwtauth.Authenticator(t))\nr.Get(\"/a\", a)",
        );
        let flow = run(&[("main.go", &a)], "");
        assert_eq!(
            ep(&flow, "GET", "/a").access,
            prot(Condition::Authenticated)
        );
    }

    const ECHO: &str = "package main\n\nimport (\n\t\"github.com/labstack/echo/v4\"\n\t\
        \"github.com/labstack/echo/v4/middleware\"\n\techojwt \"github.com/labstack/echo-jwt/v4\"\n)\n\nfunc main() {\n";

    #[test]
    fn gf8_echo_group_middleware_guards_the_group_only() {
        let src = format!(
            "{ECHO}e := echo.New()\ng := e.Group(\"/admin\", middleware.BasicAuth(f))\ng.GET(\"/x\", h)\ne.GET(\"/y\", h)\n}}\n"
        );
        let flow = run(&[("main.go", &src)], "");
        assert_eq!(
            ep(&flow, "GET", "/admin/x").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(ep(&flow, "GET", "/y").access, AccessState::Undeclared);
    }

    #[test]
    fn gf9_echo_jwt_literal_skipper_opens_that_path_only() {
        let src = format!(
            "{ECHO}e := echo.New()\ne.Use(echojwt.WithConfig(echojwt.Config{{\n\tSigningKey: k,\n\t\
             Skipper: func(c echo.Context) bool {{ return c.Path() == \"/login\" }},\n}}))\n\
             e.POST(\"/login\", l)\ne.GET(\"/x\", h)\n}}\n"
        );
        let flow = run(&[("main.go", &src)], "");
        assert_eq!(ep(&flow, "POST", "/login").access, AccessState::Open);
        assert_eq!(
            ep(&flow, "GET", "/x").access,
            prot(Condition::Authenticated)
        );
    }

    #[test]
    fn gf9_echo_jwt_non_literal_skipper_is_unknown_with_a_gap() {
        let src = format!(
            "{ECHO}e := echo.New()\ne.Use(echojwt.WithConfig(echojwt.Config{{Skipper: skip}}))\n\
             e.POST(\"/login\", l)\n}}\n"
        );
        let flow = run(&[("main.go", &src)], "");
        assert!(matches!(
            ep(&flow, "POST", "/login").access,
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!flow.complete && !gaps(&flow).is_empty());
    }

    const MUX: &str = "package main\n\nimport \"net/http\"\n\nfunc main() {\n\tmux := http.NewServeMux()\n\
        \tmux.Handle(\"GET /a\", auth(http.HandlerFunc(a)))\n\tmux.HandleFunc(\"GET /b\", b)\n\
        \thttp.ListenAndServe(\":8080\", logging(mux))\n}\n";

    #[test]
    fn gf10_net_http_wrappers_and_server_wrapper() {
        let flow = run(&[("main.go", MUX)], AUTH);
        assert_eq!(
            ep(&flow, "GET", "/a").access,
            prot(Condition::All(vec![
                unknown("logging"),
                Condition::Authenticated
            ]))
        );
        assert_eq!(ep(&flow, "GET", "/b").access, prot(unknown("logging")));
    }

    #[test]
    fn gf10_listed_open_wrapper_makes_unguarded_routes_open_only_by_marker() {
        let flow = run(
            &[("main.go", MUX)],
            "auth = \"authenticated\"\nlogging = \"open\"\n",
        );
        assert_eq!(
            ep(&flow, "GET", "/a").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(ep(&flow, "GET", "/b").access, AccessState::Open);
    }

    #[test]
    fn gorilla_use_covers_the_whole_subrouter_and_methods_are_read() {
        let src = "package main\n\nimport (\n\t\"net/http\"\n\t\"github.com/gorilla/mux\"\n)\n\nfunc main() {\n\
            \tr := mux.NewRouter()\n\tr.HandleFunc(\"/p\", p).Methods(http.MethodGet, \"HEAD\")\n\
            \ts := r.PathPrefix(\"/api\").Subrouter()\n\ts.HandleFunc(\"/x\", x).Methods(\"POST\")\n\ts.Use(auth)\n}\n";
        let flow = run(&[("main.go", src)], AUTH);
        assert_eq!(ep(&flow, "GET", "/p").access, AccessState::Undeclared);
        assert_eq!(ep(&flow, "HEAD", "/p").access, AccessState::Undeclared);
        assert_eq!(
            ep(&flow, "POST", "/api/x").access,
            prot(Condition::Authenticated)
        );
    }

    #[test]
    fn an_unreadable_path_is_a_gap_and_a_conditional_use_is_unknown() {
        let src = gin("r := gin.New()\nif dbg {\n\tr.Use(auth)\n}\nr.GET(\"/a\"+suffix, a)");
        let flow = run(&[("main.go", &src)], AUTH);
        assert!(
            !flow.complete && gaps(&flow).len() >= 2,
            "{:#?}",
            flow.diagnostics
        );
        let a = flow
            .endpoints
            .iter()
            .find(|e| e.path.starts_with("/a"))
            .unwrap();
        assert!(matches!(
            a.access,
            AccessState::Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn cross_package_registration_keeps_the_callers_group_state() {
        let main = "package main\n\nimport (\n\t\"github.com/gin-gonic/gin\"\n\t\"example.org/app/users\"\n)\n\nfunc main() {\n\
            \tr := gin.Default()\n\tv1 := r.Group(\"/api\")\n\tusers.Register(v1.Group(\"/users\"))\n\tv1.Use(auth)\n\
            \tusers.Me(v1.Group(\"/me\"))\n}\n";
        let users = "package users\n\nimport \"github.com/gin-gonic/gin\"\n\nfunc Register(r *gin.RouterGroup) {\n\
            \tr.POST(\"/\", reg)\n}\n\nfunc Me(r *gin.RouterGroup) {\n\tr.GET(\"\", me)\n}\n";
        let flow = run(&[("main.go", main), ("users/users.go", users)], AUTH);
        assert_eq!(
            ep(&flow, "POST", "/api/users/").access,
            AccessState::Undeclared
        );
        assert_eq!(
            ep(&flow, "GET", "/api/me").access,
            prot(Condition::Authenticated)
        );
    }

    #[test]
    fn unreadable_path_prefix_and_struct_wrappers_around_a_handler_variable() {
        let src = "package main\n\nimport (\n\t\"net/http\"\n\t\"github.com/gorilla/mux\"\n)\n\nfunc main() {\n\
            \tr := mux.NewRouter()\n\tr.HandleFunc(baseUrl+\"/cart\", cart).Methods(http.MethodPost)\n\
            \tvar handler http.Handler = r\n\thandler = &logHandler{next: handler}\n\
            \thandler = ensureSession(handler)\n\tlog.Fatal(http.ListenAndServe(addr, handler))\n}\n";
        let flow = run(&[("main.go", src)], "");
        let e = ep(&flow, "POST", "/{baseUrl}/cart");
        assert_eq!(
            e.access,
            prot(Condition::All(vec![
                unknown("ensureSession"),
                unknown("logHandler")
            ]))
        );
        assert!(!flow.complete);
    }

    fn unknown_route<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a EndpointAccess {
        let e = ep(flow, method, path);
        assert!(
            matches!(e.access, AccessState::Protected(Condition::Unknown(_))),
            "{e:#?}"
        );
        assert!(
            !flow.complete && !gaps(flow).is_empty(),
            "{:#?}",
            flow.diagnostics
        );
        e
    }

    #[test]
    fn struct_field_router_is_a_gap_and_an_unknown_route() {
        let src = "package main\n\nimport \"github.com/gin-gonic/gin\"\n\ntype S struct{ router *gin.Engine }\n\n\
            func main() {\n\ts := &S{router: gin.New()}\n\ts.router.Use(auth)\n\ts.routes()\n}\n\n\
            func (s *S) routes() {\n\ts.router.GET(\"/a\", a)\n}\n";
        unknown_route(&run(&[("main.go", src)], AUTH), "GET", "/a");
    }

    #[test]
    fn package_level_router_is_a_gap_and_an_unknown_route() {
        let src = "package main\n\nimport \"github.com/gin-gonic/gin\"\n\nvar r = gin.New()\n\n\
            func init() {\n\tr.Use(auth)\n}\n\nfunc main() {\n\tr.GET(\"/b\", b)\n}\n";
        unknown_route(&run(&[("main.go", src)], AUTH), "GET", "/b");
    }

    #[test]
    fn closure_registrar_is_a_gap_and_an_unknown_route() {
        let src =
            gin("r := gin.New()\nreg := func(r *gin.Engine) {\n\tr.GET(\"/c\", c)\n}\nreg(r)");
        unknown_route(&run(&[("main.go", &src)], ""), "GET", "/c");
    }

    #[test]
    fn interface_registrar_is_a_gap_and_an_unknown_route() {
        let src = "package main\n\nimport \"github.com/gin-gonic/gin\"\n\nfunc main() {\n\tr := gin.New()\n\
            \tfor _, x := range regs {\n\t\tx.Register(r.Group(\"/v\"))\n\t}\n}\n\n\
            func (a A) Register(g *gin.RouterGroup) {\n\tg.GET(\"/d\", d)\n}\n\n\
            func (b B) Register(g *gin.RouterGroup) {\n\tg.GET(\"/e\", e)\n}\n";
        let flow = run(&[("main.go", src)], "");
        unknown_route(&flow, "GET", "/d");
        unknown_route(&flow, "GET", "/e");
    }

    #[test]
    fn a_call_split_over_lines_and_raw_string_paths_are_followed() {
        let src = gin("r := gin.New()\nr.Use(auth)\nr.\n\tGET(\"/ml\", h)\nr.GET(`/raw`, h)");
        let flow = run(&[("main.go", &src)], AUTH);
        assert_eq!(
            ep(&flow, "GET", "/ml").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(
            ep(&flow, "GET", "/raw").access,
            prot(Condition::Authenticated)
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn casbin_with_an_unreadable_policy_file_is_not_never() {
        let main = "package main\n\nimport (\n\t\"github.com/gin-gonic/gin\"\n\t\"github.com/casbin/casbin/v2\"\n\
            \t\"github.com/gin-contrib/authz\"\n)\n\nfunc main() {\n\
            \te, _ := casbin.NewEnforcer(\"model.conf\", \"missing.csv\")\n\tr := gin.New()\n\
            \tr.Use(authz.NewAuthorizer(e))\n\tr.GET(\"/users\", l)\n}\n";
        let model = "[request_definition]\nr = sub, obj, act\n[policy_definition]\np = sub, obj, act\n\
            [policy_effect]\ne = some(where (p.eft == allow))\n[matchers]\n\
            m = r.sub == p.sub && keyMatch2(r.obj, p.obj) && r.act == p.act\n";
        let flow = run(&[("main.go", main), ("model.conf", model)], "");
        let e = ep(&flow, "GET", "/users");
        assert!(
            matches!(e.access, AccessState::Protected(Condition::Unknown(_))),
            "{e:#?}"
        );
        assert!(!flow.complete);
    }

    fn casbin(model_effect: &str, policy: &str, enforcer: &str) -> UiFlow {
        let main = format!(
            "package main\n\nimport (\n\t\"github.com/gin-gonic/gin\"\n\t\"github.com/casbin/casbin/v2\"\n\
             \t\"github.com/gin-contrib/authz\"\n)\n\nfunc main() {{\n\t{enforcer}\n\tr := gin.New()\n\
             \tr.Use(authz.NewAuthorizer(e))\n\tr.DELETE(\"/users/:id\", d)\n\tr.GET(\"/users\", l)\n}}\n"
        );
        let model = format!(
            "[request_definition]\nr = sub, obj, act\n\n[policy_definition]\np = sub, obj, act\n\n\
             [role_definition]\ng = _, _\n\n[policy_effect]\ne = {model_effect}\n\n[matchers]\n\
             m = g(r.sub, p.sub) && keyMatch2(r.obj, p.obj) && r.act == p.act\n"
        );
        run(
            &[
                ("main.go", &main),
                ("model.conf", &model),
                ("policy.csv", policy),
            ],
            "",
        )
    }

    const NEW_ENF: &str = "e, _ := casbin.NewEnforcer(\"model.conf\", \"policy.csv\")";

    #[test]
    fn gf11_casbin_allow_override_maps_policy_lines_to_roles() {
        let f = casbin(
            "some(where (p.eft == allow))",
            "p, admin, /users/:id, DELETE\ng, alice, admin\n",
            NEW_ENF,
        );
        assert_eq!(
            ep(&f, "DELETE", "/users/:id").access,
            prot(Condition::Role("admin".into()))
        );
        assert_eq!(ep(&f, "GET", "/users").access, prot(Condition::Never));
    }

    #[test]
    fn gf12_casbin_deny_override_without_deny_lines_adds_nothing() {
        let f = casbin("!some(where (p.eft == deny))", "g, alice, admin\n", NEW_ENF);
        assert_eq!(ep(&f, "GET", "/users").access, AccessState::Undeclared);
        assert!(
            f.diagnostics
                .iter()
                .any(|d| d.message.contains("deny-override"))
        );
    }

    #[test]
    fn gf13_casbin_database_policy_is_unknown_with_a_gap() {
        let f = casbin(
            "some(where (p.eft == allow))",
            "",
            "a, _ := gormadapter.NewAdapter(\"mysql\", dsn)\n\te, _ := casbin.NewEnforcer(\"model.conf\", a)",
        );
        assert_eq!(
            ep(&f, "GET", "/users").access,
            prot(unknown("casbin policy in database"))
        );
        assert!(!f.complete);
    }

    fn rule(authn: &str, authz: &str) -> UiFlow {
        let yml = format!(
            "- id: r\n  match:\n    url: \"http://api/health\"\n    methods: [GET]\n  authenticators:\n{authn}  authorizer:\n    handler: {authz}\n"
        );
        run(&[("rules/access-rules.yml", &yml)], "")
    }

    #[test]
    fn gf14_oathkeeper_noop_and_allow_is_open() {
        let f = rule("    - handler: noop\n", "allow");
        assert_eq!(ep(&f, "GET", "/health").access, AccessState::Open);
    }

    #[test]
    fn gf15_oathkeeper_jwt_scopes_become_policies() {
        let f = rule(
            "    - handler: jwt\n      config:\n        required_scope: [\"a\", \"b\"]\n",
            "allow",
        );
        assert_eq!(
            ep(&f, "GET", "/health").access,
            prot(Condition::All(vec![
                Condition::Authenticated,
                Condition::Policy("a".into()),
                Condition::Policy("b".into())
            ]))
        );
    }

    #[test]
    fn gf16_oathkeeper_anonymous_fallback_is_open_with_a_note() {
        let f = rule("    - handler: jwt\n    - handler: anonymous\n", "allow");
        assert_eq!(ep(&f, "GET", "/health").access, AccessState::Open);
        assert!(
            f.diagnostics
                .iter()
                .any(|d| d.message.contains("anonymous fallback"))
        );
    }
}
