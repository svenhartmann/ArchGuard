//! The detail panel's line kinds and their constructor: each line has a
//! kind; the view holds the translated template per kind (see
//! `start_details.slint`).

use crate::ui::StartLine;

/// Line kinds shared with `start_details.slint`.
pub mod kind {
    pub const TEXT: i32 = 0;
    pub const MUTED: i32 = 1;
    pub const PATH: i32 = 2;
    pub const CLASS: i32 = 10;
    pub const FILE_ONLY: i32 = 11;
    pub const MAX_CC: i32 = 12;
    pub const MEASURED_METHODS: i32 = 13;
    pub const KPI_VALUE: i32 = 14;
    pub const ENTRIES_MEASURED: i32 = 15;
    pub const PICK_CLASS: i32 = 16;
    pub const FILE_COVERAGE: i32 = 17;
    pub const COVERAGE_UNKNOWN: i32 = 18;
    pub const FILES_MEASURED: i32 = 19;
    pub const IN_CYCLE: i32 = 20;
    pub const NO_CYCLE: i32 = 21;
    pub const FINDINGS: i32 = 22;
    pub const METHODS: i32 = 23;
    pub const METHOD: i32 = 24;
    pub const NO_KPI: i32 = 25;
    pub const METHOD_RECORDS: i32 = 26;
    pub const MEMBER: i32 = 30;
    pub const MORE: i32 = 31;
    pub const EVIDENCE: i32 = 40;
    pub const CONNECTION: i32 = 41;
    pub const NO_EVIDENCE: i32 = 42;
    /// An open connection's dependency count (`a`, "" unknown).
    pub const REFERENCES: i32 = 43;
    /// "Select neighbor" in the architecture World: `entity` the area.
    pub const NEIGHBOR: i32 = 44;
    // The services World (`services/view.rs`).
    pub const SVC_NOTE: i32 = 50;
    pub const SVC_EXTERNAL: i32 = 51;
    /// `a` → `b`, `entity` the kind (`services::KINDS`), `flag` cyclic.
    pub const SVC_LINK: i32 = 52;
    pub const SVC_AT: i32 = 53;
    /// "Select neighbor": `entity` the island.
    pub const SVC_NEIGHBOR: i32 = 54;
    pub const SVC_NONE: i32 = 55;
    // Source dependencies (`source_deps.rs`).
    /// `a` the node, `b`/`c` incoming/outgoing; `flag`: a class's file.
    pub const SRC_HEAD: i32 = 60;
    /// "Used by" (`entity` 0) or "Depends on" (1) · `a`.
    pub const SRC_DIR: i32 = 61;
    /// A neighbour `a` with `b` references: opens it in the Graph.
    pub const SRC_NODE: i32 = 62;
    pub const SRC_NONE: i32 = 63;
    // Per-file test coverage (`coverage_detail.rs`).
    pub const COV_HEAD: i32 = 70;
    /// Lines `a`, branches `b`; `c` "changed" / "unknown" freshness.
    pub const COV_FILE: i32 = 71;
    /// Review index `a`, `b` transitive dependent files.
    pub const COV_REVIEW: i32 = 72;
    pub const COV_NONE: i32 = 73;
    pub const COV_SOURCE: i32 = 74;
    pub const COV_ERROR: i32 = 75;
    // The coverage and impact review (`renderTestCoveragePanel`).
    /// The entry; `c` its section key, `open`.
    pub const COV_REVIEW_OPEN: i32 = 76;
    /// Import status `a`, `b` shown of `c` imported files.
    pub const COV_STATUS: i32 = 77;
    pub const COV_DEFINITION: i32 = 78;
    pub const COV_UNVERIFIED: i32 = 79;
    /// A ranked file `a`: opens it in the Graph.
    pub const COV_ROW: i32 = 87;
    pub const COV_IMPORT: i32 = 88;
    /// No row: `a` "not_imported" or the import status.
    pub const COV_EMPTY: i32 = 89;
    // A class's measurements (`class_detail.rs`).
    /// The rating `entity`, value `a` among `b` measured classes.
    pub const KPI_NOTE: i32 = 80;
    pub const KPI_OBSERVED: i32 = 81;
    pub const OTHER_METRICS: i32 = 82;
    /// Metric `a`: value `b`.
    pub const KPI_ROW: i32 = 83;
    /// Cyclomatic `a`, maximum nesting `b`.
    pub const METHOD_CC: i32 = 84;
    /// Cognitive `a`, MI `b`.
    pub const METHOD_MAINT: i32 = 85;
    pub const METHOD_NO_CC: i32 = 86;
}

pub fn line(
    kind: i32,
    a: impl Into<String>,
    b: impl Into<String>,
    c: impl Into<String>,
) -> StartLine {
    StartLine {
        kind,
        a: a.into().into(),
        b: b.into().into(),
        c: c.into().into(),
        entity: -1,
        flag: false,
        id: -1,
        open: false,
    }
}
