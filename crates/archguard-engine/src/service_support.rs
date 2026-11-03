//! Helpers shared by the service topology scanners.
use std::path::Path;

use archguard_core::{ServiceDiagnostic, ServiceTopology};

pub(super) fn diagnostic(
    report: &mut ServiceTopology,
    file: &str,
    service: Option<&str>,
    message: impl Into<String>,
) {
    report.complete = false;
    report.diagnostics.push(ServiceDiagnostic {
        file: file.into(),
        service: service.map(str::to_owned),
        message: message.into(),
    });
}
pub(super) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
