//! Tests of the C# service layers; the call model is the real Roslyn helper (skipped without a .NET SDK).
use archguard_core::{DiagnosticSeverity, ServiceHow, UiFlow};

fn dotnet() -> bool {
    let ok = std::process::Command::new("dotnet")
        .arg("--list-sdks")
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    if !ok {
        eprintln!("skipped: no .NET SDK on PATH");
    }
    ok
}

const CSPROJ: &str = "<Project Sdk=\"Microsoft.NET.Sdk.Web\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>";

/// The flow of a project with the controller calling `IDocs` and `docs` as the service file.
fn run(controller_attr: &str, docs: &str) -> Option<UiFlow> {
    if !dotnet() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let ctl = format!(
        "using Microsoft.AspNetCore.Mvc;\nusing Microsoft.AspNetCore.Authorization;\nusing System.Threading.Tasks;\nnamespace App;\n{controller_attr}\n[ApiController, Route(\"api/docs\")]\npublic class DocsController : ControllerBase {{\n  private readonly IDocs _docs;\n  public DocsController(IDocs d) {{ _docs = d; }}\n  [HttpGet(\"{{id}}\")] public async Task<IActionResult> Get(int id) {{ await _docs.Read(id); return Ok(); }}\n}}\n"
    );
    let svc = format!(
        "using Microsoft.AspNetCore.Authorization;\nusing System.Threading.Tasks;\nnamespace App;\npublic interface IDocs {{ Task<bool> Read(int id); }}\n{docs}\n"
    );
    for (name, text) in [
        ("App.csproj", CSPROJ.to_string()),
        ("Ctl.cs", ctl),
        ("Svc.cs", svc),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    Some(crate::ui_flow::analyze(dir.path()).unwrap())
}

const IMPERATIVE: &str = "public class Docs : IDocs {\n  private readonly IAuthorizationService _auth;\n  public Docs(IAuthorizationService a) { _auth = a; }\n  public async Task<bool> Read(int id) {\n    var r = await _auth.AuthorizeAsync(null, id, \"Read\");\n    return r.Succeeded;\n  }\n}";
const PLAIN: &str = "public class Docs : IDocs {\n  public Task<bool> Read(int id) { return Task.FromResult(true); }\n}";

fn layers(flow: &UiFlow) -> Vec<(ServiceHow, String, String)> {
    assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
    flow.service_layers_of(&flow.endpoints[0])
        .map(|l| (l.how, l.target.clone(), l.note.clone()))
        .collect()
}
fn gaps(flow: &UiFlow) -> Vec<String> {
    (flow.diagnostics.iter())
        .filter(|d| d.severity == DiagnosticSeverity::Gap)
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn imperative_authorize_async_is_a_hint_with_a_gap() {
    let Some(flow) = run("", IMPERATIVE) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:#?}");
    assert_eq!((l[0].0, l[0].1.as_str()), (ServiceHow::Hint, "Docs.Read"));
    assert!(l[0].2.contains("AuthorizeAsync"), "{}", l[0].2);
    assert!(!flow.complete);
    assert!(
        gaps(&flow).iter().any(|g| g.contains("outcome")),
        "{:?}",
        gaps(&flow)
    );
}

#[test]
fn authorize_on_a_service_class_is_a_hint_only() {
    let Some(flow) = run("", &format!("[Authorize]\n{PLAIN}")) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:#?}");
    assert_eq!((l[0].0, l[0].1.as_str()), (ServiceHow::Hint, "Docs.Read"));
    assert!(l[0].2.contains("only on controllers"), "{}", l[0].2);
}

#[test]
fn authorize_on_a_service_method_is_a_hint_only() {
    let Some(flow) = run(
        "",
        "public class Docs : IDocs {\n  [Authorize(Roles = \"A\")]\n  public Task<bool> Read(int id) { return Task.FromResult(true); }\n}",
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:#?}");
    assert_eq!(l[0].0, ServiceHow::Hint);
}

#[test]
fn an_inherited_method_is_read_in_its_base_class_without_a_gap() {
    let Some(flow) = run(
        "",
        "public class DocsBase : IDocs {\n  [Authorize]\n  public virtual Task<bool> Read(int id) { return Task.FromResult(true); }\n}\npublic class Docs : DocsBase { }",
    ) else {
        return;
    };
    assert_eq!(layers(&flow).len(), 1, "{:#?}", flow.service_layers);
    assert!(gaps(&flow).is_empty(), "{:?}", gaps(&flow));
}

#[test]
fn no_check_below_the_endpoint_gives_no_layer() {
    let Some(flow) = run("", PLAIN) else {
        return;
    };
    assert!(layers(&flow).is_empty());
}

#[test]
fn authorize_on_the_controller_is_no_service_layer() {
    let Some(flow) = run("[Authorize]", PLAIN) else {
        return;
    };
    assert!(layers(&flow).is_empty(), "{:#?}", flow.service_layers);
}
