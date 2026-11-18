//! Profiler samples of one request (CONVENTIONS.md, "Samples"): per method the
//! samples where it is the leaf frame (self) and where it is on the stack
//! (total, once per stack). Statistical estimates, never measured durations.
use super::span::Profile;
use archguard_core::{RuntimeSampledMethod, RuntimeSamples};
use std::collections::{HashMap, HashSet};

/// Methods kept per request.
const MAX_METHODS: usize = 50;

#[derive(Default)]
pub(super) struct Sampled {
    samples: u64,
    /// Interval and clock of the first sampled span.
    // shortcut: assumes one interval and clock per request (probes write one per process); compare all spans once a producer mixes them.
    first: Option<(f64, String)>,
    methods: HashMap<String, (u64, u64)>,
}

impl Sampled {
    /// Adds a span's samples; returns how many the span owns.
    pub fn add(&mut self, profile: Option<&Profile>) -> u64 {
        let Some(p) = profile else { return 0 };
        // Counts come from the input file: saturate instead of overflowing.
        let listed = p.stacks.iter().fold(0u64, |n, (_, c)| n.saturating_add(*c));
        let samples = p.samples.max(listed);
        self.samples = self.samples.saturating_add(samples);
        self.first
            .get_or_insert_with(|| (p.interval_ms, p.clock.clone()));
        for (frames, count) in &p.stacks {
            if let Some(leaf) = frames.last() {
                let m = self.methods.entry(leaf.clone()).or_default();
                m.0 = m.0.saturating_add(*count);
            }
            let mut seen = HashSet::new();
            for f in frames.iter().filter(|f| seen.insert(f.as_str())) {
                let m = self.methods.entry(f.clone()).or_default();
                m.1 = m.1.saturating_add(*count);
            }
        }
        samples
    }

    pub fn finish(self) -> Option<RuntimeSamples> {
        let (interval_ms, clock) = self.first?;
        let mut methods: Vec<RuntimeSampledMethod> = self
            .methods
            .into_iter()
            .map(
                |(function, (self_samples, total_samples))| RuntimeSampledMethod {
                    function,
                    self_samples,
                    total_samples,
                },
            )
            .collect();
        methods.sort_by(|a, b| {
            b.self_samples
                .cmp(&a.self_samples)
                .then(b.total_samples.cmp(&a.total_samples))
                .then_with(|| a.function.cmp(&b.function))
        });
        methods.truncate(MAX_METHODS);
        Some(RuntimeSamples {
            samples: self.samples,
            interval_ms,
            clock,
            methods,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(samples: u64, stacks: &[(&str, u64)]) -> Profile {
        Profile {
            interval_ms: 10.0,
            clock: "cpu".into(),
            samples,
            stacks: stacks
                .iter()
                .map(|(s, c)| (s.split(';').map(String::from).collect(), *c))
                .collect(),
        }
    }

    #[test]
    fn self_is_the_leaf_total_counts_a_recursive_frame_once() {
        let mut s = Sampled::default();
        assert_eq!(s.add(None), 0);
        let owned = s.add(Some(&profile(7, &[("a;b;b", 3), ("a;c", 2)])));
        assert_eq!(owned, 7, "samples of unlisted stacks still count");
        let r = s.finish().unwrap();
        assert_eq!(r.samples, 7);
        let m = |f: &str| r.methods.iter().find(|m| m.function == f).unwrap();
        assert_eq!((m("b").self_samples, m("b").total_samples), (3, 3));
        assert_eq!((m("a").self_samples, m("a").total_samples), (0, 5));
        assert_eq!(r.methods[0].function, "b");
    }

    /// Counts come from the input file: huge ones saturate instead of overflowing.
    #[test]
    fn huge_counts_saturate() {
        let mut s = Sampled::default();
        let huge = profile(1, &[("x;a", u64::MAX), ("x;b", 1)]);
        assert_eq!(s.add(Some(&huge)), u64::MAX);
        assert_eq!(s.add(Some(&huge)), u64::MAX);
        let r = s.finish().unwrap();
        assert_eq!(r.samples, u64::MAX);
        let x = r.methods.iter().find(|m| m.function == "x").unwrap();
        assert_eq!((x.self_samples, x.total_samples), (0, u64::MAX));
    }

    #[test]
    fn no_sampled_span_means_unknown() {
        let mut s = Sampled::default();
        s.add(None);
        assert!(s.finish().is_none());
    }
}
