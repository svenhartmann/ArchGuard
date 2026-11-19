//! `archguard report`: the shareable single-file report, the same file
//! the desktop writes with File → Export → Report… (`archguard_engine::share_report`).
use std::path::Path;

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

pub(crate) fn run(cli: &Cli, root: &Path, out: Option<&Path>) -> i32 {
    if cli.format != Format::Text {
        return usage("report writes Markdown; leave out --format");
    }
    let made = archguard_engine::load_config(cli.config.as_deref(), root).and_then(|loaded| {
        let run =
            archguard_engine::analyze_run_quiet(root, &loaded, archguard_engine::Level::Module)?;
        archguard_engine::share_report(root, &run)
    });
    let text = match made {
        Ok(text) => text,
        Err(error) => return analysis_error(error),
    };
    match out {
        Some(path) => {
            if let Err(error) = crate::insights_cmd::write_document(path, &text) {
                return analysis_error(error);
            }
            if !cli.quiet {
                outln!("Wrote {}", path.display());
            }
        }
        None if !cli.quiet => emit(&text),
        None => {}
    }
    EXIT_OK
}
