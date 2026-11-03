'use strict';
// Only aggregate when requested. Element selection and translation do not change
// the source-call scope, so they can reuse both the counts and numeric ordering.
const callRankingUi={mode:'connections',kind:'class',sort:'incoming',limit:20,scope:null,source:null,model:null,order:null,rows:[],measurements:null};
function selectCallRanking(kind,id) {
  const field=kind==='class'?'class':'method',direction=callRankingUi.sort;
  const relation=callRelations({ignoreElement:true}).find(r=>r.calls.some(c=>
    (direction!=='outgoing'&&c['to_'+field]===id)||(direction!=='incoming'&&c['from_'+field]===id)));
  callUi.relation=relation?.key??null;
  if(callTreeUi.model)callTreeOpenPath(callTreeUi.model,kind,id);
  selectCallTreeElement(kind,id);
  document.getElementById('calls-evidence').focus();
}
function renderCallRankings() {
  const active=callRankingUi.mode==='rankings';
  for(const mode of ['connections','rankings','cycles'])document.getElementById('calls-show-'+mode).setAttribute('aria-pressed',String(callRankingUi.mode===mode));
  const cycles=callRankingUi.mode==='cycles';
  document.getElementById('calls-relations').hidden=callRankingUi.mode!=='connections';
  document.getElementById('calls-ranking').hidden=!active;
  document.getElementById('calls-cycles').hidden=!cycles;
  for(const id of ['calls-resolution','calls-internal'])document.getElementById(id).disabled=cycles;
  if(!active)return;
  const relations=callRelations({ignoreElement:true});
  if(callRankingUi.scope!==callRelationsCache.scope||callRankingUi.source!==callModel){
    callRankingUi.model=computeCallRankings(relations.flatMap(r=>r.calls),callClasses,callMethods);
    callRankingUi.scope=callRelationsCache.scope;callRankingUi.source=callModel;
    callRankingUi.order=null;callRankingUi.limit=20;callRankingUi.measurements=null;
  }
  if(callRankingUi.kind==='class'&&['cyclomatic','max_nesting'].includes(callRankingUi.sort))callRankingUi.sort='incoming';
  const model=callRankingUi.model,kind=callRankingUi.kind,metric=callRankingUi.sort,order=kind+':'+metric;
  const measured=['cyclomatic','max_nesting'].includes(metric);
  if(callRankingUi.order!==order){
    if(measured&&!callRankingUi.measurements)callRankingUi.measurements=rankedMethodMeasurements(model.methods);
    callRankingUi.rows=(measured?callRankingUi.measurements.rows.slice():(kind==='class'?model.classes:model.methods).filter(row=>row[metric]>0))
      .sort((a,b)=>b[metric]-a[metric]||(a.id<b.id?-1:a.id>b.id?1:0));
    callRankingUi.order=order;callRankingUi.limit=20;
  }
  document.getElementById('calls-ranking-kind').value=kind;
  document.getElementById('calls-ranking-sort').value=metric;
  for(const option of document.getElementById('calls-ranking-sort').querySelectorAll('option[data-method-only]'))option.disabled=kind!=='method';
  const rows=callRankingUi.rows,shown=rows.slice(0,callRankingUi.limit);
  document.getElementById('calls-ranking-status').textContent=shown.length+' / '+rows.length+' · '+
    (measured?t('AST complexity and nesting of declared methods, including methods without recorded calls.','AST-Komplexität und Verschachtelung deklarierter Methoden, auch ohne erfasste Aufrufe.'):t('Static call sites, not runtime frequency.','Statische Aufrufstellen, keine Laufzeithäufigkeit.'))+' '+
    t('Coverage: ','Abdeckung: ')+callCoverageLabel(callModel.status)+'.';
  let coverage=t('Coverage: ','Abdeckung: ')+callCoverageLabel(callModel.status)+'.';
  if(callModel.status==='partial')coverage+=' '+t('Counts are incomplete and may change when missing dependencies are resolved.','Die Zählung ist unvollständig und kann sich nach Auflösung fehlender Abhängigkeiten ändern.');
  if(model.unresolved_targets)coverage+=' '+model.unresolved_targets+t(' unresolved target sites in this scope.',' unaufgelöste Ziel-Aufrufstellen in diesem Umfang.');
  if(model.missing_callers)coverage+=' '+model.missing_callers+t(' sites without a declared caller method (including initializers).',' Aufrufstellen ohne deklarierte Aufrufermethode (einschließlich Initialisierern).');
  if(model.missing_targets)coverage+=' '+model.missing_targets+t(' project target sites without a declared method.',' Projekt-Ziel-Aufrufstellen ohne deklarierte Methode.');
  coverage+=' '+t('Project types, module owners and declared functions/methods are ranked. Module owners group top-level functions and initializers; they are not declared classes. Calls to external or unknown targets can count as outgoing; no target is invented.','Projekttypen, Module und deklarierte Funktionen/Methoden werden gelistet. Module gruppieren freie Funktionen und Initialisierer; sie sind keine deklarierten Klassen. Aufrufe externer oder unbekannter Ziele können ausgehend zählen; es werden keine Ziele erfunden.');
  if(measured)coverage+=' '+methodComplexityDescription()+' '+t('Language, package and search select declarations. Target and internal-call filters only affect the accompanying In/Out counts. {count} declarations in scope have no measurement.','Sprache, Paket und Suche wählen Deklarationen. Ziel- und interne Aufruffilter beeinflussen nur die ergänzenden Ein-/Aus-Zahlen. {count} Deklarationen im Umfang haben keinen Messwert.',{count:callRankingUi.measurements.missing});
  document.getElementById('calls-ranking-coverage').textContent=coverage;
  const list=document.getElementById('calls-ranking-list');list.textContent='';
  for(const [index,row] of shown.entries()){
    const method=kind==='method'?callMethods.get(row.id):null;
    const label=method?callClassLabel(method.class_id)+'#'+method.signature:callClassLabel(row.id);
    const entry=button('',()=>selectCallRanking(kind,row.id),'explorer-row call-relation call-rank');
    entry.setAttribute('data-call-rank',JSON.stringify({kind,id:row.id}));
    entry.setAttribute('aria-pressed',String(callUi.element?.kind===kind&&callUi.element.id===row.id));
    for(const direction of ['incoming','outgoing','total'])entry.setAttribute('data-rank-'+direction,String(row[direction]));
    if(method){const measurement=methodComplexity(method);for(const key of ['cyclomatic','max_nesting'])if(measurement)entry.setAttribute('data-rank-'+key,String(measurement[key]));}
    entry.appendChild(element('strong',(index+1)+'. '+label));
    entry.appendChild(element('small',t('Incoming {incoming} · Outgoing {outgoing} · Involved sites {total}','Eingehend {incoming} · Ausgehend {outgoing} · Beteiligte Stellen {total}',row)));
    if(method)entry.appendChild(element('small',methodComplexityText(method)));
    const cls=callClasses.get(method?.class_id??row.id),language=callClassLanguage(cls);
    if(language)entry.appendChild(element('small',callLanguageLabel(language)));
    if(cls?.kind==='module')entry.appendChild(element('small',t('Module · top-level functions','Modul · freie Funktionen')));
    list.appendChild(entry);
  }
  if(!rows.length)list.appendChild(element('p',measured?t('No measured method bodies in this declaration scope. Missing metrics are unknown, not zero.','Keine gemessenen Methodenkörper in diesem Deklarationsumfang. Fehlende Kennzahlen sind unbekannt, nicht null.'):t('No recorded call sites for this direction and these filters.','Keine erfassten Aufrufstellen für diese Richtung und diese Filter.'),'empty'));
  document.getElementById('calls-ranking-more').hidden=shown.length>=rows.length;
}
for(const mode of ['connections','rankings','cycles'])document.getElementById('calls-show-'+mode).addEventListener('click',()=>{
  if((callRankingUi.mode==='cycles')!==(mode==='cycles')){callUi.element=null;callUi.relation=null;callUi.siteLimit=100;}
  callRankingUi.mode=mode;renderCalls();
});
for(const field of ['kind','sort'])document.getElementById('calls-ranking-'+field).addEventListener('change',event=>{
  callRankingUi[field]=event.target.value;renderCallRankings();
});
document.getElementById('calls-ranking-more').addEventListener('click',()=>{
  const next=callRankingUi.rows[callRankingUi.limit];callRankingUi.limit+=20;renderCallRankings();
  const entry=Array.from(document.getElementById('calls-ranking-list').querySelectorAll('button[data-call-rank]'))
    .find(row=>JSON.parse(row.getAttribute('data-call-rank')).id===next?.id);
  if(entry)entry.focus();
});
