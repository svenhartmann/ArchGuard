//! Connections between islands: the port of `repositoryEvidenceSource`,
//! `repositoryWorldConnections`, `repositoryVisibleConnections` and
//! `repositoryFlowPath` in `viewer_repository.js`. Dependencies between
//! services become one link per (from, to, kind) between their islands;
//! cycles are checked per kind.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use archguard_engine::core::{
    Graph, ServiceDependencyKind as Kind, ServiceEvidence, ServiceTopology,
};

use super::inventory::Island;

/// Where a proof comes from (the "Evidence" filter); `All` combines them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    All,
    Code,
    Configuration,
    Observed,
}

/// The "Direction" filter, relative to the selected island.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Both,
    Incoming,
    Outgoing,
    /// Every hop along the arrows.
    Downstream,
    /// Every hop against the arrows.
    Upstream,
}

/// The kinds "Communication" shows: all but the startup order.
pub const COMMUNICATION: [Kind; 5] = [
    Kind::HttpCall,
    Kind::GrpcCall,
    Kind::Event,
    Kind::ConfiguredHttp,
    Kind::ConfiguredReference,
];

/// One merged connection between two islands (indices into the inventory).
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub from: usize,
    pub to: usize,
    /// `None`: a package dependency of the source graph.
    pub kind: Option<Kind>,
    pub evidence: Vec<ServiceEvidence>,
    pub cyclic: bool,
}

/// `repositoryEvidenceSource`: imported trace spans are observations;
/// calls and events come from code, everything else from configuration.
pub fn source_of(kind: Kind, proof: &ServiceEvidence) -> Source {
    if proof.file == ".archguard/otel-traces.json" && proof.pointer.starts_with("/resourceSpans/") {
        Source::Observed
    } else if matches!(kind, Kind::HttpCall | Kind::GrpcCall | Kind::Event) {
        Source::Code
    } else {
        Source::Configuration
    }
}

/// `worldFlowCarrier`: calls, events and configured endpoints carry data;
/// configuration references and the startup order do not.
pub fn carrier(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::HttpCall | Kind::GrpcCall | Kind::Event | Kind::ConfiguredHttp
    )
}

/// `repositoryWorldConnections`: dependencies between the islands of
/// `pool` (indices into `inventory`), merged per (from, to, kind) in
/// topology order, with only the proofs of `source`.
pub fn connections(
    t: &ServiceTopology,
    inventory: &[Island],
    pool: &[usize],
    source: Source,
) -> Vec<Link> {
    let by_id: HashMap<&str, usize> = pool
        .iter()
        .map(|&i| (inventory[i].id.as_str(), i))
        .collect();
    // A service's island: its own (a deployment), else its project's.
    let island: HashMap<&str, usize> = t
        .services
        .iter()
        .filter_map(|s| {
            let at = by_id
                .get(s.id.as_str())
                .or_else(|| by_id.get(s.project.as_deref()?))?;
            Some((s.id.as_str(), *at))
        })
        .collect();
    let mut links: Vec<Link> = Vec::new();
    let mut merged: BTreeMap<(usize, usize, Kind), usize> = BTreeMap::new();
    for d in &t.dependencies {
        let (Some(&from), Some(&to)) = (island.get(d.from.as_str()), island.get(d.to.as_str()))
        else {
            continue;
        };
        let proofs = d
            .evidence
            .iter()
            .filter(|p| source == Source::All || source_of(d.kind, p) == source);
        let proofs: Vec<ServiceEvidence> = proofs.cloned().collect();
        if from == to || (proofs.is_empty() && source != Source::All) {
            continue;
        }
        let at = *merged.entry((from, to, d.kind)).or_insert_with(|| {
            links.push(Link {
                from,
                to,
                kind: Some(d.kind),
                evidence: Vec::new(),
                cyclic: false,
            });
            links.len() - 1
        });
        links[at].evidence.extend(proofs);
    }
    mark_cycles(&mut links, inventory.len());
    links
}

/// References kept per package dependency.
const PACKAGE_REFS: usize = 10;

/// Package dependencies: the source graph's edges between the islands'
/// source projects (a module named by the stripped project path), merged
/// per pair, the first references as evidence. Deployments and the
/// repository root are left out.
pub fn packages(g: &Graph, inventory: &[Island], roots: &[String]) -> Vec<Link> {
    let modules: HashMap<String, usize> = inventory
        .iter()
        .enumerate()
        .filter(|(_, i)| i.source.as_deref() == Some(i.path.as_str()) && i.path != ".")
        .map(|(at, i)| (archguard_engine::strip_root(&i.path, roots), at))
        .collect();
    // A node's island: the nearest ancestor that is a project's module.
    let parent: HashMap<_, _> = g.nodes.iter().map(|n| (n.id, n)).collect();
    let island = |mut id| loop {
        let n = parent.get(&id)?;
        if let Some(&at) = modules.get(&n.name) {
            return Some(at);
        }
        id = n.parent?;
    };
    let mut merged: BTreeMap<(usize, usize), Vec<ServiceEvidence>> = BTreeMap::new();
    for e in &g.edges {
        let (Some(from), Some(to)) = (island(e.from), island(e.to)) else {
            continue;
        };
        if from == to {
            continue;
        }
        let proofs = merged.entry((from, to)).or_default();
        let room = PACKAGE_REFS.saturating_sub(proofs.len());
        proofs.extend(e.refs.iter().take(room).map(|r| ServiceEvidence {
            file: r.file.clone(),
            pointer: String::new(),
            line: Some(r.line),
            description: r.raw.trim().to_string(),
        }));
    }
    merged
        .into_iter()
        .map(|((from, to), evidence)| Link {
            from,
            to,
            kind: None,
            evidence,
            cyclic: false,
        })
        .collect()
}

/// A link is cyclic when its ends share a strongly connected component of
/// the links of its kind.
pub fn mark_cycles(links: &mut [Link], n: usize) {
    let kinds: BTreeSet<Option<Kind>> = links.iter().map(|l| l.kind).collect();
    for kind in kinds {
        let typed = || links.iter().filter(|l| l.kind == kind);
        let mut component = vec![usize::MAX; n];
        for (c, members) in crate::tree_order::scc(n, typed().map(|l| (l.from, l.to)))
            .into_iter()
            .enumerate()
        {
            for m in members {
                component[m] = c;
            }
        }
        for l in links.iter_mut().filter(|l| l.kind == kind) {
            l.cyclic = component[l.from] == component[l.to];
        }
    }
}

/// `repositoryVisibleConnections`: indices into `links` for a kind
/// (`None`: communication) or the package dependencies, cycles only and,
/// with a selected island, a direction.
pub fn visible(
    links: &[Link],
    kind: Option<Kind>,
    packages: bool,
    cycles_only: bool,
    direction: Direction,
    selected: Option<usize>,
) -> Vec<usize> {
    let shown = (0..links.len()).filter(|&i| {
        let l = &links[i];
        let k = match (packages, kind) {
            (true, _) => l.kind.is_none(),
            (_, None) => l.kind.is_some_and(|k| COMMUNICATION.contains(&k)),
            (_, k) => l.kind == k,
        };
        k && (!cycles_only || l.cyclic)
    });
    let shown: Vec<usize> = shown.collect();
    let Some(at) = selected else { return shown };
    let keep = |l: &Link| match direction {
        Direction::Incoming => l.to == at,
        Direction::Outgoing => l.from == at,
        _ => l.from == at || l.to == at,
    };
    match direction {
        Direction::Downstream => flow_path(links, &shown, at, true),
        Direction::Upstream => flow_path(links, &shown, at, false),
        _ => shown.into_iter().filter(|&i| keep(&links[i])).collect(),
    }
}

/// `repositoryFlowPath`: every hop from `start` along the arrows
/// (downstream) or against them; only carriers extend a path.
fn flow_path(links: &[Link], shown: &[usize], start: usize, downstream: bool) -> Vec<usize> {
    let carriers: Vec<usize> = shown
        .iter()
        .copied()
        .filter(|&i| links[i].kind.is_some_and(carrier))
        .collect();
    let ends = |i: usize| {
        let l = &links[i];
        if downstream {
            (l.from, l.to)
        } else {
            (l.to, l.from)
        }
    };
    let mut reached = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(at) = queue.pop_front() {
        for &i in &carriers {
            let (from, to) = ends(i);
            if from == at && reached.insert(to) {
                queue.push_back(to);
            }
        }
    }
    carriers
        .into_iter()
        .filter(|&i| reached.contains(&ends(i).0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::inventory::{inventory, tests as inv};
    use super::*;
    use archguard_engine::core::ServiceDependency;

    fn proof(file: &str, pointer: &str) -> ServiceEvidence {
        ServiceEvidence {
            file: file.into(),
            pointer: pointer.into(),
            line: Some(3),
            description: format!("{file} {pointer}"),
        }
    }

    fn dep(from: &str, to: &str, kind: Kind, evidence: Vec<ServiceEvidence>) -> ServiceDependency {
        ServiceDependency {
            from: from.into(),
            to: to.into(),
            kind,
            evidence,
        }
    }

    const TRACES: &str = ".archguard/otel-traces.json";

    /// Two application projects with one service each, as
    /// `test_viewer_repository_connections.js`.
    fn pair(deps: Vec<ServiceDependency>) -> (ServiceTopology, Vec<Island>) {
        let (a, b) = (
            inv::project("functions/search", "application"),
            inv::project("functions/import", "application"),
        );
        let services = vec![
            inv::service("svc:a", Some(&a.id), ""),
            inv::service("svc:b", Some(&b.id), ""),
        ];
        let mut t = inv::topology(vec![a, b], services);
        t.dependencies = deps;
        let islands = inventory(&t);
        (t, islands)
    }

    #[test]
    fn evidence_sources_follow_the_web() {
        let traced = proof(TRACES, "/resourceSpans/0");
        assert_eq!(source_of(Kind::HttpCall, &traced), Source::Observed);
        assert_eq!(source_of(Kind::Event, &proof("A.cs", "")), Source::Code);
        assert_eq!(
            source_of(Kind::Startup, &proof("compose.yaml", "/x")),
            Source::Configuration
        );
        // A trace file without a span pointer is not an observation.
        assert_eq!(source_of(Kind::HttpCall, &proof(TRACES, "")), Source::Code);
        assert!(carrier(Kind::ConfiguredHttp) && carrier(Kind::Event));
        assert!(!carrier(Kind::ConfiguredReference) && !carrier(Kind::Startup));
    }

    /// Two dependencies of one kind between the same islands merge, with
    /// both proofs; cycles count per kind and per evidence source.
    #[test]
    fn links_merge_per_kind_and_cycles_per_kind() {
        let (t, islands) = pair(vec![
            dep(
                "svc:a",
                "svc:b",
                Kind::HttpCall,
                vec![proof("Client.java", "spring-client")],
            ),
            dep(
                "svc:a",
                "svc:b",
                Kind::HttpCall,
                vec![proof("Other.java", "")],
            ),
            dep(
                "svc:b",
                "svc:a",
                Kind::HttpCall,
                vec![proof(TRACES, "/resourceSpans/0")],
            ),
            dep("svc:b", "svc:a", Kind::Event, vec![proof("Sub.cs", "")]),
        ]);
        let pool: Vec<usize> = (0..islands.len()).collect();
        let all = connections(&t, &islands, &pool, Source::All);
        let summary: Vec<_> = all
            .iter()
            .map(|l| (l.from, l.to, l.kind, l.cyclic, l.evidence.len()))
            .collect();
        assert_eq!(
            summary,
            [
                (0, 1, Some(Kind::HttpCall), true, 2),
                (1, 0, Some(Kind::HttpCall), true, 1),
                (1, 0, Some(Kind::Event), false, 1)
            ],
            "the event back edge makes no HTTP cycle, nor one of its own"
        );
        let code = connections(&t, &islands, &pool, Source::Code);
        assert_eq!(code.len(), 2);
        assert!(
            code.iter().all(|l| !l.cyclic),
            "a trace cannot invent a code cycle"
        );
        let observed = connections(&t, &islands, &pool, Source::Observed);
        assert_eq!((observed.len(), observed[0].from), (1, 1));
    }

    /// A chain a → b → c with a configuration reference c → d: incoming,
    /// outgoing, both, and every hop along or against the arrows, where
    /// the reference never extends a path.
    #[test]
    fn directions_and_flow_paths() {
        let l = |from, to, kind| Link {
            from,
            to,
            kind: Some(kind),
            evidence: Vec::new(),
            cyclic: false,
        };
        let links = [
            l(0, 1, Kind::HttpCall),
            l(1, 2, Kind::Event),
            l(2, 3, Kind::ConfiguredReference),
            l(4, 1, Kind::Startup),
        ];
        let v = |d, s| visible(&links, None, false, false, d, s);
        assert_eq!(
            v(Direction::Both, None),
            [0, 1, 2],
            "startup is not communication"
        );
        assert_eq!(v(Direction::Both, Some(1)), [0, 1]);
        assert_eq!(v(Direction::Incoming, Some(1)), [0]);
        assert_eq!(v(Direction::Outgoing, Some(1)), [1]);
        assert_eq!(v(Direction::Downstream, Some(0)), [0, 1]);
        assert_eq!(v(Direction::Upstream, Some(3)), Vec::<usize>::new());
        assert_eq!(v(Direction::Upstream, Some(2)), [0, 1]);
        assert_eq!(
            visible(
                &links,
                Some(Kind::Startup),
                false,
                false,
                Direction::Both,
                None
            ),
            [3]
        );
        let mut cyclic = links.clone();
        cyclic[1].cyclic = true;
        assert_eq!(
            visible(&cyclic, None, false, true, Direction::Both, None),
            [1]
        );
    }
}
