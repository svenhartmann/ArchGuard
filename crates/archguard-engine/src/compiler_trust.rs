//! Whether ArchGuard may run a project's own compiler tooling (rust-analyzer,
//! the project's TypeScript compiler), which reads configuration from the
//! repository. Off by default; turned on outside the repository: by the
//! operator with `ARCHGUARD_TRUST_PROJECT_COMPILER=1`, or by an embedding
//! application with [`trust_project_compiler`], which counts only where the
//! tooling runs confined (ADR 0007).

use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const TRUST_ENV: &str = "ARCHGUARD_TRUST_PROJECT_COMPILER";

static TRUSTED: AtomicBool = AtomicBool::new(false);

/// Trust (or stop trusting) project compilers for this process.
pub fn trust_project_compiler(on: bool) {
    TRUSTED.store(on, Ordering::Relaxed);
}

pub(crate) fn trusted() -> bool {
    let confined = TRUSTED.load(Ordering::Relaxed) && crate::compiler_sandbox::available();
    confined || std::env::var_os(TRUST_ENV).is_some_and(|v| v == "1")
}
