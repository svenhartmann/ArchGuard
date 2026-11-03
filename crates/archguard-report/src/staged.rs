use archguard_core::{BranchArchitectureDiff, StagedArchitectureDiff};
/// Offline comparison page: all labels and source text are rendered as text.
pub fn staged_diff_html(report: &StagedArchitectureDiff) -> String {
    diff_html(serde_json::to_string(report).expect("diff serializes"))
}
pub fn branch_diff_html(report: &BranchArchitectureDiff) -> String {
    diff_html(serde_json::to_string(report).expect("diff serializes"))
}
fn diff_html(data: String) -> String {
    let data = data
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    include_str!("viewer_staged_diff.html")
        .replace(
            "/*__APP__*/",
            concat!(
                include_str!("viewer_scc.js"),
                "\n",
                include_str!("viewer_package_diff.js"),
                "\n",
                include_str!("viewer_staged_diff.js")
            ),
        )
        .replace("__DIFF_DATA__", &data)
}
