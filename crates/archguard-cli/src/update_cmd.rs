//! `archguard update`: arguments and output. The lookup, checks and install
//! live in `archguard_update::binary`.
use archguard_update::binary::{self, Outcome};
use archguard_update::release;
use serde_json::json;

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

const TARGET: &str = env!("ARCHGUARD_TARGET");
const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn run(cli: &Cli, check: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("update supports text and json");
    }
    match binary::update(&release::api_base(), TARGET, CURRENT, check) {
        Ok(outcome) => {
            report(cli.format, &outcome);
            EXIT_OK
        }
        Err(error) => analysis_error(format!("update failed: {error}")),
    }
}

fn report(format: Format, outcome: &Outcome) {
    if format == Format::Json {
        let value = json!({
            "current": CURRENT,
            "latest": outcome.latest,
            "target": TARGET,
            "update_available": outcome.update_available,
            "updated": outcome.updated,
        });
        emit(&serde_json::to_string_pretty(&value).expect("update report serializes"));
    } else if outcome.updated {
        println!("Updated archguard {CURRENT} → {}.", outcome.latest);
    } else if outcome.update_available {
        println!(
            "archguard {} is available (installed: {CURRENT}). Run `archguard update` to install it.",
            outcome.latest
        );
    } else {
        println!(
            "archguard {CURRENT} is up to date (latest release: {}).",
            outcome.latest
        );
    }
}
