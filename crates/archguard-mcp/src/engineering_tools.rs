//! Read-only engineering evidence, flattened before bounded pagination.
use archguard_engine::{
    ApiContractReport, CohesionReport, MutationReport, QualityHistoryReport, TestEvidenceReport,
    TestImpactReport,
};
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct EngineeringParams {
    /// Project inside the MCP server root.
    pub path: Option<String>,
    /// Case-insensitive substring of record identities or descriptions.
    pub query: Option<String>,
    /// First flattened record; follow next_offset.
    pub offset: Option<usize>,
    /// 1..=200 processed records, further bounded by 4 KiB.
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TestImpactParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Exact graph file/package names or mapped project-relative source paths.
    /// Omit to list imported tests and their measured durations.
    pub targets: Option<Vec<String>>,
}
pub fn validate(p: &EngineeringParams) -> Result<(), ErrorData> {
    if !(1..=200).contains(&p.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    if p.query
        .as_ref()
        .is_some_and(|q| q.len() > 512 || q.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "query must be at most 512 bytes without NUL",
        ));
    }
    Ok(())
}
pub fn validate_tests(p: &TestImpactParams) -> Result<(), ErrorData> {
    validate(&p.page)?;
    if p.targets.as_ref().is_some_and(|v| {
        v.len() > 256
            || v.iter()
                .any(|t| t.is_empty() || t.len() > 4096 || t.contains('\0'))
    }) {
        return Err(crate::tools::invalid_params(
            "targets must contain at most 256 nonempty exact names of at most 4096 bytes, without NUL",
        ));
    }
    Ok(())
}
pub fn metrics_value(
    code: &archguard_engine::CodeMetricsReport,
    knowledge: &archguard_engine::KnowledgeReport,
    runs: &archguard_engine::MeasurementReport,
    p: &EngineeringParams,
) -> Result<Value, ErrorData> {
    let mut rows = Vec::new();
    // Provider coverage and repair actions are independently queryable records.
    for analyzer in &code.analyzers {
        rows.push(json!({"kind":"analyzer_coverage","analyzer":analyzer}));
    }
    for (name, (measured, total)) in &code.coverage {
        rows.push(json!({"kind":"metric_coverage","metric":name,"measured":measured,"declarations":total}));
    }
    for action in &code.coverage_actions {
        rows.push(json!({"kind":"coverage_action","message":action}));
    }
    for class in &code.classes {
        let mut value = json!(class);
        for key in ["cbo", "rfc", "dit", "noc", "wmc"] {
            value[key].as_object_mut().unwrap().remove("evidence");
        }
        rows.push(json!({"kind":"class_metrics","class":value}));
        for (name, metric) in [
            ("cbo", &class.cbo),
            ("rfc", &class.rfc),
            ("dit", &class.dit),
            ("noc", &class.noc),
            ("wmc", &class.wmc),
        ] {
            for target in &metric.evidence {
                rows.push(json!({"kind":"class_metric_evidence","class_id":class.class_id,"metric":name,"target":target}));
            }
        }
    }
    for method in &code.methods {
        rows.push(json!({"kind":"method_metrics","method":method}));
    }
    for (level, entries) in [("file", &knowledge.files), ("package", &knowledge.packages)] {
        for entry in entries {
            let mut value = json!(entry);
            value.as_object_mut().unwrap().remove("authors");
            rows.push(json!({"kind":"knowledge","level":level,"measurement":value}));
            for author in &entry.authors {
                rows.push(json!({"kind":"contributor","level":level,"node":entry.node,"contribution":author}));
            }
        }
    }
    for group in &runs.groups {
        let mut value = json!(group);
        value.as_object_mut().unwrap().remove("run_ids");
        rows.push(json!({"kind":"measured_runs","measurement":value}));
        for run in &group.run_ids {
            rows.push(json!({"kind":"run_evidence","measurement_kind":group.kind,"name":group.name,"revision":group.revision,"run_id":run}));
        }
    }
    for d in code
        .diagnostics
        .iter()
        .chain(&knowledge.diagnostics)
        .chain(&runs.diagnostics)
    {
        rows.push(json!({"kind":"diagnostic","message":d}));
    }
    let q = query(p);
    page(
        &code.status,
        json!({"definition":code.definition,"classes":code.classes.len(),"methods":code.methods.len(),"knowledge_status":knowledge.status,"knowledge_window":knowledge.since,"measurements_status":runs.status,"runs":runs.runs,"flaky_test_percent":runs.flaky_test_percent,"limitations":"Versioned source metrics; unknown values remain null. Git concentration is a heuristic. Run revisions/environments are producer-declared and unverified. No project code is executed by this tool."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        p,
        |r| r.clone(),
    )
}
fn query(p: &EngineeringParams) -> String {
    p.query.as_deref().unwrap_or("").trim().to_lowercase()
}
fn matches(q: &str, values: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    q.is_empty()
        || values
            .into_iter()
            .any(|s| s.as_ref().to_lowercase().contains(q))
}
pub fn duplicates_value(
    r: &archguard_engine::DuplicationReport,
    p: &EngineeringParams,
) -> Result<Value, ErrorData> {
    #[derive(Clone, Copy)]
    enum Row<'a> {
        Group(&'a archguard_engine::DuplicateGroup),
        Location(
            &'a archguard_engine::DuplicateGroup,
            &'a archguard_engine::DuplicateOccurrence,
        ),
        Snippet(&'a archguard_engine::DuplicateGroup),
        Diagnostic(&'a str),
    }
    let q = query(p);
    let mut rows = Vec::new();
    for g in &r.groups {
        // A matching path retains the whole group so its other copies remain discoverable.
        if matches(&q, [&g.id, &g.status, &g.snippet])
            || g.occurrences
                .iter()
                .any(|o| matches(&q, [&o.path, &o.status]))
        {
            rows.push(Row::Group(g));
            for o in &g.occurrences {
                rows.push(Row::Location(g, o));
            }
            rows.push(Row::Snippet(g));
        }
    }
    for d in &r.diagnostics {
        if matches(&q, [d]) {
            rows.push(Row::Diagnostic(d));
        }
    }
    // A missing import has metadata, not evidence records.
    if r.status == "not_imported" {
        rows.clear();
    }
    page(
        &r.status,
        json!({"groups_total":r.groups.len(),"occurrences_total":r.groups.iter().map(|g|g.occurrences.len()).sum::<usize>(),"producer":"CPD","producer_version":r.producer_version,"producer_timestamp":r.producer_timestamp,"producer_errors":r.producer_errors,"producer_revision":"unverified","report_path":r.report_path,"imported_at_unix":r.imported_at_unix,"limitations":"Source hashes detect only changes after import. CPD scope, token normalization and producer revision are unverified. No project-wide duplication percentage is inferred."}),
        rows.iter(),
        p,
        |row| match row {
            Row::Group(g) => {
                json!({"kind":"duplicate_group","id":g.id,"tokens":g.tokens,"lines":g.lines,"status":g.status,"occurrences":g.occurrences.len()})
            }
            Row::Location(g, o) => json!({"kind":"duplicate_location","group":g.id,"occurrence":o}),
            Row::Snippet(g) => {
                json!({"kind":"duplicate_snippet","group":g.id,"snippet":g.snippet,"snippet_truncated":g.snippet_truncated,"fragment_sha256":g.fragment_sha256})
            }
            Row::Diagnostic(d) => json!({"kind":"diagnostic","message":d}),
        },
    )
}
pub(crate) fn page<I, F>(
    status: &str,
    mut metadata: Value,
    rows: I,
    p: &EngineeringParams,
    build: F,
) -> Result<Value, ErrorData>
where
    I: Iterator + Clone,
    F: Fn(I::Item) -> Value,
{
    validate(p)?;
    let total = rows.clone().count();
    let offset = p.offset.unwrap_or(0).min(total);
    let mut omitted_metadata = Vec::new();
    while metadata.to_string().len() > 1500 {
        let key = metadata
            .as_object()
            .unwrap()
            .iter()
            .max_by_key(|(_, v)| v.to_string().len())
            .map(|(k, _)| k.clone());
        let Some(key) = key else { break };
        metadata.as_object_mut().unwrap().remove(&key);
        omitted_metadata.push(key);
    }
    let mut value = json!({"status":status,"metadata":metadata,"metadata_truncated":!omitted_metadata.is_empty(),"omitted_metadata":omitted_metadata,"total":total,"offset":offset,"processed":0,"returned":0,"next_offset":null,"truncated":false,"items":[],"record_scope":"Flattened evidence records; query filters records, overall status and totals remain unchanged. Follow next_offset only while inputs remain unchanged."});
    let mut processed = 0;
    let mut returned = 0;
    let mut omitted = false;
    for row in rows
        .skip(offset)
        .take(p.limit.unwrap_or(crate::server::DEFAULT_LIMIT))
    {
        value["items"].as_array_mut().unwrap().push(build(row));
        if value.to_string().len() > crate::tools::MAX_RESPONSE_BYTES - 150 {
            value["items"].as_array_mut().unwrap().pop();
            if processed > 0 {
                break;
            }
            value["items"].as_array_mut().unwrap().push(json!({"omitted":true,"index":offset,"reason":"Record exceeds 4 KiB response budget; use CLI JSON for full evidence"}));
            processed = 1;
            omitted = true;
            break;
        }
        processed += 1;
        returned += 1;
    }
    value["processed"] = processed.into();
    value["returned"] = returned.into();
    value["next_offset"] = if offset + processed < total {
        (offset + processed).into()
    } else {
        Value::Null
    };
    value["truncated"] = (offset > 0 || offset + processed < total || omitted).into();
    Ok(value)
}
#[derive(Clone, Copy)]
enum CohesionRow<'a> {
    Package(&'a archguard_engine::CohesionPackage),
    Member(&'a archguard_engine::CohesionPackage, usize, &'a str),
    Boundary(
        &'a archguard_engine::CohesionPackage,
        usize,
        &'a archguard_engine::CohesionBoundary,
    ),
}
pub fn cohesion_value(r: &CohesionReport, p: &EngineeringParams) -> Result<Value, ErrorData> {
    let q = query(p);
    let mut rows = Vec::new();
    for package in &r.packages {
        rows.push(CohesionRow::Package(package));
        for (i, group) in package.groups.iter().enumerate() {
            for file in &group.files {
                rows.push(CohesionRow::Member(package, i, file));
            }
            for edge in &group.boundaries {
                rows.push(CohesionRow::Boundary(package, i, edge));
            }
        }
    }
    let filtered = rows.iter().filter(|row| match row {
        CohesionRow::Package(p) => matches(&q, [&p.node, &p.reason]),
        CohesionRow::Member(p, _, file) => matches(&q, [p.node.as_str(), file]),
        CohesionRow::Boundary(p, _, edge) => matches(&q, [&p.node, &edge.from, &edge.to]),
    });
    page(
        &r.status,
        json!({"definition":r.definition,"packages_total":r.packages.len(),"split_candidates":r.packages.iter().filter(|p|p.split_candidate).count(),"diagnostics":r.diagnostics.iter().take(2).collect::<Vec<_>>(),"diagnostics_total":r.diagnostics.len(),"limitations":"Observed file dependencies and supplied co-change evidence only; disconnected groups are review candidates, not proof of domain boundaries. No source files are moved."}),
        filtered,
        p,
        |row| match row {
            CohesionRow::Package(p) => {
                json!({"kind":"package","node":p.node,"files":p.files,"internal_dependencies":p.internal_dependencies,"incoming_dependencies":p.incoming_dependencies,"outgoing_dependencies":p.outgoing_dependencies,"internal_share":p.internal_share,"groups":p.groups.len(),"split_candidate":p.split_candidate,"co_change_status":p.co_change_status,"cross_group_co_changes":p.cross_group_co_changes,"reason":p.reason})
            }
            CohesionRow::Member(p, i, file) => {
                json!({"kind":"group_member","package":p.node,"group":i,"file":file,"group_files":p.groups[*i].files.len(),"boundaries_truncated":p.groups[*i].boundaries_truncated})
            }
            CohesionRow::Boundary(p, i, edge) => {
                json!({"kind":"group_boundary","package":p.node,"group":i,"from":edge.from,"to":edge.to,"evidence":edge.evidence})
            }
        },
    )
}
#[derive(Clone, Copy)]
enum HistoryRow<'a> {
    Point(
        &'a archguard_engine::QualityHistoryPoint,
        &'a archguard_engine::QualityModulePoint,
    ),
    Measurement(&'a str, &'a str, &'a archguard_engine::RecordedMetric),
    Record(&'a archguard_engine::QualityHistoryEntry),
    Module(&'a archguard_engine::ModuleQualitySnapshot),
    Delta(&'a str, &'a str, &'a archguard_engine::QualityMetricDelta),
}
pub fn history_value(r: &QualityHistoryReport, p: &EngineeringParams) -> Result<Value, ErrorData> {
    let mut rows = Vec::new();
    for entry in &r.records {
        rows.push(HistoryRow::Record(entry));
    }
    for point in &r.series {
        for module in &point.modules {
            rows.push(HistoryRow::Point(point, module));
        }
    }
    if let Some(latest) = &r.latest {
        for module in &latest.modules {
            rows.push(HistoryRow::Module(module));
            for (name, metric) in &module.extended {
                rows.push(HistoryRow::Measurement(&module.node, name, metric));
            }
        }
    }
    if let Some(c) = &r.comparison {
        for m in &c.modules {
            for (name, d) in [
                ("line_coverage", &m.line_coverage),
                ("mean_complexity", &m.mean_complexity),
                ("max_complexity", &m.max_complexity),
                ("active_findings", &m.active_findings),
            ] {
                rows.push(HistoryRow::Delta(&m.node, name, d));
            }
            for (name, metric) in &m.extended {
                rows.push(HistoryRow::Delta(&m.node, name, metric));
            }
        }
    }
    let q = query(p);
    let filtered = rows.iter().filter(|row| match row {
        HistoryRow::Point(_, module) => matches(&q, [&module.node]),
        HistoryRow::Measurement(node, key, _) => matches(&q, [*node, *key]),
        HistoryRow::Record(e) => matches(
            &q,
            [
                e.path.as_str(),
                e.tag.as_deref().unwrap_or(""),
                e.git_commit.as_deref().unwrap_or(""),
            ],
        ),
        HistoryRow::Module(m) => matches(&q, [&m.node]),
        HistoryRow::Delta(n, metric, _) => matches(&q, [*n, *metric]),
    });
    page(
        &r.status,
        json!({"records_total":r.records.len(),"records_truncated":r.records_truncated,"latest_inputs":r.latest.as_ref().map(|s|json!({"timestamp":s.timestamp,"git_commit":s.git_commit,"source_sha256":s.source_sha256,"config_sha256":s.config_sha256,"analyzer_signature":s.analyzer_signature,"analyzer_versions_known":s.analyzer_versions_known,"producer_revision":s.producer_revision})),"comparison_timestamps":r.comparison.as_ref().map(|c|[c.before_timestamp,c.after_timestamp]),"diagnostics":r.diagnostics.iter().take(2).collect::<Vec<_>>(),"diagnostics_total":r.diagnostics.len(),"producer_revision_authenticated":false,"limitations":"Locally recorded evidence only. Unavailable or incomparable metric deltas are not zero improvement; unchanged source hashes do not authenticate CI reports."}),
        filtered,
        p,
        |row| match row {
            HistoryRow::Point(point, module) => {
                json!({"kind":"history_point","timestamp":point.timestamp,"tag":point.tag,"module":module,"comparability":"Recorded point; use metric_delta records for comparability"})
            }
            HistoryRow::Measurement(node, key, metric) => {
                json!({"kind":"latest_metric","node":node,"metric":key,"measurement":metric})
            }
            HistoryRow::Record(e) => json!({"kind":"history_record","record":e}),
            HistoryRow::Module(m) => {
                let mut module = serde_json::to_value(m).expect("snapshot");
                module.as_object_mut().unwrap().remove("extended");
                json!({"kind":"latest_module","module":module})
            }
            HistoryRow::Delta(n, metric, d) => {
                json!({"kind":"metric_delta","node":n,"metric":metric,"measurement":d})
            }
        },
    )
}
pub fn tests_value(
    r: &TestEvidenceReport,
    impact: &TestImpactReport,
    p: &TestImpactParams,
) -> Result<Value, ErrorData> {
    validate_tests(p)?;
    let selected = p.targets.as_ref().is_some_and(|t| !t.is_empty());
    let selected_tests: std::collections::BTreeMap<_, _> = impact
        .selected
        .iter()
        .map(|t| (t.test_id.as_str(), t))
        .collect();
    let mut rows = Vec::new();
    for test in &r.tests {
        if selected && !selected_tests.contains_key(test.id.as_str()) {
            continue;
        }
        rows.push((test, None));
        for mapping in &test.mappings {
            rows.push((test, Some(mapping)));
        }
    }
    let q = query(&p.page);
    let filtered = rows.iter().filter(|(t, m)| {
        matches(
            &q,
            [
                t.id.as_str(),
                &t.name,
                &t.suite,
                t.class_name.as_deref().unwrap_or(""),
                m.map(|m| m.path.as_str()).unwrap_or(""),
            ],
        )
    });
    page(
        &r.status,
        json!({"report_path":r.report_path,"mapping_path":r.mapping_path,"imported_at_unix":r.imported_at_unix,"tests_total":r.tests.len(),"impact_status":impact.status,"selected_tests":impact.selected.len(),"unmapped_tests":impact.unmapped_tests,"stale_tests":impact.stale_tests,"unresolved_targets":impact.unresolved_targets,"unresolved_targets_total":impact.unresolved_targets.len(),"safe_to_skip_other_tests":false,"producer_revision_verified":false,"limitations":"JUnit duration is observed seconds, not a prediction. Source associations are explicit producer input; reverse static dependency recommendations do not prove full test impact. Mapped line samples are capped at 20 per record.","diagnostics":r.diagnostics.iter().take(2).collect::<Vec<_>>(),"diagnostics_total":r.diagnostics.len()}),
        filtered,
        &p.page,
        |(t, m)| match m {
            None => {
                json!({"kind":"test","id":t.id,"name":t.name,"suite":t.suite,"class_name":t.class_name,"outcome":t.outcome,"duration_seconds":t.duration_seconds,"message":t.message,"mapping_count":t.mappings.len(),"recommendation":selected_tests.get(t.id.as_str()).map(|m|&m.reason)})
            }
            Some(m) => {
                json!({"kind":"test_source","test_id":t.id,"path":m.path,"node":m.node,"status":m.status,"source_sha256":m.source_sha256,"lines":m.lines.iter().take(20).collect::<Vec<_>>(),"lines_total":m.lines.len(),"lines_truncated":m.lines.len()>20})
            }
        },
    )
}
#[derive(Clone, Copy)]
enum MutationRow<'a> {
    File(&'a archguard_engine::MutationFile),
    Mutant(
        &'a archguard_engine::MutationFile,
        &'a archguard_engine::MutationRecord,
    ),
    Test(
        &'a archguard_engine::MutationFile,
        &'a archguard_engine::MutationRecord,
        &'a str,
        &'static str,
    ),
}
pub fn mutations_value(r: &MutationReport, p: &EngineeringParams) -> Result<Value, ErrorData> {
    let mut rows = Vec::new();
    for file in &r.files {
        rows.push(MutationRow::File(file));
        for m in &file.mutants {
            rows.push(MutationRow::Mutant(file, m));
            for test in &m.covered_by {
                rows.push(MutationRow::Test(file, m, test, "covered_by"));
            }
            for test in &m.killed_by {
                rows.push(MutationRow::Test(file, m, test, "killed_by"));
            }
        }
    }
    let q = query(p);
    let filtered = rows.iter().filter(|row| match row {
        MutationRow::File(f) => matches(
            &q,
            [f.path.as_str(), f.node.as_deref().unwrap_or(""), &f.status],
        ),
        MutationRow::Mutant(f, m) => matches(
            &q,
            [
                f.path.as_str(),
                &m.id,
                &m.status,
                &m.mutator_name,
                m.description.as_deref().unwrap_or(""),
            ],
        ),
        MutationRow::Test(f, m, t, _) => matches(&q, [f.path.as_str(), &m.id, &m.status, t]),
    });
    page(
        &r.status,
        json!({"report_path":r.report_path,"producer":r.producer,"producer_version":r.producer_version,"imported_at_unix":r.imported_at_unix,"files_total":r.files.len(),"mutants_total":r.summary.total,"summary":r.summary,"diagnostics":r.diagnostics.iter().take(2).collect::<Vec<_>>(),"diagnostics_total":r.diagnostics.len(),"test_configuration_verified":false,"limitations":"killed_percent is Killed/(Killed+Survived+NoCoverage); Ignored/CompileError excluded. Timeout/Pending/RuntimeError or stale/unmapped sources make it unknown. Not the producer mutation score. Exact embedded source matching does not establish test/config freshness."}),
        filtered,
        p,
        |row| match row {
            MutationRow::File(f) => {
                json!({"kind":"mutation_file","path":f.path,"node":f.node,"language":f.language,"source_sha256":f.source_sha256,"source_status":f.status,"mutants":f.mutants.len()})
            }
            MutationRow::Mutant(f, m) => {
                json!({"kind":"mutation","path":f.path,"node":f.node,"source_status":f.status,"id":m.id,"status":m.status,"mutator_name":m.mutator_name,"location":m.location,"description":m.description,"replacement":m.replacement,"original":m.original,"original_truncated":m.original_truncated,"duration_ms":m.duration_ms,"covered_by_count":m.covered_by.len(),"killed_by_count":m.killed_by.len()})
            }
            MutationRow::Test(f, m, t, relation) => {
                json!({"kind":"mutation_test","path":f.path,"mutation_id":m.id,"test_id":t,"relation":relation})
            }
        },
    )
}
#[derive(Clone, Copy)]
enum ContractRow<'a> {
    Contract(&'a archguard_engine::ApiContract),
    Operation(
        &'a archguard_engine::ApiContract,
        &'a archguard_engine::ApiOperation,
    ),
    Usage(
        &'a archguard_engine::ApiContract,
        &'a archguard_engine::ApiUsage,
    ),
    Caller(&'a archguard_engine::ApiContract, &'a str),
    Change(
        &'a archguard_engine::ApiContract,
        &'a archguard_engine::ApiChange,
    ),
    Consumer(
        &'a archguard_engine::ApiContract,
        &'a archguard_engine::ApiChange,
        &'a str,
    ),
}
pub fn contracts_value(r: &ApiContractReport, p: &EngineeringParams) -> Result<Value, ErrorData> {
    let mut rows = Vec::new();
    for c in &r.contracts {
        rows.push(ContractRow::Contract(c));
        for o in &c.operations {
            rows.push(ContractRow::Operation(c, o));
        }
        for u in &c.usages {
            rows.push(ContractRow::Usage(c, u));
        }
        for caller in &c.configured_callers {
            rows.push(ContractRow::Caller(c, caller));
        }
        for change in &c.changes {
            rows.push(ContractRow::Change(c, change));
            for consumer in &change.consumers {
                rows.push(ContractRow::Consumer(c, change, consumer));
            }
        }
    }
    let q = query(p);
    let filtered = rows.iter().filter(|row| match row {
        ContractRow::Contract(c) => matches(&q, [&c.provider, &c.source_path]),
        ContractRow::Operation(c, o) => matches(
            &q,
            [
                c.provider.as_str(),
                &o.id,
                o.operation_id.as_deref().unwrap_or(""),
            ],
        ),
        ContractRow::Usage(c, u) => matches(&q, [&c.provider, &u.consumer, &u.operation]),
        ContractRow::Caller(c, t) => matches(&q, [c.provider.as_str(), t]),
        ContractRow::Change(c, m) => matches(&q, [&c.provider, &m.operation, &m.kind, &m.message]),
        ContractRow::Consumer(c, m, t) => matches(&q, [c.provider.as_str(), &m.operation, t]),
    });
    page(
        &r.status,
        json!({"contracts_total":r.contracts.len(),"operations_total":r.contracts.iter().map(|c|c.operations.len()).sum::<usize>(),"changes_total":r.contracts.iter().map(|c|c.changes.len()).sum::<usize>(),"diagnostics":r.diagnostics.iter().take(2).collect::<Vec<_>>(),"diagnostics_total":r.diagnostics.len(),"limitations":r.limitations.iter().take(3).collect::<Vec<_>>(),"limitations_total":r.limitations.len(),"usage_evidence":"Declared operation usages remain distinct from configured HTTP callers; neither proves observed runtime calls.","comparison_base":"previous explicit import"}),
        filtered,
        p,
        |row| match row {
            ContractRow::Contract(c) => {
                json!({"kind":"contract","provider":c.provider,"mapped_service":c.mapped_service,"source_path":c.source_path,"source_sha256":c.source_sha256,"source_status":c.source_status,"openapi":c.openapi,"imported_at_unix":c.imported_at_unix,"usage_status":c.usage_status,"comparison_status":c.comparison_status,"previous_imported_at_unix":c.previous_imported_at_unix,"operations":c.operations.len(),"usages":c.usages.len(),"configured_callers":c.configured_callers.len(),"changes":c.changes.len()})
            }
            ContractRow::Operation(c, o) => {
                json!({"kind":"operation","provider":c.provider,"operation":o})
            }
            ContractRow::Usage(c, u) => {
                json!({"kind":"declared_usage","provider":c.provider,"usage":u})
            }
            ContractRow::Caller(c, t) => {
                json!({"kind":"configured_caller","provider":c.provider,"consumer":t,"proves_operation_usage":false})
            }
            ContractRow::Change(c, m) => {
                json!({"kind":"api_change","provider":c.provider,"operation":m.operation,"change":m.kind,"severity":m.severity,"message":m.message,"declared_consumers":m.consumers.len()})
            }
            ContractRow::Consumer(c, m, t) => {
                json!({"kind":"affected_consumer","provider":c.provider,"operation":m.operation,"change":m.kind,"consumer":t,"evidence":"explicit declared usage"})
            }
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn duplication_pages_preserve_locations_and_progress_past_oversized_snippets() {
        let report: archguard_engine::DuplicationReport = serde_json::from_value(serde_json::json!({
            "schema_version":1,"status":"partial","producer_errors":1,"diagnostics":["scan incomplete"],
            "groups":[{"id":"group","lines":2,"tokens":10,"status":"partial","snippet":"z".repeat(4096),"snippet_truncated":true,"fragment_sha256":"a".repeat(64),
                "occurrences":(0..50).map(|i|serde_json::json!({"path":format!("src/{i}.ts"),"line":1,"end_line":2,"status":"unmapped"})).collect::<Vec<_>>() }]
        })).unwrap();
        let mut records = Vec::new();
        let mut offset = 0;
        loop {
            let result = super::duplicates_value(
                &report,
                &super::EngineeringParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(result.to_string().len() <= 4096);
            assert_eq!(result["status"], "partial");
            assert_eq!(result["metadata"]["occurrences_total"], 50);
            records.extend(result["items"].as_array().unwrap().iter().cloned());
            let Some(next) = result["next_offset"].as_u64() else {
                break;
            };
            assert!(next as usize > offset);
            offset = next as usize;
        }
        assert_eq!(
            records
                .iter()
                .filter(|r| r["kind"] == "duplicate_location")
                .count(),
            50
        );
        assert_eq!(records.iter().filter(|r| r["omitted"] == true).count(), 1);
        assert!(records.iter().any(|r| r["kind"] == "diagnostic"));
        let filtered = super::duplicates_value(
            &report,
            &super::EngineeringParams {
                query: Some("SRC/49.TS".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            filtered["total"], 52,
            "a path match retains the whole group, not only the searched copy"
        );
    }
    use super::*;
    fn mutation_report() -> MutationReport {
        serde_json::from_value(json!({"schema_version":1,"status":"partial","report_path":"mutations.json","imported_at_unix":1,"producer":"Example","producer_version":"1","diagnostics":["Unverified test/config revision"],"summary":{"total":2,"killed":1,"survived":1,"no_coverage":0,"timeouts":0,"pending":0,"ignored":0,"compile_errors":0,"runtime_errors":0,"killed_percent":50.0},"files":[{"path":"src/a.ts","node":"a.ts","language":"typescript","source_sha256":"h","status":"matches_reported_source","mutants":(0..2).map(|i|json!({"id":format!("m{i}"),"mutator_name":"BooleanLiteral","status":if i==0{"Killed"}else{"Survived"},"location":{"start":{"line":1,"column":1},"end":{"line":1,"column":2}},"description":null,"replacement":"false","original":"true","original_truncated":false,"duration_ms":1.25,"covered_by":["t1","t2"],"killed_by":if i==0 {vec!["t1"]}else{vec![]}})).collect::<Vec<_>>()}]})).unwrap()
    }
    #[test]
    fn mutation_pages_preserve_outcomes_and_flatten_test_associations() {
        let report = mutation_report();
        let mut offset = 0;
        let mut kinds = Vec::new();
        let mut records = Vec::new();
        loop {
            let page = mutations_value(
                &report,
                &EngineeringParams {
                    offset: Some(offset),
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(page.to_string().len() <= 4096);
            assert_eq!(page["status"], "partial");
            assert_eq!(page["metadata"]["summary"]["killed_percent"], 50.0);
            for record in page["items"].as_array().unwrap() {
                kinds.push(record["kind"].as_str().unwrap().to_owned());
                records.push(record.clone());
            }
            if let Some(next) = page["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(kinds.iter().filter(|k| k.as_str() == "mutation").count(), 2);
        assert_eq!(
            kinds
                .iter()
                .filter(|k| k.as_str() == "mutation_test")
                .count(),
            5
        );
        assert!(records.iter().any(|r| r["status"] == "Survived"));
        let filtered = mutations_value(
            &report,
            &EngineeringParams {
                query: Some("SURVIVED".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered["metadata"]["mutants_total"], 2);
        assert!(filtered["total"].as_u64().unwrap() > 0);
    }
    #[test]
    fn oversized_rows_progress_and_optional_metadata_cannot_break_budget() {
        let mut report = mutation_report();
        report.report_path = Some("p".repeat(10000));
        report.diagnostics = vec!["d".repeat(10000)];
        report.files[0].mutants[0].description = Some("z".repeat(10000));
        let page = mutations_value(
            &report,
            &EngineeringParams {
                offset: Some(1),
                limit: Some(200),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(page.to_string().len() <= 4096);
        assert_eq!(page["processed"], 1);
        assert_eq!(page["returned"], 0);
        assert_eq!(page["next_offset"], 2);
        assert_eq!(page["items"][0]["omitted"], true);
        assert_eq!(page["metadata_truncated"], true);
    }
    #[test]
    fn cohesion_members_remain_accessible_without_a_giant_package_record() {
        let report:CohesionReport=serde_json::from_value(json!({"schema_version":1,"status":"partial","definition":"internal/(internal+outgoing)","diagnostics":["Observed dependencies only"],"packages":[{"node":"pkg","files":400,"internal_dependencies":1,"incoming_dependencies":2,"outgoing_dependencies":3,"internal_share":1.0/4.0,"groups":[{"files":(0..400).map(|n|format!("pkg/{n}.ts")).collect::<Vec<_>>(),"internal_dependencies":1,"incoming_dependencies":2,"outgoing_dependencies":3,"boundaries":[],"boundaries_truncated":false}],"split_candidate":false,"co_change_status":"unavailable","cross_group_co_changes":0,"reason":"Single connected group"}]})).unwrap();
        let first = cohesion_value(
            &report,
            &EngineeringParams {
                limit: Some(200),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first["total"], 401);
        assert_eq!(first["items"][0]["kind"], "package");
        assert_eq!(first["metadata"]["packages_total"], 1);
        assert!(first["next_offset"].is_number());
        assert!(first.to_string().len() <= 4096);
        let last = cohesion_value(
            &report,
            &EngineeringParams {
                query: Some("pkg/399.ts".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(last["total"], 1);
        assert_eq!(last["items"][0]["file"], "pkg/399.ts");
    }
    #[test]
    fn test_recommendations_and_missing_mapping_stay_explicit() {
        let report:TestEvidenceReport=serde_json::from_value(json!({"schema_version":1,"status":"partial","report_path":"junit.xml","mapping_path":null,"imported_at_unix":1,"tests":[{"id":"id","suite":"unit","name":"works","class_name":"ATest","duration_seconds":2.5,"outcome":"passed","message":null,"mappings":[]}],"diagnostics":["No source mapping"]})).unwrap();
        let impact = archguard_engine::test_impact(
            &archguard_engine::core::GraphBuilder::new().build("x".into()),
            &report,
            &[],
        );
        let page = tests_value(&report, &impact, &TestImpactParams::default()).unwrap();
        assert_eq!(page["status"], "partial");
        assert_eq!(page["metadata"]["impact_status"], "not_requested");
        assert_eq!(page["metadata"]["unmapped_tests"], 1);
        assert_eq!(page["items"][0]["duration_seconds"], 2.5);
        assert_eq!(page["metadata"]["safe_to_skip_other_tests"], false);
    }
    #[test]
    fn absent_history_and_contracts_are_not_clean_results_and_invalid_inputs_fail() {
        let history:QualityHistoryReport=serde_json::from_value(json!({"schema_version":1,"status":"not_recorded","records":[],"records_truncated":false,"latest":null,"comparison":null,"diagnostics":[]})).unwrap();
        let page = history_value(&history, &EngineeringParams::default()).unwrap();
        assert_eq!(page["status"], "not_recorded");
        assert_eq!(page["total"], 0);
        let contracts:ApiContractReport=serde_json::from_value(json!({"schema_version":1,"status":"not_imported","contracts":[],"diagnostics":[],"limitations":["Configured callers do not prove operation usage"]})).unwrap();
        let page = contracts_value(&contracts, &EngineeringParams::default()).unwrap();
        assert_eq!(page["status"], "not_imported");
        assert_eq!(page["total"], 0);
        for p in [
            EngineeringParams {
                limit: Some(0),
                ..Default::default()
            },
            EngineeringParams {
                limit: Some(201),
                ..Default::default()
            },
            EngineeringParams {
                query: Some("\0".into()),
                ..Default::default()
            },
        ] {
            assert!(validate(&p).is_err());
        }
        assert!(
            validate_tests(&TestImpactParams {
                targets: Some(vec!["x".into(); 257]),
                ..Default::default()
            })
            .is_err()
        );
    }
}
