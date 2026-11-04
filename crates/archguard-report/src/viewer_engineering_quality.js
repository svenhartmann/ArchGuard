'use strict';
// Measurements stay attached to the original source snapshot, never to virtual moves.
const engineeringUi={section:null,query:'',target:'',targetQuery:'',limit:20,contractsLimit:20,cohesionPackage:null,cohesionGroup:null,cohesionLimit:20};
// A section `serve` left out of the page is not data until it was fetched.
function engineeringData(key){const value=data.insights?.[key];return value&&!deferredSection(key)?value:null;}
function engineeringStatus(status){return ({unchanged_since_import:t('unchanged since import','seit Import unverändert'),changed_since_import:t('changed since import','seit Import geändert'),unmapped:t('unmapped','nicht zugeordnet'),not_recorded:t('not recorded','nicht aufgezeichnet'),compared:t('compared','verglichen'),no_baseline:t('no previous import','kein vorheriger Import'),unverified:t('unverified','nicht verifiziert'),comparable:t('comparable','vergleichbar'),incomparable:t('not comparable','nicht vergleichbar'),present:t('present','vorhanden'),added:t('added','hinzugefügt'),removed:t('removed','entfernt')})[status]||coverageStatusText(status||'not_imported');}
function engineeringText(container,text,tag='p',className=''){const raw=String(text??'');container.appendChild(element(tag,raw.length>4096?raw.slice(0,4096)+'…':raw,className));if(raw.length>4096)container.appendChild(element('small',t('Text shortened; full evidence is included in the export.','Text gekürzt; vollständige Belege stehen im Export.'),'muted'));}
function engineeringLocate(container,name,label){
  const original=data.nodes.find(n=>n.name===name),current=original&&nodeById(work,original.id);if(!current)return;
  const action=button(label||t('Locate in graph','Im Graphen anzeigen'),()=>{showTab('graph');revealNode(current.name);},'btn btn-secondary');
  action.setAttribute('data-engineering-node',name);container.appendChild(action);
}
function engineeringCard(container,name){const card=element('article',undefined,'engineering-card');engineeringText(card,name,'h3');container.appendChild(card);return card;}
function engineeringPercent(value){return Number.isFinite(value)?value.toFixed(1)+'%':'—';}
function engineeringNumber(value){return Number.isFinite(value)?Number(value.toFixed(2)).toString():'—';}
function engineeringScope(){
  if(!engineeringUi.target)return null;
  const node=data.nodes.find(n=>n.name===engineeringUi.target);if(!node)return new Set();
  const children=new Map();for(const n of data.nodes){if(!children.has(n.parent))children.set(n.parent,[]);children.get(n.parent).push(n.id);}
  const ids=new Set([node.id]),stack=[node.id];while(stack.length){for(const child of children.get(stack.pop())||[]){if(!ids.has(child)){ids.add(child);stack.push(child);}}}
  return ids;
}
function engineeringMatches(value){return !engineeringUi.query||String(value).toLowerCase().includes(engineeringUi.query.toLowerCase());}
function engineeringRows(){
  const scope=engineeringScope(),names=scope&&new Set(data.nodes.filter(n=>scope.has(n.id)).map(n=>n.name));
  if(['classes','methods','heatmap'].includes(engineeringUi.section)){
    const paths=scope&&new Set((data.reviewContext?.sources?.files||[]).filter(f=>scope.has(f.node_id)).map(f=>f.path));
    return (engineeringData('code_metrics')?.[engineeringUi.section==='heatmap'?'classes':engineeringUi.section]||[]).filter(row=>(!scope||names.has(row.node)||names.has(row.file)||paths.has(row.file))&&engineeringMatches([row.class_id,row.method_id,row.file].join(' '))).slice().sort((a,b)=>engineeringUi.section!=='methods'?(b.cbo?.value??-1)-(a.cbo?.value??-1)||compareNames(a.class_id,b.class_id):(b.cognitive??-1)-(a.cognitive??-1)||compareNames(a.method_id,b.method_id));
  }
  if(engineeringUi.section==='knowledge')return (engineeringData('knowledge_metrics')?.files||[]).filter(row=>(!names||names.has(row.node))&&engineeringMatches(row.node)).slice().sort((a,b)=>(b.top_share??-1)-(a.top_share??-1)||b.touches-a.touches||compareNames(a.node,b.node));
  if(engineeringUi.section==='measurements')return (engineeringData('measurement_runs')?.groups||[]).filter(row=>engineeringMatches([row.kind,row.name,row.revision,row.environment,row.producer].join(' ')));
  if(engineeringUi.section==='cohesion')return (engineeringData('cohesion')?.packages||[]).filter(p=>(!names||names.has(p.node)||p.groups.some(g=>g.files.some(f=>names.has(f))))&&engineeringMatches(p.node)).slice().sort((a,b)=>Number(b.split_candidate)-Number(a.split_candidate)||compareNames(a.node,b.node));
  if(engineeringUi.section==='history'){
    const lineage=new Set(names||[]),byId=new Map(data.nodes.map(n=>[n.id,n]));let selected=data.nodes.find(n=>n.name===engineeringUi.target);const seen=new Set();
    const removedPrefix=selected?.kind==='module'?selected.name.replace(/\/+$/,'')+'/':null;
    if(selected?.kind==='file'&&selected.parent==null)lineage.add('.');
    while(selected&&!seen.has(selected.id)){seen.add(selected.id);lineage.add(selected.name);selected=byId.get(selected.parent);}
    const history=engineeringData('quality_history'),latest=history?.latest?.modules||[],present=new Set(latest.map(p=>p.node));
    const removed=(history?.comparison?.modules||[]).filter(p=>p.status==='removed'&&!present.has(p.node)).map(p=>({node:p.node,_removed:true}));
    return latest.concat(removed).filter(p=>(!names||lineage.has(p.node)||(p._removed&&removedPrefix&&p.node.startsWith(removedPrefix)))&&engineeringMatches(p.node));
  }
  if(engineeringUi.section==='tests'){
    const affected=new Set(scope||[]),incoming=new Map();for(const edge of data.edges){if(!incoming.has(edge.to))incoming.set(edge.to,[]);incoming.get(edge.to).push(edge.from);}
    const stack=[...affected];while(stack.length){for(const id of incoming.get(stack.pop())||[]){if(!affected.has(id)){affected.add(id);stack.push(id);}}}
    const affectedNames=new Set(data.nodes.filter(n=>affected.has(n.id)).map(n=>n.name));
    return (engineeringData('test_evidence')?.tests||[]).map(test=>{
      const fresh=test.mappings.filter(m=>m.node&&m.status==='unchanged_since_import');
      return {test,matched:fresh.filter(m=>affectedNames.has(m.node)),unknown:!fresh.length||fresh.length<test.mappings.length};
    }).filter(row=>(!scope||row.matched.length||row.unknown)&&engineeringMatches(row.test.name+' '+row.test.suite+' '+row.test.id))
      .sort((a,b)=>b.matched.length-a.matched.length||(b.test.duration_seconds??-1)-(a.test.duration_seconds??-1)||compareNames(a.test.id,b.test.id));
  }
  if(engineeringUi.section==='duplicates')return (engineeringData('duplicates')?.groups||[]).filter(g=>(!names||g.occurrences.some(o=>names.has(o.node)))&&engineeringMatches(g.id+' '+g.status+' '+g.occurrences.map(o=>o.path).join(' ')+' '+g.snippet)).slice().sort((a,b)=>b.tokens-a.tokens||compareNames(a.id,b.id));
  const rows=[],incoming=new Map();for(const edge of data.edges){if(!incoming.has(edge.to))incoming.set(edge.to,new Set());incoming.get(edge.to).add(edge.from);}
  const ids=new Map(data.nodes.map(n=>[n.name,n.id]));
  for(const file of engineeringData('mutations')?.files||[]){if(names&&!names.has(file.node))continue;for(const mutant of file.mutants||[])if(engineeringMatches(file.path+' '+mutant.mutator_name+' '+mutant.status))rows.push({file,mutant,incoming:incoming.get(ids.get(file.node))?.size||0});}
  return rows.sort((a,b)=>({Survived:0,NoCoverage:1,Pending:2}[a.mutant.status]??3)-({Survived:0,NoCoverage:1,Pending:2}[b.mutant.status]??3)||b.incoming-a.incoming||compareNames(a.file.path,b.file.path));
}
function renderCohesionCard(container,p){
  const card=element('article',undefined,'engineering-card cohesion-card'),heading=element('div',undefined,'cohesion-heading'),title=element('h3',p.node.split('/').pop()||p.node);title.setAttribute('title',p.node);heading.appendChild(title);container.appendChild(card);card.appendChild(heading);
  const selected=engineeringUi.cohesionPackage===p.node,choose=button(selected?t('Selected','Ausgewählt'):t('Inspect groups','Gruppen untersuchen'),()=>{engineeringUi.cohesionPackage=p.node;engineeringUi.cohesionGroup=null;engineeringUi.cohesionLimit=20;renderEngineeringQuality();},'btn btn-quiet');choose.setAttribute('aria-pressed',String(selected));choose.setAttribute('data-cohesion-package',p.node);heading.appendChild(choose);
  const connected=p.groups.filter(g=>g.internal_dependencies>0).length,isolated=p.groups.filter(g=>g.files.length===1&&g.internal_dependencies===0).length;
  engineeringText(card,[plural(p.files,'{count} file','{count} files','{count} Datei','{count} Dateien'),plural(connected,'{count} connected group','{count} connected groups','{count} zusammenhängende Gruppe','{count} zusammenhängende Gruppen'),plural(isolated,'{count} isolated file','{count} isolated files','{count} isolierte Datei','{count} isolierte Dateien')].join(' · '));
  if(p.split_candidate)engineeringText(card,t('Review package split: check the responsibilities of these groups.','Paketaufteilung prüfen: Verantwortlichkeiten der Gruppen vergleichen.'),'p','detail-verdict');
  else if(!p.internal_dependencies)engineeringText(card,t('Too few internal connections to assess a package split.','Zu wenige interne Verbindungen, um eine Paketaufteilung zu bewerten.'),'p','muted');
  if(!selected)return;
  const context=element('div');
  engineeringText(context,p.node,'p','inspector-full-name');
  engineeringText(context,t('Direct files only; nested packages are separate. Isolation refers to connections inside this package.','Nur direkte Dateien; Unterpakete sind separat. Isolation bezieht sich auf Verbindungen innerhalb dieses Pakets.'),'p','muted');
  engineeringText(card,p.co_change_status==='not_checked'?t('Co-change not checked.','Gemeinsame Änderungen nicht geprüft.'):t('{count} cross-group co-change pairs · {status}','{count} gemeinsam geänderte Paare zwischen Gruppen · {status}',{count:p.cross_group_co_changes,status:p.co_change_status==='available'?t('history available','Historie verfügbar'):t('incomplete history','unvollständige Historie')}),'p','muted');
  const measurement=element('details',undefined,'context-note');measurement.appendChild(element('summary',t('Measurements and interpretation','Messwerte und Einordnung')));
  engineeringText(measurement,t('Internal share: {share} · internal / (internal + outgoing)','Interner Anteil: {share} · intern / (intern + ausgehend)',{share:engineeringPercent(p.internal_share==null?null:p.internal_share*100)}));
  engineeringText(measurement,t('{internal} internal · {outgoing} outgoing · {incoming} incoming file dependencies','{internal} interne · {outgoing} ausgehende · {incoming} eingehende Dateiabhängigkeiten',{internal:p.internal_dependencies,outgoing:p.outgoing_dependencies,incoming:p.incoming_dependencies}));
  engineeringText(measurement,p.reason);context.appendChild(measurement);engineeringLocate(context,p.node);
  if(!p.groups.length){card.appendChild(context);return;}
  if(engineeringUi.cohesionGroup===null){engineeringUi.cohesionGroup=p.groups.map((g,index)=>({g,index})).sort((a,b)=>Number(b.g.internal_dependencies>0)-Number(a.g.internal_dependencies>0)||b.g.files.length-a.g.files.length)[0].index;}
  const active=Math.min(engineeringUi.cohesionGroup,p.groups.length-1),workbench=element('div',undefined,'cohesion-workbench'),groups=element('div',undefined,'cohesion-groups'),detail=element('div');workbench.appendChild(groups);workbench.appendChild(detail);card.appendChild(workbench);card.appendChild(context);
  const ordered=p.groups.map((g,index)=>({g,index})).sort((a,b)=>Number(b.g.internal_dependencies>0)-Number(a.g.internal_dependencies>0)||b.g.files.length-a.g.files.length),visibleGroups=ordered.slice(0,engineeringUi.cohesionLimit||20);
  if(!visibleGroups.some(row=>row.index===active))visibleGroups.push(ordered.find(row=>row.index===active));
  for(const {index,g}of visibleGroups){
    const action=button(t('Group {number}','Gruppe {number}',{number:index+1})+' · '+plural(g.files.length,'{count} file','{count} files','{count} Datei','{count} Dateien'),()=>{engineeringUi.cohesionGroup=index;renderEngineeringQuality();document.getElementById('engineering-list').querySelectorAll('button[data-cohesion-group]').find(b=>b.getAttribute('data-cohesion-group')===String(index))?.focus();},'cohesion-group');
    action.setAttribute('data-cohesion-group',String(index));action.setAttribute('aria-pressed',String(index===active));
    action.appendChild(element('span',g.files.slice(0,2).map(f=>f.split('/').pop()).join(', ')+(g.files.length>2?' +'+(g.files.length-2):'')));groups.appendChild(action);
  }
  if(p.groups.length>(engineeringUi.cohesionLimit||20))groups.appendChild(button(t('Show next 20 groups','Nächste 20 Gruppen'),()=>{engineeringUi.cohesionLimit=(engineeringUi.cohesionLimit||20)+20;renderEngineeringQuality();},'btn btn-quiet'));
  const group=p.groups[active],names=new Set(group.files),nodes=data.nodes.filter(n=>names.has(n.name)),ids=new Set(nodes.map(n=>n.id)),edges=data.edges.filter(e=>ids.has(e.from)&&ids.has(e.to));
  renderDetailGraph(detail,{nodes,edges,label:t('Group {number} · original source graph','Gruppe {number} · ursprünglicher Quellgraph',{number:active+1}),onNode:n=>{showTab('graph');revealNode(nodeById(work,n.id)?.name||n.name);}});
  if(nodes.length<group.files.length)engineeringText(detail,t('Some group files are absent from the loaded source graph.','Einige Gruppendateien fehlen im geladenen Quellgraphen.'),'p','muted');
  const files=element('details',undefined,'context-note');files.appendChild(element('summary',t('All files','Alle Dateien')+' · '+group.files.length));detail.appendChild(files);let fileCount=0;
  const fileList=element('div');files.appendChild(fileList);const fileMore=button(t('Show next 20 files','Nächste 20 Dateien'),()=>fillFiles(),'btn btn-quiet');files.appendChild(fileMore);
  function fillFiles(){for(const name of group.files.slice(fileCount,fileCount+20)){engineeringText(fileList,name,'p','inspector-full-name');engineeringLocate(fileList,name);}fileCount+=20;fileMore.hidden=fileCount>=group.files.length;}
  files.addEventListener('toggle',()=>{if(files.open&&!fileCount)fillFiles();});
  const boundaries=element('details',undefined,'context-note');boundaries.appendChild(element('summary',t('External dependencies · {incoming} in / {outgoing} out','Externe Abhängigkeiten · {incoming} ein / {outgoing} aus',{incoming:group.incoming_dependencies,outgoing:group.outgoing_dependencies})));detail.appendChild(boundaries);
  for(const boundary of group.boundaries||[]){engineeringText(boundaries,boundary.from+' → '+boundary.to,'p','inspector-full-name');for(const ref of boundary.evidence||[])engineeringText(boundaries,ref.file+':'+ref.line+' '+ref.raw,'pre');}
  if(group.boundaries_truncated)engineeringText(boundaries,t('Boundary samples truncated by analysis.','Grenzbelege durch Analyse begrenzt.'),'p','muted');
}
function renderHistoryCard(container,p){
  const card=engineeringCard(container,p.node+(p._removed?' · '+t('absent from latest snapshot','im letzten Snapshot nicht vorhanden'):'')),history=engineeringData('quality_history'),delta=history?.comparison?.modules?.find(row=>row.node===p.node);
  engineeringText(card,t('Latest snapshot · {files} files · producer revision: {revision}','Letzter Snapshot · {files} Dateien · Producer-Revision: {revision}',{files:p.files??'—',revision:history?.latest?.producer_revision||'unverified'}),'p','muted');
  engineeringText(card,t('Coverage: {status} · {known} / {total} files','Abdeckung: {status} · {known} / {total} Dateien',{status:engineeringStatus(p.coverage_status||'unavailable'),known:p.coverage_files??'—',total:p.files??'—'}),'p','muted');
  engineeringText(card,t('Compiler analysis: {status} · {measured} / {declared} methods measured','Compileranalyse: {status} · {measured} / {declared} Methoden gemessen',{status:engineeringStatus(p.semantic_status||'unavailable'),measured:p.measured_methods??'—',declared:p.declared_methods??'—'}),'p','muted');
  engineeringText(card,t('Findings: {status}','Befunde: {status}',{status:engineeringStatus(p.findings_status||'unavailable')}),'p','muted');
  renderQualityHistoryChart(card,p.node,history?.series||[]);

  const metrics=[['line_coverage',t('Line coverage','Zeilenabdeckung'),p.line_coverage?.total?100*p.line_coverage.covered/p.line_coverage.total:null],['mean_complexity',t('Mean complexity','Mittlere Komplexität'),p.mean_complexity],['max_complexity',t('Max. complexity','Max. Komplexität'),p.max_complexity],['active_findings',t('Active findings','Aktive Befunde'),p.active_findings]];
  for(const [key,label,value]of metrics){const m=delta?.[key];engineeringText(card,label+': '+(key==='line_coverage'?engineeringPercent(value):engineeringNumber(value))+(m?' · '+engineeringNumber(m.before)+' → '+engineeringNumber(m.after)+' · Δ '+engineeringNumber(m.delta)+' · '+engineeringStatus(m.status):''));if(m?.reason)engineeringText(card,m.reason,'small','muted');}
  if(!delta)engineeringText(card,t('No comparable previous module measurement.','Keine vergleichbare vorherige Modulmessung.'),'p','muted');
  if(p.extended){const extra=element('details');extra.appendChild(element('summary',t('Class, method and package history','Klassen-, Methoden- und Paketverlauf')));for(const [key,m] of Object.entries(p.extended)){const change=delta?.extended?.[key];engineeringText(extra,key+': '+engineeringNumber(m.value)+' · '+m.measured+'/'+m.population+(change?' · Δ '+engineeringNumber(change.delta)+' · '+engineeringStatus(change.status):''));}card.appendChild(extra);}
  engineeringLocate(card,p.node);
}
function renderQualityHistoryChart(container,node,series){
  const points=series.map(s=>({timestamp:s.timestamp,tag:s.tag,metrics:s.modules?.find(m=>m.node===node)?.metrics||{}}));
  const labels={'cognitive.max':t('Max. Cognitive','Max. Cognitive'),'lcom4.mean':'LCOM4 Ø','cbo.mean':'CBO Ø','mean_complexity':t('Mean complexity','Mittlere Komplexität'),'maintainability.mean':'MI Ø','package.cyclic_dependencies':t('Cyclic package dependencies','Zyklische Paketabhängigkeiten'),'git.bus_factor_50':t('Bus factor 50','Bus-Faktor 50'),'line_coverage':t('Line coverage (%)','Zeilenabdeckung (%)')};
  const keys=Object.keys(labels).filter(k=>points.some(p=>Number.isFinite(p.metrics[k])));if(points.length<2||!keys.length)return;
  const box=element('div',undefined,'quality-history-chart'),select=element('select');select.setAttribute('aria-label',t('Historical metric','Historische Kennzahl'));for(const key of keys){const option=element('option',labels[key]);option.value=key;select.appendChild(option);}select.value=keys[0];box.appendChild(select);
  const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('viewBox','0 0 480 110');svg.setAttribute('width','100%');svg.setAttribute('height','110');svg.setAttribute('role','img');svg.setAttribute('data-quality-history',node);box.appendChild(svg);container.appendChild(box);
  const shape=(tag,attrs,text)=>{const n=document.createElementNS('http://www.w3.org/2000/svg',tag);for(const [key,value]of Object.entries(attrs))n.setAttribute(key,String(value));if(text!==undefined)n.textContent=text;svg.appendChild(n);return n;};
  function draw(){svg.textContent='';svg.setAttribute('aria-label',labels[select.value]+' · '+t('Recorded samples','Aufgezeichnete Messpunkte'));const maximum=Math.max(1,...points.map(p=>p.metrics[select.value]).filter(Number.isFinite));shape('line',{x1:35,y1:80,x2:465,y2:80,stroke:'var(--border)'});shape('text',{x:2,y:18,fill:'var(--muted)','font-size':11},engineeringNumber(maximum));shape('text',{x:2,y:80,fill:'var(--muted)','font-size':11},'0');
    points.forEach((point,i)=>{const value=point.metrics[select.value];if(!Number.isFinite(value))return;const dot=shape('circle',{cx:40+i*410/(points.length-1),cy:80-65*value/maximum,r:4,fill:'var(--accent)'}),title=document.createElementNS('http://www.w3.org/2000/svg','title');title.textContent=(point.tag||new Date(point.timestamp*1000).toISOString())+' · '+engineeringNumber(value);dot.appendChild(title);});
    shape('text',{x:35,y:103,fill:'var(--muted)','font-size':11},t('Older','Älter'));shape('text',{x:465,y:103,fill:'var(--muted)','font-size':11,'text-anchor':'end'},t('Newer','Neuer'));
  }select.addEventListener('change',draw);draw();engineeringText(box,t('Recorded samples. Delta labels describe comparability.','Aufgezeichnete Messpunkte. Die Delta-Angaben zeigen die Vergleichbarkeit.'),'small','muted');
}
function renderClassMetricsCard(container,row){
  const card=engineeringCard(container,row.class_id);card.setAttribute('data-class-metrics',row.class_id);engineeringText(card,row.file,'small','path');
  const grid=element('div',undefined,'quality-metric-grid');card.appendChild(grid);
  for(const key of CLASS_METRICS){const value=classMetricValue(row,key),cell=element('div'),label=element('strong',key.toUpperCase()+': '+(row[key]?.status==='not_applicable'?t('n/a','n. a.'):engineeringNumber(value)));label.setAttribute('data-metric-key',key);cell.appendChild(explainMetric(label,key,row[key]?.status==='not_applicable'?t('Not applicable · Go has no class inheritance','Nicht anwendbar · Go hat keine Klassenvererbung'):value===null&&key!=='lcom4'?t('Incomplete · observed {count}','Unvollständig · beobachtet {count}',{count:row[key]?.observed??'—'}):''));grid.appendChild(cell);}
  const actions=element('div',undefined,'class-metric-actions');card.appendChild(actions);if(row.node)engineeringLocate(actions,row.node);
  const calls=button(t('Class calls','Aufrufe der Klasse'),()=>openClassMetricCalls(row),'btn btn-secondary');calls.setAttribute('data-class-calls',row.class_id);calls.disabled=!(callModel.classes?.length);actions.appendChild(calls);
  const help=element('details');help.appendChild(element('summary',t('Dependency evidence','Abhängigkeitsbelege')));
  let filled=false;help.addEventListener('toggle',()=>{if(!help.open||filled)return;filled=true;for(const key of CLASS_METRICS.filter(k=>k!=='lcom4')){engineeringText(help,key.toUpperCase(),'h4');for(const ref of (row[key]?.evidence||[]).slice(0,100))engineeringText(help,ref,'p','path');if((row[key]?.evidence?.length||0)>100)engineeringText(help,t('First 100; export contains all evidence.','Erste 100; der Export enthält alle Belege.'));}});card.appendChild(help);
}
function renderMethodMetricsCard(container,row){
  const card=engineeringCard(container,row.method_id);engineeringText(card,row.file+':'+row.line,'small','path');
  for(const [key,label,value]of [['cognitive','Cognitive',row.cognitive],['maintainability','MI',row.maintainability_index]])card.appendChild(explainMetric(element('p',label+': '+engineeringNumber(value)),key));
  const detail=element('details');detail.appendChild(element('summary',t('Halstead and measurement rules','Halstead und Messregeln')));
  for(const [key,label,value]of [['halsteadVolume',t('Volume','Volumen'),row.halstead?.volume],['halsteadDifficulty',t('Difficulty','Schwierigkeit'),row.halstead?.difficulty],['halsteadEffort',t('Effort','Aufwand'),row.halstead?.effort]])detail.appendChild(explainMetric(element('p',label+': '+engineeringNumber(value)),key));
  engineeringText(detail,t('Source lines: {lines} · source-maintainability/v1. Compare the same language and measurement definition.','Quellzeilen: {lines} · source-maintainability/v1. Nur gleiche Sprache und Messdefinition vergleichen.',{lines:row.source_lines??'—'}));card.appendChild(detail);
}
function renderKnowledgeCard(container,row){
  const card=engineeringCard(container,row.node);engineeringText(card,t('Contribution concentration','Beitragsverteilung'),'small','muted');
  engineeringText(card,t('Bus factor 50: {factor} · largest share {share}','Bus-Faktor 50: {factor} · größter Anteil {share}',{factor:engineeringNumber(row.bus_factor_50),share:engineeringPercent(row.top_share==null?null:row.top_share*100)}));
  const bar=element('div',undefined,'knowledge-bar');bar.setAttribute('role','img');bar.setAttribute('aria-label',t('Largest contribution share','Größter Beitragsanteil')+': '+engineeringPercent(row.top_share==null?null:row.top_share*100));const fill=element('span');fill.style.width=(Number.isFinite(row.top_share)?Math.max(0,Math.min(100,row.top_share*100)):0)+'%';bar.appendChild(fill);card.appendChild(bar);
  engineeringText(card,t('{people} contributors · {touches} file changes','{people} Beitragende · {touches} Dateiänderungen',{people:row.contributors,touches:row.touches}));
  const detail=element('details');detail.appendChild(element('summary',t('Contributors and interpretation','Beitragende und Einordnung')));engineeringText(detail,t('Smallest number of authors responsible for at least 50% of file/commit touches in the selected Git window. A concentration heuristic; no claim about actual expertise or availability. Shallow history leaves the factor unknown.','Kleinste Anzahl von Personen mit mindestens 50% der Datei-/Commit-Berührungen im Git-Zeitfenster. Ein Hinweis auf Konzentration, keine Aussage über tatsächliches Wissen oder Verfügbarkeit. Bei unvollständiger Historie bleibt der Faktor unbekannt.'));for(const person of (row.authors||[]).slice(0,50))engineeringText(detail,person.author+' · '+person.touches);card.appendChild(detail);engineeringLocate(card,row.node);
}
function renderMeasurementCard(container,row){
  const card=engineeringCard(container,row.name);engineeringText(card,({test:t('Repeated test','Wiederholter Test'),build:t('Build duration','Build-Dauer'),runtime:t('Runtime measurement','Laufzeitmessung')})[row.kind]||row.kind,'small','muted');
  if(row.kind==='test')engineeringText(card,row.inconsistent===true?t('Mixed results in the same context','Wechselnde Ergebnisse im selben Kontext'):row.inconsistent===false?t('Consistent observed results','Gleichbleibende beobachtete Ergebnisse'):t('Too few runs to assess stability','Zu wenige Läufe für eine Stabilitätsbewertung'));
  engineeringText(card,'Median: '+engineeringNumber(row.median_seconds)+' s · p95: '+engineeringNumber(row.p95_seconds)+' s · n='+row.duration_samples);
  engineeringText(card,t('{passed} passed · {failed} failed · {skipped} skipped','{passed} erfolgreich · {failed} fehlgeschlagen · {skipped} übersprungen',row));
  const detail=element('details');detail.appendChild(element('summary',t('Measurement context','Messkontext')));for(const value of [row.revision,row.environment,row.producer])engineeringText(detail,value,'p','path');engineeringText(detail,t('Producer-declared context, unverified. Durations use successful runs. Different revisions and environments remain separate; mixed results indicate instability, not its cause.','Vom Messwerkzeug angegebener, unverifizierter Kontext. Zeitwerte verwenden erfolgreiche Läufe. Revisionen und Umgebungen bleiben getrennt; wechselnde Ergebnisse zeigen Instabilität, nicht deren Ursache.'));card.appendChild(detail);
}
function renderTestEvidenceCard(container,row){
  const test=row.test,card=engineeringCard(container,test.suite+' · '+test.name);
  engineeringText(card,test.outcome+' · '+t('Duration: {duration} s','Dauer: {duration} s',{duration:engineeringNumber(test.duration_seconds)}));
  engineeringText(card,row.matched.length?t('Conservative candidate: mapped code is affected by the selected dependency change.','Konservativer Kandidat: zugeordneter Code ist von der gewählten Abhängigkeitsänderung betroffen.'):row.unknown?t('Code mapping unknown or stale; no safe exclusion is possible.','Codezuordnung unbekannt oder veraltet; kein sicherer Ausschluss möglich.'):t('Explicit test-to-code mapping.','Explizite Test-zu-Code-Zuordnung.'),'p','muted');
  for(const mapping of test.mappings.slice(0,20)){engineeringText(card,mapping.path+(mapping.lines.length?':'+mapping.lines.slice(0,10).join(','):'')+' · '+mapping.status,'p','path');engineeringLocate(card,mapping.node);}
  if(test.mappings.length>20)engineeringText(card,t('Additional mappings remain in the export.','Weitere Zuordnungen stehen im Export.'),'p','muted');
  if(test.message)engineeringText(card,test.message,'pre');
}
function renderMutationCard(container,row){
  const {file,mutant,incoming}=row,card=engineeringCard(container,file.path+':'+mutant.location.start.line+' · '+mutant.status);
  engineeringText(card,mutant.mutator_name+' · '+t('{count} distinct incoming files','{count} unterschiedliche eingehende Dateien',{count:incoming}));
  engineeringText(card,file.status,'p','muted');engineeringText(card,(mutant.original||'')+' → '+(mutant.replacement||''),'pre');
  if(mutant.original_truncated)engineeringText(card,t('Original source excerpt shortened.','Ursprünglicher Quellausschnitt gekürzt.'),'small','muted');
  if(mutant.description)engineeringText(card,mutant.description);
  engineeringLocate(card,file.node);
}
function renderDuplicateCard(container,group){
  const card=engineeringCard(container,t('Duplicate block · {tokens} tokens','Duplikatblock · {tokens} Tokens',{tokens:group.tokens}));
  engineeringText(card,engineeringStatus(group.status)+' · '+t('{lines} reported lines · {count} locations','{lines} gemeldete Zeilen · {count} Fundstellen',{lines:group.lines,count:group.occurrences.length}));
  const locations=element('div');card.appendChild(locations);let shown=0;
  const more=button(t('Show next 20 locations','Nächste 20 Fundstellen'),()=>fill(),'btn btn-secondary');card.appendChild(more);
  function fill(){for(const o of group.occurrences.slice(shown,shown+20)){
    engineeringText(locations,o.path+':'+o.line+'–'+o.end_line+' · '+engineeringStatus(o.status),'p','path');engineeringLocate(locations,o.node);
  }shown+=20;more.hidden=shown>=group.occurrences.length;}fill();
  if(group.snippet)engineeringText(card,group.snippet,'pre');
  if(group.snippet_truncated)engineeringText(card,t('Producer excerpt shortened to 4096 characters during import; use the original CPD report for the full fragment.','Producer-Ausschnitt beim Import auf 4096 Zeichen gekürzt; der vollständige Ausschnitt steht im ursprünglichen CPD-Bericht.'),'small','muted');
}
function renderEngineeringTargets(){
  const target=document.getElementById('engineering-target');target.textContent='';const all=element('option',t('Entire source scope','Gesamter Quellumfang'));all.value='';target.appendChild(all);
  const query=engineeringUi.targetQuery.trim().toLowerCase(),shown=[];let matching=0;
  for(const node of data.nodes)if(!query||node.name.toLowerCase().includes(query)){matching++;if(shown.length<2000)shown.push(node);}
  const listed=shown.length,selected=data.nodes.find(n=>n.name===engineeringUi.target),retained=selected&&!shown.includes(selected);if(retained)shown.push(selected);
  for(const node of shown){const option=element('option',node.name);option.value=node.name;target.appendChild(option);}target.value=engineeringUi.target;
  const scope=document.getElementById('engineering-scope-summary');
  scope.textContent=engineeringUi.target?t('Scope: {path}','Umfang: {path}',{path:engineeringUi.target}):t('All packages','Alle Pakete');
  const note=document.getElementById('engineering-target-note');note.hidden=matching<=2000&&!query&&!retained;
  note.textContent=t('{shown} / {total} matching targets.','{shown} / {total} passende Ziele.',{shown:listed,total:matching})+(matching>2000?' '+t('Only the first 2000 are listed. Refine the package or file search to find other targets.','Nur die ersten 2000 werden aufgelistet. Die Paket- oder Dateisuche eingrenzen, um andere Ziele zu finden.'):'')+(retained?' '+t('The current selection is also retained.','Die aktuelle Auswahl bleibt zusätzlich erhalten.'):'');
}
function renderEngineeringQuality(){
  if(ui.workspace!=='quality')return;
  if(deferredSection('code_metrics')){const host=document.getElementById('engineering-list');host.textContent='';awaitDeferred(host,['code_metrics'],renderEngineeringQuality);return;}
  if(!engineeringUi.section)engineeringUi.section=engineeringData('code_metrics')?.classes?.length?'heatmap':'cohesion';
  const keys={heatmap:'code_metrics',classes:'code_metrics',methods:'code_metrics',knowledge:'knowledge_metrics',measurements:'measurement_runs',cohesion:'cohesion',history:'quality_history',tests:'test_evidence',mutations:'mutations',duplicates:'duplicates'},source=engineeringData(keys[engineeringUi.section]),rows=engineeringUi.section==='heatmap'?classHeatmapRows(engineeringRows()):engineeringRows();
  document.getElementById('engineering-heading').textContent=t('Software quality','Softwarequalität');
  renderEngineeringTargets();
  document.getElementById('engineering-section').value=engineeringUi.section;
  const computed=['heatmap','classes','methods','knowledge'].includes(engineeringUi.section),statusText=computed?({ready:t('measured','gemessen'),partial:t('partial analysis','teilweise analysiert'),unavailable:t('unavailable','nicht verfügbar'),not_applicable:t('not applicable','nicht anwendbar')})[source?.status]||t('unavailable','nicht verfügbar'):engineeringStatus(source?.status);
  document.getElementById('engineering-status').textContent=statusText+' · '+t('{shown} / {total} matching records','{shown} / {total} passende Einträge',{shown:Math.min(rows.length,engineeringUi.limit),total:rows.length});
  const note=document.getElementById('engineering-note');note.textContent='';
  engineeringText(note,t('Original analysis and imported measurements. Graph simulations do not rewrite these results.','Ursprüngliche Analyse und importierte Messungen. Graphsimulationen verändern diese Ergebnisse nicht.'),'p','muted');
  const descriptions={
    heatmap:t('Classes × metrics. Colors compare measured values; gray retains unknowns.','Klassen × Kennzahlen. Farben vergleichen gemessene Werte; Grau erhält unbekannte Werte.'),
    classes:t('Class coupling, response set, inheritance and complexity. Unknown measurements remain —.','Klassenkopplung, Aufrufumfang, Vererbung und Komplexität. Unbekannte Messungen bleiben —.'),
    methods:t('Method maintainability, ordered by Cognitive source complexity.','Wartbarkeit pro Methode, sortiert nach Cognitive-Quellcodekomplexität.'),
    knowledge:t('Contribution concentration in the configured Git history window.','Beitragsverteilung im konfigurierten Git-Zeitfenster.'),
    measurements:t('Recorded build, runtime and repeated-test runs. Project-wide observations are independent of the source selection.','Aufgezeichnete Build-, Laufzeit- und wiederholte Testläufe. Projektweite Messungen sind unabhängig von der Quellauswahl.'),
    cohesion:t('Immediate files in each package. Internal share is structural, not proof of good domain boundaries; disconnected groups are review candidates.','Direkte Dateien je Paket. Der interne Anteil ist strukturell und belegt keine guten fachlichen Grenzen; getrennte Gruppen sind Prüfkandidaten.'),
    history:t('Latest two recorded snapshots. Incompatible measurements have no delta; imported producer revisions remain unverified.','Die letzten zwei aufgezeichneten Snapshots. Unvergleichbare Messungen erhalten kein Delta; importierte Producer-Revisionen bleiben unverifiziert.'),
    tests:t('Matching source associations first, then measured duration. Selecting a file includes tests mapped to its transitive dependents. Unknown or incomplete mappings mean test omission is not safe.','Passende Quellzuordnungen zuerst, danach gemessene Dauer. Eine Dateiauswahl enthält Tests ihrer transitiven Aufrufer. Unbekannte oder unvollständige Zuordnungen erlauben kein sicheres Weglassen von Tests.'),
    mutations:t('Survived and uncovered mutations first, then incoming-file count. Killed ratio = killed / (killed + survived + no coverage); incomplete evidence has no score. This is not ordinary line coverage.','Überlebende und unabgedeckte Mutationen zuerst, danach eingehende Dateien. Erkennungsquote = erkannt / (erkannt + überlebt + keine Abdeckung); unvollständige Belege erhalten keinen Wert. Dies ist keine normale Zeilenabdeckung.'),
    duplicates:t('CPD token matches, largest first. Inspect all locations before extracting shared code. Source hashes detect changes after import; producer revision, scan scope and token normalization remain unverified. No project-wide duplication percentage is inferred.','CPD-Tokenübereinstimmungen, größte zuerst. Vor einer gemeinsamen Abstraktion alle Fundstellen prüfen. Quellhashes erkennen Änderungen nach dem Import; Producer-Revision, Scanumfang und Tokennormalisierung bleiben unverifiziert. Es wird kein projektweiter Duplikatanteil abgeleitet.')
  };
  engineeringText(note,descriptions[engineeringUi.section]);
  const diagnostics=element('details');diagnostics.appendChild(element('summary',t('Measurement coverage and definitions','Messabdeckung und Definitionen')));for(const diagnostic of (source?.diagnostics||[]).slice(0,20))engineeringText(diagnostics,diagnostic,'p','muted');for(const action of source?.coverage_actions||[])engineeringText(diagnostics,action);if(source?.diagnostics?.length||source?.coverage_actions?.length)note.appendChild(diagnostics);
  const commands={cohesion:'archguard cohesion . --format json',history:'archguard quality-history . --record --tag after-refactoring',tests:'archguard test-evidence . --report junit.xml --mapping test-map.json',mutations:'archguard mutations . --report mutation-report.json',duplicates:'archguard duplicates . --report cpd.xml'};
  Object.assign(commands,{heatmap:'archguard metrics . --extended --format json',classes:'archguard metrics . --extended --format json',methods:'archguard metrics . --extended --format json',knowledge:'archguard metrics . --extended --format json',measurements:'archguard metrics . --extended --report runs.json'});
  if(!source||['not_imported','not_recorded'].includes(source.status)||engineeringUi.section==='history')engineeringText(note,commands[engineeringUi.section],'pre');
  if(engineeringUi.section==='history'){
    engineeringText(note,t('{count} recent snapshots shown; full measurements are stored in .archguard/quality-history.','{count} letzte Snapshots angezeigt; vollständige Messungen liegen in .archguard/quality-history.',{count:source?.records?.length||0}),'p','muted');
    const history=element('details');history.appendChild(element('summary',t('Snapshot provenance','Herkunft der Snapshots')));if(source?.latest){engineeringText(history,'Source SHA256: '+source.latest.source_sha256+' · Config SHA256: '+source.latest.config_sha256,'p','path');for(const analyzer of source.latest.analyzers||[])engineeringText(history,analyzer.language+' · '+analyzer.backend+' · '+(analyzer.version||t('version unknown','Version unbekannt'))+' · '+(analyzer.project||''),'p','path');}for(const r of source?.records||[])engineeringText(history,(r.tag||'')+' · '+(r.git_commit||'no Git commit')+' · '+r.path+' · '+r.source_sha256,'p','path');note.appendChild(history);
  }
  if(engineeringUi.section==='mutations')engineeringText(note,t('Detected mutation ratio: {value}','Erkennungsquote der Mutationen: {value}',{value:engineeringPercent(source?.summary?.killed_percent)}));
  if(engineeringUi.section==='measurements')engineeringText(note,t('Mixed test groups: {percent}','Testgruppen mit wechselnden Ergebnissen: {percent}',{percent:engineeringPercent(source?.flaky_test_percent)}));
  if(engineeringUi.section==='duplicates'&&source)engineeringText(note,'CPD · '+(source.producer_version||t('version unknown','Version unbekannt'))+' · '+t('{count} reported scan errors','{count} gemeldete Scanfehler',{count:source.producer_errors||0}),'p','muted');
  const list=document.getElementById('engineering-list');hideMetricTooltip(list);list.textContent='';list.classList.toggle('cohesion-list',engineeringUi.section==='cohesion');
  if(engineeringUi.section==='cohesion'&&!rows.some(p=>p.node===engineeringUi.cohesionPackage)){engineeringUi.cohesionPackage=rows[0]?.node||null;engineeringUi.cohesionGroup=null;engineeringUi.cohesionLimit=20;}
  const render={classes:renderClassMetricsCard,methods:renderMethodMetricsCard,knowledge:renderKnowledgeCard,measurements:renderMeasurementCard,cohesion:renderCohesionCard,history:renderHistoryCard,tests:renderTestEvidenceCard,mutations:renderMutationCard,duplicates:renderDuplicateCard}[engineeringUi.section];
  if(engineeringUi.section==='heatmap')renderClassHeatmap(list,rows);
  else for(const row of rows.slice(0,engineeringUi.limit))render(list,row);
  if(!rows.length)engineeringText(list,t('No matching measurements. Missing data is not evidence of good quality.','Keine passenden Messungen. Fehlende Daten belegen keine gute Qualität.'),'p','empty');
  document.getElementById('engineering-more').hidden=engineeringUi.limit>=rows.length;
}
function renderApiContracts(){
  const source=engineeringData('contracts'),list=document.getElementById('contracts-list');list.textContent='';
  document.getElementById('contracts-status').textContent=engineeringStatus(source?.status)+' · '+t('Compared with the previous explicit import; declared usage is not a runtime trace.','Vergleich mit dem vorherigen expliziten Import; deklarierte Nutzung ist kein Laufzeit-Trace.');
  for(const c of (source?.contracts||[]).slice(0,engineeringUi.contractsLimit)){
    const card=engineeringCard(list,c.provider);
    engineeringText(card,c.source_path+' · '+c.source_status+' · '+engineeringStatus(c.comparison_status),'p','path');
    engineeringText(card,t('{count} declared HTTP operations','{count} deklarierte HTTP-Operationen',{count:c.operations.length}));
    const inventory=element('details');inventory.appendChild(element('summary',t('Endpoints, schemas and declared usage','Endpunkte, Schemas und deklarierte Nutzung')));card.appendChild(inventory);
    const operations=element('div');inventory.appendChild(operations);let opCount=0;
    const opMore=button(t('Show next 20 operations','Nächste 20 Operationen'),()=>fillOperations(),'btn btn-secondary');inventory.appendChild(opMore);
    function fillOperations(){for(const op of c.operations.slice(opCount,opCount+20)){
      engineeringText(operations,op.id+(op.operation_id?' · '+op.operation_id:''),'h4');if(op.pointer)engineeringText(operations,c.source_path+'#'+op.pointer,'p','path');
      const refs=[...(op.request_references||[]),...(op.response_references||[])];if(refs.length)engineeringText(operations,t('Referenced schemas: ','Referenzierte Schemas: ')+[...new Set(refs)].join(', '),'p','path');
      engineeringText(operations,t('Declared consumers: ','Deklarierte Konsumenten: ')+(c.usages.filter(u=>u.operation===op.id||u.operation==='*').map(u=>u.consumer+(u.mapped_service?'':' ('+t('unmapped','nicht zugeordnet')+')')).join(', ')||t('unknown','unbekannt')));
    }opCount+=20;opMore.hidden=opCount>=c.operations.length;}fillOperations();

    engineeringText(card,t('Configured callers (operation usage unverified): ','Konfigurierte Aufrufer (Operationsnutzung unverifiziert): ')+(c.configured_callers.join(', ')||'—'),'p','muted');
    engineeringText(card,t('Usage mapping: ','Nutzungszuordnung: ')+c.usage_status,'p','muted');
    if(!c.mapped_service)engineeringText(card,t('Provider is not mapped to the current service inventory.','Provider ist im aktuellen Dienstinventar nicht zugeordnet.'),'p','muted');
    let shown=0;const changes=element('div');card.appendChild(changes);
    const more=button(t('Show next 20 changes','Nächste 20 Änderungen'),()=>fill(),'btn btn-secondary');card.appendChild(more);
    function fill(){for(const change of c.changes.slice(shown,shown+20)){engineeringText(changes,change.operation+' · '+change.severity+' · '+change.kind,'h4');engineeringText(changes,change.message);engineeringText(changes,t('Declared consumers: ','Deklarierte Konsumenten: ')+(change.consumers.join(', ')||t('unknown','unbekannt')));}shown+=20;more.hidden=shown>=c.changes.length;}fill();
    if(!c.changes.length)engineeringText(card,c.comparison_status==='no_baseline'?t('Record another import to compare changes.','Für einen Änderungsvergleich erneut importieren.'):t('No changes in the compared contract projection; this is not a complete compatibility proof.','Keine Änderungen in der verglichenen Vertragsprojektion; dies ist kein vollständiger Kompatibilitätsnachweis.'));
    for(const d of c.diagnostics||[])engineeringText(card,d,'p','muted');
  }
  if(!source?.contracts?.length)engineeringText(list,'archguard contracts . --report openapi.yaml --service SERVICE_ID --usage api-usage.json','pre');
  for(const note of (source?.limitations||[]).concat(source?.diagnostics||[]).slice(0,20))engineeringText(list,note,'p','muted');
  document.getElementById('contracts-more').hidden=engineeringUi.contractsLimit>=(source?.contracts?.length||0);
}
function engineeringExport(base){return {...base,kind:'quality',coverage:'Original source measurements; all loaded records, independent of filters and simulation.',filters:{...base.filters,section:engineeringUi.section,target:engineeringUi.target,query:engineeringUi.query},...Object.fromEntries(['code_metrics','knowledge_metrics','measurement_runs','cohesion','quality_history','test_evidence','mutations','duplicates'].map(key=>[key,engineeringData(key)]))};}
function engineeringMarkdown(snapshot){
  const cell=exportMarkdownCell,lines=['# Software quality: '+cell(snapshot.project),'','Original analysis and imported evidence. Graph simulations do not change measurements.'];
  if(snapshot.coverage)lines.push('',cell(snapshot.coverage));
  function section(key,title,missing='not_imported'){
    const source=snapshot[key];lines.push('','## '+title,'','Status: '+cell(source?.status||missing));
    for(const [key,label]of [['report_path','Report'],['mapping_path','Source mapping'],['imported_at_unix','Imported at (Unix seconds)']])if(source?.[key]!=null)lines.push(label+': '+cell(source[key]));
    for(const note of source?.diagnostics||[])lines.push('',cell(note));return source;
  }
  const code=section('code_metrics','Class and method metrics','unavailable');
  for(const row of code?.classes||[])lines.push('',cell(row.class_id)+' · '+cell(row.file),'CBO / RFC / DIT / NOC / WMC / LCOM4: '+[row.cbo?.value,row.rfc?.value,row.dit?.value,row.noc?.value,row.wmc?.value,row.lcom4].map(engineeringNumber).join(' / '));
  for(const row of code?.methods||[])lines.push('',cell(row.method_id)+' · '+cell(row.file+':'+row.line),'Cognitive: '+engineeringNumber(row.cognitive)+' · MI: '+engineeringNumber(row.maintainability_index)+' · Halstead volume / difficulty / effort: '+[row.halstead?.volume,row.halstead?.difficulty,row.halstead?.effort].map(engineeringNumber).join(' / '));
  const knowledge=section('knowledge_metrics','Git contribution concentration','unavailable');
  for(const row of knowledge?.files||[])lines.push('',cell(row.node)+' · Bus factor 50: '+engineeringNumber(row.bus_factor_50)+' · largest share: '+engineeringPercent(row.top_share==null?null:row.top_share*100)+' · '+row.touches+' file/commit touches');
  const measurements=section('measurement_runs','Build, runtime and repeated tests');
  for(const row of measurements?.groups||[])lines.push('',[row.kind,row.name,row.revision,row.environment,row.producer].map(cell).join(' · '),'Median / p95: '+engineeringNumber(row.median_seconds)+' / '+engineeringNumber(row.p95_seconds)+' s · samples: '+row.duration_samples+' · mixed test outcomes: '+cell(row.inconsistent??'unknown'));
  const cohesion=section('cohesion','Package cohesion');
  for(const p of cohesion?.packages||[])lines.push('','### Package '+cell(p.node),'',cell(p.reason),'','Internal share: '+engineeringPercent(p.internal_share==null?null:p.internal_share*100)+'.',...p.groups.map(g=>'- '+g.files.map(cell).join(', ')));
  const history=section('quality_history','Quality history','not_recorded');
  if(history){lines.push('','Producer revision: '+cell(history.latest?.producer_revision||'unverified')+'. Deltas retain comparability status.');
    for(const [key,label]of [['source_sha256','Source SHA256'],['config_sha256','Config SHA256'],['timestamp','Recorded at (Unix seconds)']])if(history.latest?.[key]!=null)lines.push(label+': '+cell(history.latest[key]));
    for(const analyzer of history.latest?.analyzers||[])lines.push('Analyzer: '+[analyzer.language,analyzer.backend,analyzer.version||'version unknown',analyzer.project||''].map(cell).join(' · '));
    for(const p of history.latest?.modules||[])lines.push('',cell(p.node),'Coverage: '+cell(p.coverage_status)+' · '+engineeringPercent(p.line_coverage?.total?100*p.line_coverage.covered/p.line_coverage.total:null)+' · '+cell(p.coverage_files)+' / '+cell(p.files)+' files','Methods: '+cell(p.semantic_status)+' · '+cell(p.measured_methods)+' / '+cell(p.declared_methods)+' · complexity mean/max '+engineeringNumber(p.mean_complexity)+' / '+engineeringNumber(p.max_complexity),'Findings: '+cell(p.findings_status)+' · '+engineeringNumber(p.active_findings));
    for(const m of history.comparison?.modules||[])lines.push('',cell(m.node),...['line_coverage','mean_complexity','max_complexity','active_findings'].map(k=>cell(k)+': '+cell(JSON.stringify(m[k]))),...Object.entries(m.extended||{}).map(([k,v])=>cell(k)+': '+cell(JSON.stringify(v))));
  }
  const tests=section('test_evidence','Test evidence');
  for(const test of tests?.tests||[])lines.push('',cell(test.id)+' · '+cell(test.name)+' · '+cell(test.outcome)+' · '+engineeringNumber(test.duration_seconds)+' s',...test.mappings.map(m=>cell(m.path)+' · '+cell(m.status)));
  const mutations=section('mutations','Mutation evidence');
  if(mutations){
    lines.push('','Producer: '+cell(mutations.producer||'unknown')+' · '+cell(mutations.producer_version||'version unknown'),'Killed ratio: '+engineeringPercent(mutations.summary?.killed_percent)+'.');
    for(const file of mutations.files||[])for(const mutant of file.mutants)lines.push('',cell(file.path+':'+mutant.location.start.line)+' · '+cell(file.status)+' · '+cell(mutant.status)+' · '+cell(mutant.mutator_name),cell(mutant.original)+' → '+cell(mutant.replacement));
  }
  const duplicates=section('duplicates','Code duplication');
  if(duplicates){
    lines.push('','CPD: '+cell(duplicates.producer_version||'version unknown')+' · '+cell(duplicates.producer_timestamp||'timestamp unknown'),'Producer revision: unverified · reported scan errors: '+cell(duplicates.producer_errors));
    for(const group of duplicates.groups||[]){
      lines.push('','### Duplicate block '+cell(group.id),'',cell(group.tokens)+' tokens · '+cell(group.lines)+' reported lines · '+cell(group.status));
      for(const o of group.occurrences)lines.push('- '+cell(o.path+':'+o.line+'–'+o.end_line)+' · '+cell(o.status)+' · '+cell(o.source_sha256||'hash unavailable'));
      if(group.snippet)lines.push('',cell(group.snippet));
      if(group.snippet_truncated)lines.push('Producer excerpt shortened during import; full fragment remains in the original CPD report.');
    }
  }
  return lines.join('\n')+'\n';
}
function appendContractsMarkdown(lines,snapshot){
  if(!snapshot.contracts)return;const cell=exportMarkdownCell;lines.push('','## API contracts','',cell(snapshot.contracts.status));
  for(const c of snapshot.contracts.contracts||[]){
    lines.push('',cell(c.provider)+' · '+cell(c.source_path)+' · '+cell(c.source_status)+' · '+cell(c.comparison_status));
    lines.push('Provider mapping: '+(c.mapped_service?'mapped':'unmapped'),'Usage mapping: '+cell(c.usage_status),'Configured callers (operation usage unverified): '+(c.configured_callers||[]).map(cell).join(', '));
    for(const [key,label]of [['source_sha256','Source SHA256'],['usage_path','Usage file'],['usage_sha256','Usage SHA256'],['imported_at_unix','Imported at (Unix seconds)'],['previous_imported_at_unix','Previous import (Unix seconds)']])if(c[key]!=null)lines.push(label+': '+cell(c[key]));
    for(const op of c.operations)lines.push('',cell(op.id)+' · '+cell(op.pointer),'Schemas: '+[...(op.request_references||[]),...(op.response_references||[])].map(cell).join(', '));
    for(const change of c.changes)lines.push('',cell(change.operation)+' · '+cell(change.kind)+' · '+cell(change.message),'Declared consumers: '+change.consumers.map(cell).join(', '));
    for(const usage of c.usages)lines.push('',cell(usage.consumer)+' → '+cell(usage.operation)+' · '+(usage.mapped_service?'mapped':'unmapped'));
    for(const note of c.diagnostics||[])lines.push('',cell(note));
  }for(const note of [...(snapshot.contracts.limitations||[]),...(snapshot.contracts.diagnostics||[])])lines.push('',cell(note));
}
for(const field of ['section','target','query'])document.getElementById('engineering-'+field).addEventListener(field==='query'?'input':'change',event=>{engineeringUi[field]=event.target.value;engineeringUi.limit=20;renderEngineeringQuality();});
document.getElementById('engineering-target-query').addEventListener('input',event=>{engineeringUi.targetQuery=event.target.value;renderEngineeringTargets();});
document.getElementById('engineering-more').addEventListener('click',()=>{engineeringUi.limit+=20;renderEngineeringQuality();});
document.getElementById('contracts-more').addEventListener('click',()=>{engineeringUi.contractsLimit+=20;renderApiContracts();});
