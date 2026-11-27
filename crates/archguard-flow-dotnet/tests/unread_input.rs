//! An input that cannot be read is a gap, never a silent skip: the flow is incomplete.
//! Covers the attribute scan of `ui_flow_blazor` over plain `.cs` files.

use archguard_flow_dotnet::ui_flow_blazor;

/// `FF FE` announces UTF-16, the single byte after it is half a unit: `InvalidData`.
const BROKEN_UTF16: &[u8] = &[0xFF, 0xFE, 0x41];

#[test]
fn unreadable_cs_file_is_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("App.csproj"), "<Project/>").unwrap();
    std::fs::write(
        dir.path().join("Home.razor"),
        "@page \"/\"\n<h1>Home</h1>\n",
    )
    .unwrap();
    // In a project the page does not reference: only the attribute scan reads it.
    std::fs::create_dir(dir.path().join("Lib")).unwrap();
    std::fs::write(dir.path().join("Lib/Lib.csproj"), "<Project/>").unwrap();
    std::fs::write(dir.path().join("Lib/MyAuth.cs"), BROKEN_UTF16).unwrap();
    let flow = ui_flow_blazor::extract(dir.path());
    assert!(
        !flow.complete,
        "an unreadable Lib/MyAuth.cs must not leave the flow complete"
    );
    assert!(
        flow.diagnostics.iter().any(|d| d.file == "Lib/MyAuth.cs"),
        "no gap names Lib/MyAuth.cs: {:?}",
        flow.diagnostics
    );
}
