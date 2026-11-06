//! Source-scoped class metrics and versioned AST maintainability measurements.
use archguard_core::CallAnalysis;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CountMetric {
    pub value: Option<u64>,
    pub observed: u64,
    pub status: String,
    pub evidence: Vec<String>,
}
fn not_applicable() -> CountMetric {
    CountMetric {
        value: None,
        observed: 0,
        status: "not_applicable".into(),
        evidence: Vec::new(),
    }
}
fn count(observed: u64, complete: bool, evidence: Vec<String>) -> CountMetric {
    CountMetric {
        value: complete.then_some(observed),
        observed,
        status: if complete { "ready" } else { "partial" }.into(),
        evidence,
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassMetrics {
    pub class_id: String,
    pub node: String,
    pub file: String,
    pub cbo: CountMetric,
    pub rfc: CountMetric,
    pub dit: CountMetric,
    pub noc: CountMetric,
    pub wmc: CountMetric,
    pub lcom4: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HalsteadMetrics {
    pub distinct_operators: u32,
    pub distinct_operands: u32,
    pub operators: u32,
    pub operands: u32,
    pub volume: f64,
    pub difficulty: Option<f64>,
    pub effort: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodMetrics {
    pub method_id: String,
    pub node: Option<String>,
    pub class_id: String,
    pub file: String,
    pub line: u32,
    pub cognitive: Option<u32>,
    pub halstead: Option<HalsteadMetrics>,
    pub source_lines: Option<u32>,
    pub maintainability_index: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeMetricsReport {
    pub definition: String,
    pub status: String,
    pub classes: Vec<ClassMetrics>,
    pub methods: Vec<MethodMetrics>,
    pub diagnostics: Vec<String>,
    pub analyzers: Vec<archguard_core::CallAnalyzer>,
    pub coverage: BTreeMap<String, (usize, usize)>,
    pub coverage_actions: Vec<String>,
}
fn member(name: &str, id: &str) -> bool {
    !matches!(
        name,
        "<init>" | "<clinit>" | ".ctor" | ".cctor" | "constructor"
    ) && id.matches('#').count() == 1
}

/// Go embedding is composition and Rust has no implementation inheritance:
/// DIT and NOC do not apply to their types.
fn no_inheritance(class: &archguard_core::CallClass) -> bool {
    matches!(class.language.as_deref(), Some("go" | "rust"))
}

pub fn code_metrics(calls: &CallAnalysis) -> CodeMetricsReport {
    let class_nodes: BTreeMap<_, _> = calls.classes.iter().map(|c| (&c.id, c)).collect();
    let details = calls
        .details
        .as_ref()
        .filter(|d| d.definition == "source-types-state/v1");
    let states: BTreeMap<_, _> = details
        .into_iter()
        .flat_map(|d| &d.class_states)
        .map(|s| (&s.class_id, s))
        .collect();
    let inheritance: BTreeMap<_, _> = details
        .into_iter()
        .flat_map(|d| &d.inheritance)
        .map(|s| (&s.class_id, s))
        .collect();
    let raw: BTreeMap<_, _> = details
        .into_iter()
        .flat_map(|d| &d.method_metrics)
        .map(|m| (&m.method_id, m))
        .collect();
    let cohesion: BTreeMap<_, _> = crate::class_cohesion(calls)
        .into_iter()
        .map(|c| (c.class_id, c.lcom4))
        .collect();
    let mut class_methods = BTreeMap::<_, Vec<_>>::new();
    let mut class_calls = BTreeMap::<_, Vec<_>>::new();
    let mut class_uses = BTreeMap::<_, Vec<_>>::new();
    let mut children = BTreeMap::<_, BTreeSet<_>>::new();
    for m in &calls.methods {
        if member(&m.name, &m.id) {
            class_methods.entry(&m.class_id).or_default().push(m);
        }
    }
    for c in &calls.calls {
        class_calls.entry(&c.from_class).or_default().push(c);
    }
    for u in details.into_iter().flat_map(|d| &d.type_uses) {
        class_uses.entry(&u.from_class).or_default().push(u);
    }
    for i in inheritance.values() {
        if i.status == "ready"
            && let Some(base) = &i.base_class
        {
            children.entry(base).or_default().insert(i.class_id.clone());
        }
    }
    let mut classes = Vec::new();
    for class in &calls.classes {
        // Function groups and Rust markers for undeclarable types are no classes.
        if matches!(class.kind.as_str(), "module" | "unknown") {
            continue;
        }
        let methods = class_methods.get(&class.id).cloned().unwrap_or_default();
        let edges = class_calls.get(&class.id).cloned().unwrap_or_default();
        let uses = class_uses.get(&class.id).cloned().unwrap_or_default();
        let ready = states.get(&class.id).is_some_and(|s| s.status == "ready");
        let complete = ready
            && details.is_some_and(|d| d.enumeration_complete == Some(true))
            && edges.iter().all(|e| {
                e.resolution != "unresolved" && e.to_method.is_some() && e.to_class.is_some()
            })
            && uses
                .iter()
                .all(|u| u.resolution != "unresolved" && u.to_class.is_some());
        let targets: BTreeSet<_> = edges
            .iter()
            .filter(|e| e.resolution != "unresolved")
            .filter_map(|e| e.to_class.clone())
            .chain(
                uses.iter()
                    .filter(|u| u.resolution != "unresolved")
                    .filter_map(|u| u.to_class.clone()),
            )
            .filter(|id| id != &class.id)
            .collect();
        let method_ids: BTreeSet<_> = methods.iter().map(|m| &m.id).collect();
        let responses: BTreeSet<_> = methods
            .iter()
            .map(|m| m.id.clone())
            .chain(
                edges
                    .iter()
                    .filter(|e| {
                        e.resolution != "unresolved"
                            && e.from_method
                                .as_ref()
                                .is_some_and(|m| method_ids.contains(m))
                    })
                    .filter_map(|e| e.to_method.clone()),
            )
            .collect();
        let hierarchy = inheritance.get(&class.id);
        let descendants = children.get(&class.id).cloned().unwrap_or_default();
        let measured: Vec<_> = methods
            .iter()
            .filter_map(|m| {
                m.complexity
                    .as_ref()
                    .filter(|c| c.definition == "ast-decisions-v1")
            })
            .collect();
        classes.push(ClassMetrics {
            class_id: class.id.clone(),
            node: class.graph_node.clone(),
            file: class.file.clone(),
            cbo: count(
                targets.len() as u64,
                complete,
                targets.into_iter().collect(),
            ),
            rfc: count(
                responses.len() as u64,
                complete,
                responses.into_iter().collect(),
            ),
            dit: if no_inheritance(class) {
                not_applicable()
            } else {
                count(
                    u64::from(hierarchy.and_then(|h| h.depth).unwrap_or(0)),
                    hierarchy.is_some_and(|h| h.status == "ready" && h.depth.is_some()),
                    hierarchy
                        .and_then(|h| h.base_class.clone())
                        .into_iter()
                        .collect(),
                )
            },
            noc: if no_inheritance(class) {
                not_applicable()
            } else {
                count(
                    descendants.len() as u64,
                    calls.status == "ready"
                        && calls
                            .classes
                            .iter()
                            .filter(|c| {
                                !no_inheritance(c)
                                    && !matches!(
                                        c.kind.as_str(),
                                        "module" | "interface" | "annotation"
                                    )
                            })
                            .all(|c| inheritance.get(&c.id).is_some_and(|h| h.status == "ready")),
                    descendants.into_iter().collect(),
                )
            },
            wmc: count(
                measured.iter().map(|m| u64::from(m.cyclomatic)).sum(),
                !methods.is_empty() && measured.len() == methods.len(),
                methods
                    .iter()
                    .filter(|m| m.complexity.is_some())
                    .map(|m| m.id.clone())
                    .collect(),
            ),
            lcom4: cohesion.get(&class.id).copied().flatten(),
        });
    }
    let mut methods = Vec::new();
    for method in &calls.methods {
        let measured = raw.get(&method.id).filter(|m| {
            m.definition == "source-maintainability/v1"
                && m.distinct_operators <= m.operators
                && m.distinct_operands <= m.operands
        });
        let halstead = measured.map(|m| {
            let vocabulary = f64::from(m.distinct_operators) + f64::from(m.distinct_operands);
            let volume = if vocabulary > 1.0 {
                (f64::from(m.operators) + f64::from(m.operands)) * vocabulary.log2()
            } else {
                0.0
            };
            let difficulty = (m.distinct_operands > 0).then(|| {
                f64::from(m.distinct_operators) / 2.0 * f64::from(m.operands)
                    / f64::from(m.distinct_operands)
            });
            HalsteadMetrics {
                distinct_operators: m.distinct_operators,
                distinct_operands: m.distinct_operands,
                operators: m.operators,
                operands: m.operands,
                volume,
                difficulty,
                effort: difficulty.map(|d| d * volume),
            }
        });
        let maintainability_index = halstead
            .as_ref()
            .zip(measured)
            .zip(
                method
                    .complexity
                    .as_ref()
                    .filter(|m| m.definition == "ast-decisions-v1"),
            )
            .filter(|((h, m), _)| h.volume > 0.0 && m.source_lines > 0)
            .map(|((h, m), c)| {
                ((171.0
                    - 5.2 * h.volume.ln()
                    - 0.23 * f64::from(c.cyclomatic)
                    - 16.2 * f64::from(m.source_lines).ln())
                    * 100.0
                    / 171.0)
                    .clamp(0.0, 100.0)
            });
        methods.push(MethodMetrics {
            method_id: method.id.clone(),
            node: class_nodes
                .get(&method.class_id)
                .filter(|c| c.file == method.file)
                .map(|c| c.graph_node.clone()),
            class_id: method.class_id.clone(),
            file: method.file.clone(),
            line: method.line,
            cognitive: measured.map(|m| m.cognitive),
            source_lines: measured.map(|m| m.source_lines),
            halstead,
            maintainability_index,
        });
    }
    classes.sort_by(|a, b| a.class_id.cmp(&b.class_id));
    methods.sort_by(|a, b| a.method_id.cmp(&b.method_id));
    let coverage = BTreeMap::from([
        (
            "cbo".into(),
            (
                classes.iter().filter(|c| c.cbo.value.is_some()).count(),
                classes.len(),
            ),
        ),
        (
            "rfc".into(),
            (
                classes.iter().filter(|c| c.rfc.value.is_some()).count(),
                classes.len(),
            ),
        ),
        (
            "dit".into(),
            (
                classes.iter().filter(|c| c.dit.value.is_some()).count(),
                classes.len(),
            ),
        ),
        (
            "noc".into(),
            (
                classes.iter().filter(|c| c.noc.value.is_some()).count(),
                classes.len(),
            ),
        ),
        (
            "lcom4".into(),
            (
                classes.iter().filter(|c| c.lcom4.is_some()).count(),
                classes.len(),
            ),
        ),
        (
            "cognitive".into(),
            (
                methods.iter().filter(|m| m.cognitive.is_some()).count(),
                methods.len(),
            ),
        ),
        (
            "maintainability".into(),
            (
                methods
                    .iter()
                    .filter(|m| m.maintainability_index.is_some())
                    .count(),
                methods.len(),
            ),
        ),
    ]);
    let mut coverage_actions = BTreeSet::new();
    for a in &calls.analyzers {
        if a.status != "ready" {
            coverage_actions.insert(match a.language.as_str(){
        "java"=>"Java: supply existing dependencies via [semantic.java].classpath or .archguard/java-classpath.txt (one platform-separated path list), and the matching Java release. ArchGuard reads dependencies without running Maven/Gradle or annotation processors.",
        "csharp"=>"C#: use the project target framework's installed .NET reference pack and existing restored obj/project.assets.json. Configure [semantic.csharp].project/target_framework when selection is ambiguous. ArchGuard never restores packages or runs MSBuild.",
        "typescript"|"javascript"=>"TypeScript/JavaScript: supply the project's installed compiler and tsconfig via [semantic.typescript].compiler/project and set ARCHGUARD_TRUST_PROJECT_COMPILER=1 for trusted repositories. Missing dependencies and compiler errors remain explicit.",
        "rust"=>"Rust: complexity, cognitive and Halstead come from syntax, as do abstractness and implemented traits. Calls and type references are resolved only with rust-analyzer and miss code from build scripts and proc macros, so CBO and RFC stay partial. DIT and NOC are not applicable: Rust has no implementation inheritance.",
        _=>"This language has no compiler-backed class/method metrics. Import/dependency and Git measurements remain available."
    }.to_owned());
        }
    }
    let mut report=CodeMetricsReport{definition:"source-metrics/v1".into(),status:calls.status.clone(),analyzers:calls.analyzers.clone(),coverage,coverage_actions:coverage_actions.into_iter().collect(),classes,methods,diagnostics:vec![
        "CBO: distinct outbound resolved declaration-type/call targets, excluding self and inheritance. RFC: declared non-constructor members plus distinct direct call targets; static dispatch only. DIT: superclass edges including compiler-known external bases; interfaces excluded. NOC: direct subclasses in analyzed sources only. WMC: sum of declared non-constructor body cyclomatic complexity; missing bodies remain unknown.".into(),
        "Cognitive source variant: control breaks plus nesting, else/else-if and logical-operator sequences; nested executable scopes excluded. Recursion is not included. Halstead uses AST expression/statement kinds as operators and identifier/literal spellings as operands, excluding blocks/parentheses and nested scopes; source lines are distinct AST token-start lines. MI = clamp((171 - 5.2 ln(volume) - 0.23 cyclomatic - 16.2 ln(source lines)) * 100 / 171, 0, 100); empty volume/lines remain unknown. Language AST vocabularies differ; compare only the same language and definition. Scores are review aids, not measured maintenance effort.".into()
    ]};
    report.diagnostics.extend(calls.diagnostics.iter().cloned());
    report
}
