//! ArchGuard's own requests in the desktop app (ADR 0004, `ARCHGUARD_PROBE=1`):
//! building a tab, showing a built one again, and every action on a tab.
//! The probe starts with the first build or project analysis and records
//! into that project's `.archguard/runtime/`; before it starts, spans cost
//! next to nothing and record nothing.
use std::path::Path;
use std::sync::OnceLock;

use archguard_engine::self_trace;

/// The tab name of a tab state type: `crate::quality_ui::QualityState` → `quality`.
pub(crate) fn tab_name<T>() -> &'static str {
    let module = std::any::type_name::<T>()
        .split("::")
        .nth(1)
        .unwrap_or("tab");
    module.strip_suffix("_ui").unwrap_or(module)
}

/// `work` as one request `name`, starting the probe for `root` if needed.
pub(crate) fn traced<T, W>(root: &Path, name: &str, work: W) -> impl FnOnce() -> T + Send + 'static
where
    W: FnOnce() -> T + Send + 'static,
{
    static PROBE: OnceLock<Option<self_trace::Probe>> = OnceLock::new();
    PROBE.get_or_init(|| self_trace::start(root, "archguard-desktop"));
    let span = self_trace::entry_span(name);
    move || span.in_scope(work)
}

/// Run `f` now as one request `name` (UI thread: navigation, tab actions).
pub(crate) fn request<R>(name: &str, f: impl FnOnce() -> R) -> R {
    self_trace::entry_span(name).in_scope(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo;

    #[test]
    fn tab_names_come_from_the_module() {
        assert_eq!(tab_name::<crate::quality_ui::QualityState>(), "quality");
        assert_eq!(tab_name::<Demo>(), "tab_support");
    }
}
