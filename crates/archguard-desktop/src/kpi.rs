//! Every key figure the tabs show, defined once: its label (which names
//! the level it is counted on), the note shown as its help text, its
//! precision; an unknown value reads as `—`, never `0`. Tabs render
//! figures through [`Kpi::item`]; the values stay as the analyses compute
//! them.

use crate::format::{Locale, UNKNOWN};
use crate::tab_support::Texts;
use crate::ui::KpiItem;

/// How many decimals a figure has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    Int,
    Dec(usize),
}

/// One figure's value as an analysis delivers it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Count(usize),
    Real(f64),
    /// An observed lower bound (truncated evidence).
    AtLeast(usize),
    Unknown,
}

impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Value::Count(n)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Unknown, Into::into)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Kpi {
    pub id: &'static str,
    /// Template key of the label (`ui/texts.slint`).
    pub label: &'static str,
    /// Template key of the scope note, shown as the figure's help text.
    pub note: &'static str,
    pub precision: Precision,
}

const fn kpi(id: &'static str, label: &'static str, note: &'static str) -> Kpi {
    Kpi {
        id,
        label,
        note,
        precision: Precision::Int,
    }
}

pub const SHOWN_MODULES: Kpi = kpi(
    "graph.shown_modules",
    "Shown modules",
    "Rows of the tree that are not expanded: the level the Graph's figures are counted on.",
);
pub const SHOWN_DEPENDENCIES: Kpi = kpi(
    "graph.dependencies",
    "Dependencies (shown level)",
    "Directed dependencies between the shown modules; dependencies inside one module are not counted.",
);
pub const SHOWN_CYCLE_GROUPS: Kpi = kpi(
    "graph.cycle_groups",
    "Cycle groups (shown level)",
    "Groups of shown modules that depend on each other in a cycle. Expanding a package can reveal cycles inside it.",
);
pub const SHOWN_ACD: Kpi = Kpi {
    precision: Precision::Dec(2),
    ..kpi(
        "graph.acd",
        "ACD (shown level)",
        "Average component dependency (Lakos) of the shown modules: how many shown modules one depends on, itself included, on average.",
    )
};
// Graph → More metrics: the review summary of the shown modules.
pub const SHOWN_CCD: Kpi = kpi(
    "graph.ccd",
    "CCD (shown level)",
    "Cumulative component dependency (Lakos) of the shown modules: for every shown module, the shown modules it reaches, itself included, summed up. Structural coupling, not measured compilation time.",
);
pub const SHOWN_NCCD: Kpi = Kpi {
    precision: Precision::Dec(2),
    ..kpi(
        "graph.nccd",
        "NCCD (shown level)",
        "Normalized CCD: CCD divided by (N + 1) × log2(N + 1) − N, the balanced-tree reference for N shown modules. Dimensionless; a comparison of structure, not a runtime or build-time measurement.",
    )
};
pub const LARGEST_CYCLE_GROUP: Kpi = kpi(
    "graph.largest_cycle_group",
    "Largest cycle group",
    "Shown modules in the largest group that depends on itself in a cycle; each member reaches every other. Size shows structural entanglement.",
);
pub const MODULES_IN_CYCLES: Kpi = kpi(
    "graph.modules_in_cycles",
    "Modules in cycles",
    "Shown modules that belong to any cycle group. Package cycles and file cycles can differ.",
);
pub const HIGH_PRIORITY: Kpi = kpi(
    "graph.high_priority",
    "High review priority",
    "Shown modules with a review priority of at least 40 of 100: a transparent heuristic from cycles, coupling, impact and Git change touches, not a probability of defects.",
);
pub const MODULE_ACD: Kpi = Kpi {
    precision: Precision::Dec(2),
    ..kpi(
        "arc42.acd",
        "ACD (module level)",
        "Average component dependency (Lakos) of the modules at depth 2, as the arc42 quality chapter computes it.",
    )
};
pub const FILE_CYCLE_GROUPS: Kpi = kpi(
    "matrix.file_cycle_groups",
    "File cycle groups",
    "Groups of source files that depend on each other in a cycle.",
);
pub const MATRIX_FILES: Kpi = kpi("matrix.files", "Files", "Source files in the matrix scope.");
pub const DEPENDENCY_EVIDENCE: Kpi = kpi(
    "matrix.evidence",
    "Dependency evidence records",
    "Recorded import and reference sites between files; ≥ marks counts cut off by the analysis.",
);
pub const AREA_CYCLE_GROUPS: Kpi = kpi(
    "start.area_cycle_groups",
    "Area cycle groups",
    "Groups of the shown areas that depend on each other in a cycle.",
);
pub const PACKAGE_CYCLES: Kpi = kpi(
    "architecture.package_cycles",
    "Package cycles",
    "Groups of the diagram's packages that depend on each other in a cycle (package projection).",
);
/// All figures, for the registry's own checks.
#[cfg(test)]
pub const ALL: [&Kpi; 19] = [
    &SHOWN_MODULES,
    &SHOWN_DEPENDENCIES,
    &SHOWN_CYCLE_GROUPS,
    &SHOWN_ACD,
    &SHOWN_CCD,
    &SHOWN_NCCD,
    &LARGEST_CYCLE_GROUP,
    &MODULES_IN_CYCLES,
    &HIGH_PRIORITY,
    &MODULE_ACD,
    &FILE_CYCLE_GROUPS,
    &MATRIX_FILES,
    &DEPENDENCY_EVIDENCE,
    &AREA_CYCLE_GROUPS,
    &PACKAGE_CYCLES,
    &SECURITY_SCANNED,
    &SECURITY_AFFECTED,
    &SECURITY_FINDINGS,
    &SECURITY_TARGETS,
];

pub const SECURITY_SCANNED: Kpi = kpi(
    "security.packages_scanned",
    "Packages scanned",
    "Dependency packages the saved scan checked; — until a scan ran.",
);
pub const SECURITY_AFFECTED: Kpi = kpi(
    "security.affected_packages",
    "Affected packages",
    "Scanned packages with at least one known vulnerability; — until a current scan is available.",
);
pub const SECURITY_FINDINGS: Kpi = kpi(
    "security.findings",
    "Findings",
    "Known vulnerabilities reported for the scanned packages; — until a current scan is available.",
);
pub const SECURITY_TARGETS: Kpi = kpi(
    "security.targets",
    "Manifest / lockfile targets",
    "Manifests and lockfiles the saved scan read; — until a scan ran.",
);

impl Kpi {
    /// `value` in this figure's precision.
    pub fn value(&self, locale: Locale, value: Value) -> String {
        match (value, self.precision) {
            (Value::Unknown, _) => UNKNOWN.into(),
            (Value::Count(n), Precision::Int) => locale.count(n),
            (Value::Count(n), Precision::Dec(p)) => locale.dec(n as f64, p),
            (Value::Real(v), Precision::Int) => locale.dec(v, 0),
            (Value::Real(v), Precision::Dec(p)) => locale.dec(v, p),
            (Value::AtLeast(n), _) => locale.lower_bound(i64::try_from(n).unwrap_or(i64::MAX)),
        }
    }

    /// Label, value and note for a stat card.
    pub fn item(&self, texts: &Texts, value: impl Into<Value>) -> KpiItem {
        KpiItem {
            label: texts.t(self.label).into(),
            value: self.value(texts.locale(), value.into()).into(),
            note: texts.t(self.note).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn ids_and_labels_are_unique() {
        let ids: BTreeSet<_> = ALL.iter().map(|k| k.id).collect();
        let labels: BTreeSet<_> = ALL.iter().map(|k| k.label).collect();
        assert_eq!(ids.len(), ALL.len());
        assert_eq!(labels.len(), ALL.len());
    }

    /// The registry's keys are not literal `texts.t` calls, so the scan
    /// in `texts_check` does not see them; check them here.
    #[test]
    fn every_label_and_note_is_translated() {
        let de = Texts::german();
        for k in ALL {
            assert_ne!(de.t(k.label), k.label, "{} label has German", k.id);
            assert_ne!(de.t(k.note), k.note, "{} note has German", k.id);
        }
    }

    /// Both ACD figures name their level and share one precision.
    #[test]
    fn acd_names_its_level_with_two_decimals() {
        assert_eq!(SHOWN_ACD.value(Locale::En, Value::Real(4.111)), "4.11");
        assert_eq!(SHOWN_ACD.value(Locale::De, Value::Real(4.111)), "4,11");
        assert_eq!(MODULE_ACD.value(Locale::De, Value::Real(3.545)), "3,54");
        assert_eq!(SHOWN_ACD.precision, MODULE_ACD.precision);
        let de = Texts::german();
        assert_eq!(de.t(SHOWN_ACD.label), "ACD (angezeigte Ebene)");
        assert_eq!(de.t(MODULE_ACD.label), "ACD (Modulebene)");
    }

    #[test]
    fn counts_unknowns_and_bounds() {
        let k = &DEPENDENCY_EVIDENCE;
        assert_eq!(k.value(Locale::De, 2387.into()), "2.387");
        assert_eq!(k.value(Locale::En, Value::AtLeast(2387)), "≥2,387");
        assert_eq!(k.value(Locale::En, Value::Unknown), "—", "never 0");
        assert_eq!(k.value(Locale::En, None::<usize>.into()), "—");
    }

    #[test]
    fn item_fills_label_value_and_note() {
        let item = SHOWN_ACD.item(&Texts::default(), Value::Real(1.0));
        assert_eq!(item.label, "ACD (shown level)");
        assert_eq!(item.value, "1.00");
        assert_eq!(item.note, SHOWN_ACD.note);
    }
}
