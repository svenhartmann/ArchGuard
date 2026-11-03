'use strict';
// Declared project dependencies and original source evidence stay separate from runtime topology.
const workspaceUi={section:'dependencies',project:'',direction:'all',query:'',target:'',targetQuery:'',targetInitialized:false,cyclesOnly:false,limit:20,lastServiceProject:null};
const workspaceIndex={model:null,projects:new Map(),source:null,files:new Map(),nodes:new Map()};
function workspaceModel(){return data.insights?.workspace||null;}
function workspaceSources(){return data.insights?.workspace_sources||null;}
function workspaceProjects(){const model=workspaceModel();if(workspaceIndex.model!==model){workspaceIndex.model=model;workspaceIndex.projects=new Map((model?.projects||[]).map(p=>[p.id,p]));}return workspaceIndex.projects;}
function workspaceProjectLabel(id){const p=workspaceProjects().get(id);return p?p.path+' · '+p.name:String(id||'—');}
function workspaceText(parent,text,tag='p',className=''){engineeringText(parent,text,tag,className);}
function workspaceCycle(edge,groups){return (groups||[]).some(group=>group.includes(edge.from)&&group.includes(edge.to));}
function workspaceMatchesProject(from,to){
  if(!workspaceUi.project||workspaceUi.direction==='all')return true;
  const incoming=to===workspaceUi.project,outgoing=from===workspaceUi.project;
  return workspaceUi.direction==='incoming'?incoming:workspaceUi.direction==='outgoing'?outgoing:incoming||outgoing;
}
function workspaceQuery(values){return !workspaceUi.query||values.some(value=>String(value||'').toLowerCase().includes(workspaceUi.query.toLowerCase()));}
function workspaceCalls(){const source=workspaceSources();if(source?.calls)return source.calls;if(source?.call_summary){prepareCallModel();return callModel;}return null;}
function workspaceFileIndex(){const source=workspaceSources();if(workspaceIndex.source!==source){workspaceIndex.source=source;workspaceIndex.files=new Map((source?.files||[]).map(file=>[file.id,file]));workspaceIndex.nodes=new Map((source?.graph?.nodes||[]).map(node=>[node.id,node.name]));}return workspaceIndex;}
function workspaceTestImpact(){
  const source=workspaceSources(),target=workspaceUi.target,files=source?.files||[],selectedFiles=new Set();
  if(target==='@reported'&&data.insights?.workspace_impact){
    const saved=data.insights.workspace_impact,index=workspaceFileIndex(),tests=new Map((source?.tests||[]).map(test=>[test.id,test]));
    const unknown=test=>!(test.mappings||[]).length||(test.mappings||[]).some(m=>!m.node||!index.files.has(m.node)||m.status!=='unchanged_since_import');
    const selected=(saved.selected||[]).map(row=>{const test=tests.get(row.test_id)||{id:row.test_id,mappings:[]};return {...test,...row,id:row.test_id,unknown:unknown(test)};}),selectedIds=new Set(selected.map(test=>test.id));
    return {...saved,selected,unknown:[...tests.values()].filter(test=>!selectedIds.has(test.id)&&unknown(test)).map(test=>({...test,unknown:true,matched_sources:[]})),safe_to_skip_other_tests:false};
  }
  if(target.startsWith('@project:')){for(const file of files)if(file.project===target.slice(9))selectedFiles.add(file.id);}
  else if(files.some(file=>file.id===target))selectedFiles.add(target);
  const affected=new Set(selectedFiles),index=workspaceFileIndex(),incoming=new Map();
  for(const edge of source?.graph?.edges||[]){const from=index.nodes.get(edge.from),to=index.nodes.get(edge.to);if(from&&to){if(!incoming.has(to))incoming.set(to,[]);incoming.get(to).push(from);}}
  const queue=[...affected];for(let i=0;i<queue.length;i++)for(const from of incoming.get(queue[i])||[])if(!affected.has(from)){affected.add(from);queue.push(from);}
  const selected=[],unknown=[];
  for(const test of source?.tests||[]){
    const mappings=test.mappings||[],valid=mappings.filter(m=>m.node&&index.files.has(m.node)&&m.status==='unchanged_since_import');
    const matches=valid.filter(m=>affected.has(m.node));
    const uncertain=!mappings.length||valid.length!==mappings.length;
    const row={...test,unknown:uncertain,matched_sources:matches.map(m=>m.node),reason:matches.some(m=>selectedFiles.has(m.node))?'mapped_changed_source':'mapped_static_dependent'};
    if(!target||matches.length)selected.push(row);else if(uncertain)unknown.push(row);
  }
  const unknownTests=(source?.tests||[]).filter(test=>!(test.mappings||[]).length||(test.mappings||[]).some(m=>!m.node||!index.files.has(m.node)||m.status!=='unchanged_since_import')).length,unresolved=target&&!selectedFiles.size?[target]:[];
  const affectedProjects=[...new Set(files.filter(f=>affected.has(f.id)).map(f=>f.project))],importedProjects=new Set((source?.tests||[]).map(test=>test.project)),withoutTests=affectedProjects.filter(project=>!importedProjects.has(project));
  const diagnostics=withoutTests.length?['No imported test evidence for affected projects: '+withoutTests.join(', ')]:[];
  return {status:!source?'unavailable':source.status==='ready'&&source.tests?.length&&!unknownTests&&!unresolved.length&&!withoutTests.length?'ready':'partial',targets:[...selectedFiles],unresolved_targets:unresolved,affected_files:[...affected],affected_projects:affectedProjects,selected,unknown,unknown_tests:unknownTests,safe_to_skip_other_tests:false,diagnostics};
}
function workspaceRows(){
  const model=workspaceModel(),source=workspaceSources(),section=workspaceUi.section;
  if(section==='dependencies')return (model?.dependencies||[]).filter(edge=>workspaceMatchesProject(edge.from,edge.to)&&(!workspaceUi.cyclesOnly||workspaceCycle(edge,model.cycles))&&workspaceQuery([workspaceProjectLabel(edge.from),workspaceProjectLabel(edge.to),edge.scope,edge.kind]));
  if(section==='unresolved')return (model?.unresolved||[]).filter(row=>(!workspaceUi.project||row.project===workspaceUi.project)&&workspaceQuery([workspaceProjectLabel(row.project),row.target,row.reason]));
  if(section==='sources'){
    const index=workspaceFileIndex();
    return (source?.graph?.edges||[]).map(edge=>({...edge,from:index.nodes.get(edge.from),to:index.nodes.get(edge.to)})).filter(edge=>edge.from&&edge.to&&workspaceMatchesProject(index.files.get(edge.from)?.project,index.files.get(edge.to)?.project)&&(!workspaceUi.cyclesOnly||workspaceCycle(edge,source.cycles))&&workspaceQuery([edge.from,edge.to]));
  }
  if(section==='calls'){
    const calls=workspaceCalls(),index=workspaceFileIndex(),owners=new Map((calls?.classes||[]).map(cls=>[cls.id,index.files.get(cls.graph_node)?.project]));
    return (calls?.calls||[]).filter(call=>workspaceMatchesProject(owners.get(call.from_class),owners.get(call.to_class))&&workspaceQuery([call.from_method,call.from_class,call.to_method,call.to_class,call.target_label,call.file]));
  }
  if(section==='tests'){
    const impact=workspaceTestImpact();return [...impact.selected,...impact.unknown].filter(row=>(!workspaceUi.project||row.project===workspaceUi.project)&&workspaceQuery([row.id,row.name,row.suite,workspaceProjectLabel(row.project)])).sort((a,b)=>Number(a.unknown)-Number(b.unknown)||(Number.isFinite(b.duration_seconds)?b.duration_seconds:-1)-(Number.isFinite(a.duration_seconds)?a.duration_seconds:-1));
  }
  return [];
}
function workspaceEvidence(card,records){
  if(!records?.length)return;
  const details=element('details'),summary=element('summary',t('{count} source evidence records','{count} Quellenbelege',{count:records.length}));details.appendChild(summary);card.appendChild(details);
  let limit=0;const list=element('div');details.appendChild(list);
  function more(){const end=Math.min(records.length,limit+20);for(const record of records.slice(limit,end)){const row=element('div',undefined,'workspace-evidence');workspaceText(row,serviceEvidenceLocation(record),'strong');workspaceText(row,record.description??record.raw??record.code??'');list.appendChild(row);}limit=end;next.hidden=limit>=records.length;}
  const next=button(t('More evidence','Weitere Belege'),more,'btn btn-secondary');next.hidden=true;details.appendChild(next);
  details.addEventListener('toggle',()=>{if(details.open&&!limit)more();});
}
function workspaceProjectLink(card,id){const project=workspaceProjects().get(id);if(project)appendRepositoryProjectNavigation(card,project);}
function workspaceRenderCard(list,row){
  const section=workspaceUi.section,card=element('article',undefined,'engineering-card workspace-relation');list.appendChild(card);
  if(section==='dependencies'){
    workspaceText(card,workspaceProjectLabel(row.from)+' → '+workspaceProjectLabel(row.to),'h3');
    card.setAttribute('data-direction',row.to===workspaceUi.project?'incoming':row.from===workspaceUi.project?'outgoing':'context');
    workspaceText(card,row.kind+' · '+row.scope+(workspaceCycle(row,workspaceModel()?.cycles)?' · '+t('cyclic project group','zyklische Projektgruppe'):''),'p','muted');
    workspaceEvidence(card,row.evidence);workspaceProjectLink(card,row.from);workspaceProjectLink(card,row.to);
  }else if(section==='unresolved'){
    workspaceText(card,workspaceProjectLabel(row.project)+' → '+row.target,'h3');workspaceText(card,row.reason);workspaceEvidence(card,row.evidence);
  }else if(section==='sources'){
    workspaceText(card,row.from+' → '+row.to,'h3');workspaceText(card,t('{count} dependency evidence records','{count} Abhängigkeitsbelege',{count:row.count??row.refs?.length??0}),'p','muted');
    workspaceEvidence(card,row.refs);engineeringLocate(card,row.from,t('Show source','Quelle anzeigen'));engineeringLocate(card,row.to,t('Show target','Ziel anzeigen'));
  }else if(section==='calls'){
    workspaceText(card,(row.from_method||row.from_class)+' → '+(row.to_method||row.to_class||row.target_label||'—'),'h3');
    workspaceText(card,(row.resolution||'unresolved')+' · '+row.file+':'+row.line,'p','muted');workspaceText(card,row.code||'','pre');
  }else if(section==='tests'){
    workspaceText(card,workspaceProjectLabel(row.project)+' · '+row.name,'h3');
    workspaceText(card,row.suite+' · '+row.outcome+' · '+engineeringNumber(row.duration_seconds)+' s'+(row.unknown?' · '+t('unknown or stale mapping','unbekannte oder veraltete Zuordnung'):''),'p','muted');
    if(workspaceUi.target&&row.matched_sources.length)workspaceText(card,row.reason==='mapped_changed_source'?t('Mapped directly to changed source.','Direkt der geänderten Quelle zugeordnet.'):t('Mapped to a static dependent, including other projects.','Einem statischen Aufrufer zugeordnet, auch über Projektgrenzen.'));
    workspaceEvidence(card,(row.mappings||[]).map(m=>({file:m.path,description:m.status})));for(const name of row.matched_sources||[])engineeringLocate(card,name);
  }
}
function renderWorkspaceTargets(){
  const source=workspaceSources(),control=document.getElementById('workspace-target');control.textContent='';
  if(!workspaceUi.targetInitialized){workspaceUi.targetInitialized=true;if(data.insights?.workspace_impact?.targets?.length)workspaceUi.target='@reported';}
  const all=element('option',t('All imported tests','Alle importierten Tests'));all.value='';control.appendChild(all);
  if(data.insights?.workspace_impact){const saved=element('option',t('Recorded target set','Aufgezeichnete Zielauswahl'));saved.value='@reported';control.appendChild(saved);}
  const projectsWithFiles=new Set((source?.files||[]).map(file=>file.project));
  const query=workspaceUi.targetQuery.toLowerCase(),options=[...(workspaceModel()?.projects||[]).filter(p=>projectsWithFiles.has(p.id)).map(p=>({id:'@project:'+p.id,label:t('Project: ','Projekt: ')+workspaceProjectLabel(p.id)})),...(source?.files||[]).map(file=>({id:file.id,label:file.path+' · '+workspaceProjectLabel(file.project)}))];
  const matching=options.filter(option=>!query||option.label.toLowerCase().includes(query)),shown=matching.slice(0,2000),selected=options.find(option=>option.id===workspaceUi.target);if(selected&&!shown.includes(selected))shown.push(selected);
  for(const item of shown){const option=element('option',item.label);option.value=item.id;control.appendChild(option);}control.value=workspaceUi.target;
  document.getElementById('workspace-target-query').value=workspaceUi.targetQuery;
  document.getElementById('workspace-target-note').textContent=matching.length>2000?t('First 2000 of {count} matching targets; refine the target search. The selected target remains available.','Die ersten 2000 von {count} passenden Zielen; Zielsuche eingrenzen. Das ausgewählte Ziel bleibt erhalten.',{count:matching.length}):'';
}
function renderWorkspace(){
  const model=workspaceModel(),source=workspaceSources(),panel=document.getElementById('workspace-panel'),list=document.getElementById('workspace-list');
  document.getElementById('workspace-heading').textContent=t('Project workspace','Projektverbund');
  const analyze=document.getElementById('workspace-analyze'),back=document.getElementById('workspace-back'),live=repositoryLive();
  analyze.hidden=!live;back.hidden=!live||!source;
  if(live)analyze.setAttribute('href','/?workspace=sources');else analyze.removeAttribute('href');
  if(live&&source)back.setAttribute('href','/');else back.removeAttribute('href');
  document.getElementById('workspace-status').textContent=model?t('{projects} projects · {edges} declared dependencies · {cycles} cyclic groups · {status}','{projects} Projekte · {edges} deklarierte Abhängigkeiten · {cycles} zyklische Gruppen · {status}',{projects:model.projects.length,edges:model.dependencies.length,cycles:model.cycles.length,status:model.status}):t('Project dependency analysis not included.','Projektabhängigkeitsanalyse nicht enthalten.');
  if(!panel.open)return;
  if(serviceUi.project!==workspaceUi.lastServiceProject){workspaceUi.project=serviceUi.project||'';workspaceUi.lastServiceProject=serviceUi.project;workspaceUi.limit=20;}
  const selector=document.getElementById('workspace-project');selector.textContent='';const all=element('option',t('All projects','Alle Projekte'));all.value='';selector.appendChild(all);
  for(const project of model?.projects||[]){const option=element('option',workspaceProjectLabel(project.id));option.value=project.id;selector.appendChild(option);}selector.value=workspaceUi.project;
  for(const key of ['section','direction','query'])document.getElementById('workspace-'+key).value=workspaceUi[key];
  document.getElementById('workspace-direction').disabled=!workspaceUi.project||['tests','unresolved'].includes(workspaceUi.section);
  const cycles=document.getElementById('workspace-cycles-only');cycles.setAttribute('aria-pressed',String(workspaceUi.cyclesOnly));cycles.disabled=!['dependencies','sources'].includes(workspaceUi.section);
  const tests=workspaceUi.section==='tests';document.getElementById('workspace-target-control').hidden=!tests;document.getElementById('workspace-target-note').hidden=!tests;if(tests)renderWorkspaceTargets();
  const note=document.getElementById('workspace-note');
  if(!model)note.textContent=t('Generate project dependencies with archguard workspace PATH --format html.','Projektabhängigkeiten mit archguard workspace PFAD --format html erzeugen.');
  else if(!source&&['sources','calls','tests'].includes(workspaceUi.section))note.textContent=t('Source evidence is loaded explicitly: archguard workspace PATH --sources --format html. No source cycles or test coverage are inferred from the manifest graph.','Quellbelege werden explizit geladen: archguard workspace PFAD --sources --format html. Aus dem Manifestgraph werden keine Quellzyklen oder Testabdeckung abgeleitet.');
  else if(tests){const impact=workspaceTestImpact();note.textContent=impact.status+' · '+t('{files} affected files · {projects} affected projects · {unknown} tests with unknown or stale mappings across the workspace. Static recommendations do not establish that other tests can be skipped.','{files} betroffene Dateien · {projects} betroffene Projekte · {unknown} Tests mit unbekannten oder veralteten Zuordnungen im Projektverbund. Statische Empfehlungen belegen nicht, dass andere Tests ausgelassen werden können.',{files:impact.affected_files.length,projects:impact.affected_projects.length,unknown:impact.unknown_tests})+(impact.unresolved_targets.length?' '+t('Selected change target is unavailable.','Das ausgewählte Änderungsziel ist nicht verfügbar.'):'')+(!(source.tests||[]).length?' '+t('No test evidence imported.','Keine Testbelege importiert.'):'')+(workspaceUi.target==='@reported'?' '+t('Recorded targets: ','Aufgezeichnete Ziele: ')+impact.targets.join(', '):'');}
  else note.textContent=workspaceUi.section==='dependencies'?t('Arrows point from consumer to dependency. Cyclic groups are strongly connected components (SCCs), not ordered cycle paths. Manifest dependencies are distinct from runtime service calls.','Pfeile führen vom Verbraucher zur Abhängigkeit. Zyklische Gruppen sind stark zusammenhängende Komponenten (SCCs), keine geordneten Zykluspfade. Manifestabhängigkeiten sind getrennt von Dienstaufrufen zur Laufzeit.'):
    workspaceUi.section==='sources'?t('Original project-qualified file dependencies. Source cycles are SCCs; project dependency cycles do not imply file cycles.','Ursprüngliche Dateiabhängigkeiten mit Projektidentität. Quellzyklen sind SCCs; Projektzyklen belegen keine Dateizyklen.'):
    workspaceUi.section==='calls'?t('Compiler call evidence retains project identities and unresolved targets. Static resolution is not a runtime trace.','Compiler-Aufrufbelege erhalten Projektidentitäten und ungeklärte Ziele. Statische Auflösung ist keine Laufzeitaufzeichnung.'):
    t('Unresolved or ambiguous manifest references remain separate from resolved dependencies.','Ungeklärte oder mehrdeutige Manifestverweise bleiben von aufgelösten Abhängigkeiten getrennt.');
  const rows=workspaceRows();list.textContent='';for(const row of rows.slice(0,workspaceUi.limit))workspaceRenderCard(list,row);
  if(!rows.length)workspaceText(list,t('No recorded evidence matches these filters. Check analysis coverage below.','Keine erfassten Belege passen zu diesen Filtern. Analyseabdeckung unten beachten.'),'p','empty');
  document.getElementById('workspace-count').textContent=t('{shown} / {total} records shown','{shown} / {total} Belege angezeigt',{shown:Math.min(rows.length,workspaceUi.limit),total:rows.length});document.getElementById('workspace-more').hidden=rows.length<=workspaceUi.limit;
  const diagnostics=document.getElementById('workspace-diagnostics-list');diagnostics.textContent='';
  for(const [name,report]of [[t('Manifest analysis','Manifestanalyse'),model],[t('Source analysis','Quellanalyse'),source]]){
    workspaceText(diagnostics,name+': '+(report?.status||t('not included','nicht enthalten')),'strong');
    for(const item of [...(report?.diagnostics||[]),...(report?.limitations||[])].slice(0,100))workspaceText(diagnostics,item);
    if((report?.diagnostics?.length||0)+(report?.limitations?.length||0)>100)workspaceText(diagnostics,t('First 100 diagnostic entries shown; export contains all entries.','Erste 100 Diagnoseeinträge angezeigt; Export enthält alle Einträge.'));
  }
  if(source)for(const project of (source.projects||[]).slice(0,100))workspaceText(diagnostics,workspaceProjectLabel(project.project)+' · '+project.status+' · '+project.files+' '+t('files','Dateien')+' · Calls: '+project.call_status);
  if((source?.projects?.length||0)>100)workspaceText(diagnostics,t('First 100 project coverage entries shown; export contains all entries.','Erste 100 Projektabdeckungen angezeigt; Export enthält alle Einträge.'));
  if(tests)for(const note of workspaceTestImpact().diagnostics||[])workspaceText(diagnostics,note);
}
function workspaceExport(){
  const model=workspaceModel(),source=workspaceSources();
  return {...(model?{workspace:model,workspace_filters:{section:workspaceUi.section,project:workspaceUi.project||null,direction:workspaceUi.direction,query:workspaceUi.query,cycles_only:workspaceUi.cyclesOnly,target:workspaceUi.target||null}}:{}),...(source?{workspace_sources:{...source,calls:workspaceCalls()},workspace_impact:workspaceUi.target?workspaceTestImpact():(data.insights?.workspace_impact||null)}:{})};
}
function appendWorkspaceMarkdown(lines,snapshot){
  const model=snapshot.workspace,source=snapshot.workspace_sources,cell=exportMarkdownCell;if(!model)return;
  lines.push('','## Project workspace','','Status: '+cell(model.status),'Complete original workspace evidence; UI filters and list pages do not remove exported records.');
  if(!snapshot.projects?.length)for(const project of model.projects||[])lines.push('',cell(project.id)+' · '+cell(project.name)+' · '+cell(project.path)+' · '+cell(project.ecosystem)+' · '+cell(project.manifest));
  lines.push('','| Consumer | Dependency | Kind | Scope |','| --- | --- | --- | --- |');
  for(const edge of model.dependencies||[]){lines.push('| '+[edge.from,edge.to,edge.kind,edge.scope].map(cell).join(' | ')+' |');for(const record of edge.evidence||[])lines.push('',cell(serviceEvidenceLocation(record))+' · '+cell(record.description));}
  for(const cycle of model.cycles||[])lines.push('','Project SCC: '+cycle.map(cell).join(', '));
  for(const row of model.unresolved||[]){lines.push('',cell(row.project)+' → '+cell(row.target)+' · '+cell(row.reason));for(const record of row.evidence||[])lines.push('',cell(serviceEvidenceLocation(record))+' · '+cell(record.description));}
  for(const note of [...(model.diagnostics||[]),...(model.limitations||[])])lines.push('',cell(note));
  if(!source){lines.push('','Source analysis: not included.');return;}
  lines.push('','### Project-qualified source evidence','','Status: '+cell(source.status));
  const nodeNames=new Map((source.graph?.nodes||[]).map(node=>[node.id,node.name]));
  for(const project of source.projects||[])lines.push('',cell(project.project)+' · '+cell(project.status)+' · '+cell(project.files)+' files · Calls: '+cell(project.call_status),...(project.diagnostics||[]).map(cell));
  for(const file of source.files||[])lines.push('',cell(file.id)+' · '+cell(file.path));
  for(const edge of source.graph?.edges||[]){lines.push('',cell(nodeNames.get(edge.from))+' → '+cell(nodeNames.get(edge.to)));for(const record of edge.refs||[])lines.push(cell(record.file+':'+record.line)+' · '+cell(record.raw));}
  for(const cycle of source.cycles||[])lines.push('','File SCC: '+cycle.map(cell).join(', '));
  for(const call of source.calls?.calls||[])lines.push('',cell(call.from_method||call.from_class)+' → '+cell(call.to_method||call.to_class||call.target_label)+' · '+cell(call.resolution),cell(call.file+':'+call.line)+' · '+cell(call.code));
  for(const test of source.tests||[]){lines.push('',cell(test.id)+' · '+cell(test.project)+' · '+cell(test.name)+' · '+cell(test.outcome)+' · '+engineeringNumber(test.duration_seconds)+' s');for(const mapping of test.mappings||[])lines.push(cell(mapping.path)+' · '+cell(mapping.status));}
  for(const note of [...(source.diagnostics||[]),...(source.limitations||[]),...(source.calls?.diagnostics||[])])lines.push('',cell(note));
  if(snapshot.workspace_impact){const impact=snapshot.workspace_impact;lines.push('','### Change impact','','Status: '+cell(impact.status),'Unknown tests: '+cell(impact.unknown_tests),'Other tests cannot be declared safe to skip.',...(impact.affected_files||[]).map(cell));for(const test of impact.selected||[])lines.push('',cell(test.test_id||test.id)+' · '+cell(test.project)+' · '+cell(test.name)+' · '+cell(test.reason));for(const note of impact.diagnostics||[])lines.push('',cell(note));}
}
document.getElementById('workspace-panel').addEventListener('toggle',renderWorkspace);
for(const id of ['workspace-analyze','workspace-back'])document.getElementById(id).addEventListener('click',repositoryNavigate);
for(const key of ['section','project','direction','query','target'])document.getElementById('workspace-'+key).addEventListener(key==='query'?'input':'change',event=>{workspaceUi[key]=event.target.value;workspaceUi.limit=20;if(key==='project')workspaceUi.direction=workspaceUi.project?'both':'all';renderWorkspace();});
document.getElementById('workspace-target-query').addEventListener('input',event=>{workspaceUi.targetQuery=event.target.value;renderWorkspaceTargets();});
document.getElementById('workspace-cycles-only').addEventListener('click',()=>{workspaceUi.cyclesOnly=!workspaceUi.cyclesOnly;workspaceUi.limit=20;renderWorkspace();});
document.getElementById('workspace-more').addEventListener('click',()=>{workspaceUi.limit+=20;renderWorkspace();});
