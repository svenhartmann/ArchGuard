'use strict';
// Cache the full source-call SCCs. Filters choose whole groups; they never cut
// off the return path. Element focus only selects evidence within that group.
const callCycleUi={source:null,model:null,scope:null,kind:'class',groups:[],selected:null,group:null,relations:[],members:new Set(),limit:20,memberLimit:50,edgeLimit:50,witnessLimit:20};
function callCycleLabel(kind,id) {
  const method=kind==='method'?callMethods.get(id):null;
  return method?callClassLabel(method.class_id)+'#'+method.signature:callClassLabel(id);
}
function prepareCallCycles() {
  prepareCallModel();
  if(callCycleUi.source!==callModel){
    callCycleUi.model=computeCallCycles(callModel.calls||[],callClasses,callMethods);
    callCycleUi.source=callModel;callCycleUi.scope=null;
  }
  const kind=callCycleUi.kind,query=callUi.query.trim().toLowerCase();
  const scope=JSON.stringify([kind,callUi.language,callUi.package,query]);
  if(scope!==callCycleUi.scope){
    const groups=kind==='class'?callCycleUi.model.classes:callCycleUi.model.methods;
    callCycleUi.groups=groups.filter(group=>group.members.some(id=>{
      const cls=callClasses.get(kind==='method'?callMethods.get(id)?.class_id:id);
      return cls&&(callUi.language==='all'||callClassLanguage(cls)===callUi.language)&&
        (callUi.package==='*'||cls.package===callUi.package)&&
        (!query||[callCycleLabel(kind,id),cls.package,cls.file].join(' ').toLowerCase().includes(query));
    }));
    callCycleUi.scope=scope;callCycleUi.limit=20;
  }
  const group=callCycleUi.groups.find(group=>group.id===callCycleUi.selected)||callCycleUi.groups[0]||null;
  callCycleUi.selected=group?.id??null;
  if(group!==callCycleUi.group){
    callCycleUi.group=group;callCycleUi.members=new Set(group?.members||[]);
    callCycleUi.memberLimit=50;callCycleUi.edgeLimit=50;callCycleUi.witnessLimit=20;
    callCycleUi.relations=(group?.edges||[]).map(edge=>({
      key:JSON.stringify(['cycle',kind,edge.from,edge.to]),
      from:kind==='class'?edge.from:callMethods.get(edge.from).class_id,
      to:kind==='class'?edge.to:callMethods.get(edge.to).class_id,
      cycle_from:edge.from,cycle_to:edge.to,label:callCycleLabel(kind,edge.to),resolution:'project',calls:edge.calls
    }));
    callUi.element=null;callUi.relation=null;callUi.siteLimit=100;callTreeUi.limit=200;
  }
}
function activeCallRelations(options={}) {
  if(callRankingUi.mode!=='cycles')return callRelations(options);
  prepareCallCycles();return callCycleUi.relations;
}
function isCallCycleMember(kind,id) {
  return callRankingUi.mode==='cycles'&&callCycleUi.group?.kind===kind&&callCycleUi.members.has(id);
}
function selectCallCycleEdge(from,to) {
  const relation=callCycleUi.relations.find(row=>row.cycle_from===from&&row.cycle_to===to);
  if(!relation)return;
  callUi.element=null;callUi.relation=relation.key;callUi.siteLimit=100;
  renderCalls();document.getElementById('calls-evidence').focus();
}
function renderCallCycles() {
  if(callRankingUi.mode!=='cycles')return;
  prepareCallCycles();
  const state=callCycleUi,{kind,group,groups}=state;
  const total=(kind==='class'?state.model.classes:state.model.methods).length;
  document.getElementById('calls-cycle-kind').value=kind;
  let status=t('{shown} / {total} recorded cyclic groups','{shown} / {total} erfasste zyklische Gruppen',{shown:groups.length,total})+' · '+
    (kind==='class'?t('types / modules','Typen / Module'):t('functions / methods / recursion','Funktionen / Methoden / Rekursion'))+' · '+
    t('Coverage: ','Abdeckung: ')+callCoverageLabel(callModel.status)+'.';
  if(!groups.length)status+=' '+t('No recorded cyclic group matches this scope.','Keine erfasste zyklische Gruppe passt zu diesem Umfang.');
  if(callModel.status!=='ready')status+=' '+t('Incomplete analysis cannot rule out further cycles.','Eine unvollständige Analyse schließt weitere Zyklen nicht aus.');
  document.getElementById('calls-cycles-status').textContent=status;
  document.getElementById('calls-cycles-coverage').textContent=
    t('Each group is a strongly connected component (SCC), shown with all its internal connections and one closed example path. This does not enumerate every possible elementary cycle. Type/module cycles aggregate calls; they do not necessarily form a function/method cycle. Module owners group top-level functions, not declared classes. Function/method cycles include direct and mutual recursion within one owner. ',
      'Jede Gruppe ist eine stark zusammenhängende Komponente (SCC), mit allen internen Verbindungen und einem geschlossenen Beispielpfad. Es werden nicht alle möglichen elementaren Zyklen aufgezählt. Typ-/Modulzyklen fassen Aufrufe zusammen; sie ergeben nicht zwingend einen Funktions-/Methodenzyklus. Module gruppieren freie Funktionen und sind keine deklarierten Klassen. Funktions-/Methodenzyklen enthalten direkte und gegenseitige Rekursion innerhalb eines Elements. ')+
    t('Language, package and search select complete groups when a member matches. Target and internal-call controls are paused here to preserve return paths. Static compiler targets do not prove that a loop occurs at runtime. ',
      'Sprache, Paket und Suche wählen vollständige Gruppen, sobald ein Element passt. Ziel- und interne Aufruffilter sind hier ausgesetzt, damit Rückwege erhalten bleiben. Statische Compilerziele belegen keinen tatsächlich durchlaufenen Zyklus zur Laufzeit. ')+
    t('{unresolved} unresolved target sites; {missing} project sites without complete method identities in the source snapshot.',
      '{unresolved} unaufgelöste Ziel-Aufrufstellen; {missing} Projekt-Aufrufstellen ohne vollständige Methodenidentitäten im Quellcode-Snapshot.',{unresolved:state.model.unresolved_targets,missing:state.model.missing_methods});
  const list=document.getElementById('calls-cycles-list');list.textContent='';
  const shown=groups.slice(0,state.limit);
  // Keep a retained selection visible when broader filters reset pagination.
  if(group&&!shown.includes(group))shown.push(group);
  for(const item of shown){
    const row=button('',()=>{state.selected=item.id;callUi.element=null;callUi.relation=null;renderCalls();document.getElementById('calls-cycle-detail').focus();},'explorer-row call-relation');
    row.setAttribute('data-call-cycle',item.id);row.setAttribute('aria-pressed',String(item===group));
    const preview=item.members.slice(0,2).map(id=>callCycleLabel(kind,id)).join(', ')+(item.members.length>2?' … +'+(item.members.length-2):'');
    row.appendChild(element('strong',(groups.indexOf(item)+1)+'. ↻ '+preview));
    row.appendChild(element('small',t('{members} elements · {edges} connections · {sites} call sites','{members} Elemente · {edges} Verbindungen · {sites} Aufrufstellen',{members:item.members.length,edges:item.edges.length,sites:item.call_sites})));
    list.appendChild(row);
  }
  document.getElementById('calls-cycles-more').hidden=state.limit>=groups.length;
  document.getElementById('calls-cycle-detail').hidden=!group;
  for(const id of ['calls-cycle-path','calls-cycle-members','calls-cycle-edges'])document.getElementById(id).textContent='';
  for(const id of ['calls-cycle-title','calls-cycle-witness','calls-cycle-members-title','calls-cycle-edges-title'])document.getElementById(id).textContent='';
  if(!group)return;
  document.getElementById('calls-cycle-title').textContent=t('Selected group · {members} elements · {edges} connections','Gewählte Gruppe · {members} Elemente · {edges} Verbindungen',{members:group.members.length,edges:group.edges.length});
  const steps=group.witness.length-1;
  document.getElementById('calls-cycle-witness').textContent=t('Closed example path: ','Geschlossener Beispielpfad: ')+
    (steps<=20?group.witness.map(id=>callCycleLabel(kind,id)).join(' → '):
      t('{steps} directed steps. Follow the numbered connections below; the full closed path is included in the export.','{steps} gerichtete Schritte. Folge den nummerierten Verbindungen unten; der vollständige geschlossene Pfad ist im Export enthalten.',{steps}));
  const path=document.getElementById('calls-cycle-path');
  for(let i=0;i<Math.min(steps,state.witnessLimit);i++){
    const from=group.witness[i],to=group.witness[i+1];
    const row=button((i+1)+'. '+callCycleLabel(kind,from)+' → '+callCycleLabel(kind,to),()=>selectCallCycleEdge(from,to),'btn btn-secondary call-cycle-edge');
    row.setAttribute('data-cycle-path-edge',JSON.stringify([from,to]));path.appendChild(row);
  }
  document.getElementById('calls-cycle-witness-more').hidden=state.witnessLimit>=steps;
  document.getElementById('calls-cycle-members-title').textContent=t('All {count} members','Alle {count} Elemente',{count:group.members.length});
  const members=document.getElementById('calls-cycle-members');
  for(const id of group.members.slice(0,state.memberLimit)){
    const row=button(callCycleLabel(kind,id),()=>{if(callTreeUi.model)callTreeOpenPath(callTreeUi.model,kind,id);selectCallTreeElement(kind,id);document.getElementById('calls-tree').focus();},'btn btn-secondary call-cycle-member');
    row.setAttribute('data-cycle-member',JSON.stringify({kind,id}));row.setAttribute('aria-pressed',String(callUi.element?.kind===kind&&callUi.element.id===id));members.appendChild(row);
  }
  document.getElementById('calls-cycle-members-more').hidden=state.memberLimit>=group.members.length;
  document.getElementById('calls-cycle-edges-title').textContent=t('All {count} internal connections','Alle {count} internen Verbindungen',{count:group.edges.length});
  const edges=document.getElementById('calls-cycle-edges');
  for(const relation of state.relations.slice(0,state.edgeLimit)){
    const {cycle_from:from,cycle_to:to}=relation;
    const row=button(callCycleLabel(kind,from)+' → '+callCycleLabel(kind,to)+' · '+relation.calls.length+t(' call sites',' Aufrufstellen'),()=>selectCallCycleEdge(from,to),'btn btn-secondary call-cycle-edge');
    row.setAttribute('data-cycle-edge',JSON.stringify([from,to]));row.setAttribute('aria-pressed',String(callUi.relation===relation.key));edges.appendChild(row);
  }
  document.getElementById('calls-cycle-edges-more').hidden=state.edgeLimit>=group.edges.length;
}
document.getElementById('calls-cycle-kind').addEventListener('change',event=>{callCycleUi.kind=event.target.value;callUi.element=null;callUi.relation=null;renderCalls();});
document.getElementById('calls-cycles-show-all').addEventListener('click',()=>{
  callUi.language='all';callUi.package='*';callUi.query='';callUi.element=null;callUi.relation=null;
  document.getElementById('calls-filter').value='';renderCalls();
});
for(const [id,field,step] of [['calls-cycles-more','limit',20],['calls-cycle-members-more','memberLimit',50],['calls-cycle-edges-more','edgeLimit',50],['calls-cycle-witness-more','witnessLimit',20]]){
  document.getElementById(id).addEventListener('click',()=>{callCycleUi[field]+=step;renderCallCycles();});
}
