//! An input that cannot be read is a gap, never a silent skip: the flow is incomplete.
//! Covers `ui_flow_apps` (`pom.xml` modules) and `ui_flow_backend` (`.cshtml` pages,
//! the Quarkus Spring build probe, the `.csproj` target framework).

use archguard_core::UiFlow;
use archguard_flow_backend::ui_flow_backend;

/// `FF FE` announces UTF-16, the single byte after it is half a unit: `InvalidData`.
const BROKEN_UTF16: &[u8] = &[0xFF, 0xFE, 0x41];

const JAVA: &[u8] =
    b"@RestController public class C { @GetMapping(\"/a\") public String a() { return \"\"; } }\n";
const CS: &[u8] = b"[ApiController][Route(\"api/o\")]\npublic class OrdersController : ControllerBase {\n    [HttpGet] public IActionResult Get() => Ok();\n}\n";

fn extract(files: &[(&str, &[u8])]) -> UiFlow {
    let dir = tempfile::tempdir().unwrap();
    for (rel, bytes) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    ui_flow_backend::extract(dir.path())
}

fn assert_gap(flow: &UiFlow, file: &str) {
    assert!(
        !flow.complete,
        "an unreadable {file} must not leave the flow complete"
    );
    assert!(
        flow.diagnostics.iter().any(|d| d.file == file),
        "no gap names {file}: {:?}",
        flow.diagnostics
    );
}

#[test]
fn unreadable_pom_is_a_gap() {
    assert_gap(
        &extract(&[("pom.xml", BROKEN_UTF16), ("C.java", JAVA)]),
        "pom.xml",
    );
}

#[test]
fn unreadable_gradle_build_is_a_gap() {
    assert_gap(
        &extract(&[("build.gradle", BROKEN_UTF16), ("C.java", JAVA)]),
        "build.gradle",
    );
}

#[test]
fn unreadable_cshtml_page_is_a_gap() {
    let files = [
        ("App.csproj", &b"<Project/>"[..]),
        ("O.cs", CS),
        ("Pages/Index.cshtml", BROKEN_UTF16),
    ];
    assert_gap(&extract(&files), "Pages/Index.cshtml");
}

#[test]
fn unreadable_csproj_is_a_gap() {
    assert_gap(
        &extract(&[("App.csproj", BROKEN_UTF16), ("O.cs", CS)]),
        "App.csproj",
    );
}
