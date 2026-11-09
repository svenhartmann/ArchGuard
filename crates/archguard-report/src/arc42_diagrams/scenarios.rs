//! Chapter 6 without traces (ADR 0006): runtime scenarios derived from the
//! code for every language with a call model. From each entry point (a
//! method that calls but is not called, `main` first) the resolved calls
//! are followed in source order; every call that crosses building blocks
//! becomes a message (each block on its own, no remainder box). Static: no measured order,
//! branches or loops.

use archguard_core::{
    CallAnalysis, Diagram, DiagramElement, DiagramKind, DiagramRelation, DocLang, Graph,
    MethodCall, RelationKind,
};
use std::collections::{BTreeMap, BTreeSet};

use crate::arc42_blocks::Blocks;

const MAX_SCENARIOS: usize = 6;
/// A root that calls this many methods is a dispatcher (a CLI `main`, a
/// router): each of its handlers becomes a scenario of its own.
const DISPATCH: usize = 3;
/// How many calls below a root a dispatcher may sit.
const DISPATCH_DEPTH: usize = 3;
const MAX_MESSAGES: usize = 25;
const MAX_DEPTH: usize = 6;

/// Block scenarios, each followed by the same scenario between classes
/// when that shows more.
pub fn diagrams(
    l: DocLang,
    blocks: &Blocks<'_>,
    graph: &Graph,
    calls: &CallAnalysis,
) -> Vec<Diagram> {
    let boxes = blocks.class_blocks(graph, calls);
    let (out, called) = outgoing(calls);
    let (class_boxes, class_labels) = classes(calls);
    let by_class = Walk {
        out: &out,
        boxes: &class_boxes,
        calls,
    };
    let walk = Walk {
        out: &out,
        boxes: &boxes,
        calls,
    };
    let mut scenarios: Vec<(usize, usize, Entry, Vec<Message>)> = entries(&out, &called)
        .into_iter()
        .enumerate()
        .map(|(i, entry)| {
            let messages = walk.from(entry.method);
            let reach = messages
                .iter()
                .flat_map(|x| [x.from, x.to])
                .collect::<BTreeSet<_>>()
                .len();
            (reach, i, entry, messages)
        })
        .filter(|(_, _, _, messages)| !messages.is_empty())
        .collect();
    scenarios.sort_by_key(|(reach, i, entry, _)| (!entry.main, std::cmp::Reverse(*reach), *i));
    scenarios
        .into_iter()
        .take(MAX_SCENARIOS)
        .flat_map(|(_, _, entry, messages)| {
            let block = diagram(l, &entry, &messages, &blocks.labels, calls, Level::Blocks);
            let finer = by_class.from(entry.method);
            let twin = (finer.len() > messages.len())
                .then(|| diagram(l, &entry, &finer, &class_labels, calls, Level::Classes));
            std::iter::once(block).chain(twin)
        })
        .collect()
}

/// The scenario of `method` between classes (Calls view); `None` when it
/// calls no other class.
pub fn class_sequence(l: DocLang, calls: &CallAnalysis, method: &str) -> Option<Diagram> {
    let (out, _) = outgoing(calls);
    let (boxes, labels) = classes(calls);
    let walk = Walk {
        out: &out,
        boxes: &boxes,
        calls,
    };
    let messages = walk.from(method);
    let entry = Entry {
        method,
        via: None,
        main: false,
    };
    (!messages.is_empty()).then(|| diagram(l, &entry, &messages, &labels, calls, Level::Classes))
}

/// Resolved project calls per calling method in source order, and every
/// called method.
fn outgoing(calls: &CallAnalysis) -> (BTreeMap<&str, Vec<&MethodCall>>, BTreeSet<&str>) {
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
    (out, called)
}

/// Every class its own participant, named after it; a file module
/// (Python, JS) after its file.
fn classes(calls: &CallAnalysis) -> (BTreeMap<&str, usize>, Vec<String>) {
    let boxes = calls
        .classes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i));
    let labels = calls.classes.iter().map(|c| {
        if c.name == "(module)" {
            c.file.rsplit('/').next().unwrap_or(&c.file).to_string()
        } else {
            c.name.clone()
        }
    });
    (boxes.collect(), labels.collect())
}

/// What a scenario's participants are.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    Blocks,
    Classes,
}

/// Where a scenario starts; `via` is the dispatcher that calls it.
struct Entry<'a> {
    method: &'a str,
    via: Option<&'a str>,
    /// Reached from a `main`: the program's own entry, shown first.
    main: bool,
}

/// Roots (calling, not called) in id order. Where a root or a method up
/// to [`DISPATCH_DEPTH`] calls below it dispatches (a CLI `main` calling
/// `run`, which matches on the command), its handlers are the entries.
fn entries<'a>(
    out: &BTreeMap<&'a str, Vec<&'a MethodCall>>,
    called: &BTreeSet<&str>,
) -> Vec<Entry<'a>> {
    let mut list = Vec::new();
    for &root in out.keys().filter(|m| !called.contains(*m)) {
        match dispatcher(out, root) {
            Some((via, handlers)) => list.extend(handlers.into_iter().map(|method| Entry {
                method,
                via: Some(via),
                main: is_main(root),
            })),
            None => list.push(Entry {
                method: root,
                via: None,
                main: is_main(root),
            }),
        }
    }
    list
}

/// The distinct project methods `method` calls, in source order.
fn callees<'a>(out: &BTreeMap<&'a str, Vec<&'a MethodCall>>, method: &str) -> Vec<&'a str> {
    let mut list: Vec<&'a str> = Vec::new();
    for c in out.get(method).into_iter().flatten() {
        if let Some(to) = c.to_method.as_deref()
            && !list.contains(&to)
        {
            list.push(to);
        }
    }
    list
}

/// The first method from `root` (breadth first, up to [`DISPATCH_DEPTH`])
/// that calls [`DISPATCH`] or more methods, with those methods.
fn dispatcher<'a>(
    out: &BTreeMap<&'a str, Vec<&'a MethodCall>>,
    root: &'a str,
) -> Option<(&'a str, Vec<&'a str>)> {
    let mut level = vec![root];
    let mut seen = BTreeSet::from([root]);
    for _ in 0..=DISPATCH_DEPTH {
        let mut next = Vec::new();
        for method in level {
            let handlers = callees(out, method);
            if handlers.len() >= DISPATCH {
                return Some((method, handlers));
            }
            next.extend(handlers.into_iter().filter(|h| seen.insert(*h)));
        }
        level = next;
    }
    None
}

/// `main` in any language (`main`, `Main`, `__main__`).
fn is_main(method: &str) -> bool {
    short(method).trim_matches('_').eq_ignore_ascii_case("main")
}

/// One call between boxes; `count` repeats of it in the scenario.
struct Message {
    from: usize,
    to: usize,
    target: String,
    count: usize,
    /// The message (index) this call is made within.
    caller: Option<usize>,
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
        self.visit(entry, 0, &mut seen, &mut messages, None);
        collapse(messages)
    }

    fn visit(
        &self,
        method: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
        messages: &mut Vec<Message>,
        // The message that entered `method`'s block; calls inside a block
        // keep it.
        caller: Option<usize>,
    ) {
        for c in self.out.get(method).into_iter().flatten() {
            if messages.len() >= MAX_MESSAGES {
                return;
            }
            let mut inner = caller;
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
                    count: 1,
                    caller,
                });
                inner = Some(messages.len() - 1);
            }
            let Some(next) = c.to_method.as_deref() else {
                continue;
            };
            if depth + 1 < MAX_DEPTH && seen.insert(next.to_string()) {
                self.visit(next, depth + 1, seen, messages, inner);
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

/// The same call between the same blocks within the same caller once,
/// with how often it occurs; callers renumbered to the kept messages.
fn collapse(messages: Vec<Message>) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut kept: Vec<usize> = Vec::new();
    for mut m in messages {
        m.caller = m.caller.map(|c| kept[c]);
        let same = |o: &Message| {
            (o.from, o.to, &o.target, o.caller) == (m.from, m.to, &m.target, m.caller)
        };
        match out.iter().position(same) {
            Some(i) => {
                out[i].count += 1;
                kept.push(i);
            }
            None => {
                out.push(m);
                kept.push(out.len() - 1);
            }
        }
    }
    out
}

/// `pkg.Class#run(int)` or `php:a.php::A#run@9` → `run`.
fn short(method: &str) -> &str {
    let name = method.rsplit_once('#').map_or(method, |(_, n)| n);
    let end = name.find(['(', '@']).unwrap_or(name.len());
    &name[..end]
}

fn diagram(
    l: DocLang,
    entry: &Entry<'_>,
    messages: &[Message],
    labels: &[String],
    calls: &CallAnalysis,
    level: Level,
) -> Diagram {
    let classes = level == Level::Classes;
    let mut order: Vec<usize> = Vec::new();
    for m in messages {
        for b in [m.from, m.to] {
            if !order.contains(&b) {
                order.push(b);
            }
        }
    }
    let name = |b: usize| {
        labels.get(b).cloned().unwrap_or_else(|| {
            let word = match classes {
                true => l.pick("Class", "Klasse"),
                false => l.pick("Building block", "Baustein"),
            };
            format!("{word} {b}")
        })
    };
    let at = |b: usize| order.iter().position(|&x| x == b).unwrap_or(0);
    let entry_name = method_name(calls, entry.method);
    Diagram {
        id: format!(
            "runtime-scenario-{}{}",
            if classes { "classes-" } else { "" },
            slug(entry.method)
        ),
        chapter: 6,
        title: format!(
            "{} · {entry_name} · {} ({})",
            match classes {
                true => l.pick("Class scenario", "Klassen-Szenario"),
                false => l.pick("Scenario", "Szenario"),
            },
            order.first().map(|&b| name(b)).unwrap_or_default(),
            l.pick("static", "statisch")
        ),
        kind: DiagramKind::Sequence,
        note: note(l, entry, calls, level),
        unit: l.pick("call sites", "Aufrufstellen").into(),
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
                label: Some(if m.count > 1 {
                    format!("{}. {} ({}×)", n + 1, m.target, m.count)
                } else {
                    format!("{}. {}", n + 1, m.target)
                }),
                error: false,
                caller: m.caller,
                reply: false,
            })
            .collect(),
        shown: true,
    }
}

fn method_name(calls: &CallAnalysis, method: &str) -> String {
    calls
        .methods
        .iter()
        .find(|m| m.id == method)
        .map_or_else(|| short(method).to_string(), |m| m.name.clone())
}

/// Where the scenario starts and what it does not show.
fn note(l: DocLang, entry: &Entry<'_>, calls: &CallAnalysis, level: Level) -> String {
    let between = match level {
        Level::Classes => l.pick("classes", "Klassen"),
        Level::Blocks => l.pick("building blocks", "Bausteinen"),
    };
    let start = calls.methods.iter().find(|m| m.id == entry.method);
    let place = start
        .map(|m| format!(" ({}:{})", m.file, m.line))
        .unwrap_or_default();
    let via = entry
        .via
        .map(|v| {
            format!(
                "{} {}",
                l.pick(", called by", ", aufgerufen von"),
                method_name(calls, v)
            )
        })
        .unwrap_or_default();
    let text = l.pick(
        "Derived from resolved calls from {}{}{}, in source order down to depth {}; equal calls between the same {} are merged. Static: no measured order, no branches or loops; only calls between {}.",
        "Abgeleitet aus aufgelösten Aufrufen ab {}{}{}, in Quelltext-Reihenfolge bis Tiefe {}; gleiche Aufrufe zwischen denselben {} sind zusammengefasst. Statisch: keine gemessene Reihenfolge, keine Verzweigungen oder Schleifen; nur Aufrufe zwischen {}.",
    );
    crate::arc42_text::fill(
        text,
        &[
            &method_name(calls, entry.method),
            &place,
            &via,
            &MAX_DEPTH,
            &between,
            &between,
        ],
    )
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
        let blocks = Blocks::new(
            &graph,
            &config,
            &services,
            &BTreeMap::new(),
            &facts,
            archguard_core::DocLang::De,
        );
        let calls = calls();
        let all = diagrams(DocLang::De, &blocks, &graph, &calls);
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

    /// A second class in block p1 that `helper` calls: invisible between
    /// blocks, so the scenario gets a twin between classes.
    #[test]
    fn calls_inside_a_block_add_a_class_scenario() {
        let mut builder = archguard_core::GraphBuilder::new();
        for i in 0..3 {
            builder.ensure_path(&format!("org/demo/p{i}/S.java"), Some("java"));
        }
        let graph = builder.build("test".into());
        let (config, services) = (Config::default(), ServiceTopology::default());
        let facts = Arc42Blocks::default();
        let blocks = Blocks::new(
            &graph,
            &config,
            &services,
            &BTreeMap::new(),
            &facts,
            archguard_core::DocLang::De,
        );
        let mut calls = calls();
        let mut class = calls.classes[1].clone();
        (class.id, class.name) = ("C1b".into(), "C1b".into());
        let mut method = calls.methods[1].clone();
        (method.id, method.class_id, method.name) =
            ("C1b#check()".into(), "C1b".into(), "check".into());
        let mut call = calls.calls[0].clone();
        (call.from_class, call.from_method) = ("C1".into(), Some("C1#helper()".into()));
        (call.to_class, call.to_method) = (Some("C1b".into()), Some("C1b#check()".into()));
        call.line = 10;
        calls.classes.push(class);
        calls.methods.push(method);
        calls.calls.push(call);
        let all = diagrams(DocLang::De, &blocks, &graph, &calls);
        let ids: Vec<&str> = all.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "runtime-scenario-c0-main",
                "runtime-scenario-classes-c0-main"
            ]
        );
        let twin = &all[1];
        assert!(
            twin.title.starts_with("Klassen-Szenario") && twin.note.contains("zwischen Klassen")
        );
        let labels: Vec<&str> = twin
            .relations
            .iter()
            .filter_map(|r| r.label.as_deref())
            .collect();
        assert_eq!(labels, ["1. run", "2. load", "3. check", "4. save"]);
    }

    #[test]
    fn a_dispatcher_splits_into_one_short_scenario_per_handler() {
        let mut builder = archguard_core::GraphBuilder::new();
        for i in 0..3 {
            builder.ensure_path(&format!("org/demo/p{i}/S.java"), Some("java"));
        }
        let graph = builder.build("test".into());
        let (config, services) = (Config::default(), ServiceTopology::default());
        let facts = Arc42Blocks::default();
        let blocks = Blocks::new(
            &graph,
            &config,
            &services,
            &BTreeMap::new(),
            &facts,
            archguard_core::DocLang::De,
        );
        let class = |i: usize| {
            serde_json::json!({"id":format!("C{i}"),"name":format!("C{i}"),"package":format!("p{i}"),
            "file":format!("org/demo/p{i}/S.java"),"graph_node":format!("org/demo/p{i}/S.java"),"line":1,"kind":"class"})
        };
        let method = |id: &str, class: &str| {
            serde_json::json!({"id":id,"class_id":class,"name":short(id),
            "signature":"","file":"S.java","line":1,"is_static":false})
        };
        let call = |from: &str, to: &str, line: u32| {
            let class_of = |m: &str| m.split('#').next().unwrap().to_string();
            serde_json::json!({"from_class":class_of(from),"from_method":from,"to_class":class_of(to),"to_method":to,
                "target_label":to,"dispatch":"static","resolution":"project","file":"S.java","line":line,"column":1,"code":""})
        };
        let calls: CallAnalysis = serde_json::from_value(serde_json::json!({"status":"ready","java_release":17,"analyzers":[],"files":3,
            "classes":[class(0),class(1),class(2)],
            "methods":[method("C0#main()","C0"),method("C0#h1()","C0"),method("C0#h2()","C0"),method("C0#h3()","C0"),
                method("C1#a()","C1"),method("C2#b()","C2")],
            "calls":[call("C0#main()","C0#h1()",2),call("C0#main()","C0#h2()",3),call("C0#main()","C0#h3()",4),
                call("C0#h1()","C1#a()",5),call("C0#h1()","C1#a()",6),call("C1#a()","C2#b()",7),
                call("C0#h2()","C2#b()",8),call("C0#h3()","C1#a()",9)],
            "diagnostics":[]}))
        .unwrap();
        let all = diagrams(DocLang::De, &blocks, &graph, &calls);
        let titles: Vec<&str> = all.iter().map(|d| d.title.as_str()).collect();
        assert_eq!(titles.len(), 3, "{titles:?}");
        assert!(titles.iter().any(|t| t.contains("h1")) && titles.iter().any(|t| t.contains("h2")));
        let h1 = all.iter().find(|d| d.title.contains("h1")).unwrap();
        let labels: Vec<&str> = h1
            .relations
            .iter()
            .filter_map(|r| r.label.as_deref())
            .collect();
        assert_eq!(
            labels,
            ["1. a (2×)", "2. b"],
            "repeated calls collapse into one message"
        );
        assert!(
            h1.note.contains("main"),
            "the note names where the handler is called from"
        );
    }

    #[test]
    fn a_method_becomes_a_sequence_between_classes() {
        let mut analysis = calls();
        analysis.classes[2].name = "(module)".into();
        let d =
            class_sequence(DocLang::De, &analysis, "C0#main()").expect("main calls other classes");
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["C0", "C1", "S.java"], "a file module by its file");
        let steps: Vec<_> = d
            .relations
            .iter()
            .map(|r| (r.label.as_deref().unwrap_or(""), r.caller))
            .collect();
        assert_eq!(
            steps,
            [("1. run", None), ("2. load", Some(0)), ("3. save", Some(0))],
            "helper stays inside C1, its call to C2 is made within run"
        );
        assert!(
            class_sequence(DocLang::De, &calls(), "C2#save()").is_none(),
            "calls nothing"
        );
    }
}
