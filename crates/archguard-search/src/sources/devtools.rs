//! Kibana Dev Tools text (ticket 33): files `*.json`, `*.txt`, `*.console` that are no JSON but
//! hold console requests (`PUT idx` from column 0, JSON body below; `http_parse::is_console`).
//! Several requests per file, whole-line `#` and `//` comments and `"""` strings in the body are
//! read. Path -> artifact is the one mapping of `http.rs` (`Scan`); `scope` = `devtools:<folder>`,
//! evidence = the request line. A body that is no JSON and a name with a variable are gaps; an
//! unreadable `.console` file is a gap. Host URLs stay with `http.rs`, prose and valid JSON
//! create nothing; `json_files` leaves recognized console files alone. Trust boundary: files
//! below the root through the safe reader, at most 5 000 files, no test paths, nothing executed.
use super::http::Scan;
use super::http_parse::{console_requests, is_console};
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::ServiceTopology;
use archguard_flow_common::support::{relative, test_path};

const MAX_FILES: usize = 5_000;
const SKIPPED_DIRS: &[&str] = &[".terraform", ".git", "node_modules"];

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        es_gap(&mut out, EsGap::DefinitionScanStop, ".", None, &[]);
    }
    for rel in files {
        let Some(text) = (ctx.read)(ctx.root, &ctx.root.join(&rel)) else {
            if rel.ends_with(".console") {
                es_gap(&mut out, EsGap::DefinitionUnread, &rel, None, &[]);
            }
            continue;
        };
        if serde_json::from_str::<serde_json::Value>(&text).is_ok() || !is_console(&text) {
            continue;
        }
        let mut scan = Scan::new(ctx, &rel, &mut out, "devtools");
        console_requests(&text).iter().for_each(|r| scan.request(r));
    }
    SourceOutput::from_scratch(out)
}

fn candidates(ctx: &SearchContext) -> Vec<String> {
    let mut files: Vec<String> = ctx
        .discovery
        .walk(ctx.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(ctx.root, e.path()))
        .filter(|rel| {
            [".json", ".txt", ".console"]
                .iter()
                .any(|e| rel.ends_with(e))
        })
        .filter(|rel| !test_path(rel) && !rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)))
        .collect();
    files.sort();
    files
}
