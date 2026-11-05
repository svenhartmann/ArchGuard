//! What a tab receives after an analysis.

use std::path::Path;

use archguard_engine::AnalysisRun;
use archguard_engine::core::ReviewSources;

/// Everything a tab may need after an analysis: the project root, the run
/// (graph, report, effective config) and the review evidence (Git).
pub struct Loaded<'a> {
    pub root: &'a Path,
    pub run: &'a AnalysisRun,
    pub sources: &'a ReviewSources,
}
