//! `archguard simulate` (T40, Kap. 9): virtual refactorings on the
//! module view — the CLI twin of the analysis crate's `simulate` and of
//! the MCP `simulate` tool (Kap. 10.2).
//!
//! Ops come from the flags (`--cut A,B`, `--move X:PARENT`,
//! `--merge A,B`, each repeatable) and from `--ops DATEI`, a JSON array
//! in the frozen T37 wire format of
//! [`archguard_analysis::simulate::SimOp`] — the format the T39 viewer's
//! "Export SimOps" button writes. Flag ops run first, file ops are
//! appended in file order. The printed [`SimDelta`] is the diff of the
//! before/after analyses; the graph on disk is never touched.

use std::path::Path;

use archguard_engine::LoadedConfig;
use archguard_engine::analysis::SimOp;
use archguard_engine::analysis::simulate;
use archguard_engine::core::Graph;

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, format_label, usage};
use crate::view_util::{effective_layers, module_cut};

/// Handler for `Command::Simulate`; the flag vectors hold the raw
/// `VON,BIS` / `KNOTEN:ELTERN` strings, `ops_file` is `--ops`.
#[expect(clippy::too_many_arguments, reason = "one argument per CLI flag")]
pub(crate) fn run(
    cli: &Cli,
    format: Format,
    cuts: &[String],
    moves: &[String],
    merges: &[String],
    ops_file: Option<&Path>,
    loaded: &LoadedConfig,
    graph: &Graph,
) -> i32 {
    if !matches!(format, Format::Text | Format::Json) {
        return usage(format!(
            "invalid value '{}' for '--format': simulate supports text and json",
            format_label(format)
        ));
    }
    // Flag ops first, then the file ops (Kap. 9: "--ops Datei …
    // verschmilzt mit den Flag-Ops").
    let mut ops: Vec<SimOp> = Vec::new();
    for spec in cuts {
        let Some((from, to)) = split_pair(spec, ',', "--cut") else {
            return usage(format!("invalid --cut '{spec}': expected FROM,TO"));
        };
        ops.push(SimOp::CutEdge { from, to });
    }
    for spec in moves {
        let Some((node, new_parent)) = split_pair(spec, ':', "--move") else {
            return usage(format!("invalid --move '{spec}': expected NODE:PARENT"));
        };
        ops.push(SimOp::MoveNode { node, new_parent });
    }
    for spec in merges {
        let Some((survivor, absorbed)) = split_pair(spec, ',', "--merge") else {
            return usage(format!(
                "invalid --merge '{spec}': expected SURVIVOR,ABSORBED"
            ));
        };
        ops.push(SimOp::Merge { survivor, absorbed });
    }
    if let Some(path) = ops_file {
        match read_ops(path) {
            Ok(file_ops) => ops.extend(file_ops),
            Err(err) => return analysis_error(err),
        }
    }
    if ops.is_empty() {
        return usage("no ops: use --cut, --move, --merge or --ops DATEI");
    }

    let cut = module_cut(graph, loaded);
    let view = archguard_engine::core::aggregate(graph, &cut);
    let layers = effective_layers(&view, loaded);
    let delta = match simulate(graph, &cut, &ops, &layers) {
        Ok(delta) => delta,
        Err(err) => return analysis_error(err),
    };
    if cli.quiet {
        return EXIT_OK;
    }
    match format {
        Format::Json => emit(
            &serde_json::to_string_pretty(&delta)
                .expect("SimDelta is plain data and always serializable"),
        ),
        _ => {
            println!("resolved:   {}", join_ids(&delta.resolved_findings));
            println!("introduced: {}", join_ids(&delta.introduced_findings));
            println!("ACD: {:.3} -> {:.3}", delta.acd_before, delta.acd_after);
            println!(
                "SCCs: {} -> {}",
                delta.scc_count_before, delta.scc_count_after
            );
        }
    }
    EXIT_OK
}

/// Split a `a<sep>b` flag value into two non-empty owned parts.
fn split_pair(spec: &str, sep: char, _flag: &str) -> Option<(String, String)> {
    let mut parts = spec.splitn(2, sep);
    let (Some(first), Some(second)) = (parts.next(), parts.next()) else {
        return None;
    };
    if first.is_empty() || second.is_empty() {
        return None;
    }
    Some((first.to_string(), second.to_string()))
}

/// Read and parse the `--ops` JSON array (frozen T37 shape); IO and
/// parse problems surface as displayable errors for exit 3, mirroring
/// `read_baseline`.
fn read_ops(path: &Path) -> Result<Vec<SimOp>, std::io::Error> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| std::io::Error::other(format!("ops {}: {err}", path.display())))?;
    serde_json::from_str(&text).map_err(|err| {
        std::io::Error::other(format!(
            "ops {}: invalid SimOps JSON (expected the T37 array shape, e.g. \
             [{{\"cut_edge\":{{\"from\":\"a\",\"to\":\"b\"}}}}]): {err}",
            path.display()
        ))
    })
}

/// Finding-id list as one readable field.
fn join_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "-".to_string()
    } else {
        ids.join(", ")
    }
}
