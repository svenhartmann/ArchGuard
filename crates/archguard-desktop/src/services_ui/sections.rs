//! Details → pattern candidates and API contracts: read once per document,
//! on the tab's worker thread.

use std::path::Path;

use archguard_engine::AnalysisRun;
use archguard_engine::core::{CallAnalysis, ServiceTopology};

use super::contracts;
use super::patterns;
use super::state::ServicesState;

/// Pattern candidates and API contracts of `run` against the tab's
/// topology; an unreadable topology counts as empty.
pub fn read(s: &mut ServicesState, root: &Path, run: &AnalysisRun, calls: &CallAnalysis) {
    let read = s.topology.clone();
    let empty = ServiceTopology::default();
    let topology = read.as_ref().as_ref().unwrap_or(&empty);
    s.patterns = patterns::Patterns::new(archguard_engine::analyze_architecture_patterns(
        root,
        &run.config,
        &run.graph,
        calls,
        topology,
    ));
    s.contracts = contracts::Contracts::new(archguard_engine::read_api_contracts(root, topology));
}
