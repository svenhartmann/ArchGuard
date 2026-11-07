//! Which elements and relations of a package diagram lie on a cycle.

use archguard_core::Diagram;
use std::collections::BTreeSet;

/// Marks elements and relations inside a strongly connected group.
pub fn mark(d: &mut Diagram) {
    let ids: Vec<&str> = d.elements.iter().map(|e| e.id.as_str()).collect();
    let n = ids.len();
    let at = |id: &str| ids.iter().position(|&i| i == id);
    let mut reach = vec![vec![false; n]; n];
    for r in &d.relations {
        if let (Some(a), Some(b)) = (at(&r.from), at(&r.to)) {
            reach[a][b] = true;
        }
    }
    for k in 0..n {
        for i in 0..n {
            for j in 0..n {
                reach[i][j] = reach[i][j] || (reach[i][k] && reach[k][j]);
            }
        }
    }
    let mut on_cycle = BTreeSet::new();
    for r in &mut d.relations {
        if let (Some(a), Some(b)) = (at(&r.from), at(&r.to)) {
            r.cycle = reach[b][a];
            if r.cycle {
                on_cycle.extend([a, b]);
            }
        }
    }
    for i in on_cycle {
        d.elements[i].cycle = true;
    }
}
