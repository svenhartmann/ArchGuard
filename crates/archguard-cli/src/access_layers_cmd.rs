//! `archguard access-layers`: the access chain per endpoint (LOGIC.md section 7, item 8).

use std::path::Path;

use archguard_engine::core::{
    ChainFrom, ChainRow, ServiceLayer, access_chain, layer_text, state_text,
};

use crate::args::{AccessFrom as From, Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

pub(crate) fn run(cli: &Cli, root: &Path, from: From, only: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("access-layers supports --format text|json");
    }
    let loaded = match archguard_engine::load_config(cli.config.as_deref(), root) {
        Ok(loaded) => loaded,
        Err(error) => return analysis_error(error),
    };
    let flow = match crate::ui_flow_cmd::extract(cli, root, archguard_engine::ui_flow::analyze_with)
    {
        Ok(flow) => flow,
        Err(error) => return analysis_error(error),
    };
    let from = match from {
        From::Endpoint => ChainFrom::Endpoint,
        From::Screen => ChainFrom::Screen,
    };
    let rows = access_chain(
        &flow,
        from,
        only,
        loaded.config.views.access_layers.show_unguarded,
    );
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        emit(&serde_json::to_string_pretty(&rows).expect("rows serialize"));
    } else {
        emit(&text(&rows));
    }
    EXIT_OK
}

fn layers(items: &[ServiceLayer]) -> String {
    if items.is_empty() {
        return "-".into();
    }
    items.iter().map(layer_text).collect::<Vec<_>>().join("; ")
}

/// One block per row; `service-only` marks protection below the endpoint.
pub(crate) fn text(rows: &[ChainRow]) -> String {
    if rows.is_empty() {
        return "no endpoints".into();
    }
    rows.iter()
        .map(|r| {
            let screens = if r.screens.is_empty() {
                "-".to_string()
            } else {
                (r.screens.iter())
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "{} {}{}\n  screens:   {screens}\n  endpoint:  {} ({}:{})\n  service:   {}\n  data:      {}\n  effective: {}",
                r.method,
                r.path,
                if r.service_guarded {
                    "  [service-only]"
                } else {
                    ""
                },
                state_text(&r.access),
                r.file,
                r.line,
                layers(&r.services),
                layers(&r.data),
                state_text(&r.effective),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{AccessState, EnforcedAt};

    #[test]
    fn undeclared_never_reads_as_open() {
        let row = ChainRow {
            method: "GET".into(),
            path: "/a".into(),
            application: None,
            file: "C.java".into(),
            line: 1,
            screens: vec![],
            access: AccessState::Undeclared,
            effective: AccessState::Undeclared,
            services: vec![],
            data: vec![],
            enforced_at: EnforcedAt::None,
            service_guarded: false,
        };
        let t = text(&[row]);
        assert!(
            t.contains("effective: undeclared") && !t.contains("open"),
            "{t}"
        );
    }
}
