//! Painless scripts as their own call analyzer (ADR 0023): one class per `.painless` file or
//! Terraform resource, one method per script, measured by `archguard_extract::painless`.
//! Calls stay unresolved. Scripts are read from `.painless` files and from literal
//! `script = { source = … }` objects in resource attributes (also inside `jsonencode(…)`).
// shortcut: scripts inside JSON strings and `script { }` blocks are not found; add when a repo needs it.
use crate::services::service_terraform_hcl::{literal, object};
use archguard_core::{CallAnalysis, CallAnalyzer, CallClass, CallMethod, Config, MethodComplexity};
use archguard_extract::Excludes;
use archguard_extract::painless::measure;
use hcl_edit::expr::Expression;
use hcl_edit::{Span, parser};
use sha2::{Digest, Sha256};
use std::path::Path;

const LIMIT: u64 = 1024 * 1024;
const DEFINITION: &str = "ast-decisions-v1";
const NOTE: &str = archguard_core::CallAnalyzer::PAINLESS_TOKENS;

/// `.painless` and `.tf` files outside `.terraform/`, sorted by path; text is `None` when unreadable.
fn sources(root: &Path, config: &Config) -> Result<Vec<(String, Option<String>)>, String> {
    // Same exclusion rule as `walk_files` (`project.exclude`); that walk lists source languages only.
    let excluded = Excludes::new(&config.project.exclude).map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    let base = root.to_path_buf();
    let walker = ignore::WalkBuilder::new(root)
        .require_git(false)
        .filter_entry(move |e| {
            let rel = e.path().strip_prefix(&base).unwrap_or(e.path());
            let rel = rel.to_string_lossy().replace('\\', "/");
            let dir = e.file_type().is_some_and(|t| t.is_dir());
            e.depth() == 0 || (!excluded.is_match(&rel, dir) && e.file_name() != ".terraform")
        })
        .build();
    for entry in walker {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let wanted = path
            .extension()
            .is_some_and(|x| x == "painless" || x == "tf");
        if !wanted || !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        let text = crate::project_files::read_repo_file(root, rel, LIMIT)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        found.push((rel.to_string_lossy().replace('\\', "/"), text));
    }
    found.sort();
    Ok(found)
}

/// Content identity of everything the analyzer reads; the memo key knows only graph sources.
pub(crate) fn painless_calls_fingerprint(root: &Path, config: &Config) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    for (path, text) in sources(root, config)? {
        hash.update(path.as_bytes());
        hash.update(b"\0");
        hash.update(text.as_deref().unwrap_or("\0unreadable").as_bytes());
        hash.update(b"\0");
    }
    Ok(hash.finalize().into())
}

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_painless_calls(root: &Path, config: &Config) -> CallAnalysis {
    let mut report = CallAnalysis {
        java_release: 0,
        ..Default::default()
    };
    let files = match sources(root, config) {
        Ok(files) => files,
        Err(error) => {
            report.status = "unavailable".into();
            report.diagnostics.push(error);
            return report;
        }
    };
    for (path, text) in files {
        let before = report.classes.len();
        match text {
            None => report.diagnostics.push(format!(
                "{path}: could not be read; its scripts are unknown."
            )),
            Some(text) if path.ends_with(".painless") => file_script(&mut report, &path, &text),
            Some(text) => terraform_scripts(&mut report, &path, &text),
        }
        report.files += usize::from(report.classes.len() > before);
    }
    // Unreadable or invalid files may hide scripts: keep their diagnostics even without a script.
    if report.files == 0 && report.diagnostics.is_empty() {
        return report;
    }
    let ready = report.diagnostics.is_empty();
    report.status = if ready { "ready" } else { "partial" }.into();
    report.diagnostics.push(NOTE.into());
    report.analyzers.push(CallAnalyzer {
        language: "painless".into(),
        backend: "archguard painless".into(),
        status: report.status.clone(),
        files: report.files,
        version: None,
        project: None,
        diagnostics: report.diagnostics.clone(),
    });
    report
}

fn class(path: &str, id: String) -> CallClass {
    CallClass {
        language: Some("painless".into()),
        name: id.clone(),
        id,
        package: String::new(),
        file: path.into(),
        graph_node: path.into(),
        line: 1,
        kind: "script".into(),
        is_abstract: None,
        visibility: None,
        supertypes: vec![],
        supertype_sites: None,
        declaration_files: None,
    }
}

/// One method for `source`, or a gap diagnostic when it cannot be measured.
fn method(report: &mut CallAnalysis, class_id: &str, (name, line): (&str, u32), source: &str) {
    let path = report
        .classes
        .last()
        .map_or("", |c| c.file.as_str())
        .to_owned();
    let Some(m) = measure(source) else {
        let message = format!(
            "{path}:{line}: Painless script could not be measured (unbalanced brackets or unterminated string, comment or regex); it has no method."
        );
        report.diagnostics.push(message);
        return;
    };
    let lines = u32::try_from(source.trim_end().lines().count()).unwrap_or(u32::MAX);
    report.methods.push(CallMethod {
        id: format!("{class_id}#{name}()"),
        class_id: class_id.into(),
        name: name.into(),
        signature: format!("{name}()"),
        file: path,
        line,
        end_line: Some(line + lines.saturating_sub(1)),
        is_static: false,
        complexity: Some(MethodComplexity {
            cyclomatic: m.cyclomatic,
            max_nesting: m.max_nesting,
            definition: DEFINITION.into(),
        }),
    });
}

fn file_script(report: &mut CallAnalysis, path: &str, text: &str) {
    report.classes.push(class(path, path.into()));
    let stem = Path::new(path).file_stem().and_then(|s| s.to_str());
    method(report, path, (stem.unwrap_or("script"), 1), text);
}

enum Source {
    Literal(String),
    /// `file(…)`/`join(…)`: the referenced `.painless` files count themselves.
    File,
    Dynamic,
}

fn source_of(e: &Expression) -> Source {
    match (literal(e), e) {
        (Some(text), _) => Source::Literal(text),
        (None, Expression::FuncCall(c)) if matches!(c.name.name.as_str(), "file" | "join") => {
            Source::File
        }
        _ => Source::Dynamic,
    }
}

/// `source` expressions of painless `script = { … }` objects anywhere in `e`.
fn scripts<'e>(e: &'e Expression, out: &mut Vec<&'e Expression>) {
    match e {
        Expression::FuncCall(call) => call.args.iter().for_each(|a| scripts(a, out)),
        Expression::Array(items) => items.iter().for_each(|i| scripts(i, out)),
        Expression::Object(_) => {
            for (key, value) in object(e) {
                let fields = object(value);
                let field = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| *v);
                let lang = field("lang").and_then(literal);
                match field("source") {
                    Some(s) if key == "script" && lang.is_none_or(|l| l == "painless") => {
                        out.push(s);
                    }
                    _ => scripts(value, out),
                }
            }
        }
        _ => {}
    }
}

fn terraform_scripts(report: &mut CallAnalysis, path: &str, text: &str) {
    let Ok(body) = parser::parse_body(text) else {
        report.diagnostics.push(format!(
            "{path}: Terraform file is not valid HCL; its scripts are unknown."
        ));
        return;
    };
    let starts = crate::graphql_scan::line_starts(text);
    let mut dynamic = 0;
    for block in body.blocks().filter(|b| b.ident.as_str() == "resource") {
        let mut found = Vec::new();
        block
            .body
            .attributes()
            .for_each(|a| scripts(&a.value, &mut found));
        if found.is_empty() {
            continue;
        }
        let address = block.labels.iter().map(|l| l.as_str()).collect::<Vec<_>>();
        let id = format!("{path}:{}", address.join("."));
        report.classes.push(class(path, id.clone()));
        for source in found {
            let at = source.span().map_or(0, |s| s.start);
            let line = u32::try_from(starts.partition_point(|&s| s <= at)).unwrap_or(1);
            match source_of(source) {
                Source::Literal(text) => {
                    method(report, &id, (&format!("script@{line}"), line), &text)
                }
                Source::File => {}
                Source::Dynamic => dynamic += 1,
            }
        }
    }
    if dynamic > 0 {
        report.diagnostics.push(format!(
            "{path}: {dynamic} script source(s) are not literals (variables, locals, templates, other functions); they have no method."
        ));
    }
}
