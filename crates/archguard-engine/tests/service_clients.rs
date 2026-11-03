use archguard_core::ServiceDependencyKind as Kind;
use archguard_engine::analyze_services;
use std::{fs, path::Path};
use tempfile::tempdir;
fn put(r: &Path, p: &str, s: &str) {
    let p = r.join(p);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
#[test]
fn go_client_calls_follow_env_and_verified_helpers_not_unused_constructors() {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module test/client\n");
    put(d.path(), "server/go.mod", "module test/server\n");
    put(
        d.path(),
        "compose.yaml",
        "services:\n  client:\n    build: ./client\n    environment: {RPC_ADDR: 'server:5000'}\n  server:\n    build: ./server\n",
    );
    put(
        d.path(),
        "client/main.go",
        r#"package main
import ("os"; "google.golang.org/grpc")
type app struct { address string; active pb.ApiClient; unused pb.OtherClient }
func load(out *string, key string) { v := os.Getenv(key); *out = v }
func dial(address string) *grpc.ClientConn { c, err := grpc.NewClient(address); if err != nil { panic(err) }; return c }
func main() {
 svc := new(app)
 load(&svc.address, "RPC_ADDR")
 c := dial(svc.address)
 svc.active = pb.NewApiClient(c)
 svc.unused = pb.NewOtherClient(c)
}
func (s *app) run() { s.active.Get(ctx, req) }
"#,
    );
    let r = analyze_services(d.path()).unwrap();
    let e: Vec<_> = r
        .dependencies
        .iter()
        .filter(|e| e.kind == Kind::GrpcCall)
        .collect();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].to, "compose.yaml::server");
    assert!(e[0].evidence.iter().any(|p| p.description.contains("call")));
}
#[test]
fn unused_go_constructor_does_not_prove_rpc_call() {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module test/client\n");
    put(d.path(), "server/go.mod", "module test/server\n");
    put(
        d.path(),
        "compose.yaml",
        "services:\n  client:\n    build: ./client\n    environment: {RPC_ADDR: 'server:5000'}\n  server:\n    build: ./server\n",
    );
    put(
        d.path(),
        "client/main.go",
        "package main\nfunc main(){ address := os.Getenv(\"RPC_ADDR\"); c, _ := grpc.NewClient(address); unused := pb.NewApiClient(c); _ = unused }\n",
    );
    assert!(
        !analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::GrpcCall)
    );
}

#[test]
fn untyped_client_constructor_with_plain_string_is_not_grpc_evidence() {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module client\n");
    put(d.path(), "server/go.mod", "module server\n");
    put(
        d.path(),
        "compose.yaml",
        "services:\n  client:\n    build: ./client\n    environment: {RPC_ADDR: 'server:5000'}\n  server:\n    build: ./server\n",
    );
    put(
        d.path(),
        "client/main.go",
        "package main\nfunc main() {\n address := os.Getenv(\"RPC_ADDR\")\n client := custom.NewFakeClient(address)\n client.Get(ctx, req)\n}\n",
    );
    assert!(
        !analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::GrpcCall)
    );
}

#[test]
fn conditional_override_keeps_base_client_as_a_static_alternative() {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module client\n");
    put(d.path(), "server/go.mod", "module server\n");
    put(
        d.path(),
        "compose.yaml",
        "services:\n  client:\n    build: ./client\n    environment: {RPC_ADDR: 'server:5000'}\n  server:\n    build: ./server\n",
    );
    put(
        d.path(),
        "client/main.go",
        r#"package main
func main(){
 address := os.Getenv("RPC_ADDR")
 c, err := grpc.NewClient(address)
 client := pb.NewApiClient(c)
 if flag.Value(ctx, options.Context{}) {
   other, err := grpc.NewClient("badAddress:5000")
   client = pb.NewApiClient(other)
 }
 client.Get(ctx, req)
}
"#,
    );
    assert!(
        analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::GrpcCall && e.to == "compose.yaml::server")
    );
}

fn fixture(source: &str) -> tempfile::TempDir {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module client\n");
    put(d.path(), "server/go.mod", "module server\n");
    put(
        d.path(),
        "compose.yaml",
        "services:\n  client:\n    build: ./client\n    environment: {RPC_ADDR: 'server:5000', API_URL: 'http://server:8080'}\n  server:\n    build: ./server\n",
    );
    put(d.path(), "client/main.go", source);
    d
}
#[test]
fn unrelated_or_mutated_helper_results_are_not_connections() {
    for helper in [
        "func dial(address string) *grpc.ClientConn { c, err := grpc.NewClient(address); return unknown() }",
        "func dial(address string) *grpc.ClientConn { c, err := grpc.NewClient(address); c = unknown(); return c }",
        "func dial(address string) *grpc.ClientConn { address = unknown(); c, err := grpc.NewClient(address); return c }",
    ] {
        let d = fixture(&format!(
            "package main\n{helper}\nfunc main(){{\n a := os.Getenv(\"RPC_ADDR\")\n c := dial(a)\n client := pb.NewApiClient(c)\n client.Get(ctx,req)\n}}"
        ));
        assert!(
            !analyze_services(d.path())
                .unwrap()
                .dependencies
                .iter()
                .any(|e| e.kind == Kind::GrpcCall),
            "{helper}"
        );
    }
}
#[test]
fn modified_environment_helper_result_is_unknown() {
    let d = fixture(
        "package main\nfunc load(out *string,key string){ v := os.Getenv(key); v = unknown(); *out = v }\nfunc main(){\n load(&address,\"RPC_ADDR\")\n c,err := grpc.NewClient(address)\n client := pb.NewApiClient(c)\n client.Get(ctx,req)\n}",
    );
    assert!(
        !analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::GrpcCall)
    );
}
#[test]
fn overwritten_http_request_is_not_sent_to_original_target() {
    let d = fixture(
        "package main\nfunc main(){\n address := os.Getenv(\"API_URL\")\n req,err := http.NewRequest(\"GET\",address,nil)\n req = unknown()\n client.Do(req)\n}",
    );
    assert!(
        !analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::HttpCall)
    );
}
#[test]
fn helper_from_another_package_is_not_a_local_helper() {
    let d = fixture(
        "package main\nfunc main(){\n address := os.Getenv(\"RPC_ADDR\")\n c := dial(address)\n client := pb.NewApiClient(c)\n client.Get(ctx,req)\n}",
    );
    put(
        d.path(),
        "client/other/helper.go",
        "package other\nfunc dial(address string) *grpc.ClientConn { c,err := grpc.NewClient(address);return c }",
    );
    assert!(
        !analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::GrpcCall)
    );
}

#[test]
fn go_raw_string_documentation_is_not_executable_client_code() {
    let d = fixture(
        r#"package main
var docs = `func example(){
 address := os.Getenv("RPC_ADDR")
 c,err := grpc.NewClient(address)
 client := pb.NewApiClient(c)
 client.Get(ctx,req)
}`
"#,
    );
    assert!(
        !analyze_services(d.path())
            .unwrap()
            .dependencies
            .iter()
            .any(|e| e.kind == Kind::GrpcCall)
    );
}
