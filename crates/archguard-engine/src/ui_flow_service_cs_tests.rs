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
    let ctl = format!(
        "using Microsoft.AspNetCore.Mvc;\nusing Microsoft.AspNetCore.Authorization;\nusing System.Threading.Tasks;\nnamespace App;\n{controller_attr}\n[ApiController, Route(\"api/docs\")]\npublic class DocsController : ControllerBase {{\n  private readonly IDocs _docs;\n  public DocsController(IDocs d) {{ _docs = d; }}\n  [HttpGet(\"{{id}}\")] public async Task<IActionResult> Get(int id) {{ await _docs.Read(id); return Ok(); }}\n}}\n"
    );
    let svc = format!(
        "using Microsoft.AspNetCore.Authorization;\nusing System.Threading.Tasks;\nnamespace App;\npublic interface IDocs {{ Task<bool> Read(int id); }}\n{docs}\n"
    );
    project(&[("Ctl.cs", ctl.into_bytes()), ("Svc.cs", svc.into_bytes())])
}

/// The flow of a project made of `files` (plus the csproj).
fn project(files: &[(&str, Vec<u8>)]) -> Option<UiFlow> {
    if !dotnet() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("App.csproj"), CSPROJ).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }
    Some(crate::ui_flow::analyze(dir.path()).unwrap())
}

/// A controller with the `actions`, holding the concrete `Docs`.
fn controller(actions: &str) -> (&'static str, Vec<u8>) {
    let text = format!(
        "using Microsoft.AspNetCore.Mvc;\nusing System.Threading.Tasks;\nnamespace App;\n[ApiController, Route(\"api/docs\")]\npublic class DocsController : ControllerBase {{\n  private readonly Docs _docs;\n  public DocsController(Docs d) {{ _docs = d; }}\n{actions}\n}}\n"
    );
    ("Ctl.cs", text.into_bytes())
}

const USINGS: &str = "using Microsoft.AspNetCore.Authorization;\nusing System;\nusing System.Threading;\nusing System.Threading.Tasks;\nnamespace App;\n";

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

/// The gaps of this reader (other readers add their own, e.g. for `AuthorizeFilter`).
fn service_gaps(flow: &UiFlow) -> Vec<String> {
    (gaps(flow).into_iter())
        .filter(|g| {
            g.starts_with("Check of")
                || g.contains("AuthorizeAsync")
                || g.contains("authorization check")
        })
        .collect()
}

fn no_gap_no_layer(flow: &UiFlow) {
    assert!(layers(flow).is_empty(), "{:#?}", flow.service_layers);
    assert!(service_gaps(flow).is_empty(), "{:?}", service_gaps(flow));
}

#[test]
fn a_cs_file_that_cannot_be_read_is_a_gap() {
    let Some(flow) = project(&[
        controller("  [HttpGet(\"{id}\")] public IActionResult Get(int id) { return Ok(); }"),
        // over the 1 MiB limit of `read_source`
        ("Big.cs", "// padding\n".repeat(120_000).into_bytes()),
    ]) else {
        return;
    };
    assert!(!flow.complete);
    assert!(
        gaps(&flow).iter().any(|g| g.contains("File not read")),
        "{:?}",
        gaps(&flow)
    );
}

#[test]
fn more_than_forty_layers_of_an_endpoint_leave_a_gap() {
    let methods: String = (0..41)
        .map(|i| {
            format!(
                "  [Authorize] public Task<bool> M{i}(int id) {{ return Task.FromResult(true); }}\n"
            )
        })
        .collect();
    let calls: String = (0..41).map(|i| format!("await _docs.M{i}(id); ")).collect();
    let Some(flow) = project(&[
        controller(&format!(
            "  [HttpGet(\"{{id}}\")] public async Task<IActionResult> Get(int id) {{ {calls} return Ok(); }}"
        )),
        (
            "Svc.cs",
            format!("{USINGS}public class Docs {{\n{methods}}}\n").into_bytes(),
        ),
    ]) else {
        return;
    };
    assert_eq!(layers(&flow).len(), 40);
    assert!(!flow.complete);
    assert!(
        gaps(&flow).iter().any(|g| g.contains("40")),
        "{:?}",
        gaps(&flow)
    );
}

#[test]
fn a_handler_that_cannot_be_matched_is_a_gap() {
    // two overloads on one line: equally far from the endpoint's line
    let Some(flow) = project(&[
        controller("  [HttpGet(\"a\")] public IActionResult Get(int id) { return Ok(); } [HttpGet(\"b\")] public IActionResult Get(string id) { return Ok(); }"),
        ("Svc.cs", format!("{USINGS}public class Docs {{ [Authorize] public Task<bool> M() {{ return Task.FromResult(true); }} }}\n").into_bytes()),
    ]) else {
        return;
    };
    assert!(!flow.complete);
    assert!(
        gaps(&flow).iter().any(|g| g.contains("handler")),
        "{:?}",
        gaps(&flow)
    );
}

#[test]
fn a_project_method_named_authorize_async_is_no_check() {
    let Some(flow) = run(
        "",
        "public class Policy { public Task<bool> AuthorizeAsync(int id) { return Task.FromResult(true); } }\npublic class Docs : IDocs {\n  private readonly Policy _p = new Policy();\n  public async Task<bool> Read(int id) { return await _p.AuthorizeAsync(id); }\n}",
    ) else {
        return;
    };
    no_gap_no_layer(&flow);
}

#[test]
fn authorize_lookalike_attributes_are_no_check() {
    let Some(flow) = run(
        "",
        "public class AuthorizeFilterAttribute : Attribute { }\npublic class MyAuthorizeAttribute : Attribute { }\npublic class Docs : IDocs {\n  [AuthorizeFilter]\n  [MyAuthorize]\n  public Task<bool> Read(int id) { return Task.FromResult(true); }\n}",
    ) else {
        return;
    };
    no_gap_no_layer(&flow);
}

#[test]
fn a_mention_of_authorize_near_an_unannotated_method_is_no_gap() {
    // `internal` without attributes is not a declaration of the reader; the field above only
    // contains the word
    let Some(flow) = project(&[
        controller("  [HttpGet(\"{id}\")] public async Task<IActionResult> Get(int id) { await _docs.Check(id); return Ok(); }"),
        (
            "Svc.cs",
            format!("{USINGS}public interface IMyAuthorizer {{ }}\npublic class Docs {{\n  private readonly IMyAuthorizer _a;\n  public Docs() {{ }}\n  // TODO: Authorize this later\n  internal Task<bool> Check(int id) {{ return Task.FromResult(true); }}\n}}\n").into_bytes(),
        ),
    ]) else {
        return;
    };
    no_gap_no_layer(&flow);
}

#[test]
fn a_wrapped_signature_of_the_house_formatter_is_matched() {
    let Some(flow) = project(&[
        controller("  [HttpGet(\"{id}\")] public async Task<IActionResult> Get(int id) { await _docs.Read<int>(id, default); return Ok(); }"),
        (
            "Svc.cs",
            format!("{USINGS}public class Docs {{\n    [Authorize(Roles = \"A\")]\n    [ProducesResponseType(200)]\n    public async Task<bool> Read<TKey>(\n        TKey id,\n        CancellationToken cancellationToken)\n        where TKey : struct\n    {{\n        return await Task.FromResult(true);\n    }}\n}}\n").into_bytes(),
        ),
    ]) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:#?} {:?}", gaps(&flow));
    assert_eq!((l[0].0, l[0].1.as_str()), (ServiceHow::Hint, "Docs.Read"));
    assert!(service_gaps(&flow).is_empty(), "{:?}", service_gaps(&flow));
}
