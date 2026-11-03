//! PR-comment tool for the review dashboard (R32 skeleton, R35 tool).
//!
//! The `pr_comment` tool serves the Markdown body of
//! [`archguard_engine::report::render_pr_comment`] (R25) in pages: the same
//! engine calls the CLI makes for `archguard review --format pr-comment
//! --base <ref>` (R27) — the working-tree run through [`crate::tools::analyze_run`],
//! the base side through [`archguard_engine::compare_to_ref`] on an isolated
//! base checkout, the decisions through [`archguard_engine::load_register`] —
//! delivered as contiguous line blocks under the same 4 KiB guard as the
//! sibling tools. Read-only: unlike [`crate::review_decision_tools`] this
//! tool never writes anything; it reads the register, whose writing belongs
//! to R34.

use archguard_engine::report::render_pr_comment;
use archguard_engine::{collect_review_sources, compare_to_ref, load_register};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::ArchguardTools;

/// Input of the `pr_comment` tool (R35: `{ path?, base?, offset?, limit? }`).
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PrCommentParams {
    /// Project directory inside the MCP server root.
    pub path: Option<String>,
    /// Git commit/ref the comment diffs against; required, like the CLI.
    pub base: Option<String>,
    /// First line block; follow next_offset.
    pub offset: Option<usize>,
    /// 1..=200 line blocks, further bounded by 4 KiB.
    pub limit: Option<usize>,
}

/// Consecutive source lines forming one block; blocks are the pagination
/// unit so a page boundary can only fall between rendered lines.
const BLOCK_LINES: usize = 20;

/// Reject out-of-range pages before any analysis runs.
pub fn validate(params: &PrCommentParams) -> Result<(), ErrorData> {
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    Ok(())
}

/// Split the rendered Markdown into deterministic, contiguous line blocks:
/// the document is cut at newlines and chunked into consecutive groups of
/// at most [`BLOCK_LINES`] lines, so concatenating the blocks of all pages
/// in order reproduces the renderer output byte for byte.
fn line_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split('\n')
        .collect::<Vec<&str>>()
        .chunks(BLOCK_LINES)
        .map(|chunk| chunk.join("\n"))
        .collect()
}

/// The CLI command that renders the same document in one piece (R27).
fn source_command(params: &PrCommentParams, base: &str) -> String {
    let target = params.path.as_deref().unwrap_or("<pfad>");
    format!("archguard review {target} --format pr-comment --base {base} --out")
}

/// Build the paginated `pr_comment` answer for one rendered document.
/// Blocks are strings of contiguous lines; a block that cannot fit the
/// 4 KiB budget is replaced by an explicit omission marker on its own page
/// and the cursor still advances past it.
pub fn comment_value(
    markdown: &str,
    base: &str,
    params: &PrCommentParams,
) -> Result<Value, ErrorData> {
    validate(params)?;
    let blocks = line_blocks(markdown);
    let total = blocks.len();
    let offset = params.offset.unwrap_or(0).min(total);
    let mut reply = json!({
        "status": "ok",
        "base": base,
        "source": source_command(params, base),
        "coverage_scope": "Markdown comes from render_pr_comment (R25) over the current working-tree analysis and the compare_to_ref base report computed on an isolated base checkout under the current effective policy — the same engine calls as the CLI (R27). Findings are the High/Medium priority review entries diffed by node name. A missing register is an empty register: 'Keine akzeptierten Risiken registriert.' is a fact, not an error, and nothing is invented for it. The base snapshot carries no git history, so base-side churn evidence is absent. Blocks are contiguous groups of up to 20 lines; joining page blocks in order reproduces the document.",
        "total_blocks": total, "offset": offset, "returned": 0, "processed": 0,
        "next_offset": null, "truncated": false, "blocks": [],
    });
    let mut processed = 0;
    let mut returned = 0;
    let mut omitted = false;
    for block in blocks
        .into_iter()
        .skip(offset)
        .take(params.limit.unwrap_or(crate::server::DEFAULT_LIMIT))
    {
        reply["blocks"]
            .as_array_mut()
            .unwrap()
            .push(block.clone().into());
        // Reserve cursor/count space even on the final, largest page.
        if reply.to_string().len() > crate::tools::MAX_RESPONSE_BYTES - 100 {
            reply["blocks"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            reply["blocks"].as_array_mut().unwrap().push(json!({
                "omitted": true, "index": offset + processed,
                "reason": "Block exceeds response byte budget; run the source command (--out writes .archguard/reports/pr-comment.md) for the complete Markdown"
            }));
            omitted = true;
        } else {
            returned += 1;
        }
        processed += 1;
    }
    reply["returned"] = returned.into();
    reply["processed"] = processed.into();
    reply["next_offset"] = if offset + processed < total {
        (offset + processed).into()
    } else {
        Value::Null
    };
    reply["truncated"] = (offset > 0 || offset + processed < total || omitted).into();
    Ok(reply)
}

#[tool_router(router = pr_comment_router, vis = "pub(crate)")]
impl ArchguardTools {
    /// The architecture PR comment against a Git base, paginated (R35).
    #[tool(
        description = "Render the architecture PR comment (R25 renderer) comparing a Git base against the working tree: KPI deltas, High/Medium priority findings new since base and fixed since base, plus the accepted risks from the decision register — delivered as contiguous line blocks whose concatenation reproduces the Markdown byte for byte. base is required (a Git commit/ref, like the CLI --base); an unknown ref is rejected as invalid params. Read-only: unlike review_decisions this tool never writes, it only reads the register — a missing register renders as no accepted risks, a fact, not an error, and nothing is invented for it. The base side is analysed on an isolated checkout under the current effective policy (R27), so base-side git churn evidence is absent. Blocks page with offset/limit within 4 KiB; an oversized block is omitted explicitly and the cursor advances. source names the equivalent CLI command for the whole document."
    )]
    async fn pr_comment(
        &self,
        Parameters(params): Parameters<PrCommentParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let base = crate::tools::required_str(&params.base, "base")?;
        let run = crate::tools::analyze_run(&self.root, params.path.as_deref())?;
        let config = &run.loaded.config;
        // The same pipeline the CLI pr_comment command runs (R27): current
        // priorities over the working tree, base priorities from the
        // isolated comparison checkout, register as annotation only.
        let sources =
            collect_review_sources(&run.root, config, &run.file_graph, &config.review.git_since)
                .map_err(|err| ErrorData::internal_error(format!("review sources: {err}"), None))?;
        let current = archguard_engine::analysis::analyze_review(
            &run.file_graph,
            &run.view,
            &sources,
            &config.review.entry_points,
        )
        .map_err(|err| ErrorData::internal_error(format!("review analysis: {err}"), None))?;
        // User-input problems (unknown ref, no repository) surface as -32602
        // naming the base; the engine already classes them as config errors.
        let comparison = compare_to_ref(&run.root, &run.loaded, &run.file_graph, &run.report, base)
            .map_err(|err| {
                crate::tools::invalid_params(format!("base comparison for `{base}`: {err}"))
            })?;
        let base_report = comparison.base_review.ok_or_else(|| {
            ErrorData::internal_error(
                "comparison returned no base review report".to_string(),
                None,
            )
        })?;
        let register = load_register(&run.root)
            .map_err(|err| ErrorData::internal_error(format!("review register: {err}"), None))?;
        comment_value(
            &render_pr_comment(&base_report, &current, &register),
            base,
            &params,
        )
        .map(|value| value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ArchguardTools;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ErrorCode;

    fn params(offset: Option<usize>, limit: Option<usize>) -> PrCommentParams {
        PrCommentParams {
            offset,
            limit,
            ..Default::default()
        }
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The CLI fixture (R27): base commit with the a/b cycle; the caller
    /// adds the c/d cycle uncommitted so it shows up as new.
    fn repo_with_base_cycle() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("archguard.toml"), "").expect("config");
        std::fs::create_dir(dir.path().join("src")).expect("src");
        std::fs::write(dir.path().join("src/a.ts"), "import './b';\n").expect("a");
        std::fs::write(dir.path().join("src/b.ts"), "import './a';\n").expect("b");
        git(dir.path(), &["init", "--quiet", "--template=", "."]);
        git(dir.path(), &["add", "-A"]);
        git(
            dir.path(),
            &[
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "user.name=test",
                "commit",
                "--quiet",
                "-m",
                "base",
            ],
        );
        dir
    }

    fn add_new_cycle(dir: &std::path::Path) {
        std::fs::write(dir.join("src/c.ts"), "import './d';\n").expect("c");
        std::fs::write(dir.join("src/d.ts"), "import './c';\n").expect("d");
    }

    async fn page(handler: &ArchguardTools, offset: Option<usize>, limit: Option<usize>) -> Value {
        let answer = handler
            .pr_comment(Parameters(PrCommentParams {
                base: Some("HEAD".into()),
                offset,
                limit,
                ..Default::default()
            }))
            .await
            .unwrap();
        assert!(
            answer.len() <= crate::tools::MAX_RESPONSE_BYTES,
            "every page must fit the budget"
        );
        let value: Value = serde_json::from_str(&answer).expect("valid JSON");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["base"], "HEAD");
        assert!(
            value["source"]
                .as_str()
                .unwrap()
                .contains("archguard review <pfad> --format pr-comment --base HEAD"),
            "the render command hint must be exact: {}",
            value["source"]
        );
        value
    }

    #[test]
    fn pr_comment_tool_is_registered() {
        let tools = ArchguardTools::full_router().list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "pr_comment")
            .expect("the PR comment must be discoverable over MCP");
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        for field in ["path", "base", "offset", "limit"] {
            assert!(schema["properties"].get(field).is_some(), "missing {field}");
        }
        // No query parameter sneaked in beyond the four spec fields.
        assert_eq!(schema["properties"].as_object().unwrap().len(), 4);
        let description = tool.description.as_deref().unwrap();
        assert!(
            description.to_lowercase().contains("read-only"),
            "the read-only promise must be visible: {description}"
        );
        assert!(
            description.contains("base is required"),
            "the mandatory base must be visible: {description}"
        );
    }

    #[tokio::test]
    async fn missing_base_is_invalid_params_naming_it() {
        let root = tempfile::tempdir().unwrap();
        let handler = ArchguardTools::new(root.path().into());
        let err = handler
            .pr_comment(Parameters(PrCommentParams::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("base"), "{err:?}");
        // An empty base is the same user-input error.
        let err = handler
            .pr_comment(Parameters(PrCommentParams {
                base: Some(String::new()),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        for bad in [params(None, Some(0)), params(None, Some(201))] {
            assert!(validate(&bad).is_err(), "must reject: {bad:?}");
        }
    }

    #[tokio::test]
    async fn unknown_base_is_invalid_params_naming_the_ref() {
        let root = repo_with_base_cycle();
        add_new_cycle(root.path());
        let handler = ArchguardTools::new(root.path().into());
        let err = handler
            .pr_comment(Parameters(PrCommentParams {
                base: Some("this-ref-does-not-exist".into()),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS, "{err:?}");
        assert!(err.message.contains("this-ref-does-not-exist"), "{err:?}");
    }

    #[tokio::test]
    async fn diffs_the_new_cycle_and_paginates_within_the_byte_budget() {
        let root = repo_with_base_cycle();
        add_new_cycle(root.path());
        let handler = ArchguardTools::new(root.path().into());
        let first = page(&handler, None, None).await;
        let total = first["total_blocks"].as_u64().unwrap() as usize;
        assert!(total >= 1);
        assert_eq!(first["returned"], total as u64, "fixture fits one page");
        assert_eq!(first["next_offset"], Value::Null);
        assert_eq!(first["truncated"], false);
        let full = first["blocks"].as_array().unwrap().clone();

        // Page through with one block per page: every page is within the
        // budget (asserted in `page`), the cursor advances to the end, no
        // block repeats and the concatenation is the whole document.
        let mut offset = 0;
        let mut seen = Vec::new();
        let mut unique = std::collections::BTreeSet::new();
        loop {
            let page = page(&handler, Some(offset), Some(1)).await;
            assert_eq!(page["total_blocks"], total as u64);
            assert_eq!(page["returned"], 1);
            let block = page["blocks"][0].as_str().expect("plain block");
            assert!(unique.insert(block.to_string()), "block repeated: {block}");
            seen.push(block.to_string());
            if let Some(next) = page["next_offset"].as_u64() {
                assert!(next as usize > offset, "cursor must advance");
                offset = next as usize;
            } else {
                assert_eq!(offset + 1, total, "last page is the last block");
                break;
            }
        }
        assert_eq!(seen.len(), total);
        assert_eq!(unique.len(), total, "every block appears exactly once");
        let joined = seen.join("\n");
        assert_eq!(
            joined,
            full.iter()
                .map(|b| b.as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
            "paging must reproduce the whole document"
        );
        // The R25 renderer sections the CLI golden asserts are present.
        assert!(joined.contains("## Architektur-Review"), "{joined}");
        assert!(
            joined.contains("### Neue Befunde seit base"),
            "the new-finding section is the PR payload: {joined}"
        );
        assert!(
            !joined.contains("Keine neuen Befunde gegenüber base."),
            "the uncommitted c/d cycle must show up as new: {joined}"
        );
        assert!(
            joined.contains("c.ts") && joined.contains("d.ts"),
            "{joined}"
        );

        // A path escaping the server root is rejected before any access.
        let err = handler
            .pr_comment(Parameters(PrCommentParams {
                path: Some("../outside".into()),
                base: Some("HEAD".into()),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn missing_register_renders_without_accepted_risks() {
        let root = repo_with_base_cycle();
        assert!(
            !root.path().join(".archguard/reviews.json").exists(),
            "fixture has no register"
        );
        let handler = ArchguardTools::new(root.path().into());
        let value = page(&handler, None, None).await;
        let joined = value["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("### Akzeptierte Risiken"), "{joined}");
        assert!(
            joined.contains("Keine akzeptierten Risiken registriert."),
            "a missing register is an empty register, not an error: {joined}"
        );
    }

    #[test]
    fn oversized_blocks_omit_explicitly_and_the_cursor_advances() {
        let big = "x".repeat(10_000);
        // A single oversized document: the block is omitted explicitly and
        // as the only record the cursor is exhausted (Null, honestly).
        let page = comment_value(&big, "HEAD", &params(None, Some(1))).unwrap();
        assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(page["returned"], 0);
        assert_eq!(page["processed"], 1);
        assert_eq!(page["next_offset"], Value::Null);
        assert_eq!(page["truncated"], true);
        assert_eq!(page["blocks"][0]["omitted"], true);
        assert_eq!(page["blocks"][0]["index"], 0);
        assert!(
            page["blocks"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("byte budget")
        );
        // Oversized first, small sibling after: the marker occupies the page
        // and the cursor still moves past it toward the rest. The big line
        // sits inside the first 20-line block, `klein` forms block two.
        let mut first_block = vec![big.clone()];
        first_block.extend((1..BLOCK_LINES).map(|i| format!("z{i}")));
        let doc = format!("{}\nklein", first_block.join("\n"));
        let page = comment_value(&doc, "HEAD", &params(None, Some(2))).unwrap();
        assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(page["total_blocks"], 2);
        assert_eq!(page["blocks"][0]["omitted"], true);
        assert_eq!(page["blocks"][1], "klein");
        assert_eq!(page["returned"], 1);
        assert_eq!(page["processed"], 2);
        // Small first, oversized second: the page ends after what fits and
        // points at the oversized block instead of losing it.
        let small_block = (0..BLOCK_LINES)
            .map(|i| format!("s{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let doc = format!("{small_block}\n{big}");
        let page = comment_value(&doc, "HEAD", &params(None, Some(2))).unwrap();
        assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(page["returned"], 1);
        assert_eq!(page["processed"], 1);
        assert_eq!(page["blocks"][0], small_block);
        assert_eq!(page["next_offset"], 1);
        assert_eq!(page["truncated"], true);
        // Offsets beyond the document clamp to the end without error. The
        // two lines form ONE block (blocks group up to BLOCK_LINES lines),
        // so the end is offset 1.
        let page = comment_value("a\nb", "HEAD", &params(Some(99), Some(1))).unwrap();
        assert_eq!(page["offset"], 1);
        assert_eq!(page["returned"], 0);
        assert_eq!(page["next_offset"], Value::Null);
    }
}
