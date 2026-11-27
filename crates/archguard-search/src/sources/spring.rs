//! Spring Data Elasticsearch / OpenSearch annotations in Java and Kotlin (ticket 23). Rules:
//! an annotation counts only when the file imports it from `org.springframework.data.elasticsearch
//! .annotations` (also for Spring Data OpenSearch); flavor is `opensearch` when the module's
//! pom.xml/build.gradle(.kts) declares `org.opensearch.client:spring-data-opensearch*` or the file
//! imports `org.opensearch.data.*`, else empty; a class
//! with `@Document(indexName = …)` is an `index` named by a literal or a string constant of its
//! service (Java; a Kotlin constant is a gap), `scope` = `spring:<module>`, evidence = the class
//! line. `@Field(type, name, analyzer, searchAnalyzer, normalizer)` is a field named `name` or the
//! property; a missing or `Auto` type keeps the field only with an analysis name. `@MultiField` is
//! the main field plus `<field>.<suffix>` per `@InnerField`. `@Setting(shards, replicas,
//! refreshInterval)` are settings; `settingPath` and `mappingPath` are classpath JSON below
//! `<module>/src/main/resources` and replace the annotation attributes (Spring: no other mapping
//! processing). `@Document` has no default index name (Javadoc). All or nothing per attribute: a
//! SpEL, placeholder, variable, escaping path or unreadable resource is a gap. Trust boundary:
//! files below the root, at most 5 000 source files, no test paths, no network; contents are data.
// shortcut: flavor is a plain substring match anywhere in the module's own build file (a parent
// pom, `gradle/libs.versions.toml` and a nested Gradle module without build file are missed) and
// the `org.opensearch.data` import counts per file, not per module; lift when a corpus needs it.
// shortcut: Kotlin classes need a `{}` body, nested classes share their outer class's fields and
// resources are found only under `src/main/resources`; lift when a corpus needs it.
use super::constants::{Constants, lang, literal};
use super::json::detail_from_json;
use super::spring_syntax::{Arg, Class, Scan, find};
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{
    SearchArtifact, SearchField, SearchIndexDetail, ServiceEvidence, ServiceTopology,
};
use archguard_flow_common::lang_text::line;
use archguard_flow_common::support::{owners, relative, test_path};
use serde_json::Value;

const MAX_FILES: usize = 5_000;
const MARKER: &str = "org.springframework.data.elasticsearch";
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const SETTINGS: [(&str, &str); 3] = [
    ("shards", "number_of_shards"),
    ("replicas", "number_of_replicas"),
    ("refreshInterval", "refresh_interval"),
];

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        es_gap(&mut out, EsGap::SpringScanStop, ".", None, &[]);
    }
    let mut constants = Constants::default();
    let mut annotated = vec![];
    for rel in &files {
        let Some(text) = (ctx.read)(ctx.root, &ctx.root.join(rel)) else {
            continue; // the code source reports unreadable files
        };
        constants.collect(&service_key(ctx, rel), ext(rel), rel, &text);
        if text.contains(MARKER) {
            annotated.push((rel, text));
        }
    }
    for (rel, text) in &annotated {
        let mut file = FileScan {
            ctx,
            rel,
            constants: &constants,
            out: &mut out,
        };
        file.run(text);
    }
    SourceOutput::from_scratch(out)
}

fn candidates(ctx: &SearchContext) -> Vec<String> {
    let mut files: Vec<String> = ctx
        .discovery
        .walk(ctx.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(ctx.root, e.path()))
        .filter(|rel| matches!(ext(rel), "java" | "kt") && !test_path(rel))
        .collect();
    files.sort();
    files
}

fn ext(rel: &str) -> &'static str {
    if rel.ends_with(".kt") { "kt" } else { "java" }
}

fn service_key(ctx: &SearchContext, rel: &str) -> String {
    owners(ctx.topology, rel).join(",")
}

/// The module folder (before `src/main/`, else `.`) and its classpath root.
fn module_of(rel: &str) -> (String, String) {
    let at = rel
        .match_indices("src/main/")
        .map(|(p, _)| p)
        .find(|&p| p == 0 || rel.as_bytes()[p - 1] == b'/');
    let module = at.map_or("", |p| rel[..p].trim_end_matches('/'));
    match module {
        "" => (".".into(), "src/main/resources".into()),
        m => (m.into(), format!("{m}/src/main/resources")),
    }
}

/// An argument is given: present and not the empty string (Spring: an empty path is unset).
fn given(args: &[Arg], key: &str) -> bool {
    find(args, key).is_some_and(|a| a.val != "\"\"")
}

struct FileScan<'a, 'c> {
    ctx: &'a SearchContext<'c>,
    rel: &'a str,
    constants: &'a Constants,
    out: &'a mut ServiceTopology,
}

impl FileScan<'_, '_> {
    /// The module's build file declares `org.opensearch.client:spring-data-opensearch*`.
    fn opensearch_build(&mut self) -> bool {
        let dir = module_of(self.rel).0;
        BUILD_FILES.iter().any(|f| {
            let path = if dir == "." {
                (*f).into()
            } else {
                format!("{dir}/{f}")
            };
            let full = self.ctx.root.join(&path);
            match (self.ctx.read)(self.ctx.root, &full) {
                Some(t) => {
                    t.contains("org.opensearch.client") && t.contains("spring-data-opensearch")
                }
                None => {
                    if full.exists() {
                        es_gap(self.out, EsGap::Unread, &path, None, &[]);
                    }
                    false
                }
            }
        })
    }

    fn gap(&mut self) {
        es_gap(self.out, EsGap::SpringNotLiteral, self.rel, None, &[]);
    }

    /// Literal text of an argument (no SpEL or placeholder), else a gap.
    fn text(&mut self, args: &[Arg], key: &str) -> Option<String> {
        let raw = find(args, key)?.val;
        let lit = literal(raw).filter(|s| !s.contains('{'));
        if lit.is_none() {
            self.gap();
        }
        lit
    }

    fn run(&mut self, text: &str) {
        let scan = Scan::new(text, ext(self.rel) == "kt");
        let flavor = if scan.imports_opensearch_data() || self.opensearch_build() {
            "opensearch"
        } else {
            ""
        };
        for class in &scan.classes {
            if let Some(a) = self.artifact(&scan, class, flavor) {
                self.out.elasticsearch.artifacts.push(a);
            }
        }
    }

    fn artifact(&mut self, s: &Scan, class: &Class, flavor: &str) -> Option<SearchArtifact> {
        let head = s.head(class);
        let named = |n: &str| {
            let a = head.iter().find(|a| a.name == n)?;
            s.provides(n).then(|| s.args_of(a))
        };
        s.provides("Document").then_some(())?;
        let (index, declared) = match self.index_name(&named("Document")?) {
            Some(i) => i,
            None => {
                self.gap();
                return None;
            }
        };
        let mut detail = SearchIndexDetail::default();
        let mapping = named("Mapping").filter(|a| given(a, "mappingPath"));
        match mapping {
            Some(a) => {
                detail.fields = self
                    .resource_of(&a, "mappingPath")
                    .map_or(vec![], |v| detail_from_json(Some(&v), None).fields)
            }
            None => detail.fields = self.annotated_fields(s, class),
        }
        self.settings(&mut detail, named("Setting").as_deref());
        let mut evidence = vec![ServiceEvidence {
            file: self.rel.into(),
            pointer: class.name.clone(),
            line: Some(line(s.text, class.keyword)),
            description: format!(
                "Index {index} declared by @Document on class {}; read as data, no cluster access.",
                class.name
            ),
        }];
        evidence.extend(declared);
        Some(SearchArtifact {
            flavor: flavor.to_owned(),
            kind: "index".into(),
            name: Some(index),
            address: format!("{}#{}", self.rel, class.name),
            scope: format!("spring:{}", module_of(self.rel).0),
            patterns: vec![],
            references: vec![],
            evidence,
            origins: vec![],
            detail: (detail != SearchIndexDetail::default()).then_some(detail),
        })
    }

    /// A literal or a string constant of the service; `None` is a gap (SpEL, variable, none).
    fn index_name(&self, args: &[Arg]) -> Option<(String, Option<ServiceEvidence>)> {
        let raw = find(args, "indexName")?.val;
        if let Some(l) = literal(raw).filter(|s| !s.contains('{')) {
            return Some((l, None));
        }
        let key = service_key(self.ctx, self.rel);
        let (name, at) = self.constants.resolve(&key, lang(ext(self.rel)), raw)?;
        Some((name, Some(at)))
    }

    /// `settingPath` replaces the attributes; each attribute is a literal number or string.
    fn settings(&mut self, d: &mut SearchIndexDetail, args: Option<&[Arg]>) {
        let Some(args) = args else { return };
        if given(args, "settingPath") {
            if let Some(v) = self.resource_of(args, "settingPath") {
                let from_file = detail_from_json(None, Some(&v));
                (d.settings, d.analysis) = (from_file.settings, from_file.analysis);
            }
            return;
        }
        for (key, name) in SETTINGS {
            let Some(raw) = find(args, key).map(|a| a.val) else {
                continue;
            };
            let number = raw
                .chars()
                .all(|c| c.is_ascii_digit())
                .then(|| raw.to_owned());
            match literal(raw).or(number).filter(|v| !v.is_empty()) {
                Some(v) => drop(d.settings.insert(name.into(), v)),
                None => self.gap(),
            }
        }
    }

    /// The JSON file a classpath path argument names, below the module's resources only.
    fn resource_of(&mut self, args: &[Arg], key: &str) -> Option<Value> {
        let path = self.text(args, key)?;
        let p = path.strip_prefix('/').unwrap_or(&path);
        let safe = !p.contains(['\\', ':']) && p.split('/').all(|c| !matches!(c, "" | "." | ".."));
        let file = format!("{}/{p}", module_of(self.rel).1);
        let value = safe
            .then(|| (self.ctx.read)(self.ctx.root, &self.ctx.root.join(&file)))
            .flatten()
            .and_then(|t| serde_json::from_str(&t).ok());
        if value.is_none() {
            self.gap();
        }
        value
    }

    fn annotated_fields(&mut self, s: &Scan, class: &Class) -> Vec<SearchField> {
        let mut out = vec![];
        for (i, a) in s.anns.iter().enumerate() {
            let inside = a.start > class.name_end && a.end <= class.close;
            if !inside || !s.provides(&a.name) {
                continue;
            }
            let args = s.args_of(a);
            match a.name.as_str() {
                "Field" => out.extend(self.main_field(&args, s.property(i))),
                "MultiField" => self.multi(s, &args, s.property(i), &mut out),
                _ => {}
            }
        }
        out
    }

    fn main_field(&mut self, args: &[Arg], property: Option<String>) -> Option<SearchField> {
        let path = if find(args, "name").is_some() {
            self.text(args, "name")?
        } else {
            property.or_else(|| {
                self.gap();
                None
            })?
        };
        self.typed(args, path)
    }

    /// The field of `path` with its type and analysis names; `Auto` is Spring's default and acts like
    /// no type: kept only with an analysis name (Spring writes those regardless).
    fn typed(&mut self, args: &[Arg], path: String) -> Option<SearchField> {
        let kind = match find(args, "type") {
            None => String::new(),
            Some(a) => {
                let parts: Vec<&str> = a.val.split('.').map(str::trim).collect();
                match parts.as_slice() {
                    [.., "FieldType", "Auto"] => String::new(),
                    [.., "FieldType", "TokenCount"] => "token_count".into(),
                    [.., "FieldType", t] => t.to_ascii_lowercase(),
                    _ => {
                        self.gap();
                        return None;
                    }
                }
            }
        };
        let field = SearchField {
            path,
            kind,
            analyzer: self.text(args, "analyzer"),
            search_analyzer: self.text(args, "searchAnalyzer"),
            normalizer: self.text(args, "normalizer"),
            ..Default::default()
        };
        let named = field.analyzer.is_some()
            || field.search_analyzer.is_some()
            || field.normalizer.is_some();
        (!field.kind.is_empty() || named).then_some(field)
    }

    fn multi(
        &mut self,
        s: &Scan,
        args: &[Arg],
        property: Option<String>,
        out: &mut Vec<SearchField>,
    ) {
        let Some(main) = find(args, "mainField") else {
            return;
        };
        let Some(field) = self.main_field(&s.call_args(main.at), property) else {
            return;
        };
        let path = field.path.clone();
        out.push(field);
        let Some(other) = find(args, "otherFields") else {
            return;
        };
        let span = &s.m[other.at..other.at + other.val.len()];
        for (p, _) in span.match_indices("InnerField") {
            let inner = s.call_args(other.at + p);
            if let Some(suffix) = self.text(&inner, "suffix") {
                out.extend(self.typed(&inner, format!("{path}.{suffix}")));
            }
        }
    }
}
