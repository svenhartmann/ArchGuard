//! Lookups over the imported spans: parents, children, the request a span
//! belongs to, and database spans that wrap a driver span.
use super::span::{Site, Span};
use archguard_core::RuntimeEvidence;
use std::collections::{HashMap, HashSet};

const MAX_PARENT_HOPS: usize = 256;

type SpanKey<'a> = (&'a str, &'a str);

pub(super) struct Index<'a> {
    pub spans: &'a [Span],
    by_id: HashMap<SpanKey<'a>, usize>,
    /// Children per span, in input order.
    children: HashMap<usize, Vec<usize>>,
    /// Database spans that have a database span as a child.
    pub wrappers: HashSet<usize>,
}

impl<'a> Index<'a> {
    pub fn new(spans: &'a [Span]) -> Self {
        let by_id: HashMap<_, _> = spans
            .iter()
            .enumerate()
            .map(|(i, s)| ((s.trace.as_str(), s.id.as_str()), i))
            .collect();
        let mut index = Index {
            spans,
            by_id,
            children: HashMap::new(),
            wrappers: HashSet::new(),
        };
        for (i, span) in spans.iter().enumerate() {
            let Some(p) = index.parent(span) else {
                continue;
            };
            index.children.entry(p).or_default().push(i);
            if span.db.is_some() && wraps(&spans[p], span) {
                index.wrappers.insert(p);
            }
        }
        index
    }

    pub fn parent(&self, span: &Span) -> Option<usize> {
        self.by_id
            .get(&(span.trace.as_str(), span.parent.as_deref()?))
            .copied()
    }

    pub fn children(&self, i: usize) -> &[usize] {
        self.children.get(&i).map_or(&[], Vec::as_slice)
    }

    /// Ancestors of `span`, nearest first, bounded against cycles in bad input.
    pub fn ancestors(&self, span: &'a Span) -> impl Iterator<Item = usize> + '_ {
        let mut current = span;
        (0..MAX_PARENT_HOPS).map_while(move |_| {
            let p = self.parent(current)?;
            current = &self.spans[p];
            Some(p)
        })
    }

    /// Nearest entry span of the same service, not crossing into another service.
    pub fn entry(&self, span: &'a Span) -> Option<usize> {
        self.ancestors(span)
            .take_while(|p| self.spans[*p].service == span.service)
            .find(|p| self.spans[*p].entry.is_some())
    }

    /// The call site on the span itself or on the database spans wrapping it.
    pub fn site(&self, span: &'a Span) -> Option<Site> {
        let mut inner = span;
        for p in self.ancestors(span) {
            if inner.site.is_some() || !wraps(&self.spans[p], inner) {
                break;
            }
            inner = &self.spans[p];
        }
        inner.site.clone()
    }
}

/// `parent` is a database span around `child` (an ORM span around the driver
/// span): the child starts within the parent's time window. A finished span
/// left as the ambient parent does not wrap later spans.
fn wraps(parent: &Span, child: &Span) -> bool {
    parent.db.is_some() && parent.start <= child.start && child.start <= parent.end
}

/// Span duration in milliseconds.
pub(super) fn ms(span: &Span) -> f64 {
    (span.end - span.start) as f64 / 1_000_000.0
}

pub(super) fn evidence(spans: &[Span], inputs: &[String], i: usize) -> RuntimeEvidence {
    let s = &spans[i];
    RuntimeEvidence {
        file: inputs.get(s.input).cloned().unwrap_or_default(),
        line: s.line,
        trace_id: s.trace.clone(),
        span_id: s.id.clone(),
    }
}
