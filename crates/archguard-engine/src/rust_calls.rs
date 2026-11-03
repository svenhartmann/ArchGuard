//! Syntax-only Rust measurements: declarations and per-function complexity from `syn`.
//! Nothing is compiled or executed, so calls and types stay unresolved and the
//! report is `partial`. The counts follow `ast-decisions-v1` and
//! `source-maintainability/v1` of the compiler-backed scanners, mapped to Rust:
//! - cyclomatic: `if`, `for`, `while`, `loop`, `let … else`, each `match` arm except
//!   a final `_`, each arm guard, `&&`, `||` and `?` (the counterpart of `?.`).
//! - cognitive: as the scanners; `?` is an early exit and adds nothing.
//! - closures belong to their function one nesting level deeper; nested `fn` items
//!   and `#[cfg(test)]`/`#[test]` code are not measured.
//! - macro arguments count when they parse as expressions; other bodies stay opaque.
use std::collections::BTreeSet;
use std::path::Path;

use archguard_core::{
    CallAnalysis, CallAnalyzer, CallClass, CallMethod, Config, MethodComplexity, SemanticDetails,
    SourceMethodMetrics,
};
use archguard_extract::{Lang, strip_root};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, ImplItem, Item, Signature, TraitItem, Visibility};

const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
const LIMITATION: &str = "Syntax-only analysis: complexity and Halstead per function. Calls and types are not resolved, and macro bodies that are not expressions stay opaque.";

#[derive(Default)]
struct Measure {
    cyclomatic: u32,
    cognitive: u32,
    depth: u32,
    max_nesting: u32,
    operators: BTreeSet<String>,
    operands: BTreeSet<String>,
    operator_count: u32,
    operand_count: u32,
    lines: BTreeSet<usize>,
}

impl Measure {
    fn operator(&mut self, key: &str, line: usize) {
        self.operators.insert(key.to_owned());
        self.operator_count += 1;
        self.lines.insert(line);
    }
    fn operand(&mut self, key: String, line: usize) {
        self.operands.insert(key);
        self.operand_count += 1;
        self.lines.insert(line);
    }
    fn nested(&mut self, body: impl FnOnce(&mut Self)) {
        self.depth += 1;
        self.max_nesting = self.max_nesting.max(self.depth);
        body(self);
        self.depth -= 1;
    }
    fn branch(&mut self, node: &syn::ExprIf, ladder: bool) {
        self.operator("if", node.if_token.span().start().line);
        self.cyclomatic += 1;
        self.cognitive += if ladder { 1 } else { 1 + self.depth };
        self.max_nesting = self.max_nesting.max(self.depth + 1);
        self.visit_expr(&node.cond);
        self.nested(|m| m.visit_block(&node.then_branch));
        if let Some((_, otherwise)) = &node.else_branch {
            if let Expr::If(next) = otherwise.as_ref() {
                self.branch(next, true);
            } else {
                self.cognitive += 1;
                self.nested(|m| m.visit_expr(otherwise));
            }
        }
    }
    /// Operands of a run of `&&`/`||`, with each change of operator counted once.
    fn sequence(&mut self, node: &Expr, list: &mut Vec<bool>) {
        match node {
            Expr::Paren(inner) => self.sequence(&inner.expr, list),
            Expr::Binary(binary) if logical(&binary.op).is_some() => {
                let and = logical(&binary.op) == Some(true);
                self.operator(if and { "&&" } else { "||" }, binary.op.span().start().line);
                self.cyclomatic += 1;
                self.sequence(&binary.left, list);
                list.push(and);
                self.sequence(&binary.right, list);
            }
            other => self.visit_expr(other),
        }
    }
}

fn logical(op: &syn::BinOp) -> Option<bool> {
    match op {
        syn::BinOp::And(_) => Some(true),
        syn::BinOp::Or(_) => Some(false),
        _ => None,
    }
}

fn expression_kind(node: &Expr) -> &'static str {
    match node {
        Expr::Array(_) => "array",
        Expr::Assign(_) => "assign",
        Expr::Async(_) => "async",
        Expr::Await(_) => "await",
        Expr::Block(_) => "block",
        Expr::Break(_) => "break",
        Expr::Call(_) => "call",
        Expr::Cast(_) => "cast",
        Expr::Closure(_) => "closure",
        Expr::Const(_) => "const",
        Expr::Continue(_) => "continue",
        Expr::Field(_) => "field",
        Expr::ForLoop(_) => "for",
        Expr::Index(_) => "index",
        Expr::Let(_) => "let",
        Expr::Loop(_) => "loop",
        Expr::Macro(_) => "macro",
        Expr::Match(_) => "match",
        Expr::MethodCall(_) => "method_call",
        Expr::Range(_) => "range",
        Expr::Reference(_) => "reference",
        Expr::Repeat(_) => "repeat",
        Expr::Return(_) => "return",
        Expr::Struct(_) => "struct",
        Expr::Try(_) => "try",
        Expr::TryBlock(_) => "try_block",
        Expr::Tuple(_) => "tuple",
        Expr::Unary(_) => "unary",
        Expr::Unsafe(_) => "unsafe",
        Expr::While(_) => "while",
        Expr::Yield(_) => "yield",
        _ => "other",
    }
}

impl<'ast> Visit<'ast> for Measure {
    fn visit_expr(&mut self, node: &'ast Expr) {
        let line = node.span().start().line;
        match node {
            Expr::If(branch) => return self.branch(branch, false),
            Expr::Paren(inner) => return self.visit_expr(&inner.expr),
            Expr::Path(_) | Expr::Lit(_) => return visit::visit_expr(self, node),
            Expr::Binary(binary) if logical(&binary.op).is_some() => {
                let mut list = Vec::new();
                self.sequence(node, &mut list);
                self.cognitive += 1 + list.windows(2).filter(|w| w[0] != w[1]).count() as u32;
                return;
            }
            Expr::Binary(binary) => {
                let key = binary.op.to_token_stream().to_string();
                self.operator(&key, line);
                return visit::visit_expr(self, node);
            }
            _ => self.operator(expression_kind(node), line),
        }
        match node {
            Expr::ForLoop(_) | Expr::While(_) | Expr::Loop(_) => {
                self.cyclomatic += 1;
                self.cognitive += 1 + self.depth;
                self.nested(|m| visit::visit_expr(m, node));
            }
            Expr::Match(choice) => {
                self.cognitive += 1 + self.depth;
                self.visit_expr(&choice.expr);
                let last = choice.arms.len().saturating_sub(1);
                self.nested(|m| {
                    for (index, arm) in choice.arms.iter().enumerate() {
                        let wildcard = matches!(arm.pat, syn::Pat::Wild(_));
                        m.cyclomatic += u32::from(!(wildcard && index == last));
                        m.cyclomatic += u32::from(arm.guard.is_some());
                        m.visit_arm(arm);
                    }
                });
            }
            Expr::Try(_) => {
                self.cyclomatic += 1;
                visit::visit_expr(self, node);
            }
            Expr::Break(syn::ExprBreak { label: Some(_), .. })
            | Expr::Continue(syn::ExprContinue { label: Some(_), .. }) => {
                self.cognitive += 1;
                visit::visit_expr(self, node);
            }
            Expr::Closure(_) => self.nested(|m| visit::visit_expr(m, node)),
            _ => visit::visit_expr(self, node),
        }
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // Expression arguments are ordinary code; `matches!` patterns and DSLs stay opaque.
        if let Ok(arguments) =
            node.parse_body_with(Punctuated::<Expr, syn::Token![,]>::parse_terminated)
        {
            for argument in &arguments {
                self.visit_expr(argument);
            }
        }
    }
    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.operator("let", node.span().start().line);
        if node
            .init
            .as_ref()
            .is_some_and(|init| init.diverge.is_some())
        {
            self.cyclomatic += 1;
            self.cognitive += 1 + self.depth;
            self.max_nesting = self.max_nesting.max(self.depth + 1);
        }
        visit::visit_local(self, node);
    }
    fn visit_ident(&mut self, node: &'ast proc_macro2::Ident) {
        self.operand(node.to_string(), node.span().start().line);
    }
    fn visit_lit(&mut self, node: &'ast syn::Lit) {
        self.operand(node.to_token_stream().to_string(), node.span().start().line);
    }
    // A nested item is its own scope, not part of this function.
    fn visit_item(&mut self, _: &'ast Item) {}
}

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg")
                && attr.to_token_stream().to_string().replace(' ', "") == "#[cfg(test)]")
    })
}

struct Collector<'a> {
    file: &'a str,
    package: &'a str,
    report: &'a mut CallAnalysis,
    metrics: &'a mut Vec<SourceMethodMetrics>,
}

impl Collector<'_> {
    fn class(
        &mut self,
        name: &str,
        kind: &str,
        line: usize,
        visibility: Option<&Visibility>,
    ) -> String {
        let id = format!("{}::{name}", self.file);
        if !self.report.classes.iter().any(|class| class.id == id) {
            self.report.classes.push(CallClass {
                language: Some("rust".into()),
                id: id.clone(),
                name: name.into(),
                package: self.package.into(),
                file: self.file.into(),
                graph_node: self.file.into(),
                line: line as u32,
                kind: kind.into(),
                is_abstract: Some(kind == "trait"),
                visibility: visibility.map(visibility_name),
                supertypes: Vec::new(),
                supertype_sites: None,
                declaration_files: None,
            });
        }
        id
    }
    fn function(&mut self, class_id: &str, signature: &Signature, block: &syn::Block) {
        let line = signature.span().start().line as u32;
        let id = format!("{class_id}#{}@{line}", signature.ident);
        let mut measure = Measure {
            cyclomatic: 1,
            ..Default::default()
        };
        measure.visit_block(block);
        self.report.methods.push(CallMethod {
            id: id.clone(),
            class_id: class_id.into(),
            name: signature.ident.to_string(),
            signature: signature.to_token_stream().to_string(),
            file: self.file.into(),
            line,
            end_line: Some(block.span().end().line as u32),
            is_static: signature.receiver().is_none(),
            complexity: Some(MethodComplexity {
                cyclomatic: measure.cyclomatic,
                max_nesting: measure.max_nesting,
                definition: "ast-decisions-v1".into(),
            }),
        });
        self.metrics.push(SourceMethodMetrics {
            method_id: id,
            definition: "source-maintainability/v1".into(),
            cognitive: measure.cognitive,
            distinct_operators: measure.operators.len() as u32,
            distinct_operands: measure.operands.len() as u32,
            operators: measure.operator_count,
            operands: measure.operand_count,
            source_lines: measure.lines.len() as u32,
        });
    }
    fn items(&mut self, items: &[Item], prefix: &str) {
        for item in items {
            let scoped = |name: String| format!("{prefix}{name}");
            match item {
                Item::Fn(function) if !test_only(&function.attrs) => {
                    let name = if prefix.is_empty() {
                        "(module)".to_owned()
                    } else {
                        prefix.trim_end_matches("::").to_owned()
                    };
                    let class = self.class(&name, "module", 1, None);
                    self.function(&class, &function.sig, &function.block);
                }
                Item::Struct(node) if !test_only(&node.attrs) => {
                    self.class(
                        &scoped(node.ident.to_string()),
                        "struct",
                        node.span().start().line,
                        Some(&node.vis),
                    );
                }
                Item::Enum(node) if !test_only(&node.attrs) => {
                    self.class(
                        &scoped(node.ident.to_string()),
                        "enum",
                        node.span().start().line,
                        Some(&node.vis),
                    );
                }
                Item::Union(node) if !test_only(&node.attrs) => {
                    self.class(
                        &scoped(node.ident.to_string()),
                        "union",
                        node.span().start().line,
                        Some(&node.vis),
                    );
                }
                Item::Trait(node) if !test_only(&node.attrs) => {
                    let class = self.class(
                        &scoped(node.ident.to_string()),
                        "trait",
                        node.span().start().line,
                        Some(&node.vis),
                    );
                    for member in &node.items {
                        if let TraitItem::Fn(function) = member
                            && let Some(block) = &function.default
                            && !test_only(&function.attrs)
                        {
                            self.function(&class, &function.sig, block);
                        }
                    }
                }
                Item::Impl(node) if !test_only(&node.attrs) => {
                    let syn::Type::Path(path) = node.self_ty.as_ref() else {
                        continue;
                    };
                    let Some(segment) = path.path.segments.last() else {
                        continue;
                    };
                    let class = self.class(
                        &scoped(segment.ident.to_string()),
                        "impl",
                        node.span().start().line,
                        None,
                    );
                    for member in &node.items {
                        if let ImplItem::Fn(function) = member
                            && !test_only(&function.attrs)
                        {
                            self.function(&class, &function.sig, &function.block);
                        }
                    }
                }
                Item::Mod(node) if !test_only(&node.attrs) => {
                    if let Some((_, inner)) = &node.content {
                        self.items(inner, &format!("{prefix}{}::", node.ident));
                    }
                }
                _ => {}
            }
        }
    }
}

fn visibility_name(visibility: &Visibility) -> String {
    match visibility {
        Visibility::Public(_) => "public".into(),
        Visibility::Restricted(_) => "restricted".into(),
        Visibility::Inherited => "private".into(),
    }
}

pub(crate) fn analyze_rust_calls(root: &Path, config: &Config) -> CallAnalysis {
    let mut report = CallAnalysis {
        java_release: 0,
        ..Default::default()
    };
    let files = match crate::pipeline::walk_files(root, config) {
        Ok(files) => files,
        Err(error) => {
            report.status = "unavailable".into();
            report.diagnostics.push(error.to_string());
            return report;
        }
    };
    let mut metrics = Vec::new();
    for file in files.iter().filter(|file| file.lang == Lang::Rust) {
        report.files += 1;
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let parsed =
            crate::project_files::read_repo_file(root, Path::new(&file.rel_path), SOURCE_LIMIT)
                .map_err(|error| error.to_string())
                .and_then(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()))
                .and_then(|text| syn::parse_file(&text).map_err(|error| error.to_string()));
        match parsed {
            Ok(syntax) => {
                let package = stripped
                    .rsplit_once('/')
                    .map_or("", |(dir, _)| dir)
                    .to_owned();
                Collector {
                    file: &stripped,
                    package: &package,
                    report: &mut report,
                    metrics: &mut metrics,
                }
                .items(&syntax.items, "");
            }
            Err(error) => report.diagnostics.push(format!("{stripped}: {error}")),
        }
    }
    if report.files == 0 {
        return report;
    }
    report.status = "partial".into();
    report.diagnostics.push(LIMITATION.into());
    report.analyzers.push(CallAnalyzer {
        language: "rust".into(),
        backend: "syn".into(),
        status: "partial".into(),
        files: report.files,
        version: None,
        project: None,
        diagnostics: vec![LIMITATION.into()],
    });
    report.details = Some(SemanticDetails {
        enumeration_complete: Some(false),
        method_metrics: metrics,
        ..Default::default()
    });
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(source: &str) -> CallAnalysis {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), source).unwrap();
        analyze_rust_calls(dir.path(), &Config::default())
    }
    fn complexity(report: &CallAnalysis, name: &str) -> (u32, u32, u32) {
        let method = report.methods.iter().find(|m| m.name == name).unwrap();
        let measured = method.complexity.as_ref().unwrap();
        let raw = report.details.as_ref().unwrap().method_metrics.iter();
        let cognitive = raw
            .clone()
            .find(|m| m.method_id == method.id)
            .unwrap()
            .cognitive;
        (measured.cyclomatic, measured.max_nesting, cognitive)
    }

    #[test]
    fn counts_follow_the_shared_rules() {
        let report = analyze(
            r#"
fn plain() -> u32 { 1 }
fn branches(a: bool, b: bool) -> u32 {
    if a {
        if b { 1 } else { 2 }
    } else if b {
        3
    } else {
        4
    }
}
fn logic(a: bool, b: bool, c: bool) -> bool { a && b || c }
fn arms(x: u32) -> u32 {
    match x {
        0 => 1,
        n if n > 5 => 2,
        _ => 3,
    }
}
fn early(x: Option<u32>) -> Result<u32, ()> {
    let Some(y) = x else { return Err(()); };
    let z = "1".parse::<u32>().map_err(|_| ())?;
    Ok(y + z)
}
fn looped() { for _ in 0..2 { while true { loop { break; } } } }
fn closure(v: Vec<u32>) -> Vec<u32> { v.into_iter().map(|x| if x > 1 { x } else { 0 }).collect() }
fn in_macro(a: bool) { assert!(a && !a); }
"#,
        );
        assert_eq!(complexity(&report, "plain"), (1, 0, 0));
        // if, nested if, else-if: 1 + 2(nested) + 1(inner else) + 1(ladder) + 1(else).
        assert_eq!(complexity(&report, "branches"), (4, 2, 6));
        assert_eq!(complexity(&report, "logic"), (3, 0, 2));
        // Two counted arms and one guard; the final `_` does not count.
        assert_eq!(complexity(&report, "arms"), (4, 1, 1));
        // `let else` and `?` are decisions; only `let else` is cognitive.
        assert_eq!(complexity(&report, "early"), (3, 1, 1));
        assert_eq!(complexity(&report, "looped"), (4, 3, 6));
        // The closure's `if` sits one level deeper: 1 + 1(depth) + 1(else).
        assert_eq!(complexity(&report, "closure"), (2, 2, 3));
        assert_eq!(complexity(&report, "in_macro"), (2, 0, 1));
    }

    #[test]
    fn declarations_and_scopes_map_to_classes_and_methods() {
        let report = analyze(
            r#"
pub struct Thing;
impl Thing { pub fn new() -> Self { Self } fn get(&self) -> u32 { fn nested() { if true {} } 1 } }
trait Named { fn name(&self) -> String { String::new() } fn required(&self); }
mod inner { pub fn helper() {} }
fn free() {}
#[cfg(test)]
mod tests { fn ignored() {} }
#[test]
fn also_ignored() {}
"#,
        );
        let mut methods: Vec<_> = report.methods.iter().map(|m| m.name.as_str()).collect();
        methods.sort();
        assert_eq!(methods, ["free", "get", "helper", "name", "new"]);
        assert_eq!(
            complexity(&report, "get"),
            (1, 0, 0),
            "nested fn is its own scope"
        );
        let get = report.methods.iter().find(|m| m.name == "get").unwrap();
        assert!(!get.is_static);
        assert_eq!(report.status, "partial");
        assert_eq!(report.analyzers[0].backend, "syn");
        assert!(report.calls.is_empty());
        assert_eq!(
            report.details.as_ref().unwrap().enumeration_complete,
            Some(false)
        );
        let classes: BTreeSet<_> = report.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(classes.is_superset(&BTreeSet::from(["Thing", "Named", "inner", "(module)"])));
    }

    #[test]
    fn code_metrics_report_complexity_and_leave_call_metrics_unmeasured() {
        let report = analyze(
            "pub struct Thing;\nimpl Thing {\n    pub fn a(&self, x: bool) -> u32 { if x { 1 } else { 2 } }\n    pub fn b(&self) -> u32 { 3 }\n}\n",
        );
        let metrics = crate::code_metrics::code_metrics(&report);
        let class = metrics
            .classes
            .iter()
            .find(|class| class.class_id.ends_with("::Thing"))
            .unwrap();
        assert_eq!(class.wmc.value, Some(3));
        assert_eq!(class.cbo.value, None, "calls are not resolved");
        assert_eq!(class.rfc.value, None, "calls are not resolved");
        assert_eq!(class.dit.value, None);
        let method = metrics
            .methods
            .iter()
            .find(|method| method.method_id.contains("#a@"))
            .unwrap();
        assert_eq!(method.cognitive, Some(2));
        assert!(method.halstead.is_some());
        assert!(method.maintainability_index.is_some());
    }

    #[test]
    fn invalid_syntax_is_reported_and_other_files_still_count() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.rs"), "fn broken(").unwrap();
        std::fs::write(dir.path().join("good.rs"), "fn fine() {}").unwrap();
        let report = analyze_rust_calls(dir.path(), &Config::default());
        assert_eq!(report.methods.len(), 1);
        assert!(report.diagnostics.iter().any(|d| d.starts_with("bad.rs:")));
    }

    #[test]
    fn projects_without_rust_are_not_applicable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "export const a = 1;").unwrap();
        assert_eq!(
            analyze_rust_calls(dir.path(), &Config::default()).status,
            "not_applicable"
        );
    }
}
