'use strict';
const patternUi={limit:20};
const patternHighlight={id:null,nodes:new Set(),ancestors:new Set(),pairs:[],view:null,edges:new Set()};
function patternReport(){return data.insights?.patterns||data.insights?.review_session?.report?.architecture?.patterns||null;}
function patternLabel(value){return ({layered:t('Layered architecture','Schichtenarchitektur'),dependency_layers:t('Dependency levels','Abhängigkeitsebenen'),dependency_inversion:t('Dependency inversion','Abhängigkeitsumkehr'),pipeline:t('Pipeline structure','Kettenstruktur'),modular_monolith:t('Modular monolith candidate','Kandidat für modularen Monolithen'),ports_and_adapters:t('Ports and adapters','Ports und Adapter'),microservices:t('Microservices candidate','Microservices-Kandidat'),event_driven:t('Event-driven / messaging candidate','Ereignis-/nachrichtengetriebener Kandidat')})[value]||value;}
function patternSummary(p){
  const m=p.metrics||{};
  if(p.pattern==='dependency_layers'&&m.dependency_levels!=null)return t('{levels} levels · {files} / {total} files','{levels} Ebenen · {files} / {total} Dateien',{levels:m.dependency_levels,files:m.source_files,total:m.scope_source_files});
  if(['dependency_inversion','ports_and_adapters'].includes(p.pattern)&&m.implementations!=null)return t('{callers} consumers · {implementations} implementations','{callers} Nutzer · {implementations} Implementierungen',{callers:m.consumers??'?',implementations:m.implementations});
  if(p.pattern==='pipeline'&&p.status==='candidate')return t('Connected chain · {files} files','Zusammenhängende Kette · {files} Dateien',{files:m.active_files});
  if(p.pattern==='layered'&&m.connected_role_levels!=null)return t('{levels} connected roles · {backward} reverse dependencies','{levels} verbundene Rollen · {backward} Rückabhängigkeiten',{levels:m.connected_role_levels,backward:m.backward_dependencies??0});
  return '';
}
function renderPatternGraph(container,p){
  const originals=new Map(data.nodes.map(n=>[n.name,n])),pairs=new Set(),nodes=new Map(),edges=[];
  for(const e of [...(p.evidence||[]),...(p.counter_evidence||[])])if(originals.has(e.from)&&originals.has(e.to)){
    const a=originals.get(e.from),b=originals.get(e.to),key=JSON.stringify([a.id,b.id]);
    if(pairs.has(key))continue;const edge=data.edges.find(edge=>edge.from===a.id&&edge.to===b.id);if(!edge)continue;
    pairs.add(key);nodes.set(a.id,a);nodes.set(b.id,b);edges.push(edge);
  }
  if(edges.length)renderDetailGraph(container,{nodes:[...nodes.values()],edges,label:t('Recorded example','Belegter Ausschnitt'),...(!ops.length?{onNode:n=>revealNode(n.name)}:{})});
}
function patternStatus(value){return ({candidate:t('Candidate','Kandidat'),mixed:t('Candidate with counter-evidence','Kandidat mit Gegenbelegen'),insufficient_evidence:t('Insufficient evidence','Belege fehlen'),not_observed:t('Rule not matched','Regel nicht erfüllt')})[value]||value;}
function patternText(parent,value,tag='p'){const raw=String(value??'');parent.appendChild(element(tag,raw.length>4096?raw.slice(0,4096)+'…':raw));}
function clearPatternHighlight(){patternHighlight.id=null;patternHighlight.nodes.clear();patternHighlight.ancestors.clear();patternHighlight.pairs=[];patternHighlight.view=null;patternHighlight.edges.clear();if(ui.workspace==='architecture')renderArchitecture();else renderAll(false);}
function focusPattern(p){
  if(ops.length)return;
  patternHighlight.id=p.id;patternHighlight.nodes.clear();patternHighlight.ancestors.clear();patternHighlight.pairs=[];patternHighlight.view=null;
  const originals=new Map(data.nodes.map(n=>[n.name,n])),byId=new Map(work.nodes.map(n=>[n.id,n]));
  for(const name of p.nodes){const original=originals.get(name);if(!original)continue;patternHighlight.nodes.add(original.id);let current=byId.get(original.id);const seen=new Set();while(current&&!seen.has(current.id)){seen.add(current.id);patternHighlight.ancestors.add(current.id);current=byId.get(current.parent);}}
  for(const e of [...p.evidence,...p.counter_evidence]){const from=originals.get(e.from),to=originals.get(e.to);if(from&&to)patternHighlight.pairs.push([from.id,to.id]);}
  ui.edgeMode='all';ui.edgeModeAutomatic=false;showTab('graph');
  const current=byId.get([...patternHighlight.nodes][0]);if(current)revealNode(current.name);renderAll(false);
}
function patternNodeMarked(node){return !ops.length&&patternHighlight.ancestors.has(node.src);}
function patternEdgeMarked(link){
  if(ops.length||!patternHighlight.id)return false;
  if(patternHighlight.view!==view){
    const byId=new Map(work.nodes.map(n=>[n.id,n])),cache=new Map();
    function representative(id){if(cache.has(id))return cache.get(id);let n=byId.get(id);const seen=new Set();while(n&&!view.srcToDense.has(n.id)&&!seen.has(n.id)){seen.add(n.id);n=byId.get(n.parent);}const found=n&&view.srcToDense.get(n.id);cache.set(id,found);return found;}
    patternHighlight.edges.clear();for(const [a,b]of patternHighlight.pairs){const from=representative(a),to=representative(b);if(from!==undefined&&to!==undefined&&from!==to)patternHighlight.edges.add(from+' '+to);}patternHighlight.view=view;
  }
  return patternHighlight.edges.has(link.source.id+' '+link.target.id);
}
function renderPatternAssessments(root,servicesOnly=false){
  const report=patternReport();if(!report)return;
  const section=architectureSection(root,t('Architecture pattern candidates','Architekturmuster-Kandidaten'));

  if(patternHighlight.id)section.appendChild(button(t('Clear pattern highlight','Mustermarkierung löschen'),clearPatternHighlight,'btn btn-secondary'));
  if(ops.length)patternText(section,t('Clear the simulation to highlight original pattern evidence.','Die Simulation leeren, um Originalbelege des Musters zu markieren.'));
  const coverage=element('details');coverage.appendChild(element('summary',t('Evidence','Belege')));patternText(coverage,t('Patterns are hypotheses from source evidence. Several may coexist; they are not quality gates or probabilities.','Muster sind Hypothesen aus Quellbelegen. Mehrere können gleichzeitig vorliegen; sie sind keine Qualitätsgates oder Wahrscheinlichkeiten.'));patternText(coverage,JSON.stringify(report.coverage,null,2),'pre');for(const note of [...(report.limitations||[]),...(report.diagnostics||[])])patternText(coverage,note);section.appendChild(coverage);
  const structural=new Set(['dependency_layers','dependency_inversion','pipeline']);
  const priority=p=>!['mixed','candidate'].includes(p.status)?4:({dependency_layers:1,pipeline:2,dependency_inversion:3}[p.pattern]??0);
  const rows=(report.assessments||[]).filter(p=>!servicesOnly||['microservices','event_driven'].includes(p.pattern)).slice().sort((a,b)=>priority(a)-priority(b)||(b.metrics?.source_files??0)-(a.metrics?.source_files??0)||compareNames(a.pattern+' '+a.scope,b.pattern+' '+b.scope));
  patternText(coverage,t('{shown} / {total} assessments','{shown} / {total} Bewertungen',{shown:Math.min(rows.length,patternUi.limit),total:servicesOnly?rows.length:(report.assessments_total??rows.length)}));
  const candidates=rows.filter(p=>['candidate','mixed'].includes(p.status));
  const structures=candidates.filter(p=>structural.has(p.pattern));
  const state=report.coverage?.semantic_status;
  patternText(section,(candidates.length-structures.length)+' '+t('pattern candidates','Musterkandidaten')+' · '+structures.length+' '+t('structural observations','Strukturhinweise')+' · '+(state==='ready'?t('Compiler evidence available','Compilerbelege vorhanden'):state==='partial'?t('Partial compiler evidence','Compilerbelege teilweise'):state==='not_applicable'?t('No source analysis','Ohne Quellanalyse'):t('Compiler evidence unavailable','Compilerbelege fehlen')));
  const candidateList=element('div',undefined,'pattern-list');section.appendChild(candidateList);
  const structureList=element('div',undefined,'pattern-list');structureList.setAttribute('data-pattern-structures','true');
  if(structures.length){const heading=element('h3',t('Dependency structure','Abhängigkeitsstruktur'));heading.appendChild(contextHelp(t('Interpret structure','Struktur einordnen'),t('Recorded structure within connected regions; this does not establish the architecture style of the whole project.','Belegte Struktur zusammenhängender Bereiche; daraus folgt kein Architekturstil für das gesamte Projekt.')));structureList.appendChild(heading);section.appendChild(structureList);}
  const other=element('details',undefined,'context-note pattern-list');other.setAttribute('data-pattern-other','true');const openKey=servicesOnly?'servicesOtherOpen':'sourceOtherOpen';other.open=!!patternUi[openKey];other.addEventListener('toggle',()=>{patternUi[openKey]=other.open;});other.appendChild(element('summary',t('Other assessments','Weitere Prüfungen')+' · '+(rows.length-candidates.length)));if(rows.length>candidates.length)section.appendChild(other);
  for(const p of rows.slice(0,patternUi.limit)){
    const details=element('details'),summary=element('summary',patternLabel(p.pattern)+' · '+patternStatus(p.status)+(p.scope==='.'?'':' · '+p.scope));details.setAttribute('data-pattern',p.id);details.appendChild(summary);
    const description=patternSummary(p);if(description)summary.appendChild(element('small',description,'pattern-summary'));
    (p.status==='candidate'||p.status==='mixed'?(structural.has(p.pattern)?structureList:candidateList):other).appendChild(details);
    if(p.missing_evidence?.length){const gaps=element('details',undefined,'context-note');gaps.appendChild(element('summary',t('Interpretation and limits','Einordnung und Grenzen')));for(const gap of p.missing_evidence)patternText(gaps,gap);details.appendChild(gaps);}
    let prepared=false;details.addEventListener('toggle',()=>{if(!details.open||prepared)return;prepared=true;
      if(['candidate','mixed'].includes(p.status))renderPatternGraph(details,p);
      const metrics=element('details',undefined,'context-note');metrics.appendChild(element('summary',t('Detection rule and measurements','Erkennungsregel und Messwerte')));patternText(metrics,t('Evidence basis: ','Beleggrundlage: ')+p.basis);patternText(metrics,JSON.stringify(p.metrics,null,2),'pre');details.appendChild(metrics);
      patternText(details,t('{shown} / {total} source nodes retained','{shown} / {total} Quellknoten enthalten',{shown:p.nodes.length,total:p.nodes_total??p.nodes.length}));
      if(p.nodes.length){const focus=button(t('Highlight evidence in graph','Belege im Graphen markieren'),()=>focusPattern(p),'btn btn-secondary');focus.setAttribute('data-pattern-focus',p.id);focus.disabled=!!ops.length;details.appendChild(actionWithHelp(focus,()=>t('Mark the retained source files and evidence connections in the original graph. Clear simulated changes first.','Enthaltene Quelldateien und belegte Beziehungen im ursprünglichen Graphen markieren. Simulierte Änderungen vorher verwerfen.')));architectureNodes(details,p.nodes);}
      if(p.services?.length){patternText(details,t('Configured services: ','Konfigurierte Dienste: ')+p.services.join(', '));details.appendChild(button(t('Open service topology','Diensttopologie öffnen'),()=>showTab('services'),'btn btn-secondary'));}
      for(const [key,label,total]of [['evidence',t('Supporting evidence','Stützende Belege'),p.evidence_total],['counter_evidence',t('Counter-evidence','Gegenbelege'),p.counter_evidence_total]]){
        const block=architectureSection(details,label),items=p[key]||[];patternText(block,items.length+' / '+(total??items.length));let count=0;const list=element('div');block.appendChild(list);
        const more=button(t('Show next 20 evidence records','Nächste 20 Belege'),()=>fill(),'btn btn-secondary');block.appendChild(more);
        function fill(){for(const e of items.slice(count,count+20)){patternText(list,[e.from,e.to].filter(Boolean).join(' → '),'h4');patternText(list,(e.file||'')+(e.line?':'+e.line:'')+(e.pointer?'#'+e.pointer:''),'code');patternText(list,e.message,'pre');}count+=20;more.hidden=count>=items.length;}fill();
      }
    });
  }
  if(rows.length>patternUi.limit){const more=button(t('Show more pattern assessments','Weitere Musterbewertungen'),()=>{patternUi.limit+=20;servicesOnly?renderServices():renderArchitecture();},'btn btn-secondary');more.setAttribute('data-pattern-more','true');section.appendChild(more);}
}
function appendPatternsMarkdown(lines,report){
  if(!report)return;const cell=exportMarkdownCell;lines.push('','## Architecture pattern candidates','','Hypotheses only; independent of graph simulations and display filters.','',cell(report.definition),'',cell(JSON.stringify(report.coverage)));
  for(const p of report.assessments||[]){lines.push('','### '+cell(p.pattern)+' · '+cell(p.scope),'',cell(p.status)+' · '+cell(p.basis),'',cell(JSON.stringify(p.metrics)),'Sources: '+p.nodes.map(cell).join(', ')+' · '+p.nodes.length+' / '+p.nodes_total,'Services: '+(p.services||[]).map(cell).join(', '));
    for(const key of ['evidence','counter_evidence']){lines.push('',cell(key)+' · '+p[key].length+' / '+p[key+'_total']);for(const e of p[key])lines.push('- '+[e.kind,e.from,e.to,e.file,e.line,e.pointer,e.message].filter(v=>v!=null).map(cell).join(' · '));}
    for(const gap of p.missing_evidence||[])lines.push('- Missing: '+cell(gap));
  }for(const note of [...(report.limitations||[]),...(report.diagnostics||[])])lines.push('',cell(note));
}
