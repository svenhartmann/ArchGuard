'use strict';
// Imported findings are original source evidence. The graph only locates them;
// virtual changes never fix findings or rewrite their paths and source hashes.
const qualityUi={source:null,work:null,rows:[],bySource:new Map(),counts:new Map(),parents:new Map(),query:'',level:'all',limit:20,inspectorLimit:20,inspectorSource:null};
function qualityModel() {
  const source=data.insights?.quality;
  if(qualityUi.source!==source){
    qualityUi.source=source;qualityUi.work=null;qualityUi.limit=20;
    const ids=source?.findings?.length?new Map(data.nodes.map(node=>[node.name,node.id])):new Map();
    qualityUi.rows=(source?.findings||[]).map(finding=>({finding,sourceId:ids.get(finding.node)}));
    qualityUi.rows.sort((a,b)=>Number(qualityActive(b.finding))-Number(qualityActive(a.finding))||
      ({error:0,warning:1,note:2,none:3}[a.finding.level]??4)-({error:0,warning:1,note:2,none:3}[b.finding.level]??4)||compareNames(a.finding.id,b.finding.id));
    qualityUi.bySource=new Map();
    for(const row of qualityUi.rows){if(row.sourceId==null)continue;if(!qualityUi.bySource.has(row.sourceId))qualityUi.bySource.set(row.sourceId,[]);qualityUi.bySource.get(row.sourceId).push(row);}
  }
  if(qualityUi.work!==work){
    qualityUi.work=work;qualityUi.counts=new Map();
    const nodes=qualityUi.rows.length?new Map(work.nodes.map(node=>[node.id,node])):new Map();qualityUi.parents=new Map([...nodes.values()].map(node=>[node.id,node.parent]));
    for(const [sourceId,rows] of qualityUi.bySource){
      const count=rows.filter(row=>qualityActive(row.finding)).length;if(!count)continue;
      let node=nodes.get(sourceId);const seen=new Set();
      while(node&&!seen.has(node.id)){seen.add(node.id);qualityUi.counts.set(node.id,(qualityUi.counts.get(node.id)||0)+count);node=nodes.get(node.parent);}
    }
  }
  return source;
}
function qualityActive(finding) {
  return ['ready','partial','stale'].includes(qualityUi.source?.status)&&finding.node!=null&&finding.status==='unchanged_since_import'&&!finding.suppressed&&finding.baseline_state!=='absent'&&finding.level!=='none';
}
function qualityNodeCount(sourceId){qualityModel();return qualityUi.counts.get(sourceId)||0;}
function qualityBadgeText(count){return 'Q'+(count>999?'999+':count);}
function qualityBadgeDescription(count){return t('{count} active imported findings in this file or subtree. Open the Inspector for source evidence. Suppressed, resolved, unmapped and changed-since-import findings are excluded. Simulation does not resolve source findings.','{count} aktive importierte Befunde in dieser Datei oder im Unterbaum. Quellbelege stehen im Inspektor. Unterdrückte, aufgelöste, nicht zugeordnete und seit dem Import geänderte Befunde sind ausgenommen. Simulationen beheben keine Quellcodebefunde.',{count});}
function qualityFindingState(finding){
  const freshness={unchanged_since_import:t('source unchanged since import','Quelle seit Import unverändert'),changed_since_import:t('source changed since import · historical finding','Quelle seit Import geändert · historischer Befund'),unmapped:t('unmapped source','Quelle nicht zugeordnet')}[finding.status]||t('source freshness unknown','Quellaktualität unbekannt');
  return freshness+(finding.baseline_state==='absent'?' · '+t('absent in producer baseline comparison · historical','im Basisvergleich des Tools nicht mehr vorhanden · historisch'):'')+(finding.suppressed?' · '+t('suppressed','unterdrückt'):'');
}
function openQualityReview(){document.getElementById('review-open').click();document.getElementById('quality-review').open=true;document.getElementById('quality-heading').focus();}
function appendQualitySummary(container){
  const source=qualityModel();if(!source)return;
  const action=button(t('Imported quality: {count} findings · {status}','Importierte Qualität: {count} Befunde · {status}',{count:source.findings?.length||0,status:coverageStatusText(source.status)}),openQualityReview,'btn btn-secondary');
  action.setAttribute('data-quality-summary','true');container.appendChild(action);
}
function appendQualityFinding(container,row,navigation=true){
  const f=row.finding,card=element('article',undefined,'evidence');card.setAttribute('data-quality-finding',f.id);
  const title=f.tool+' · '+f.rule_id+' · '+qualityLevelLabel(f.level);
  card.appendChild(element('strong',title.length>512?title.slice(0,512)+'…':title));
  card.appendChild(element('p',f.message.length>4096?f.message.slice(0,4096)+'…':f.message));
  if(title.length>512||f.message.length>4096)card.appendChild(element('small',t('Long text shortened here; full evidence is included in JSON and Markdown export.','Langer Text hier gekürzt; vollständige Belege sind im JSON- und Markdown-Export enthalten.'),'muted'));
  const location=(f.path||t('No local source location','Keine lokale Quellposition'))+(f.line?':'+f.line:'')+(f.column?':'+f.column:'');
  card.appendChild(element('p',location,'path'));card.appendChild(element('small',qualityFindingState(f),'muted'));
  if(f.help_uri){try{const url=new URL(f.help_uri);if(['https:','http:'].includes(url.protocol)&&!url.username&&!url.password){const link=element('a',t('Rule documentation','Regeldokumentation'));link.setAttribute('href',url.href);link.setAttribute('target','_blank');link.setAttribute('rel','noopener noreferrer');card.appendChild(link);}}catch(_){/* Untrusted producer URLs stay inert. */}}
  const current=row.sourceId==null?null:nodeById(work,row.sourceId);
  if(navigation&&current){const action=button(t('Locate in graph','Im Graphen anzeigen'),()=>{showTab('graph');revealNode(current.name);},'btn btn-secondary');action.setAttribute('data-quality-node',f.node);card.appendChild(action);}
  container.appendChild(card);
}
function qualityLevelLabel(level){return {error:t('error','Fehler'),warning:t('warning','Warnung'),note:t('note','Hinweis'),none:t('none','keine')}[level]||String(level);}
function renderQualityFindings(){
  const source=qualityModel(),rows=qualityUi.rows.filter(({finding:f})=>(qualityUi.level==='all'||f.level===qualityUi.level)&&
    (!qualityUi.query||[f.tool,f.rule_id,f.message,f.path,f.node].join(' ').toLowerCase().includes(qualityUi.query.toLowerCase())));
  const active=qualityUi.rows.filter(row=>qualityActive(row.finding)).length;
  document.getElementById('quality-status').textContent=coverageStatusText(source?.status||'not_imported')+' · '+
    t('{active} active imported findings · {shown} / {total} matching records','{active} aktive importierte Befunde · {shown} / {total} passende Einträge',{active,shown:Math.min(rows.length,qualityUi.limit),total:rows.length});
  const explanation=document.getElementById('quality-explanation');explanation.textContent='';
  explanation.appendChild(element('p',t('Original SARIF snapshot. Report/source revision at import is unverified; source hashes detect later file changes. Missing, partial or stale reports do not establish absence of defects. Q badges count fresh, mapped, unsuppressed findings; imported severities are preserved.','Ursprünglicher SARIF-Snapshot. Die Quellrevision des Berichts ist beim Import nicht verifiziert; Datei-Hashes erkennen spätere Änderungen. Fehlende, unvollständige oder veraltete Berichte belegen keine Fehlerfreiheit. Q-Marken zählen unveränderte, zugeordnete, nicht unterdrückte Befunde; die importierten Schweregrade bleiben erhalten.')));
  if(source?.report_path)explanation.appendChild(element('p',source.report_path,'path'));
  if(source?.imported_at_unix!=null){const date=new Date(source.imported_at_unix*1000);explanation.appendChild(element('p',t('Imported: ','Importiert: ')+(Number.isFinite(date.getTime())?date.toISOString():t('unknown','unbekannt'))));}
  for(const note of source?.diagnostics||[])explanation.appendChild(element('p',note,'muted'));
  explanation.appendChild(element('pre','archguard quality . --report results.sarif'));
  const list=document.getElementById('quality-list');list.textContent='';
  for(const row of rows.slice(0,qualityUi.limit))appendQualityFinding(list,row);
  if(!rows.length)list.appendChild(element('p',t('No imported findings match these filters.','Keine importierten Befunde passen zu diesen Filtern.'),'empty'));
  document.getElementById('quality-more').hidden=qualityUi.limit>=rows.length;
}
function renderQualityInspector(container,sourceId,graph=work,refresh=renderSidebar){
  const source=qualityModel();if(!source)return;
  const selected=nodeById(graph,sourceId);if(!selected)return;
  const parents=graph===work?qualityUi.parents:new Map(graph.nodes.map(node=>[node.id,node.parent]));
  const rows=qualityUi.rows.filter(row=>{
    let id=row.sourceId;const seen=new Set();
    while(id!=null&&!seen.has(id)){if(id===sourceId)return true;seen.add(id);id=parents.get(id);}
    return false;
  });
  if(qualityUi.inspectorSource!==sourceId){qualityUi.inspectorSource=sourceId;qualityUi.inspectorLimit=20;}
  const section=element('details',undefined,'review-details inspector-section');section.setAttribute('data-quality-inspector','true');section.open=rows.length>0;
  section.appendChild(element('summary',t('Imported quality findings · {count}','Importierte Qualitätsbefunde · {count}',{count:rows.length})));
  section.appendChild(element('p',t('Original source snapshot; graph simulation does not fix these findings.','Ursprünglicher Quellcode-Snapshot; eine Graphsimulation behebt diese Befunde nicht.'),'muted'));
  for(const row of rows.slice(0,qualityUi.inspectorLimit))appendQualityFinding(section,row,false);
  if(rows.length>qualityUi.inspectorLimit)section.appendChild(button(t('Show next 20','Nächste 20 anzeigen'),()=>{qualityUi.inspectorLimit+=20;refresh();},'btn btn-secondary'));
  if(!rows.length)section.appendChild(element('p',t('No imported findings mapped here. This is not proof of defect-free code.','Hier sind keine importierten Befunde zugeordnet. Das belegt keinen fehlerfreien Code.'),'muted'));
  container.appendChild(section);
}
function exportQualityFindings(){const source=qualityModel();return source?{quality_findings:{...source,scope:'all_imported_findings_original_snapshot'}}:{};}
function appendQualityMarkdown(lines,snapshot){
  const source=snapshot.quality_findings;if(!source)return;const cell=exportMarkdownCell;
  lines.push('','## Imported quality findings','','Original SARIF snapshot, independent of graph filters and simulation. Report/source revision at import is unverified; hashes only detect later source changes.','',
    'Status: '+cell(source.status)+'.','','| Tool / rule | Level | Location | State | Message |','| --- | --- | --- | --- | --- |');
  for(const f of source.findings||[])lines.push('| '+[f.tool+' / '+f.rule_id,f.level,(f.path||'unmapped')+(f.line?':'+f.line:'')+(f.column?':'+f.column:''),f.status+(f.suppressed?' / suppressed':'')+(f.baseline_state?' / '+f.baseline_state:''),f.message].map(cell).join(' | ')+' |');
  for(const note of source.diagnostics||[])lines.push('',cell(note));
}
for(const field of ['query','level'])document.getElementById('quality-'+field).addEventListener(field==='query'?'input':'change',event=>{qualityUi[field]=event.target.value;qualityUi.limit=20;renderQualityFindings();});
document.getElementById('quality-more').addEventListener('click',()=>{qualityUi.limit+=20;renderQualityFindings();});
