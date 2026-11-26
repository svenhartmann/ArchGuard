//! Stub (red): reads nothing yet.
use crate::context::{SearchContext, SourceOutput};
use archguard_core::ServiceTopology;
use archguard_terraform::module::Resource;

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut scratch = ServiceTopology::default();
    archguard_terraform::module::walk(
        ctx.root,
        ctx.discovery,
        &mut scratch,
        ctx.read,
        &mut augment,
    );
    SourceOutput::from_scratch(scratch)
}

fn augment(_report: &mut ServiceTopology, _scope: &str, _resources: &[Resource]) {}
