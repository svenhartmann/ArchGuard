//! Chapter 6 without traces (ADR 0006): runtime scenarios derived from the
//! code for every language with a call model. From each entry point (a
//! method that calls but is not called, `main` first) the resolved calls
//! are followed in source order; every call that crosses building blocks
//! of the package overview becomes a message. Static: no measured order,
//! branches or loops.

use archguard_core::{
    CallAnalysis, Diagram, DiagramElement, DiagramKind, DiagramRelation, Graph, MethodCall,
    RelationKind,
};
use std::collections::{BTreeMap, BTreeSet};

use crate::arc42_blocks::Blocks;

const MAX_SCENARIOS: usize = 3;
const MAX_MESSAGES: usize = 25;
const MAX_DEPTH: usize = 6;

pub fn diagrams(
    blocks: &Blocks<'_>,
    elements: &[DiagramElement],
    graph: &Graph,
    calls: &CallAnalysis,
) -> Vec<Diagram> {
    let boxes = blocks.class_boxes(graph, calls);
    let mut out: BTreeMap<&str, Vec<&MethodCall>> = BTreeMap::new();
    let mut called = BTreeSet::new();
    for c in calls.calls.iter().filter(|c| c.resolution == "project") {
        let Some(from) = c.from_method.as_deref() else {
            continue;
        };
        out.entry(from).or_default().push(c);
        if let Some(to) = c.to_method.as_deref() {
            called.insert(to);
        }
    }
    for list in out.values_mut() {
        list.sort_by_key(|c| (c.line, c.column));
    }
    let walk = Walk {
        out: &out,
        boxes: &boxes,
        calls,
    };
    let mut scenarios: Vec<(usize, &str, Vec<Message>)> = out
        .keys()
        .filter(|m| !called.contains(*m))
        .map(|&m| {
            let messages = walk.from(m);
            let reach = messages
                .iter()
                .flat_map(|x| [x.from, x.to])
                .collect::<BTreeSet<_>>()
                .len();
            (reach, m, messages)
        })
        .filter(|(_, _, messages)| !messages.is_empty())
        .collect();
    scenarios.sort_by(|a, b| {
        let main = |m: &str| short(m) != "main";
        (main(a.1), std::cmp::Reverse(a.0), a.1).cmp(&(main(b.1), std::cmp::Reverse(b.0), b.1))
    });
    scenarios
        .into_iter()
        .take(MAX_SCENARIOS)
        .map(|(_, entry, messages)| diagram(entry, &messages, elements, calls))
        .collect()
}

/// One call between boxes.
struct Message {
    from: usize,
    to: usize,
    target: String,
}

struct Walk<'a> {
    out: &'a BTreeMap<&'a str, Vec<&'a MethodCall>>,
    boxes: &'a BTreeMap<&'a str, usize>,
    calls: &'a CallAnalysis,
}

impl Walk<'_> {
    fn from(&self, entry: &str) -> Vec<Message> {
        let mut messages = Vec::new();
        let mut seen = BTreeSet::from([entry.to_string()]);
        self.visit(entry, 0, &mut seen, &mut messages);
        messages
    }

    fn visit(
        &self,
        method: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
        messages: &mut Vec<Message>,
    ) {
        for c in self.out.get(method).into_iter().flatten() {
            if messages.len() >= MAX_MESSAGES {
                return;
            }
            let from = self.boxes.get(c.from_class.as_str());
            let to = c.to_class.as_deref().and_then(|t| self.boxes.get(t));
            if let (Some(&a), Some(&b)) = (from, to)
                && a != b
            {
                let target = c
                    .to_method
                    .as_deref()
                    .map_or(c.target_label.clone(), |m| self.name(m));
                messages.push(Message {
                    from: a,
                    to: b,
                    target,
                });
            }
            let Some(next) = c.to_method.as_deref() else {
                continue;
            };
            if depth + 1 < MAX_DEPTH && seen.insert(next.to_string()) {
                self.visit(next, depth + 1, seen, messages);
            }
        }
    }

    fn name(&self, method: &str) -> String {
        self.calls
            .methods
            .iter()
            .find(|m| m.id == method)
            .map_or_else(|| short(method).to_string(), |m| m.name.clone())
    }
}

/// `pkg.Class#run(int)` or `php:a.php::A#run@9` → `run`.
fn short(method: &str) -> &str {
    let name = method.rsplit_once('#').map_or(method, |(_, n)| n);
    let end = name.find(['(', '@']).unwrap_or(name.len());
    &name[..end]
}

fn diagram(
    entry: &str,
    messages: &[Message],
    elements: &[DiagramElement],
    calls: &CallAnalysis,
) -> Diagram {
    let mut order: Vec<usize> = Vec::new();
    for m in messages {
        for b in [m.from, m.to] {
            if !order.contains(&b) {
                order.push(b);
            }
        }
    }
    let name = |b: usize| {
        elements
            .iter()
            .find(|e| e.id == format!("e{b}"))
            .map_or_else(|| format!("Baustein {b}"), |e| e.name.clone())
    };
    let at = |b: usize| order.iter().position(|&x| x == b).unwrap_or(0);
    let start = calls.methods.iter().find(|m| m.id == entry);
    let place = start
        .map(|m| format!(" ({}:{})", m.file, m.line))
        .unwrap_or_default();
    let entry_name = start.map_or_else(|| short(entry).to_string(), |m| m.name.clone());
    Diagram {
        id: format!("runtime-scenario-{}", slug(entry)),
        chapter: 6,
        title: format!("Szenario · {entry_name} (statisch)"),
        kind: DiagramKind::Sequence,
        note: format!(
            "Abgeleitet aus aufgelösten Aufrufen ab dem Einstiegspunkt {entry_name}{place}, in Quelltext-Reihenfolge bis Tiefe {MAX_DEPTH}. Statisch: keine gemessene Reihenfolge, keine Verzweigungen oder Schleifen; nur Aufrufe zwischen Bausteinen der Paketübersicht."
        ),
        unit: "Aufrufstellen".into(),
        elements: order
            .iter()
            .enumerate()
            .map(|(i, &b)| DiagramElement {
                id: format!("e{i}"),
                name: name(b),
                ..participant()
            })
            .collect(),
        relations: messages
            .iter()
            .enumerate()
            .map(|(n, m)| DiagramRelation {
                from: format!("e{}", at(m.from)),
                to: format!("e{}", at(m.to)),
                kind: RelationKind::Call,
                count: 1,
                cycle: false,
                label: Some(format!("{}. {}", n + 1, m.target)),
                error: false,
            })
            .collect(),
        shown: true,
    }
}

fn participant() -> DiagramElement {
    DiagramElement {
        id: String::new(),
        name: String::new(),
        kind: archguard_core::ElementKind::Package,
        technology: None,
        detail: None,
        group: None,
        cycle: false,
    }
}

/// A stable id part: letters and digits of the method id, lower case.
fn slug(method: &str) -> String {
    let s: String = method
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc42_blocks::Arc42Blocks;
    use archguard_core::{Config, ServiceTopology};

    fn calls() -> CallAnalysis {
        let class = |i: usize| {
            serde_json::json!({"id":format!("C{i}"),"name":format!("C{i}"),"package":format!("p{i}"),
            "file":format!("org/demo/p{i}/S.java"),"graph_node":format!("org/demo/p{i}/S.java"),"line":1,"kind":"class"})
        };
        let method = |id: &str, class: &str, line: u32| {
            serde_json::json!({"id":id,"class_id":class,"name":short(id),
            "signature":"","file":"S.java","line":line,"is_static":false})
        };
        let call = |from: &str, fc: &str, to: &str, tc: &str, line: u32| {
            serde_json::json!({"from_class":fc,"from_method":from,
            "to_class":tc,"to_method":to,"target_label":to,"dispatch":"static","resolution":"project","file":"S.java","line":line,"column":1,"code":""})
        };
        serde_json::from_value(serde_json::json!({"status":"ready","java_release":17,"analyzers":[],"files":3,
            "classes":[class(0),class(1),class(2)],
            "methods":[method("C0#main()","C0",2),method("C1#run()","C1",2),method("C1#helper()","C1",8),
                method("C2#save()","C2",2),method("C2#load()","C2",5)],
            "calls":[call("C0#main()","C0","C1#run()","C1",3),call("C1#run()","C1","C1#helper()","C1",3),
                call("C1#run()","C1","C2#save()","C2",5),call("C1#helper()","C1","C2#load()","C2",9)],
            "diagnostics":[]}))
        .unwrap()
    }

    #[test]
    fn an_entry_point_becomes_a_scenario_of_calls_between_blocks_in_source_order() {
        let mut builder = archguard_core::GraphBuilder::new();
        for i in 0..3 {
            builder.ensure_path(&format!("org/demo/p{i}/S.java"), Some("java"));
        }
        let graph = builder.build("test".into());
        let (config, services) = (Config::default(), ServiceTopology::default());
        let facts = Arc42Blocks::default();
        let blocks = Blocks::new(&graph, &config, &services, &BTreeMap::new(), &facts);
        let calls = calls();
        let all = diagrams(&blocks, &blocks.elements(), &graph, &calls);
        assert_eq!(all.len(), 1, "only main calls without being called");
        let d = &all[0];
        assert_eq!(
            (d.chapter, d.kind, d.id.as_str()),
            (6, DiagramKind::Sequence, "runtime-scenario-c0-main")
        );
        let labels: Vec<&str> = d
            .relations
            .iter()
            .filter_map(|r| r.label.as_deref())
            .collect();
        assert_eq!(
            labels,
            ["1. run", "2. load", "3. save"],
            "depth first, in source order, across blocks only"
        );
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names[0].ends_with("p0") && names[2].ends_with("p2"),
            "{names:?}"
        );
        assert!(d.note.contains("main (S.java:2)") && d.title.contains("statisch"));
        let mut init = d.clone();
        init.relations[0].label = Some("1. <init>".into());
        assert!(super::super::render::mermaid(&init).contains(": 1. #lt;init#gt;\n"));
    }
}
