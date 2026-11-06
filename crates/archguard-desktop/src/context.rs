//! What a tab receives after an analysis.

use std::path::{Path, PathBuf};

use archguard_engine::AnalysisRun;
use archguard_engine::core::ReviewSources;

/// Everything a tab may need after an analysis: the project root, the run
/// (graph, report, effective config) and the review evidence (Git).
pub struct Loaded<'a> {
    pub root: &'a Path,
    pub run: &'a AnalysisRun,
    pub sources: &'a ReviewSources,
}

/// A document's finished analysis, kept so that switching documents can
/// show it again without analysing.
pub struct Analysis {
    pub root: PathBuf,
    pub run: AnalysisRun,
    pub sources: ReviewSources,
}

impl Analysis {
    pub fn loaded(&self) -> Loaded<'_> {
        Loaded {
            root: &self.root,
            run: &self.run,
            sources: &self.sources,
        }
    }
}

/// True for off-screen snapshot runs, which must not touch AppKit.
pub fn headless_run() -> bool {
    #[cfg(feature = "snapshot")]
    return crate::headless::active();
    #[cfg(not(feature = "snapshot"))]
    false
}
