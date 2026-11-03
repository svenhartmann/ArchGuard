'use strict';
// Documentation exports describe the active analysis, including its limits.
// They never change navigation or reuse a hidden graph from another workspace.
function exportSnapshot() {
  const workspace=isRepositoryOverview()&&!['services','security'].includes(ui.workspace)?'services':ui.workspace;
  const repository=repositoryModel();
  const base={schema_version:1,project:rootName,filters:{workspace},...(repository?{repository:{mode:repository.mode,root:repository.root,active_project:repository.active_project,excluded_projects:repository.excluded_projects||[]}}:{})};
  if(workspace==='security'){
    const snapshot=securitySnapshot(),findings=filteredSecurityFindings();
    return {...base,kind:'security',status:snapshot.status,
      coverage:'Original dependency scanner snapshot. All matching findings are exported, including additional list pages. Dependency-graph simulation does not change security findings.',
      filters:{...base.filters,query:securityUi.query,severity:securityUi.severity},snapshot,findings,
      metrics:{...securityCounts(snapshot),matching_findings:findings.length},
      notes:['Packages are distinct ecosystem, name and installed-version combinations; a package can have several findings. Per-target package totals can overlap.',
        'Missing, failed, partial, outdated or zero-package scans do not establish dependency safety. Scanner advisories do not measure runtime exploitability. Fixed-version strings are scanner evidence, not automatic changes.'],
      diagnostics:snapshot.diagnostics||[]};
  }
  if(workspace==='quality')return engineeringExport(base);
  if(workspace==='assessment')return assessmentExport(base);
  if(workspace==='architecture')return architectureExport(base);
  if(workspace==='services'){
    const model=serviceModel(),all=serviceDependencies(),edges=all.filter(serviceMatches);
    return {...base,...workspaceExport(),patterns:typeof patternReport==='function'?patternReport():null,contracts:engineeringData('contracts'),kind:'services',status:model?(model.complete?'complete':'partial'):'unavailable',
      coverage:'Visible service connections of the selected configuration kind; full repository project inventory independent of the visible inventory page.',
      monorepo:model?.monorepo??false,monorepo_reason:model?.monorepo_reason||'',compose_files:model?.compose_files||[],projects:serviceProjects(),
      filters:{...base.filters,kind:serviceUi.kind,direction:serviceUi.mode,reference:serviceUi.selected,selected_project:serviceUi.project,cycles_only:serviceUi.cyclesOnly,focused_cycle:serviceUi.cycle},
      metrics:{scope_nodes:model?.services.length||0,scope_dependencies:all.length,shown_nodes:model?.services.length||0,shown_dependencies:edges.length,cyclic_groups:serviceCycles().length},
      nodes:(model?.services||[]).map(service=>({...service})),
      edges:edges.map(edge=>({from:edge.from,to:edge.to,kind:edge.kind,count:edge.evidence.length,count_exact:true,cyclic_current_scope:serviceEdgeInCycle(edge),evidence:edge.evidence})),
      count_unit:'configuration evidence records',diagnostics:model?.diagnostics||[],
      notes:['Project identities include manifest paths; matching package names or technologies do not establish service connections. Libraries and test tools are inventory, not additional runtime services.', 'Configured HTTP endpoints are not evidence of executed runtime calls. Startup dependencies describe depends_on order. Each edge kind has its own cycles.']};
  }
  if(workspace==='calls'){
    if(callRankingUi.mode==='cycles'){
      prepareCallCycles();
      const group=callCycleUi.group,kind=callCycleUi.kind;
      return {...base,kind:'calls',status:callModel.status,
        coverage:'Complete selected call-cycle group and all its internal directed edges and source call sites, including additional list pages. The catalogue contains every matching cyclic group. Original call-analysis snapshot; dependency simulations do not rewrite call sites.',
        filters:{...base.filters,call_view:'cycles',cycle_kind:kind,language:callUi.language,package:callUi.package,query:callUi.query,resolution:'project',include_internal:kind==='method',selected_cycle:group?.id??null},
        selected_cycle:group?{id:group.id,kind:group.kind,members:group.members.slice(),witness:group.witness.slice(),edges:group.edges.map(edge=>({from:edge.from,to:edge.to,call_sites:edge.calls.length}))}:null,
        cycle_groups:callCycleUi.groups.map(cycle=>({id:cycle.id,kind:cycle.kind,members:cycle.members.slice(),witness:cycle.witness.slice(),edges:cycle.edges.length,call_sites:cycle.call_sites})),
        analyzers:callModel.analyzers||[],
        metrics:{cyclic_groups:callCycleUi.groups.length,shown_nodes:group?.members.length||0,shown_dependencies:group?.edges.length||0,call_sites:group?.call_sites||0},
        nodes:(group?.members||[]).map(id=>({id,name:callCycleLabel(kind,id),kind})),
        edges:(group?.edges||[]).map(edge=>({from:edge.from,to:edge.to,kind:'call',resolution:'project',count:edge.calls.length,count_exact:true,cyclic_current_scope:true,evidence:edge.calls})),
        count_unit:'static call sites',diagnostics:callModel.diagnostics||[],
        notes:['Cyclic groups are strongly connected components (SCCs), not an enumeration of every elementary cycle. Each witness is one closed example path; all internal directed edges of the selected group are exported.',
          'Language, package and search select whole groups by their members. Return paths outside the selected package remain included. Element focus and connection target/internal-call preferences do not filter cycle groups.',
          'Type/module cycles aggregate calls between different owners and do not establish a continuous function/method-call cycle. Module owners group top-level functions, not declared classes. Method cycles retain exact declared function/method identities, including overloads, internal calls and direct recursion.',
          'Static call resolution is not a runtime trace or proof of executed recursion. Partial or unavailable analysis and missing method identities do not establish the absence of other cycles.']};
    }
    const relations=callRelations(),ids=new Set(relations.flatMap(relation=>[relation.from,relation.to||relation.label]));
    const method=callUi.element?.kind==='method'?callMethods.get(callUi.element.id):null;
    return {...base,kind:'calls',status:callModel.status,
      ...(method?{selected_method:{...method}}:{}),
      coverage:'All matching class relations, including additional list pages. Original call-analysis snapshot; dependency simulations do not rewrite call sites.',
      filters:{...base.filters,language:callUi.language,package:callUi.package,query:callUi.query,resolution:callUi.resolution,include_internal:callUi.internal,element:callUi.element},
      analyzers:callModel.analyzers||[],
      metrics:{shown_nodes:ids.size,shown_dependencies:relations.length,call_sites:relations.reduce((sum,relation)=>sum+relation.calls.length,0)},
      nodes:[...ids].map(id=>({id,name:callClassLabel(id)})),
      edges:relations.map(relation=>({from:relation.from,to:relation.to||relation.label,kind:'call',resolution:relation.resolution,count:relation.calls.length,count_exact:true,evidence:relation.calls})),
      count_unit:'static call sites',diagnostics:callModel.diagnostics||[],
      notes:['Static call resolution is not a runtime trace. Partial or unavailable analysis does not establish the absence of other calls.']};
  }
  const matrix=workspace==='dsm',nodes=matrix?view.nodes:displayedGraphNodes(),ids=new Set(nodes.map(node=>node.id));
  const edges=view.edges.filter(edge=>ids.has(edge.from)&&ids.has(edge.to)&&(matrix||dependencyMatchesSelection(edge.from,edge.to)));
  const byId=new Map(view.nodes.map(node=>[node.id,node]));
  return {...base,kind:data.semantic?'dependencies':'imports',coverage:matrix?'Complete analysis scope across all matrix tiles.':'Shown graph nodes and directed connections after file, direction and cycle filters.',
    ...exportTestCoverage(),
    ...exportQualityFindings(),
    ...(data.semantic?{semantic:data.semantic,diagnostics:data.semantic.diagnostics||[]}:{}),
    filters:{...base.filters,diagram:matrix?'matrix':ui.diagram,reference:edgeReference()?.name||null,...(matrix?{}:{direction:ui.edgeMode,show_files:ui.showFiles,cycles_only:ui.cyclesOnly,search:currentFilter(),search_behavior:'Search dims graph nodes; it does not remove connections.'})},
    metrics:{scope_nodes:view.nodes.length,scope_dependencies:view.edges.length,shown_nodes:nodes.length,shown_dependencies:edges.length,cyclic_groups:sccs.length,acd:currentAcd},
    nodes:nodes.map(node=>({id:node.name,name:node.name,kind:node.kind,source_id:node.src})),
    edges:edges.map(edge=>{const link={source:byId.get(edge.from),target:byId.get(edge.to)};return {from:link.source.name,to:link.target.name,kind:data.semantic?'dependency':'import',count:edgeWeight(edge),count_exact:edge.count!==undefined&&edge.count!==null,cyclic_current_scope:cyclicKeys.has(edge.from+' '+edge.to),package_cycle_context:!!packageContext?.edges.has(edge.from+' '+edge.to),simulation_state:linkState(link),evidence:edge.refs};}),
    simulation_ops:ops.map(op=>({...op})),count_unit:data.semantic?'dependency evidence records':'import references',
    notes:['ACD and cyclic groups describe the full analysis scope, before display filters. ACD is Average Component Dependency: average transitive dependency count, including each starting node itself. It does not measure runtime performance.',
      'Reference counts are exact when count_exact is true; otherwise they are lower bounds. Evidence may be a sample. Coarser package-cycle context does not imply a cycle at the current detail level.',
      ...(data.semantic?[graphCoverageDescription(),...(!sccs.length&&graphCycleCoverageWarning()?[graphCycleCoverageWarning()]:[])]:[]),
      ...(ops.length?['This export includes simulated graph changes. Source files have not been changed.']:[])]};
}
function exportMarkdownCell(value) {
  return String(value??'').replace(/[&<>|`\\\[\]*_!~#]/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[char]||'&#'+char.charCodeAt(0)+';')).replace(/\r?\n/g,'<br>');
}
function exportMermaidLabel(value) {
  return String(value).replace(/[&<>"`#\r\n]/g,char=>'#'+char.charCodeAt(0)+';');
}
function exportMarkdown(snapshot) {
  if(snapshot.kind==='assessment')return assessmentMarkdown(snapshot);
  if(snapshot.kind==='quality')return engineeringMarkdown(snapshot);
  if(snapshot.kind==='security')return exportSecurityMarkdown(snapshot);
  const cell=exportMarkdownCell,lines=['# Architecture: '+cell(snapshot.project),'',cell(snapshot.coverage),''];
  if(snapshot.repository)lines.push('Repository: '+cell(snapshot.repository.root)+'. Scope: '+cell(snapshot.repository.mode==='overview'?'repository projects and deployments (sources analyzed separately)':snapshot.repository.active_project||'.')+'.',
    ...(snapshot.repository.excluded_projects.length?['Excluded nested projects: '+snapshot.repository.excluded_projects.map(cell).join(', ')+'.']:[]),'');
  if(snapshot.status)lines.push('Analysis status: **'+cell(snapshot.status)+'**.','');
  lines.push('## Scope and filters','','| Setting | Value |','| --- | --- |');
  for(const [key,value] of Object.entries(snapshot.filters))lines.push('| '+cell(key.replaceAll('_',' '))+' | '+cell(typeof value==='object'?JSON.stringify(value):value)+' |');
  const titles={scope_nodes:'Analysis scope nodes',scope_dependencies:'Analysis scope dependencies',shown_nodes:'Shown nodes',shown_dependencies:'Shown dependencies',cyclic_groups:'Cyclic groups in analysis scope',acd:'ACD in analysis scope',call_sites:'Matching static call sites'};
  lines.push('','## Metrics','','| Metric | Value |','| --- | --- |');
  for(const [key,value] of Object.entries(snapshot.metrics))lines.push('| '+cell(titles[key]||key)+' | '+cell(key==='acd'?value.toFixed(2):value)+' |');
  for(const note of snapshot.notes||[])lines.push('',cell(note));
  if(snapshot.cycle_groups){
    lines.push('','## Call-cycle catalogue','','| Level | Members | Directed edges | Static call sites | Closed example path |','| --- | --- | ---: | ---: | --- |');
    for(const group of snapshot.cycle_groups)lines.push('| '+[group.kind,group.members.join(', '),group.edges,group.call_sites,group.witness.join(' → ')].map(cell).join(' | ')+' |');
    if(!snapshot.cycle_groups.length)lines.push('','No recorded cyclic groups match these member filters. See the analysis status and coverage limits above.');
    if(snapshot.selected_cycle){
      const group=snapshot.selected_cycle;
      lines.push('','## Selected '+cell(group.kind)+' call-cycle group','',
        'Members: '+group.members.map(cell).join(', ')+'.','',
        'One closed example path: '+group.witness.map(cell).join(' → ')+'.','',
        'The directed graph and source evidence below include every internal edge of this selected group.');
    }
  }
  if(snapshot.selected_method){
    const method=snapshot.selected_method,metric=methodComplexity(method);
    lines.push('','## Selected method declaration','',cell(method.id),'',cell(method.file+':'+method.line),'',
      '| AST complexity | Max. nesting | Definition |','| ---: | ---: | --- |',
      '| '+[metric?.cyclomatic??'—',metric?.max_nesting??'—',metric?.definition||'unavailable'].map(cell).join(' | ')+' |','',cell(methodComplexityDescription()));
  }
  appendCoverageMarkdown(lines,snapshot);
  appendQualityMarkdown(lines,snapshot);
  appendContractsMarkdown(lines,snapshot);
  appendWorkspaceMarkdown(lines,snapshot);
  if(typeof appendPatternsMarkdown==='function')appendPatternsMarkdown(lines,snapshot.patterns);
  if(snapshot.kind==='architecture'&&snapshot.model){
    const model=snapshot.model;
    lines.push('','## Target model','',
      'The complete model and assignment coverage below ignore relationship filters. The directed graph, relationship table and source evidence use the exported filters.',
      '', 'Model: '+cell(model.name)+'. Gate severity: '+cell(model.severity)+'. Require assignment: '+cell(model.require_assignment)+'.',
      '', '| Component | Include patterns | Assigned files | Source files |','| --- | --- | ---: | --- |');
    for(const component of model.components)lines.push('| '+cell(component.name)+' | '+component.include.map(cell).join('<br>')+' | '+cell(component.node_count)+' | '+component.nodes.map(cell).join('<br>')+' |');
    lines.push('','### Public interfaces and coupling limits','','| Component | Public interface | Incoming components | Incoming limit | Outgoing components | Outgoing limit |','| --- | --- | --- | ---: | --- | ---: |');
    for(const c of model.components)lines.push('| '+[c.name,c.public==null?'Unrestricted':c.public.length?c.public.join(', '):'Closed',(c.incoming_components||[]).join(', '),c.max_incoming_components??'—',(c.outgoing_components||[]).join(', '),c.max_outgoing_components??'—'].map(cell).join(' | ')+' |');
    lines.push('','### Private source access (complete model)','');
    for(const edge of model.dependencies||[])for(const access of edge.private_accesses||[]){
      lines.push('- '+[edge.from,edge.to,access.from,access.to].map(cell).join(' → '));
      for(const ref of access.evidence||[])lines.push('  - '+cell(ref.file+':'+ref.line+' '+ref.raw));
    }
    lines.push('','### Declared allowed directions','','| Source | Target |','| --- | --- |');
    for(const edge of model.allowed_dependencies)lines.push('| '+cell(edge.from)+' | '+cell(edge.to)+' |');
    if(!model.allowed_dependencies.length)lines.push('','No cross-component directions are allowed.');
    lines.push('','### Assignment coverage','',
      cell(model.mapped_nodes)+' / '+cell(model.source_nodes)+' source files assigned. '+cell(model.unmapped_nodes)+' unassigned; '+cell(model.ambiguous_nodes)+' ambiguous.',
      '',cell(model.evaluated_edges)+' / '+cell(model.total_edges)+' source connections classified; '+cell(model.unclassified_edges)+' unclassified. Classified connections include internal component dependencies.',
      '', '| Unassigned source file |','| --- |');
    for(const name of model.unmapped)lines.push('| '+cell(name)+' |');
    if(!model.unmapped.length)lines.push('','No unassigned source files.');
    lines.push('','| Ambiguous source file | Candidate components |','| --- | --- |');
    for(const mapping of model.ambiguous)lines.push('| '+cell(mapping.node)+' | '+mapping.components.map(cell).join('<br>')+' |');
    if(!model.ambiguous.length)lines.push('','No ambiguous assignments.');
    lines.push('','Components without assigned files: '+(model.empty_components.length?model.empty_components.map(cell).join(', '):'none')+'.');
  }
  if(snapshot.kind==='services'&&snapshot.projects?.length){
    lines.push('','## Project inventory','',cell(snapshot.monorepo_reason),'',
      '| Project | Path | Kind | Ecosystem | Runtime | Manifest |','| --- | --- | --- | --- | --- | --- |');
    for(const project of snapshot.projects)lines.push('| '+[project.name,project.path,project.kind,project.ecosystem,project.runtime||'',project.manifest].map(cell).join(' | ')+' |');
    lines.push('','### Project evidence','','| Project | Location | Evidence |','| --- | --- | --- |');
    for(const project of snapshot.projects)for(const evidence of project.evidence||[])lines.push('| '+[project.id,serviceEvidenceLocation(evidence),evidence.description].map(cell).join(' | ')+' |');
  }
  lines.push('','## Directed graph','','Mermaid shows the exported relationships. SVG export preserves the displayed tree layout and colors.','','```mermaid','flowchart LR');
  const ids=new Map(snapshot.nodes.map((node,index)=>[node.id,'n'+index]));
  for(const node of snapshot.nodes)lines.push('  '+ids.get(node.id)+'["'+exportMermaidLabel(node.name)+'"]');
  for(const edge of snapshot.edges){
    const label=['import','dependency','call'].includes(edge.kind)?String(edge.count)+(edge.count_exact?'':'+'):edge.kind;
    lines.push('  '+ids.get(edge.from)+' -->|"'+exportMermaidLabel(label)+'"| '+ids.get(edge.to));
  }
  lines.push('```','','## Relationships','','Counts: '+cell(snapshot.count_unit)+'.','','| Source | Target | Kind / resolution | Count | Current-scope cycle | Package-cycle context |','| --- | --- | --- | ---: | --- | --- |');
  for(const edge of snapshot.edges)lines.push('| '+[edge.from,edge.to,edge.kind+(edge.resolution?' / '+edge.resolution:''),String(edge.count)+(edge.count_exact?'':'+'),edge.cyclic_current_scope===undefined?'not evaluated':edge.cyclic_current_scope?'yes':'no',edge.package_cycle_context?'yes':'no'].map(cell).join(' | ')+' |');
  if(!snapshot.edges.length)lines.push('','No relationships match this export scope.');
  lines.push('','## Source evidence','','| Source → target | Location | Evidence |','| --- | --- | --- |');
  for(const edge of snapshot.edges)for(const evidence of edge.evidence||[]){
    const location=evidence.file+(evidence.line?':'+evidence.line:'')+(evidence.column?':'+evidence.column:'')+(evidence.pointer?' · '+evidence.pointer:'');
    const detail=evidence.raw??evidence.code??evidence.description??'';
    lines.push('| '+[edge.from+' → '+edge.to,location,detail].map(cell).join(' | ')+' |');
  }
  if(snapshot.diagnostics?.length){lines.push('','## Coverage diagnostics','');for(const diagnostic of snapshot.diagnostics)lines.push('- '+cell(typeof diagnostic==='string'?diagnostic:[diagnostic.file,diagnostic.service,diagnostic.message].filter(Boolean).join(' · ')));}
  if(snapshot.simulation_ops?.length){lines.push('','## Simulated changes','','| Operation | Details |','| --- | --- |');for(const op of snapshot.simulation_ops)lines.push('| '+cell(op.op||op.kind||'SimOp')+' | '+cell(JSON.stringify(op))+' |');}
  return lines.join('\n')+'\n';
}
function exportSecurityMarkdown(model){
  const cell=exportMarkdownCell,snapshot=model.snapshot,lines=['# '+t('Dependency security','Sicherheit der Abhängigkeiten')+': '+cell(model.project),'',
    cell(t('Original dependency scan; graph simulations do not change these findings. This report includes every matching finding, beyond the displayed page.','Ursprünglicher Abhängigkeitsscan; Graphsimulationen verändern diese Befunde nicht. Dieser Bericht enthält alle passenden Befunde über die angezeigte Seite hinaus.')),'',
    cell(securityStatusText(snapshot)),'',
    '## '+t('Scan and coverage','Scan und Abdeckung'),'','| '+t('Field | Value','Feld | Wert')+' |','| --- | --- |'];
  const unknown=t('Not reported','Nicht gemeldet');
  if(model.repository){
    lines.push('| '+cell(t('Repository scope','Repository-Umfang'))+' | '+cell(model.repository.root)+' |');
    if(model.repository.mode==='project')lines.push('| '+cell(t('Project scope','Projektumfang'))+' | '+cell(model.repository.active_project||'.')+' |');
  }
  for(const [label,value] of [[t('Status','Status'),snapshot.status],[t('Scanner','Scanner'),snapshot.scanner],[t('Scanner version','Scanner-Version'),snapshot.scanner_version||unknown],[t('Scanned at','Scanzeitpunkt'),snapshot.scanned_at||unknown],[t('Database updated at','Datenbank aktualisiert'),snapshot.database_updated_at||unknown],[t('Input fingerprint','Eingabe-Fingerabdruck'),snapshot.fingerprint||unknown],[t('Packages scanned','Gescannte Pakete'),model.metrics.packages_scanned],[t('Affected packages','Betroffene Pakete'),model.metrics.affected_packages],[t('Findings in snapshot','Befunde im Snapshot'),model.metrics.findings],[t('Matching findings','Passende Befunde'),model.findings.length]])lines.push('| '+cell(label)+' | '+cell(value)+' |');
  lines.push('',cell(t('Packages are distinct ecosystem, name and installed-version combinations. Per-target counts can overlap. Missing, failed, incomplete, outdated or zero-package scans do not establish dependency safety.','Pakete sind eindeutige Kombinationen aus Ökosystem, Name und installierter Version. Zielbezogene Zählungen können sich überschneiden. Fehlende, fehlgeschlagene, unvollständige, veraltete oder Scans ohne Pakete belegen keine sicheren Abhängigkeiten.')),'',
    '## '+t('Filters','Filter'),'','| '+t('Setting | Value','Einstellung | Wert')+' |','| --- | --- |');
  for(const [key,value] of Object.entries(model.filters))lines.push('| '+cell(key)+' | '+cell(value)+' |');
  lines.push('','## '+t('Manifest / lockfile targets','Manifest- / Lockfile-Ziele'),'','| '+t('Path | Ecosystem | Packages','Pfad | Ökosystem | Pakete')+' |','| --- | --- | ---: |');
  for(const target of snapshot.targets||[])lines.push('| '+[target.path,target.ecosystem,target.packages].map(cell).join(' | ')+' |');
  lines.push('','## '+t('Findings','Befunde'),'','| '+t('Advisory | Package | Ecosystem | Installed | Fixed versions | Severity | Title | Evidence','Advisory | Paket | Ökosystem | Installiert | Behobene Versionen | Schweregrad | Titel | Belege')+' |','| --- | --- | --- | --- | --- | --- | --- | --- |');
  for(const finding of model.findings){
    const url=safeSecurityUrl(finding.url),advisory=cell(finding.id)+(url?' · <'+url.replace(/[<>|]/g,char=>encodeURIComponent(char))+'>':'');
    lines.push('| '+advisory+' | '+[finding.package,finding.ecosystem,finding.installed_version,finding.fixed_version||t('No fixed version reported','Keine behobene Version gemeldet'),securitySeverityLabel(finding.severity),finding.title].map(cell).join(' | ')+' | '+(finding.targets||[]).map(cell).join('<br>')+' |');
  }
  if(!model.findings.length)lines.push('',cell(t('No matching findings are recorded in this export. See the scan status and coverage above; this is not a safety guarantee.','Dieser Export enthält keine passenden Befunde. Scanstatus und Abdeckung stehen oben; dies ist keine Sicherheitsgarantie.')));
  if(model.diagnostics.length){lines.push('','## '+t('Scanner diagnostics (original text)','Scannerdiagnosen (Originaltext)'),'');for(const diagnostic of model.diagnostics)lines.push('- '+cell(diagnostic));}
  return lines.join('\n')+'\n';
}
function exportSvgSource() {
  if(isRepositoryOverview()&&ui.workspace!=='services')return null;
  const id=ui.workspace==='services'?'services-tree':ui.workspace==='graph'&&ui.diagram==='overview'?'package-tree':null;
  return id?document.getElementById(id).querySelector('svg'):null;
}
function exportSvg() {
  const source=exportSvgSource();if(!source)return null;
  if(source.getAttribute('data-tree-windowed')==='true')return withFullPackageTree(exportSvg);
  const clone=source.cloneNode(true),bounds=source.getAttribute('viewBox').split(' ').map(Number);
  // Inline resolved presentation values, so embedding the SVG in Markdown or
  // another document does not depend on the viewer's stylesheet or theme.
  const properties=['fill','fill-opacity','stroke','stroke-width','stroke-opacity','stroke-dasharray','stroke-linecap','stroke-linejoin','opacity','font-family','font-size','font-weight','font-style','text-anchor','dominant-baseline','paint-order'];
  function copyStyles(original,copy){
    const style=getComputedStyle(original),values=properties.map(name=>[name,style.getPropertyValue(name).trim()]).filter(([,value])=>value);
    if(values.length)copy.setAttribute('style',values.map(([name,value])=>name+':'+value).join(';'));
    copy.removeAttribute('tabindex');
    for(let index=0;index<original.children.length;index++)copyStyles(original.children[index],copy.children[index]);
  }
  copyStyles(source,clone);clone.setAttribute('xmlns','http://www.w3.org/2000/svg');clone.setAttribute('width',bounds[2]);clone.setAttribute('height',bounds[3]);clone.setAttribute('role','img');
  const background=treeSvg('rect',{x:bounds[0],y:bounds[1],width:bounds[2],height:bounds[3],fill:getComputedStyle(document.documentElement).getPropertyValue('--bg').trim()||'#fff'});
  clone.insertBefore(background,clone.firstChild);
  const snapshot=exportSnapshot(),legend=ui.workspace==='services'?document.getElementById('services-reference').textContent:document.getElementById('package-tree').querySelector('details[data-tree-legend]')?.textContent||'';
  clone.insertBefore(treeSvg('desc',{},snapshot.coverage+' Filters: '+JSON.stringify(snapshot.filters)+'. Source → target arrows. '+legend),clone.firstChild);
  clone.insertBefore(treeSvg('title',{},rootName+' · '+(ui.workspace==='services'?'Service configuration':'Import dependencies')),clone.firstChild);
  return '<?xml version="1.0" encoding="UTF-8"?>\n'+new XMLSerializer().serializeToString(clone)+'\n';
}
function downloadDocumentation(format) {
  let content,type;
  if(format==='svg'){
    content=exportSvg();type='image/svg+xml';
    if(content===null){feedback(t('SVG export is available in Graph → Tree and Services. Markdown and JSON support every workspace.','SVG-Export ist unter Graph → Baum und Dienste verfügbar. Markdown und JSON unterstützen jeden Arbeitsbereich.'));return;}
  }else{const snapshot=exportSnapshot();content=format==='json'?JSON.stringify(snapshot,null,2)+'\n':exportMarkdown(snapshot);type=format==='json'?'application/json':'text/markdown';}
  const anchor=document.createElement('a');anchor.href=URL.createObjectURL(new Blob([content],{type:type+';charset=utf-8'}));anchor.download='archguard-'+ui.workspace+'.'+(format==='markdown'?'md':format);anchor.click();
  setTimeout(()=>URL.revokeObjectURL(anchor.href),1000);
  document.getElementById('export-menu').open=false;feedback(t('Exported {filename} with the current scope and filter settings.','{filename} mit dem aktuellen Ausschnitt und den Filtereinstellungen exportiert.',{filename:anchor.download}));
}
for(const format of ['markdown','json','svg'])document.getElementById('export-'+format).addEventListener('click',()=>downloadDocumentation(format));
function downloadArc42(chapter) {
  const model=data.insights?.arc42,content=chapter?model?.chapters?.find(c=>c.number===Number(chapter))?.markdown:model?.markdown;
  if(typeof content!=='string')return;
  const anchor=document.createElement('a');anchor.href=URL.createObjectURL(new Blob([content],{type:'text/markdown;charset=utf-8'}));anchor.download=chapter?'arc42-'+String(chapter).padStart(2,'0')+'.md':'arc42.md';anchor.click();
  setTimeout(()=>URL.revokeObjectURL(anchor.href),0);document.getElementById('export-menu').open=false;
  feedback(t('Exported {name} from the original project evidence.','{name} aus den ursprünglichen Projektbelegen exportiert.',{name:anchor.download}));
}
document.getElementById('export-arc42').addEventListener('click',()=>downloadArc42());
document.getElementById('export-menu').addEventListener('toggle',()=>{
  const arc42=typeof data.insights?.arc42?.markdown==='string';document.getElementById('export-arc42').disabled=!arc42;document.getElementById('export-arc42-hint').hidden=arc42;
  const available=!!exportSvgSource(),button=document.getElementById('export-svg');button.disabled=!available;
  document.getElementById('export-svg-hint').hidden=available;
});
