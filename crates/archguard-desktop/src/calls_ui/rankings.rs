//! "Most calls": static call sites per type/module and function/method
//! (viewer_call_rankings.js, rankedMethodMeasurements in
//! viewer_method_metrics.js).

use std::collections::HashMap;

use archguard_engine::core::{CallMethod, MethodComplexity};

use super::model::{Filter, Index, Labels};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub incoming: usize,
    pub outgoing: usize,
    pub total: usize,
    pub cyclomatic: u32,
    pub max_nesting: u32,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Rankings {
    pub classes: Vec<Row>,
    pub methods: Vec<Row>,
    pub unresolved_targets: usize,
    pub missing_callers: usize,
    pub missing_targets: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Incoming,
    Outgoing,
    Total,
    Cyclomatic,
    MaxNesting,
}

impl Sort {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Outgoing,
            2 => Self::Total,
            3 => Self::Cyclomatic,
            4 => Self::MaxNesting,
            _ => Self::Incoming,
        }
    }

    pub fn measured(self) -> bool {
        matches!(self, Self::Cyclomatic | Self::MaxNesting)
    }

    fn value(self, r: &Row) -> usize {
        match self {
            Self::Incoming => r.incoming,
            Self::Outgoing => r.outgoing,
            Self::Total => r.total,
            Self::Cyclomatic => r.cyclomatic as usize,
            Self::MaxNesting => r.max_nesting as usize,
        }
    }
}

#[derive(Default)]
struct Rows {
    rows: Vec<Row>,
    at: HashMap<String, usize>,
}

impl Rows {
    fn row(&mut self, id: &str) -> usize {
        if let Some(&i) = self.at.get(id) {
            return i;
        }
        self.rows.push(Row {
            id: id.to_string(),
            ..Default::default()
        });
        self.at.insert(id.to_string(), self.rows.len() - 1);
        self.rows.len() - 1
    }

    /// One record may be both incoming and outgoing (recursion).
    fn site(&mut self, from: Option<&str>, to: Option<&str>) {
        let out = from.map(|id| self.row(id));
        let inc = to.map(|id| self.row(id));
        if let Some(o) = out {
            self.rows[o].outgoing += 1;
            self.rows[o].total += 1;
        }
        if let Some(i) = inc {
            self.rows[i].incoming += 1;
            if inc != out {
                self.rows[i].total += 1;
            }
        }
    }

    fn sorted(mut self) -> Vec<Row> {
        self.rows
            .sort_by(|a, b| b.incoming.cmp(&a.incoming).then_with(|| a.id.cmp(&b.id)));
        self.rows
    }
}

/// `computeCallRankings` over the given sites.
pub fn compute(calls: impl IntoIterator<Item = usize>, idx: &Index) -> Rankings {
    let (mut classes, mut methods) = (Rows::default(), Rows::default());
    let mut r = Rankings::default();
    for i in calls {
        let c = &idx.a.calls[i];
        let from = idx
            .class(Some(&c.from_class))
            .map(|_| c.from_class.as_str());
        let to = c
            .to_class
            .as_deref()
            .filter(|t| idx.class(Some(t)).is_some());
        classes.site(from, to);
        let (fm, tm) = (idx.owned(&c.from_method, from), idx.owned(&c.to_method, to));
        methods.site(fm, tm);
        r.missing_callers += usize::from(fm.is_none());
        r.missing_targets += usize::from(c.resolution == "project" && tm.is_none());
        r.unresolved_targets += usize::from(c.resolution == "unresolved");
    }
    r.classes = classes.sorted();
    r.methods = methods.sorted();
    r
}

/// `methodComplexity`: only the known counting rule is interpreted.
pub fn complexity(m: &CallMethod) -> Option<&MethodComplexity> {
    m.complexity
        .as_ref()
        .filter(|c| c.definition == "ast-decisions-v1" && c.cyclomatic >= 1)
}

/// `rankedMethodMeasurements`: declared methods in the declaration scope,
/// with their call counts; and how many have no measurement.
pub fn measurements(counts: &[Row], idx: &Index, f: &Filter) -> (Vec<Row>, usize) {
    let by_id: HashMap<&str, &Row> = counts.iter().map(|r| (r.id.as_str(), r)).collect();
    let q = f.query.trim().to_lowercase();
    let mut missing = 0;
    let mut rows = Vec::new();
    for m in &idx.a.methods {
        let Some(cls) = idx.class(Some(&m.class_id)) else {
            continue;
        };
        if !idx.in_language(cls, f.language.as_deref())
            || f.package.as_ref().is_some_and(|p| cls.package != *p)
            || (!q.is_empty()
                && !format!("{} {}", cls.name, m.signature)
                    .to_lowercase()
                    .contains(&q))
        {
            continue;
        }
        let Some(metric) = complexity(m) else {
            missing += 1;
            continue;
        };
        let mut row = by_id.get(m.id.as_str()).map_or_else(
            || Row {
                id: m.id.clone(),
                ..Default::default()
            },
            |r| (*r).clone(),
        );
        row.cyclomatic = metric.cyclomatic;
        row.max_nesting = metric.max_nesting;
        rows.push(row);
    }
    (rows, missing)
}

/// The ordered list for a kind and metric; count metrics drop zero rows.
pub fn ordered(rows: Vec<Row>, sort: Sort) -> Vec<Row> {
    let mut rows: Vec<Row> = if sort.measured() {
        rows
    } else {
        rows.into_iter().filter(|r| sort.value(r) > 0).collect()
    };
    rows.sort_by(|a, b| {
        sort.value(b)
            .cmp(&sort.value(a))
            .then_with(|| a.id.cmp(&b.id))
    });
    rows
}

/// A row's label: `Class#signature` for methods, else the class name.
pub fn label(idx: &Index, method: bool, id: &str, l: &Labels) -> String {
    let kind = if method {
        super::model::ElementKind::Method
    } else {
        super::model::ElementKind::Class
    };
    idx.qualified(kind, id, l)
}

#[cfg(test)]
mod tests {
    use super::super::model::{Resolution, fixture, relations};
    use super::*;

    fn all_sites(idx: &Index) -> Vec<usize> {
        let f = Filter {
            resolution: Resolution::All,
            internal: true,
            ..Default::default()
        };
        relations(idx, &f, &Labels::default())
            .into_iter()
            .flat_map(|r| r.calls)
            .collect()
    }

    #[test]
    fn counts_sites_per_class_and_method() {
        let idx = Index::new(fixture::analysis());
        let r = compute(all_sites(&idx), &idx);
        let blue = r.classes.iter().find(|r| r.id == "blue.Blue").unwrap();
        // In: Red → Blue, Blue → Blue; out: two to Red and the self-call.
        assert_eq!((blue.incoming, blue.outgoing, blue.total), (2, 3, 4));
        assert_eq!(r.classes[0].id, "blue.Blue");
        let name = r
            .methods
            .iter()
            .find(|r| r.id == "blue.Blue#name()")
            .unwrap();
        assert_eq!((name.incoming, name.outgoing, name.total), (2, 3, 4));
        assert_eq!(r.unresolved_targets, 1);
        assert_eq!(r.missing_callers, 0);
        assert_eq!(r.missing_targets, 1);
        let out = ordered(r.classes.clone(), Sort::Outgoing);
        assert_eq!(out[0].id, "blue.Blue");
        assert!(out.iter().all(|r| r.outgoing > 0));
    }

    #[test]
    fn measurements_list_declared_methods_without_calls_too() {
        let idx = Index::new(fixture::analysis());
        let r = compute(Vec::new(), &idx);
        let (rows, missing) = measurements(&r.methods, &idx, &Filter::default());
        assert_eq!((rows.len(), missing), (4, 0));
        let top = ordered(rows, Sort::Cyclomatic);
        assert_eq!(top[0].id, "red.Red#describe()");
        assert_eq!(top[0].cyclomatic, 4);
        let blue = Filter {
            package: Some("blue".into()),
            query: "hint".into(),
            ..Default::default()
        };
        assert_eq!(measurements(&r.methods, &idx, &blue).0.len(), 1);
    }
}
