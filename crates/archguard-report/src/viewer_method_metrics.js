'use strict';
function methodComplexity(method) {
  const metric=method?.complexity;
  return metric?.definition==='ast-decisions-v1'&&Number.isInteger(metric.cyclomatic)&&metric.cyclomatic>=1&&Number.isInteger(metric.max_nesting)&&metric.max_nesting>=0?metric:null;
}
function methodComplexityText(method) {
  const metric=methodComplexity(method);
  return t('AST complexity {complexity} · max. nesting {nesting}','AST-Komplexität {complexity} · max. Verschachtelung {nesting}',{complexity:metric?.cyclomatic??'—',nesting:metric?.max_nesting??'—'});
}
function methodComplexityDescription() {
  return t('Source AST decisions (ast-decisions-v1), not runtime performance or a bytecode/Visual Studio metric. Concrete methods start at 1; if, loops, catch, conditional expressions, &&, || and non-default switch labels add decisions. C# also counts null-conditional/coalescing operations, filters and and/or patterns. TypeScript/JavaScript also count optional chaining, null coalescing and logical assignment operators. Nesting counts control structures, not ordinary blocks; else-if stays at the same level. Nested lambda/local-function/class bodies are excluded from their parent. Missing/invalid bodies or unrecognized metric definitions have no interpreted measurement (—).',
    'Entscheidungen im Quellcode-AST (ast-decisions-v1), keine Laufzeitmessung oder Bytecode-/Visual-Studio-Kennzahl. Konkrete Methoden beginnen bei 1; if, Schleifen, catch, bedingte Ausdrücke, &&, || und switch-Labels außer default erhöhen den Wert. C# zählt außerdem Nulloperatoren, Filter und and/or-Muster. TypeScript/JavaScript zählen auch optionale Verkettung, Nullkoaleszenz und logische Zuweisungen. Verschachtelung zählt Kontrollstrukturen, keine gewöhnlichen Blöcke; else-if bleibt auf derselben Ebene. Verschachtelte Lambda-/lokale Funktions-/Klassenkörper sind im Elternwert ausgeschlossen. Fehlende/ungültige Körper oder unbekannte Kennzahldefinitionen haben keinen interpretierten Messwert (—).');
}
function appendMethodMetrics(container,id) {
  const method=callMethods.get(id);if(!method)return;
  const line=element('p',callMethodLabel(id)+' · '+methodComplexityText(method),'muted');
  line.setAttribute('data-method-metrics',id);container.appendChild(explainMetric(line,'methodComplexity'));
}
function renderSelectedMethodDeclaration(container) {
  const method=callUi.element?.kind==='method'?callMethods.get(callUi.element.id):null;if(!method)return false;
  container.appendChild(element('h3',callClassLabel(method.class_id)+'#'+method.signature));
  container.appendChild(element('p',method.file+':'+method.line,'path'));appendMethodMetrics(container,method.id);
  container.appendChild(element('p',t('No recorded call sites match this method and the current call filters. This declaration is still measurable; missing calls do not prove it is unused.','Keine erfassten Aufrufstellen passen zu dieser Methode und den aktuellen Aufruffiltern. Ihre Deklaration ist weiterhin messbar; fehlende Aufrufe belegen keine ungenutzte Methode.'),'muted'));
  const matches=(data.reviewContext?.sources?.files||[]).filter(file=>file.path===method.file),cls=callClasses.get(method.class_id);
  const source=matches.length===1?data.nodes.find(node=>node.id===matches[0].node_id):cls?.file===method.file?data.nodes.find(node=>node.name===cls.graph_node):data.nodes.find(node=>node.name===method.file);
  if(source){const action=button(t('Locate method file in graph','Methodendatei im Graphen anzeigen'),()=>{showTab('graph');openSourceEvidence(source.id,source.name);});action.setAttribute('data-method-node',source.name);action.disabled=!nodeById(work,source.id);container.appendChild(action);}
  container.appendChild(button(t('Copy declaration location','Deklarationsort kopieren'),()=>copyText(method.file+':'+method.line)));
  return true;
}
function rankedMethodMeasurements(counts) {
  const byId=new Map(counts.map(row=>[row.id,row])),q=callUi.query.trim().toLowerCase();
  let missing=0;const rows=[];
  for(const method of callMethods.values()){
    const cls=callClasses.get(method.class_id);if(!cls)continue;
    if(callUi.language!=='all'&&callClassLanguage(cls)!==callUi.language)continue;
    if(callUi.package!=='*'&&cls.package!==callUi.package)continue;
    if(q&&![cls.name,method.signature].join(' ').toLowerCase().includes(q))continue;
    const metric=methodComplexity(method);if(!metric){missing++;continue;}
    rows.push({...byId.get(method.id)||{id:method.id,incoming:0,outgoing:0,total:0},cyclomatic:metric.cyclomatic,max_nesting:metric.max_nesting});
  }
  return {rows,missing};
}

function renderSelectedClassDeclaration(container) {
  const selected=callUi.element;if(selected?.kind!=='class')return false;
  const cls=callClasses.get(selected.id);if(!cls)return false;
  container.appendChild(element('h2',cls.name));container.appendChild(element('p',cls.file+':'+cls.line,'path'));
  const details=callModel.details, metric=details?.cohesion?.find(row=>row.class_id===cls.id&&row.definition==='lcom4-source/v1');
  if(!metric){container.appendChild(element('p',t('Class cohesion unknown: compiler state-access evidence is unavailable.','Klassenkohäsion unbekannt: Compiler-Belege für Feldzugriffe fehlen.'),'muted'));return true;}
  container.appendChild(element('h3',metric.lcom4==null?t('Class cohesion unknown','Klassenkohäsion unbekannt'):t('{groups} method groups','{groups} Methodengruppen',{groups:metric.lcom4})));
  container.appendChild(element('p',t('{methods} instance methods · {fields} state members · complexity Σ {complexity}','{methods} Instanzmethoden · {fields} Zustandsfelder · Komplexität Σ {complexity}',{methods:metric.methods,fields:metric.fields,complexity:metric.cyclomatic_sum??'—'}),'muted'));
  const help=element('details',undefined,'context-note');help.appendChild(element('summary',t('What connects these methods?','Was verbindet diese Methoden?')));
  help.appendChild(element('p',t('LCOM4 groups instance methods that share a non-static field/property or call each other. Constructors, static members and local functions are excluded. Multiple groups suggest a review, not an automatic design error. Incomplete compiler evidence leaves LCOM4 unknown.','LCOM4 gruppiert Instanzmethoden, die dasselbe Feld bzw. dieselbe Property nutzen oder einander aufrufen. Konstruktoren, statische Mitglieder und lokale Funktionen zählen nicht mit. Mehrere Gruppen sind ein Prüfhinweis, kein automatischer Architekturfehler. Bei unvollständigen Compiler-Belegen bleibt LCOM4 unbekannt.')));container.appendChild(help);
  const state=details.class_states?.find(row=>row.class_id===cls.id),fields=new Map((state?.fields||[]).filter(f=>!f.is_static).map(f=>[f.id,f]));
  const groupMethods=new Set((metric.groups||[]).flat()),nodes=[...groupMethods].filter(id=>callMethods.has(id)).map(id=>({id,name:callMethods.get(id).signature})),edges=[];
  const seen=new Set();for(const access of state?.accesses||[])if(groupMethods.has(access.method)&&fields.has(access.field)){
    const key=access.method+'\0'+access.field;if(!seen.has(key)){seen.add(key);edges.push({from:access.method,to:access.field});}
  }
  for(const id of new Set(edges.map(e=>e.to)))nodes.push({id,name:t('Field: {name}','Feld: {name}',{name:fields.get(id).name})});
  for(const call of callModel.calls||[])if(call.resolution==='project'&&groupMethods.has(call.from_method)&&groupMethods.has(call.to_method))edges.push({from:call.from_method,to:call.to_method});
  renderDetailGraph(container,{nodes,edges,label:t('Methods → shared state / internal calls','Methoden → gemeinsamer Zustand / interne Aufrufe'),maxNodes:20,maxEdges:30,onNode:n=>{if(callMethods.has(n.id)){callUi.element={kind:'method',id:n.id};renderCalls();}}});
  for(const [index,group] of (metric.groups||[]).entries()){
    const section=element('details',undefined,'inspector-section');section.appendChild(element('summary',t('Group {number} · {count} methods','Gruppe {number} · {count} Methoden',{number:index+1,count:group.length})));
    for(const id of group){const method=callMethods.get(id);if(method)section.appendChild(button(method.signature,()=>{callUi.element={kind:'method',id};renderCalls();},'btn btn-secondary'));}
    container.appendChild(section);
  }
  const evidence=element('details',undefined,'inspector-section');evidence.appendChild(element('summary',t('Field-access evidence','Belege der Feldzugriffe')));
  for(const access of state?.accesses||[])if(groupMethods.has(access.method)&&fields.has(access.field))evidence.appendChild(element('p',`${callMethods.get(access.method)?.signature||access.method} → ${fields.get(access.field).name} · ${access.file}:${access.line}`,'path'));
  container.appendChild(evidence);return true;
}
