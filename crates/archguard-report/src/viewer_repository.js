'use strict';
// Repository topology and each project's source graph are separate server scopes.
let repositorySelection=null;
function repositoryModel(){const model=data.insights?.repository;return model&&['overview','project'].includes(model.mode)&&typeof model.root==='string'&&Array.isArray(model.projects)?model:null;}
function isRepositoryOverview(){return repositoryModel()?.mode==='overview';}
function repositorySelectableProjects(){
  const paths=new Set();
  return (repositoryModel()?.projects||[]).filter(project=>{
    const path=project.path;
    if(project.kind==='workspace'||typeof path!=='string'||!path||path.startsWith('/')||/[\\\u0000-\u001f]/.test(path)||path.split('/').some(part=>part==='..'||!part)||paths.has(path))return false;
    paths.add(path);return true;
  });
}
function repositoryLive(){return window.archguardLiveViewer===true&&['http:','https:'].includes(window.location?.protocol);}
function repositoryCommand(){return 'archguard serve -- '+"'"+repositoryModel().root.replaceAll("'","'\"'\"'")+"'";}
function repositoryNavigate(event){
  if(!isRepositoryOverview()&&(ops.length||ui.pending||!assessmentCanReload()||typeof scenarioCanReload==='function'&&!scenarioCanReload())){
    event.preventDefault();feedback(t('Export and clear simulated changes and review drafts, or close the pending dialog, before opening another project.','Exportiere und lösche simulierte Änderungen und Review-Entwürfe oder schließe den offenen Dialog, bevor du ein anderes Projekt öffnest.'));
  }
}
function repositoryLink(link,path){link.setAttribute('href',repositoryWorldHref(path));link.addEventListener('click',repositoryNavigate);}
function enforceRepositoryScope(){
  if(!isRepositoryOverview())return;
  if(!['start','services','security'].includes(ui.workspace))ui.workspace='start';
  ui.simulation=false;ui.pending=null;ui.selection=null;ui.cycle=null;
  ops=[];opBatches=[];pendingRepair=null;appliedRepair=null;
}
function renderRepositoryNavigation(){
  const model=repositoryModel(),bar=document.getElementById('repository-navigation');bar.hidden=!model;if(!model)return;
  const overview=model.mode==='overview',projects=repositorySelectableProjects(),selector=document.getElementById('repository-project');
  if(!projects.some(project=>project.path===repositorySelection))repositorySelection=projects.some(project=>project.path===model.active_project)?model.active_project:projects[0]?.path||null;
  // Native options keep keyboard navigation; the full inventory remains in Services.
  const shown=projects.slice(0,2000),selected=projects.find(project=>project.path===repositorySelection);
  if(selected&&!shown.includes(selected))shown[shown.length-1]=selected;
  selector.textContent='';
  for(const kind of ['application','library','tooling','unknown']){
    const entries=shown.filter(project=>(['application','library','tooling'].includes(project.kind)?project.kind:'unknown')===kind);if(!entries.length)continue;
    const group=element('optgroup');group.setAttribute('label',serviceProjectKind(kind));
    for(const project of entries){const option=element('option',project.name+' · '+project.path);option.value=project.path;group.appendChild(option);}
    selector.appendChild(group);
  }
  selector.value=repositorySelection||'';selector.disabled=!projects.length;
  document.getElementById('repository-heading').textContent=overview?t('Monorepo · services','Monorepo · Dienste'):t('Project source analysis','Quellanalyse des Projekts');
  document.getElementById('repository-heading').hidden=!overview;
  const excluded=(model.excluded_projects||[]).join(', ');
  document.getElementById('repository-scope').hidden=!excluded&&projects.length<=shown.length;
  document.getElementById('repository-scope').textContent=(overview?
    t('Projects are analyzed separately.','Projekte werden getrennt analysiert.'):
    t('Source scope: {path}.','Quellumfang: {path}.',{path:model.active_project||'.'})+(excluded?' '+t('Nested projects excluded: {paths}.','Ausgeschlossene Unterprojekte: {paths}.',{paths:excluded}):''))+
    (projects.length>shown.length?' '+t('First {count} project choices shown; use the Services inventory for additional projects.','Die ersten {count} Projekte stehen zur Auswahl; weitere Projekte findest du im Dienste-Inventar.',{count:shown.length}):'');
  if(!overview&&projects.find(p=>p.path===model.active_project)?.ecosystem==='go'){
    const note=document.getElementById('repository-scope');note.hidden=false;
    note.textContent=t('Go packages, structs, interfaces and source KPIs use the Go SDK. Dynamic calls, missing dependencies and build variants remain explicit analysis gaps; embedding is composition, not inheritance.','Go-Pakete, Structs, Interfaces und Quell-KPIs nutzen das Go SDK. Dynamische Aufrufe, fehlende Abhängigkeiten und Build-Varianten bleiben als Analyselücken sichtbar; Embedding ist Komposition, keine Vererbung.');
  }
  const open=document.getElementById('repository-open'),back=document.getElementById('repository-overview'),live=repositoryLive();
  open.textContent=t('Open project','Projekt öffnen');open.hidden=!live||!repositorySelection;
  if(!open.hidden)open.setAttribute('href',repositoryWorldHref(repositorySelection));else open.removeAttribute('href');
  back.hidden=overview||!live;if(!back.hidden)back.setAttribute('href',repositoryWorldHref(null));else back.removeAttribute('href');
  document.getElementById('repository-standalone').hidden=live;
  document.getElementById('repository-command').textContent=repositoryCommand();
  document.getElementById('repository-standalone-note').textContent=t('This HTML is a snapshot. Start the repository server, then choose a project there.','Diese HTML-Datei ist ein Snapshot. Starte den Repository-Server und wähle dort ein Projekt.');
  if(overview){
    document.getElementById('main').classList.add('explorer-closed');
    document.getElementById('main').setAttribute('data-repository-mode','overview');
    document.getElementById('toggle-explorer').setAttribute('aria-expanded','false');
    for(const id of ['explorer','explorer-resizer','toggle-explorer','filter','clear-search','refactor-menu','graph-options'])document.getElementById(id).hidden=true;
    for(const id of ['toggle-explorer','filter','clear-search','simulation-mode','repair-cycle','scenario-open'])document.getElementById(id).disabled=true;
    for(const tab of document.querySelectorAll('#tabs button')){tab.hidden=!['start','services','security'].includes(tab.dataset.tab);tab.disabled=tab.hidden;tab.classList.toggle('active',tab.dataset.tab===ui.workspace);tab.setAttribute('aria-selected',String(tab.dataset.tab===ui.workspace));}
    for(const name of ['start','graph','dsm','calls','architecture','services','security','quality','assessment'])document.getElementById('tab-'+name).classList.toggle('panel-hidden',name!==ui.workspace);
  }
  if(!overview&&live&&worldUi.bound&&ui.workspace==='start'){renderWorldPath();document.getElementById('world-back').disabled=false;}
}
function appendRepositoryProjectNavigation(panel,project){
  if(project.kind==='workspace'||!repositoryModel()||!repositorySelectableProjects().some(item=>item.id===project.id||item.path===project.path))return;
  if(repositoryLive()){
    const link=element('a',t('Open project analysis','Projektanalyse öffnen'),'btn btn-primary');link.setAttribute('data-repository-project',project.path);repositoryLink(link,project.path);panel.appendChild(link);
  }else{
    panel.appendChild(element('p',t('For source analysis, start the repository server and select this project there.','Starte für die Quellanalyse den Repository-Server und wähle dort dieses Projekt.'),'muted'));
    panel.appendChild(element('code',repositoryCommand(),'repository-command'));panel.appendChild(button(t('Copy server command','Serverbefehl kopieren'),()=>copyText(repositoryCommand()),'btn btn-secondary'));
  }
}
function initRepository(){
  document.getElementById('repository-project').addEventListener('change',event=>{
    if(repositorySelectableProjects().some(project=>project.path===event.target.value))repositorySelection=event.target.value;
    renderRepositoryNavigation();
  });
  for(const id of ['repository-open','repository-overview'])document.getElementById(id).addEventListener('click',repositoryNavigate);
  document.getElementById('repository-copy').addEventListener('click',()=>copyText(repositoryCommand()));
  enforceRepositoryScope();renderRepositoryNavigation();
}

// The repository landscape represents manifest projects, not source classes.
// Every project has equal geometry; no unmeasured source KPI is implied.
function repositoryWorldHref(path){return path===null?'/#tab=start':'?project='+encodeURIComponent(path)+'#tab=start';}
function repositoryOpenWorld(path){
  if(path!==null&&repositoryWorldInventory().some(p=>p.path===path&&p.external)){repositorySelectWorld(path);setWorldDetailsOpen(true,true);return;}
  path=repositoryWorldInventory().find(p=>p.path===path)?.sourcePath||path;
  if(path!==null&&!repositorySelectableProjects().some(project=>project.path===path))return;
  if(!repositoryLive()){feedback(t('Start the repository server to open a project: ','Zum Öffnen eines Projekts den Repository-Server starten: ')+repositoryCommand());return;}
  let blocked=false;repositoryNavigate({preventDefault(){blocked=true;}});
  if(!blocked)window.location.assign(repositoryWorldHref(path));
}
function repositoryWorldHelp(){return t('Each island represents one detected application, library or tool project. Equal island sizes and heights are navigation aids, not code or quality measurements. Click selects a service and its connections. Details shows source evidence. Double-click or Zoom into service opens packages and classes; further wheel zoom at maximum also opens a service. Projects are analyzed separately; source metrics are loaded only inside a project. Use Services for configured relationships. Wheel/pinch zooms; drag pans; right-drag or two-finger twist rotates.','Jede Insel steht für eine erkannte Anwendung, Bibliothek oder ein Werkzeugprojekt. Gleiche Inselgrößen und Höhen dienen der Navigation, nicht der Messung von Code oder Qualität. Ein Klick markiert den Service und seine Verbindungen. Details zeigt Quellbelege. Doppelklick oder In Service hineinzoomen öffnet Pakete und Klassen; weiteres Zoomen am Maximum öffnet ebenfalls den Service. Projekte werden getrennt analysiert; Quellmetriken werden erst im Projekt geladen. Konfigurierte Beziehungen stehen unter Dienste. Mausrad/Zwei-Finger-Geste zoomt, Ziehen verschiebt, Rechtsziehen oder eine Zwei-Finger-Drehung rotiert.');}
function renderRepositoryWorld(){
  if(!repositoryWorldState.initialized){
    repositoryWorldState.initialized=true;
    const scopes=repositoryDeploymentScopes();
    if(scopes.length>1){
      const rootCompose=scopes.find(s=>['compose.yaml','compose.yml','docker-compose.yaml','docker-compose.yml'].includes(s));
      repositoryWorldState.deployment=rootCompose||scopes.sort((a,b)=>(a.split('/').length-b.split('/').length)||a.localeCompare(b))[0];
    }
  }
  const host=document.getElementById('start-world');host.hidden=false;
  if(!worldUi.bound)initWorld();
  worldUi.lens='problems';worldUi.within=null;worldUi.history=[];
  const projects=repositoryWorldProjects();
  worldUi.model={scope:null,cycle_groups:0,connections:repositoryWorldConnections(),areas:projects.map(project=>({
    name:project.path,kind:'project',project,cyclic:repositoryWorldConnections().some(e=>e.cyclic&&(e.from===project.path||e.to===project.path)),findings:[],
    entities:[{id:project.id,node:project.path,project,max_complexity:null,coverage:null}]
  }))};
  // One tile per project, including large inventories (no source-file batching).
  const columns=Math.max(1,Math.ceil(Math.sqrt(projects.length)));
  worldUi.layout={areas:worldUi.model.areas.map((area,i)=>({name:area.name,x:i%columns*5,y:Math.floor(i/columns)*5,w:2,h:2,region:area})),cells:[]};
  for(const area of worldUi.layout.areas){
    // Four facets form a single project island, without inventing class counts.
    for(const [dx,dy] of [[0,0],[1,0],[0,1],[1,1]])worldUi.layout.cells.push({x:area.x+dx,y:area.y+dy,z:.08,value:null,area:area.name,entities:area.region.entities});
  }
  const heading=host.querySelector('.world-heading h2');if(heading)heading.textContent=t('Your services','Deine Services');
  for(const id of ['world-lens-problems','world-lens-complexity','world-lens-coverage','world-kpi','world-kpi-help','world-back'])document.getElementById(id).hidden=true;
  worldUi.detailButton.hidden=true;document.getElementById('world-detail').hidden=true;
  const legend=document.getElementById('world-legend');legend.textContent='';
  legend.appendChild(element('span',t('Service → packages → classes','Service → Pakete → Klassen'),'muted'));
  const path=document.getElementById('world-scope');path.textContent='';path.appendChild(element('strong',rootName));
  document.getElementById('world-status').textContent=t('Select a project to open its World View','Projekt wählen und dessen World View öffnen');
  document.getElementById('world-count').textContent=projects.length+' '+t('services / projects','Services / Projekte')+' · '+new Set(projects.filter(p=>!p.external).map(p=>p.sourcePath||p.path)).size+' '+t('local source projects','lokale Quellprojekte');
  const search=document.getElementById('world-search');search.placeholder=t('Find service or project…','Service oder Projekt suchen…');search.setAttribute('aria-label',search.placeholder);
  document.getElementById('world-canvas').setAttribute('aria-label',t('Service landscape. Click to select connections; double-click or zoom into a service to open packages and classes. Drag to pan, right-drag to rotate.','Service-Landschaft. Klick markiert Verbindungen; Doppelklick oder Hineinzoomen öffnet Pakete und Klassen. Ziehen verschiebt, Rechtsziehen dreht.'));
  const labels=document.getElementById('world-labels');labels.textContent='';
  for(const project of projects){
    const pin=button('',()=>repositorySelectWorld(project.path),'world-pin');pin.setAttribute('data-world-area',project.path);pin.setAttribute('data-world-kind','project');
    pin.appendChild(element('strong',project.displayName||(project.external||project.path==='.'?project.name:project.path.split('/').pop())));pin.appendChild(element('span','›','world-pin-next'));
    pin.appendChild(element('small',project.external?t('No local sources','Keine lokalen Quellen'):serviceProjectKind(project.kind)+' · '+(project.sourcePath||project.path)));
    pin.setAttribute('aria-label',project.name+' · '+project.path+' · '+t('Open project World View','Projekt-World-View öffnen'));
    explainAction(pin,()=>project.name+' · '+project.path+' · '+(project.runtime||project.ecosystem||serviceProjectKind(project.kind)));labels.appendChild(pin);
  }
  const picker=document.getElementById('world-area');picker.textContent='';const placeholder=element('option',t('Open project…','Projekt öffnen…'));placeholder.value='';picker.appendChild(placeholder);
  for(const project of repositoryWorldInventory()){const option=element('option',project.name+' · '+(project.external?t('No local sources','Keine lokalen Quellen'):project.path));option.value=project.path;picker.appendChild(option);}
  picker.value='';picker.setAttribute('aria-label',t('Open project World View','Projekt-World-View öffnen'));
  repositoryWorldControls();worldDetails();worldDraw();return true;
}

const repositoryWorldState={deployment:'all',source:'all',kind:'all',direction:'both',cyclesOnly:false,allProjects:false,dataFlow:false};
function repositoryWorldInventory(){
  const services=serviceModel()?.services||[],projects=repositorySelectableProjects(),known=new Set(projects.map(p=>p.id)),inventory=[];
  for(const project of projects){
    const deployments=services.filter(s=>s.project===project.id);
    if(deployments.length>1){for(const service of deployments)inventory.push({...project,id:service.id,path:'@service/'+encodeURIComponent(service.id),sourcePath:project.path,projectId:project.id,displayName:service.name,name:service.name});}
    else inventory.push({...project,displayName:deployments[0]?.name||null});
  }
  return [...inventory,...services.filter(s=>!known.has(s.project)&&(s.compose_file||/^(trace|aspire):/.test(s.id))).map(s=>({id:s.id,path:'@service/'+encodeURIComponent(s.id),name:s.name,kind:'unknown',ecosystem:'runtime',external:true,manifest:'',evidence:[]}))];
}
function repositoryDeploymentScope(service){return (service.networks||[]).find(n=>n.startsWith('kubernetes:'))||service.compose_file||(service.id.startsWith('aspire:')?service.id.split('::')[0]:'');}
function repositoryDeploymentScopes(){return [...new Set((serviceModel()?.services||[]).map(repositoryDeploymentScope).filter(Boolean))].sort();}
function repositoryWorldProjects(){
  let projects=repositoryWorldInventory();
  if(repositoryWorldState.deployment!=='all'){
    const scoped=new Set((serviceModel()?.services||[]).filter(s=>repositoryDeploymentScope(s)===repositoryWorldState.deployment).flatMap(s=>[s.id,s.project]));
    projects=projects.filter(p=>scoped.has(p.id));
  }
  const deployed=new Set((serviceModel()?.services||[]).filter(s=>s.id.startsWith('aspire:')||s.id.startsWith('spring:')||s.compose_file).flatMap(s=>[s.id,s.project]));
  const any=projects.some(p=>deployed.has(p.id));
  // Function apps deploy themselves through their build plugin: kept beside a deployment, but they filter nothing on their own.
  for(const p of serviceProjects())if(p.runtime==='azure-functions')deployed.add(p.id);
  return repositoryWorldState.allProjects||!any?projects:projects.filter(p=>p.external||deployed.has(p.id));
}
function repositoryEvidenceSource(edge,proof){return proof.file==='.archguard/otel-traces.json'&&proof.pointer?.startsWith('/resourceSpans/')?'observed':['http_call','grpc_call','event'].includes(edge.kind)?'code':'configuration';}
function repositoryWorldConnections(){
  const model=serviceModel(),projects=new Map((repositoryWorldState.deployment==='all'?repositoryWorldInventory():repositoryWorldProjects()).map(p=>[p.id,p.path]));
  const services=new Map((model?.services||[]).map(s=>[s.id,projects.get(s.id)||projects.get(s.project)])),merged=new Map();
  for(const edge of model?.dependencies||[]){const from=services.get(edge.from),to=services.get(edge.to);if(!from||!to||from===to)continue;
    const evidence=(edge.evidence||[]).filter(p=>repositoryWorldState.source==='all'||repositoryEvidenceSource(edge,p)===repositoryWorldState.source);
    if(!evidence.length&&repositoryWorldState.source!=='all')continue;
    const key=JSON.stringify([from,to,edge.kind]);if(!merged.has(key))merged.set(key,{...edge,from,to,evidence:[],serviceFrom:edge.from,serviceTo:edge.to,cyclic:false});merged.get(key).evidence.push(...evidence);
  }
  const edges=[...merged.values()],names=[...new Set(edges.flatMap(e=>[e.from,e.to]))],ids=new Map(names.map((n,i)=>[n,i]));
  for(const kind of new Set(edges.map(e=>e.kind))){const typed=edges.filter(e=>e.kind===kind),components=findSccs({nodes:names.map((name,id)=>({name,id})),edges:typed.map(e=>({from:ids.get(e.from),to:ids.get(e.to)}))});
    for(const edge of typed)edge.cyclic=components.some(c=>c.members.includes(ids.get(edge.from))&&c.members.includes(ids.get(edge.to)));
  }
  return edges;
}
function repositoryVisibleConnections(){
  const selected=worldUi.selection?.area,state=repositoryWorldState;
  const edges=repositoryWorldConnections().filter(e=>(state.kind==='all'?['http_call','grpc_call','event','configured_http','configured_reference'].includes(e.kind):e.kind===state.kind)&&(!state.cyclesOnly||e.cyclic));
  if(!selected)return edges;
  if(state.direction==='downstream'||state.direction==='upstream')return repositoryFlowPath(edges,selected,state.direction==='downstream');
  return edges.filter(e=>state.direction==='incoming'?e.to===selected:state.direction==='outgoing'?e.from===selected:e.from===selected||e.to===selected);
}
// Every hop from the selection along the arrows (downstream) or against them
// (upstream); a cycle back to the selection ends the walk. Only calls and
// events carry data, so configuration references never extend a path.
function repositoryFlowPath(connections,start,downstream){
  const edges=connections.filter(e=>worldFlowCarrier(e.kind)),reached=new Set([start]),queue=[start];
  while(queue.length){const at=queue.shift();for(const e of edges){const [from,to]=downstream?[e.from,e.to]:[e.to,e.from];if(from===at&&!reached.has(to)){reached.add(to);queue.push(to);}}}
  return edges.filter(e=>reached.has(downstream?e.from:e.to));
}
let repositoryLastTap=null;
function repositoryTapWorld(path){
  const now=worldNow(),previous=repositoryLastTap;repositoryLastTap={path,time:now};
  if(previous?.path===path&&now-previous.time<450){repositoryLastTap=null;repositoryOpenWorld(path);return;}
  repositorySelectWorld(path);
}
function repositorySelectWorld(path){
  if(!repositoryWorldInventory().some(p=>p.path===path))return;
  worldUi.selection={area:path};repositoryWorldControls();worldDetails();worldDraw();
}
function repositoryWorldControls(){
  const host=document.getElementById('world-legend');host.textContent='';
  const select=(id,label,values,value,change)=>{const box=element('label',label+' '),input=element('select');input.id=id;input.setAttribute('aria-label',label);for(const [key,text] of values){const option=element('option',text);option.value=key;input.appendChild(option);}input.value=value;input.addEventListener('change',()=>{change(input.value);repositoryWorldControls();worldDetails();worldDraw();});box.appendChild(input);host.appendChild(box);};
  const deployments=repositoryDeploymentScopes();
  if(deployments.length>1)select('world-deployment-scope',t('Deployment','Deployment'),[['all',t('All variants','Alle Varianten')],...deployments.map(s=>[s,s])],repositoryWorldState.deployment,value=>{repositoryWorldState.deployment=value;worldUi.selection=null;worldUi.camera={x:0,y:0,zoom:1,rotation:0};renderRepositoryWorld();});
  select('world-service-source',t('Evidence','Belege'),[['all',t('Combined','Kombiniert')],['code',t('Source code','Quellcode')],['configuration',t('Configuration','Konfiguration')],['observed',t('Observed traces','Beobachtete Traces')]],repositoryWorldState.source,value=>repositoryWorldState.source=value);
  select('world-service-kind',t('Connections','Verbindungen'),[['all',t('Communication','Kommunikation')],...['http_call','grpc_call','event','configured_http','configured_reference','startup'].map(k=>[k,serviceKindLabel(k)])],repositoryWorldState.kind,value=>repositoryWorldState.kind=value);
  select('world-service-direction',t('Direction','Richtung'),[['both',t('Both','Beide')],['incoming',t('Incoming','Eingehend')],['outgoing',t('Outgoing','Ausgehend')],['downstream',t('Where data flows (all hops)','Wohin Daten fließen (alle Stationen)')],['upstream',t('Where data comes from (all hops)','Woher Daten kommen (alle Stationen)')]],repositoryWorldState.direction,value=>repositoryWorldState.direction=value);
  const cycle=button(t('Cycles','Zyklen'),()=>{repositoryWorldState.cyclesOnly=!repositoryWorldState.cyclesOnly;repositoryWorldControls();worldDetails();worldDraw();},'btn btn-secondary');cycle.setAttribute('aria-pressed',String(repositoryWorldState.cyclesOnly));cycle.id='world-service-cycles';host.appendChild(cycle);
  const flow=button(t('Data flow','Datenfluss'),()=>{repositoryWorldState.dataFlow=!repositoryWorldState.dataFlow;repositoryWorldControls();worldDraw();},'btn btn-secondary');flow.setAttribute('aria-pressed',String(repositoryWorldState.dataFlow));flow.id='world-service-flow';flow.disabled=!worldMotion();flow.title=flow.disabled?t('Unavailable while reduced motion is on','Nicht verfügbar, solange reduzierte Bewegung aktiv ist'):t('Animate the direction of calls and events. Configuration references and startup order stay still.','Richtung von Aufrufen und Events animieren. Konfigurationsverweise und Startreihenfolge bleiben unbewegt.');host.appendChild(flow);worldFlowStart();
  if(worldUi.selection){host.appendChild(button(t('Overview','Übersicht'),()=>{worldUi.selection=null;repositoryWorldControls();worldDetails();worldDraw();},'btn btn-secondary'));const open=button(t('Zoom into service','In Service hineinzoomen'),()=>repositoryOpenWorld(worldUi.selection.area),'btn btn-primary');open.id='world-service-open';open.disabled=!!repositoryWorldInventory().find(p=>p.path===worldUi.selection.area)?.external;host.appendChild(open);}
  const scope=button(repositoryWorldState.allProjects?t('Services only','Nur Services'):t('All projects','Alle Projekte'),()=>{repositoryWorldState.allProjects=!repositoryWorldState.allProjects;if(repositoryWorldState.allProjects)repositoryWorldState.deployment='all';worldUi.selection=null;worldUi.camera={x:0,y:0,zoom:1,rotation:0};renderRepositoryWorld();},'btn btn-secondary');scope.id='world-service-scope';host.appendChild(scope);
  const importHelp=element('details'),summary=element('summary',t('Analysis & traces','Analyse & Traces'));importHelp.appendChild(summary);importHelp.appendChild(element('p',t('Save a local OTLP JSON export as .archguard/otel-traces.json in the analyzed repository. Service names link traces to projects; unknown services remain external. Refresh to load. No traces are collected or uploaded.','Einen lokalen OTLP-JSON-Export als .archguard/otel-traces.json im analysierten Repository speichern. Servicenamen ordnen Traces den Projekten zu; unbekannte Services bleiben extern. Zum Laden aktualisieren. Es werden keine Traces gesammelt oder hochgeladen.')));importHelp.appendChild(element('p',t('Coverage: Compose endpoints, Kubernetes workloads/selectors, Aspire references and typed clients, Spring routes/clients/messaging, bounded Go client flow and imported OTLP correlations. Configuration and inferred events are candidates. Templates, secrets, dynamic endpoints and runtime overrides remain unresolved; see Services diagnostics.','Abdeckung: Compose-Endpunkte, Kubernetes-Workloads/Selektoren, Aspire-Referenzen und typisierte Clients, Spring-Routen/Clients/Messaging, begrenzter Go-Datenfluss und importierte OTLP-Korrelationen. Konfiguration und abgeleitete Events sind Kandidaten. Templates, Secrets, dynamische Ziele und Laufzeit-Overrides bleiben unaufgelöst; siehe Diagnose unter Dienste.')));host.appendChild(importHelp);
  const edges=repositoryVisibleConnections(),cycles=new Set(edges.filter(e=>e.cyclic).flatMap(e=>[e.from,e.to]));
  document.getElementById('world-status').textContent=(repositoryWorldInventory().find(p=>p.path===worldUi.selection?.area)?.displayName||repositoryWorldInventory().find(p=>p.path===worldUi.selection?.area)?.name||t('All projects','Alle Projekte'))+' · '+edges.length+' '+t('connections','Verbindungen')+(worldUi.selection&&repositoryWorldState.direction==='downstream'?' · '+t('where the data flows, all hops','wohin die Daten fließen, alle Stationen'):worldUi.selection&&repositoryWorldState.direction==='upstream'?' · '+t('where the data comes from, all hops','woher die Daten kommen, alle Stationen'):'')+(repositoryWorldState.dataFlow?' · '+t('dots show call and event direction, not measured load','Punkte zeigen Aufruf- und Event-Richtung, keine gemessene Last'):'')+(edges.length>60?' · '+t('first 60 arrows; select a service to focus','erste 60 Pfeile; Service auswählen zum Eingrenzen'):'')+(cycles.size?' · '+t('cyclic communication marked','zyklische Kommunikation markiert'):'')+(!serviceModel()?.complete?' · '+t('Analysis incomplete; missing connections may be undetected','Analyse unvollständig; fehlende Verbindungen können unerkannt sein'):'')+(repositoryWorldState.source==='observed'&&!edges.length?' · '+t('No usable trace connections in this selection','Keine auswertbaren Trace-Verbindungen in dieser Auswahl'):'');
  const cyclic=new Set(edges.filter(e=>e.cyclic).flatMap(e=>[e.from,e.to]));for(const area of worldUi.model?.areas||[])area.cyclic=cyclic.has(area.name);
  worldUi.detailButton.hidden=false;worldUi.detailButton.disabled=!worldUi.selection;worldUi.detailButton.setAttribute('aria-expanded',String(!!worldUi.selection&&!!worldUi.detailsOpen));
}
function repositoryWorldDetails(panel){
  const path=worldUi.selection?.area,project=repositoryWorldInventory().find(p=>p.path===path);
  panel.hidden=!project||!worldUi.detailsOpen;if(panel.hidden)return;
  panel.setAttribute('role','dialog');panel.setAttribute('aria-modal','false');panel.setAttribute('aria-label',t('Service details','Service-Details'));panel.setAttribute('tabindex','-1');worldDetailPosition();
  const title=element('div',undefined,'world-detail-title');title.appendChild(element('strong',project.name));const close=button('×',()=>setWorldDetailsOpen(false,true),'btn btn-secondary');close.setAttribute('aria-label',t('Close details (Esc)','Details schließen (Esc)'));title.appendChild(close);panel.appendChild(title);
  if(!project.external)panel.appendChild(button(t('Zoom into service','In Service hineinzoomen'),()=>repositoryOpenWorld(path),'btn btn-primary'));
  else panel.appendChild(element('p',t('No local source project is associated with this service. Connections and trace evidence remain available.','Diesem Service ist kein lokales Quellprojekt zugeordnet. Verbindungen und Trace-Belege sind verfügbar.'),'muted'));
  panel.appendChild(element('p',t('Evidence is filtered by source. Traces represent only the imported observation window, not complete traffic. Events can form intended workflows; a combined cycle may mix configured, inferred and observed edges.','Belege werden nach Quelle gefiltert. Traces zeigen nur das importierte Zeitfenster, nicht den gesamten Verkehr. Event-Zyklen können gewollte Abläufe sein; kombinierte Zyklen können konfigurierte, abgeleitete und beobachtete Kanten mischen.'),'muted'));
  const edges=repositoryVisibleConnections();
  const labels=new Map(repositoryWorldInventory().map(p=>[p.path,p.displayName||p.name]));
  for(const edge of edges){const item=element('details'),title=element('summary',(labels.get(edge.from)||edge.from)+' → '+(labels.get(edge.to)||edge.to)+' · '+serviceKindLabel(edge.kind)+(edge.cyclic?' · '+t('Cycle','Zyklus'):''));item.appendChild(title);
    for(const proof of edge.evidence||[]){item.appendChild(element('strong',serviceEvidenceLocation(proof)));item.appendChild(element('p',proof.description));}
    const other=edge.from===path?edge.to:edge.from;item.appendChild(button(t('Select neighbor','Nachbarn auswählen'),()=>repositorySelectWorld(other),'btn btn-secondary'));panel.appendChild(item);
  }
  if(!edges.length)panel.appendChild(element('p',t('No connections found for these filters. Missing evidence does not prove isolation.','Für diese Filter wurden keine Verbindungen erkannt. Fehlende Belege beweisen keine Isolation.'),'muted'));
}
