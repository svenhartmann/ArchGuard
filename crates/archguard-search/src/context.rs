//! What a source reads (`SearchContext`) and what it hands back (`SourceOutput`).
use crate::findings::EsGap;
use archguard_core::{SearchArtifact, SearchIndexUsage, ServiceDiagnostic, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::Path;

/// Reads one file below the root; `None` when it cannot be read safely (no symlinks, size cap).
pub type Reader = dyn Fn(&Path, &Path) -> Option<String>;

/// Read-only input of every source. Unlike the engine's `ExtractContext` it carries the
/// discovery and the service list (code usages are per owning service), and the engine's
/// safe file reader, which cannot move into this crate (ADR 0025).
pub struct SearchContext<'a> {
    pub root: &'a Path,
    pub discovery: &'a Discovery,
    pub topology: &'a ServiceTopology,
    pub read: &'a Reader,
}

/// One source's contribution; the merge puts them together.
#[derive(Default)]
pub struct SourceOutput {
    pub artifacts: Vec<SearchArtifact>,
    pub usages: Vec<SearchIndexUsage>,
    pub diagnostics: Vec<ServiceDiagnostic>,
}

impl SourceOutput {
    /// Takes what the readers wrote into a scratch report.
    pub(crate) fn from_scratch(scratch: ServiceTopology) -> Self {
        Self {
            artifacts: scratch.elasticsearch.artifacts,
            usages: scratch.elasticsearch.usages,
            diagnostics: scratch.diagnostics,
        }
    }

    /// Saved-object notes are no gap; every other diagnostic leaves the report incomplete.
    pub(crate) fn complete(&self) -> bool {
        self.diagnostics
            .iter()
            .all(|d| EsGap::SavedObjects.matches(&d.message))
    }
}
