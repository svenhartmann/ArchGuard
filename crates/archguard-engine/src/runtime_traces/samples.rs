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
        let listed: u64 = p.stacks.iter().map(|(_, c)| c).sum();
        let samples = p.samples.max(listed);
        self.samples += samples;
        self.first
            .get_or_insert_with(|| (p.interval_ms, p.clock.clone()));
        for (frames, count) in &p.stacks {
            if let Some(leaf) = frames.last() {
                self.methods.entry(leaf.clone()).or_default().0 += count;
            }
            let mut seen = HashSet::new();
            for f in frames.iter().filter(|f| seen.insert(f.as_str())) {
                self.methods.entry(f.clone()).or_default().1 += count;
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

    #[test]
    fn no_sampled_span_means_unknown() {
        let mut s = Sampled::default();
        s.add(None);
        assert!(s.finish().is_none());
    }
}
