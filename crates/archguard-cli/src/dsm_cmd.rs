//! `archguard dsm` (T40, Kap. 9): the dependency-structure matrix of
//! the module view.
//!
//! The matrix shows the same view the analysis checked: the effective
//! [`Cut::at_depth`] zoom (or, with `--within <MODUL>`, exactly the
//! direct children of that module), optionally zoomed in further with
//! `--expand a,b` on top. Text and JSON reuse the T38 renderers of
//! `archguard-report`; the reverse topological order and the cyclic-edge
//! set come from [`view_util::dsm_layout`] because the report crate must
//! not know about SCCs.

use archguard_engine::LoadedConfig;
use archguard_engine::core::{Graph, aggregate};
use archguard_engine::report::{to_dsm_json, to_dsm_text};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, emit, format_label, usage};
use crate::view_util::{dsm_layout, expand_cut, find_node, module_cut, within_cut};

/// Handler for `Command::Dsm`; `expand` and `within` are the raw flag
/// values (validated module names, unknown ones are a usage error).
pub(crate) fn run(
    cli: &Cli,
    format: Format,
    expand: &[String],
    within: Option<&str>,
    loaded: &LoadedConfig,
    graph: &Graph,
) -> i32 {
    if !matches!(format, Format::Text | Format::Json) {
        return usage(format!(
            "invalid value '{}' for '--format': dsm supports text and json",
            format_label(format)
        ));
    }
    // Validate the cut inputs before anything is printed: an unknown
    // module must not silently render an empty matrix.
    let base = match within {
        Some(name) => match find_node(graph, name) {
            Some(id) => within_cut(graph, id),
            None => {
                return usage(format!("unknown module '{name}' for '--within'"));
            }
        },
        None => module_cut(graph, loaded),
    };
    let cut = match expand_cut(graph, &base, expand) {
        Ok(cut) => cut,
        Err(name) => return usage(format!("unknown module '{name}' for '--expand'")),
    };
    if cli.quiet {
        return EXIT_OK;
    }
    let view = aggregate(graph, &cut);
    let (order, cyclic) = dsm_layout(&view);
    match format {
        Format::Json => emit(&to_dsm_json(&view, &order, &cyclic, true)),
        _ => emit(&to_dsm_text(&view, &order, &cyclic)),
    }
    EXIT_OK
}
