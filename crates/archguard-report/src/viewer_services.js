'use strict';
// Project/deployment topology is independent from import analysis and simulated SimOps.
const serviceUi={kind:'configured_http',mode:'all',routing:'ordered',selected:null,project:null,projectLimit:100,edge:null,cyclesOnly:false,cycle:null,zoom:1,positionKey:'',viewportWidth:0};
let serviceRoutingControl=null;
function serviceKindLabel(kind){const labels={http_call:t('HTTP client','HTTP-Client'),grpc_call:t('gRPC client','gRPC-Client'),event:t('Event flow (inferred)','Event-Fluss (abgeleitet)'),configured_reference:t('Configuration reference','Konfigurationsverweis')};if(labels[kind])return labels[kind];return kind==='configured_http'?t('Configured HTTP route / endpoint','Konfigurierte HTTP-Route / Endpunkt'):kind==='startup'?t('Startup dependency (depends_on)','Startabhängigkeit (depends_on)'):kind;}
function serviceKey(edge){return JSON.stringify([edge.from,edge.to,edge.kind]);}
function serviceModel(){return data.services&&Array.isArray(data.services.services)?data.services:null;}
function serviceProjects(){return serviceModel()?.projects||[];}
function serviceProject(service){return serviceProjects().find(project=>project.id===service?.project)||null;}
function serviceProjectKind(kind){return {application:t('Application','Anwendung'),library:t('Library','Bibliothek'),tooling:t('Test / tooling','Tests / Werkzeuge'),workspace:t('Workspace','Workspace'),unknown:t('Unclassified project','Nicht klassifiziertes Projekt')}[kind]||t('Unclassified project','Nicht klassifiziertes Projekt');}
function renderServiceProjects(){
  const projects=[...serviceProjects()].sort((a,b)=>compareNames(a.path,b.path)||compareNames(a.manifest,b.manifest)),list=document.getElementById('services-project-list'),focused=document.activeElement?.getAttribute('data-service-project');
  document.getElementById('services-projects').hidden=!projects.length;list.textContent='';
  document.getElementById('services-projects-summary').textContent=t('{count} projects · applications, libraries and tools','{count} Projekte · Anwendungen, Bibliotheken und Werkzeuge',{count:projects.length});
  document.getElementById('services-projects-note').textContent=t('Manifest inventory. Only identified applications become service nodes; shared names and technologies do not establish connections.','Manifestinventar. Nur erkannte Anwendungen werden zu Dienstknoten; gemeinsame Namen und Technologien belegen keine Beziehungen.');
  for(const project of projects.slice(0,serviceUi.projectLimit)){
    const row=button('',()=>selectServiceProject(project.id),'btn btn-quiet service-project');row.setAttribute('data-service-project',project.id);row.setAttribute('aria-pressed',String(serviceUi.project===project.id));
    row.appendChild(element('span',project.path,'mono'));row.appendChild(element('strong',project.name));row.appendChild(element('span',serviceProjectKind(project.kind)+' · '+project.ecosystem,'muted'));list.appendChild(row);
  }
  document.getElementById('services-project-more').hidden=projects.length<=serviceUi.projectLimit;
  if(focused)[...list.querySelectorAll('button[data-service-project]')].find(row=>row.getAttribute('data-service-project')===focused)?.focus({preventScroll:true});
}
function selectServiceProject(id){
  serviceUi.project=id;serviceUi.edge=null;serviceUi.cycle=null;
  const matches=(serviceModel()?.services||[]).filter(service=>service.project===id);
  serviceUi.selected=matches.length===1?matches[0].id:null;
  if(serviceUi.selected&&serviceUi.mode==='all')serviceUi.mode='both';
  renderServices();
}
function appendServiceProjectEvidence(panel,project){
  appendRepositoryProjectNavigation(panel,project);
  panel.appendChild(element('p',serviceProjectKind(project.kind)+' · '+project.ecosystem,'muted'));
  panel.appendChild(element('p',t('Project path: {path}','Projektpfad: {path}',{path:project.path}),'mono'));
  panel.appendChild(element('p',t('Manifest: {file}','Manifest: {file}',{file:project.manifest}),'mono'));
  if(project.runtime)panel.appendChild(element('p',t('Runtime: {runtime}','Laufzeit: {runtime}',{runtime:project.runtime})));
  if(project.evidence?.length)panel.appendChild(element('h3',t('Project evidence (original wording)','Projektbelege (Originaltext)')));
  for(const item of project.evidence||[]){const box=element('div',undefined,'service-evidence');box.appendChild(element('strong',serviceEvidenceLocation(item)));box.appendChild(element('p',item.description));panel.appendChild(box);}
}
function serviceName(id){return serviceModel()?.services.find(s=>s.id===id)?.name||id;}
function serviceCycles(){return (serviceModel()?.cycles||[]).filter(c=>c.kind===serviceUi.kind);}
function serviceEdgeInCycle(edge){return serviceCycles().some(c=>c.nodes.includes(edge.from)&&c.nodes.includes(edge.to));}
function serviceDependencies(){return (serviceModel()?.dependencies||[]).filter(e=>e.kind===serviceUi.kind);}
function serviceMatches(edge){
  if(serviceUi.cyclesOnly&&!serviceEdgeInCycle(edge))return false;
  if(serviceUi.cycle&&(!serviceUi.cycle.includes(edge.from)||!serviceUi.cycle.includes(edge.to)))return false;
  if(serviceUi.mode==='all'||!serviceUi.selected)return true;
  const incoming=edge.to===serviceUi.selected,outgoing=edge.from===serviceUi.selected;
  return serviceUi.mode==='incoming'?incoming:serviceUi.mode==='outgoing'?outgoing:incoming||outgoing;
}
function serviceColor(edge,side){
  if(side==='right'&&serviceEdgeInCycle(edge))return C.cycle;
  if(edge.to===serviceUi.selected)return C.incoming;
  if(edge.from===serviceUi.selected)return C.outgoing;
  return C.edge;
}
function hideServiceTooltip(){document.getElementById('services-tooltip').hidden=true;}
function serviceTooltip(target,lines){
  target.appendChild(treeSvg('title',{},lines.join('\n')));
  function show(event){
    const tip=document.getElementById('services-tooltip');tip.textContent='';tip.appendChild(tooltipEl(lines));tip.hidden=false;
    const bounds=target.getBoundingClientRect(),box=tip.getBoundingClientRect();
    const x=event.clientX??bounds.right??bounds.left??0,y=event.clientY??bounds.top??0;
    tip.style.left=Math.max(8,Math.min(x+12,window.innerWidth-box.width-8))+'px';
    tip.style.top=Math.max(8,Math.min(y+12,(window.innerHeight||600)-box.height-8))+'px';
  }
  target.addEventListener('pointerenter',show);target.addEventListener('focus',show);
  target.addEventListener('pointerleave',hideServiceTooltip);target.addEventListener('blur',hideServiceTooltip);
}
function serviceEvidenceLocation(evidence){return evidence.file+(evidence.line?':'+evidence.line:'')+(evidence.pointer?' · '+evidence.pointer:'');}
function renderServiceEvidence(){
  const panel=document.getElementById('sidebar');panel.textContent='';
  const model=serviceModel();
  if(!model){panel.appendChild(element('p',t('Service topology was not included in this report.','Dieser Bericht enthält keine Diensttopologie.'),'empty'));return;}
  const edge=model.dependencies.find(e=>serviceKey(e)===serviceUi.edge);
  if(edge){
    panel.appendChild(element('h2',serviceName(edge.from)+' → '+serviceName(edge.to)));
    panel.appendChild(element('p',serviceKindLabel(edge.kind),'muted'));
    panel.appendChild(element('p',edge.kind!=='startup'&&edge.kind!=='configured_http'?t('Static source candidate, not an observed runtime call.','Statischer Quellkandidat, kein beobachteter Laufzeitaufruf.'):edge.kind==='startup'?t('Declared startup order. This does not establish an API call.','Deklarierte Startreihenfolge. Dies belegt keinen API-Aufruf.'):t('A literal HTTP(S) destination in Compose configuration. This is not evidence of an executed runtime call.','Ein wörtliches HTTP(S)-Ziel in der Compose-Konfiguration. Dies ist kein Beleg für einen ausgeführten Aufruf zur Laufzeit.'),'muted'));
    const connectionNodes=[...new Set([edge.from,edge.to])].map(id=>({id,name:serviceName(id)})),connectionEdges=model.dependencies.filter(e=>e.kind===edge.kind&&connectionNodes.some(n=>n.id===e.from)&&connectionNodes.some(n=>n.id===e.to));
    if(new Set(connectionNodes.map(n=>n.name)).size!==connectionNodes.length)for(const n of connectionNodes)n.name=n.id;
    renderDetailGraph(panel,{nodes:connectionNodes,edges:connectionEdges,label:serviceKindLabel(edge.kind),onNode:n=>selectService(n.id),onEdge:selectServiceEdge});
    if(edge.evidence?.length)panel.appendChild(element('h3',t('Source evidence (original wording)','Quellenbelege (Originaltext)')));
    for(const item of edge.evidence||[]){const box=element('div',undefined,'service-evidence');box.appendChild(element('strong',serviceEvidenceLocation(item)));box.appendChild(element('p',item.description));panel.appendChild(box);}
    if(!(edge.evidence||[]).length)panel.appendChild(element('p',t('No source evidence was included for this connection.','Für diese Beziehung wurden keine Quellenbelege mitgeliefert.'),'muted'));
  }else{
    const selected=model.services.find(s=>s.id===serviceUi.selected),project=serviceProjects().find(p=>p.id===serviceUi.project)||serviceProject(selected);
    if(selected||project){
      panel.appendChild(element('h2',project?.name||selected.name));
      if(project)appendServiceProjectEvidence(panel,project);
      if(!selected){
        const deployments=model.services.filter(service=>service.project===project.id);
        panel.appendChild(element('p',deployments.length?t('Select a deployment to inspect its connections.','Eine Bereitstellung auswählen, um ihre Beziehungen zu prüfen.'):t('No service declaration is associated with this project. Libraries and test tools remain project inventory.','Diesem Projekt ist keine Dienstdeklaration zugeordnet. Bibliotheken und Testwerkzeuge bleiben im Projektinventar.'),'muted'));
        for(const deployment of deployments)panel.appendChild(button(deployment.name,()=>selectService(deployment.id),'btn btn-quiet service-connection'));
        return;
      }
      if(selected.compose_file)panel.appendChild(element('p',t('Compose service · {file}','Compose-Dienst · {file}',{file:selected.compose_file}),'muted'));
      if(selected.compose_file)panel.appendChild(element('p',selected.build_context?t('Local build context: {path}','Lokaler Build-Kontext: {path}',{path:selected.build_context}):t('No local build context identified.','Kein lokaler Build-Kontext erkannt.')));
      if(selected.compose_file)panel.appendChild(element('p',t('Networks: {names}','Netzwerke: {names}',{names:selected.networks.join(', ')||t('none reported','keine angegeben')}),'muted'));
      if(selected.profiles.length)panel.appendChild(element('p',t('Profiles: {names}','Profile: {names}',{names:selected.profiles.join(', ')}),'muted'));
      const edges=serviceDependencies().filter(e=>e.from===selected.id||e.to===selected.id);
      panel.appendChild(element('h3',t('{kind} connections','Beziehungen: {kind}',{kind:serviceKindLabel(serviceUi.kind)})));
      for(const edge of edges){const b=button(serviceName(edge.from)+' → '+serviceName(edge.to),()=>selectServiceEdge(edge),'btn btn-quiet service-connection');b.setAttribute('data-service-evidence',serviceKey(edge));panel.appendChild(b);}
      if(!edges.length)panel.appendChild(element('p',t('No connections of this kind were found for this service.','Für diesen Dienst wurden keine Beziehungen dieser Art gefunden.'),'muted'));
    }else{
      panel.appendChild(element('h2',t('Service configuration','Dienstkonfiguration')));
      panel.appendChild(element('p',t('Select a service to focus its connections. Select an arrow to inspect its source evidence.','Einen Dienst auswählen, um seine Beziehungen zu sehen. Einen Pfeil auswählen, um seine Quellenbelege zu untersuchen.')));
      panel.appendChild(element('p',t('API configuration and startup dependencies are analyzed separately. Import metrics and manual simulation describe source modules.','API-Konfiguration und Startabhängigkeiten werden getrennt analysiert. Importmetriken und manuelle Simulation beschreiben Quellmodule.'),'muted'));
    }
  }
}
function selectService(id){
  const restoreFocus=selectionFocusRestorer();
  serviceUi.selected=id;serviceUi.project=serviceModel()?.services.find(service=>service.id===id)?.project||null;serviceUi.edge=null;serviceUi.cycle=null;
  if(serviceUi.mode==='all')serviceUi.mode='both';
  renderServices();restoreFocus();
}
function selectServiceEdge(edge){const restoreFocus=selectionFocusRestorer();serviceUi.edge=serviceKey(edge);renderServices();restoreFocus();}
function setServiceZoom(value){
  const tree=document.getElementById('services-tree'),svg=tree.querySelector('svg');if(!svg||!Number.isFinite(value))return;
  const zoom=Math.max(.01,Math.min(3,value)),box=tree.getBoundingClientRect(),width=tree.clientWidth||box.width,height=tree.clientHeight||box.height,ratio=zoom/serviceUi.zoom;
  const left=Math.max(0,((tree.scrollLeft||0)+width/2)*ratio-width/2),top=Math.max(0,((tree.scrollTop||0)+height/2)*ratio-height/2);
  const dimensions=svg.getAttribute('viewBox').split(' ').map(Number);serviceUi.zoom=zoom;
  svg.setAttribute('width',dimensions[2]*zoom);svg.setAttribute('height',dimensions[3]*zoom);tree.scrollLeft=left;tree.scrollTop=top;
  document.getElementById('services-scale').textContent=Math.round(zoom*100)+'%';hideServiceTooltip();
}
function fitServices(){
  const tree=document.getElementById('services-tree'),svg=tree.querySelector('svg');if(!svg)return;
  const dimensions=svg.getAttribute('viewBox').split(' ').map(Number),box=tree.getBoundingClientRect();
  setServiceZoom(Math.min(Math.max(1,(tree.clientWidth||box.width)-24)/dimensions[2],Math.max(1,(tree.clientHeight||box.height)-24)/dimensions[3]));tree.scrollLeft=0;tree.scrollTop=0;
}
function renderServices(){
  renderWorkspace();
  renderApiContracts();
  const patterns=document.getElementById('service-patterns');if(patterns&&typeof renderPatternAssessments==='function'){patterns.textContent='';renderPatternAssessments(patterns,true);}
  hideServiceTooltip();syncServiceLanguage();renderServiceProjects();
  const model=serviceModel(),tree=document.getElementById('services-tree'),status=document.getElementById('services-status');
  const viewportWidth=treeAvailableWidth(tree),resized=viewportWidth!==serviceUi.viewportWidth;serviceUi.viewportWidth=viewportWidth;
  const left=tree.scrollLeft||0,top=tree.scrollTop||0,focused=document.activeElement?.getAttribute('data-service-node');tree.textContent='';
  document.getElementById('services-kind').value=serviceUi.kind;document.getElementById('services-direction').value=serviceUi.mode;
  document.getElementById('services-direction').disabled=!serviceUi.selected;
  if(serviceRoutingControl)serviceRoutingControl.value=serviceUi.routing;
  document.getElementById('services-cycles-only').setAttribute('aria-pressed',String(serviceUi.cyclesOnly));
  document.getElementById('services-scale').textContent=Math.round(serviceUi.zoom*100)+'%';
  const diagnostics=document.getElementById('services-diagnostics');diagnostics.textContent='';
  const cycleBox=document.getElementById('services-cycles');cycleBox.textContent='';
  const limits=document.getElementById('services-limitations');
  limits.textContent=serviceUi.kind==='startup'?t('Startup dependencies describe depends_on order, not API calls. Cycles are computed only among startup edges.','Startabhängigkeiten beschreiben die Reihenfolge aus depends_on, keine API-Aufrufe. Zyklen werden nur unter Startabhängigkeiten berechnet.'):t('HTTP(S) endpoints found in Compose configuration are potential service dependencies, not observed runtime calls. Cycles use only these configured HTTP edges.','HTTP(S)-Endpunkte aus der Compose-Konfiguration sind potenzielle Dienstabhängigkeiten, keine beobachteten Laufzeitaufrufe. Zyklen verwenden nur diese konfigurierten HTTP-Kanten.');
  const reference=document.getElementById('services-reference');
  const routing=serviceUi.routing==='ordered'?t('Left arcs go down, right arcs go up. Incoming blue, outgoing violet.','Linke Bögen führen abwärts, rechte aufwärts. Eingehend blau, ausgehend violett.'):serviceUi.selected?t('In/Out: Incoming left (blue), outgoing right (violet).','Ein/Aus: Eingehend links (blau), ausgehend rechts (violett).'):t('In/Out: select a service for incoming / outgoing sides. Context connections stay left.','Ein/Aus: Einen Dienst für eingehende / ausgehende Seiten auswählen. Kontextbeziehungen bleiben links.');
  reference.textContent=(serviceUi.selected?t('Reference: {name} · ','Bezug: {name} · ',{name:serviceName(serviceUi.selected)}):'')+routing+' '+treeCycleColorHelp();
  if(!model){status.textContent=t('Service topology was not included in this report. Generate a new report to discover repository applications and Docker Compose services.','Dieser Bericht enthält keine Diensttopologie. Einen neuen Bericht erstellen, um Repository-Anwendungen und Docker-Compose-Dienste zu erkennen.');renderServiceEvidence();return;}
  if(model.diagnostics?.length)diagnostics.appendChild(element('p',t('Diagnostic messages (original wording)','Diagnosehinweise (Originaltext)'),'muted'));
  for(const item of model.diagnostics||[])diagnostics.appendChild(element('p',item.file+(item.service?' · '+item.service:'')+': '+item.message,'muted'));
  if(!model.compose_files.length&&!model.services.length&&!serviceProjects().length){status.textContent=t('No supported project manifests or Docker Compose files found.','Keine unterstützten Projektmanifeste oder Docker-Compose-Dateien gefunden.')+(model.complete?'':t(' Analysis incomplete – missing connections do not establish isolation; see diagnostics.',' Analyse unvollständig – fehlende Verbindungen sind kein Beleg für Isolation; siehe Diagnosehinweise.'));renderServiceEvidence();return;}
  const all=serviceDependencies(),shown=all.filter(serviceMatches),cycles=serviceCycles();
  if(!['startup','configured_http'].includes(serviceUi.kind))limits.textContent=t('Static source candidates; execution is not proven. Event cycles can represent intended workflows. Aspire references are configuration, not calls.','Statische Quellkandidaten; Ausführung ist nicht belegt. Event-Zyklen können gewollte Abläufe darstellen. Aspire-Verweise sind Konfiguration, keine Aufrufe.');
  const projectSummary=serviceProjects().length?t('{projects} projects · {apps} applications · {containers} containers · ','{projects} Projekte · {apps} Anwendungen · {containers} Container · ',{projects:serviceProjects().length,apps:serviceProjects().filter(project=>project.kind==='application').length,containers:model.services.filter(service=>service.compose_file).length}):t('{services} services in {files} Compose files · ','{services} Dienste in {files} Compose-Dateien · ',{services:model.services.length,files:model.compose_files.length});
  status.textContent=projectSummary+t('{shown} / {all} connections shown · {cycles} cyclic groups · ','{shown} / {all} Beziehungen angezeigt · {cycles} zyklische Gruppen · ',{shown:shown.length,all:all.length,cycles:cycles.length})+(model.monorepo?t('Monorepo detected: {reason}','Monorepo erkannt: {reason}',{reason:model.monorepo_reason}):t('Monorepo not established: {reason}','Monorepo nicht belegt: {reason}',{reason:model.monorepo_reason}))+(model.complete?'':t(' · Analysis incomplete – missing connections do not establish isolation',' · Analyse unvollständig – fehlende Verbindungen sind kein Beleg für Isolation'));
  for(const cycle of cycles){
    const b=button(t('Cyclic group: {names}','Zyklische Gruppe: {names}',{names:cycle.nodes.map(serviceName).join(', ')}),()=>{serviceUi.cycle=[...cycle.nodes];serviceUi.cyclesOnly=true;serviceUi.selected=null;serviceUi.project=null;serviceUi.mode='all';serviceUi.edge=null;renderServices();},'btn btn-quiet service-cycle');
    b.title=t('Cyclic group: every service can reach every other through {kind} dependencies. The listed order is not an individual cycle path.','Zyklische Gruppe: Jeder Dienst kann jeden anderen über {kind} erreichen. Die angegebene Reihenfolge ist kein einzelner Zykluspfad.',{kind:serviceUi.kind==='startup'?t('startup','Startabhängigkeiten'):t('configured HTTP','konfigurierte HTTP-Abhängigkeiten')});cycleBox.appendChild(b);
  }
  const groups=new Map(),nodes=[],hierarchy=[];
  for(const service of model.services){
    const project=serviceProject(service),key=project?'project-group:'+project.id:'compose:'+service.compose_file,name=project?project.path:service.compose_file||t('Unmapped service','Nicht zugeordneter Dienst');
    if(!groups.has(key)){groups.set(key,key);nodes.push({src:key,name,kind:'module',context:true});}
    nodes.push({src:service.id,name:service.name,kind:'module',context:false});hierarchy.push({from:groups.get(key),to:service.id});
  }
  const layout=packageTreeLayout({nodes,edges:hierarchy},all.map(e=>({...e,key:serviceKey(e),...(serviceUi.routing==='in-out'?{side:e.from===serviceUi.selected&&e.to!==serviceUi.selected?'right':'left'}:{})})),{width:serviceUi.viewportWidth});
  const positionKey=JSON.stringify(layout.nodes.map(n=>[n.src,n.x,n.y,n.w,n.h])),layoutChanged=positionKey!==serviceUi.positionKey;serviceUi.positionKey=positionKey;
  if(!layout.nodes.length){tree.appendChild(element('p',t('No service declarations were found.','Keine Dienstdeklarationen gefunden.'),'empty'));renderServiceEvidence();return;}
  const svg=treeSvg('svg',{width:layout.width*serviceUi.zoom,height:layout.height*serviceUi.zoom,viewBox:'0 0 '+layout.width+' '+layout.height,role:'group','aria-label':t('Applications, deployments and directed configuration dependencies','Anwendungen, Bereitstellungen und gerichtete Konfigurationsabhängigkeiten')});
  for(const edge of layout.hierarchy)svg.appendChild(treeSvg('path',{d:roundedTreePath(edge.points,6),class:'package-tree-hierarchy','aria-hidden':'true'}));
  const routes=new Map(layout.routes.map(r=>[r.key,r]));
  for(const edge of shown){
    const key=serviceKey(edge),route=routes.get(key);if(!route)continue;
    const group=treeSvg('g',{'data-service-edge':key,'data-service-side':route.side,class:'package-tree-edge'+(serviceUi.edge===key?' is-selected':''),tabindex:0,role:'button','aria-label':serviceName(edge.from)+' → '+serviceName(edge.to)+' · '+serviceKindLabel(edge.kind)});
    const d=route.path,color=serviceColor(edge,route.side);
    group.appendChild(treeSvg('path',{d,fill:'none',stroke:'transparent','stroke-width':8,'pointer-events':'stroke'}));
    group.appendChild(treeSvg('path',{d,fill:'none',stroke:color,'stroke-width':2,class:'tree-edge-line','data-service-line':'true','pointer-events':'none',...(edge.kind==='startup'?{'stroke-dasharray':'6 3'}:{})}));
    const tip=route.points.at(-1),prior=route.points.at(-2),angle=Math.atan2(tip.y-prior.y,tip.x-prior.x)*180/Math.PI;
    group.appendChild(treeSvg('polygon',{points:'0,0 -9,-4.5 -9,4.5',fill:color,transform:'translate('+tip.x+' '+tip.y+') rotate('+angle+')','data-service-arrow':key,'pointer-events':'none'}));
    serviceTooltip(group,[serviceName(edge.from)+' → '+serviceName(edge.to),serviceKindLabel(edge.kind),...(serviceEdgeInCycle(edge)?[treeCycleDescription(route.side,serviceUi.routing==='in-out')]:[]),...edge.evidence.slice(0,10).map(r=>serviceEvidenceLocation(r)+' · '+r.description)]);
    group.addEventListener('click',()=>selectServiceEdge(edge));group.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation?.();selectServiceEdge(edge);if(event.key==='Enter')openInspector(true);}});svg.appendChild(group);
  }
  for(const [rowIndex,node] of layout.nodes.entries()){
    const service=model.services.find(s=>s.id===node.src),selected=serviceUi.selected===node.src,inCycle=cycles.some(c=>c.nodes.includes(node.src));
    const left=node.x-node.w/2,iconX=left+node.indent+12;
    const group=treeSvg('g',{transform:'translate(0 '+node.y+')',class:'package-tree-node tree-row'+(node.context?' tree-context':'')+(selected?' is-selected':'')+(inCycle?' in-cycle':''),...(service?{'data-service-node':service.id,tabindex:0,role:'button','aria-label':service.name+' · '+(serviceProject(service)?.path||service.compose_file),'aria-pressed':selected}:{})});
    group.appendChild(treeSvg('rect',{x:left,y:-node.h/2,width:node.w,height:node.h,rx:2,class:'tree-row-background','data-service-row':'true'}));
    const iconPath=node.context?'M -7 -5 H -2 L 0 -3 H 7 V 6 H -7 Z':'M -6 -4 L 0 -7 L 6 -4 V 4 L 0 7 L -6 4 Z M -6 -4 L 0 -1 L 6 -4 M 0 -1 V 7';
    group.appendChild(treeSvg('path',{d:iconPath,transform:'translate('+iconX+' 0)',class:'tree-row-icon','aria-hidden':'true'}));
    const shorten=(text,max)=>text.length>max?text.slice(0,max-1)+'…':text;
    const labelWidth=node.w-node.indent-(service?145:36),labelLength=Math.max(6,Math.floor(labelWidth/7.5));
    const clipId='service-label-'+rowIndex,clip=treeSvg('clipPath',{id:clipId,clipPathUnits:'userSpaceOnUse'});
    clip.appendChild(treeSvg('rect',{x:iconX+14,y:-node.h/2,width:labelWidth,height:node.h}));group.appendChild(clip);
    group.appendChild(treeSvg('text',{x:iconX+14,y:5,class:'tree-node-name','clip-path':'url(#'+clipId+')'},shorten(node.name,labelLength)));
    const relation=treeSvg('g',{transform:'translate('+node.relationX+' 0)',class:'tree-relation-node'+(selected?' is-selected':'')+(inCycle?' in-cycle':''),'aria-hidden':'true',...(service?{'data-service-relation':service.id}:{})});
    relation.appendChild(treeSvg('rect',{x:-8,y:-10,width:16,height:20,rx:3,class:'tree-relation-background','pointer-events':'all'}));
    relation.appendChild(treeSvg('path',{d:iconPath,class:'tree-row-icon'}));group.appendChild(relation);
    if(service){
      const incoming=all.filter(e=>e.to===service.id).length,outgoing=all.filter(e=>e.from===service.id).length;
      group.appendChild(treeSvg('text',{x:left+node.w-108,y:4,fill:C.incoming,class:'tree-direction-count'},t('In {count}','Ein {count}',{count:incoming})));
      group.appendChild(treeSvg('text',{x:left+node.w-12,y:4,fill:C.outgoing,'text-anchor':'end',class:'tree-direction-count'},t('Out {count}','Aus {count}',{count:outgoing})));
      const project=serviceProject(service);
      serviceTooltip(group,[service.name,...(project?[project.path,project.manifest,serviceProjectKind(project.kind),...(project.runtime?[project.runtime]:[])]:[service.compose_file]),...(service.compose_file?[t('Local build context: {path}','Lokaler Build-Kontext: {path}',{path:service.build_context||t('not identified','nicht erkannt')}),t('Networks: {names}','Netzwerke: {names}',{names:service.networks.join(', ')})]:[]),t('{incoming} incoming · {outgoing} outgoing {kind} connections','{incoming} eingehende · {outgoing} ausgehende Beziehungen: {kind}',{incoming,outgoing,kind:serviceKindLabel(serviceUi.kind)})]);
      group.addEventListener('click',()=>selectService(service.id));group.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation?.();selectService(service.id);if(event.key==='Enter')openInspector(true);}});
    }else group.appendChild(treeSvg('title',{},node.name));
    svg.appendChild(group);
  }
  tree.appendChild(svg);tree.scrollTop=top;tree.scrollLeft=left;
  if(layoutChanged&&!resized){const anchor=layout.nodes.find(n=>n.src===serviceUi.selected)||layout.nodes.find(n=>!n.context),width=tree.clientWidth||tree.getBoundingClientRect().width;if(anchor&&width>0&&((anchor.x-anchor.w/2)*serviceUi.zoom<left||(anchor.x+anchor.w/2)*serviceUi.zoom>left+width))tree.scrollLeft=Math.max(0,anchor.x*serviceUi.zoom-width/2);}
  if(focused)[...tree.querySelectorAll('g[data-service-node]')].find(n=>n.getAttribute('data-service-node')===focused)?.focus({preventScroll:true});
  renderServiceEvidence();
}
function syncServiceLanguage(){
  if(!serviceRoutingControl)return;
  const label=document.getElementById('services-toolbar').querySelector('span[data-service-routing-label]');if(label)label.textContent=t('Relations','Beziehungen');
  serviceRoutingControl.setAttribute('aria-label',t('Service relation layout','Anordnung der Dienstbeziehungen'));
  serviceRoutingControl.title=t('Up / down: left downward, right upward. In/Out: incoming left and outgoing right for the selected service.','Auf / ab: links abwärts, rechts aufwärts. Ein/Aus: eingehend links und ausgehend rechts für den ausgewählten Dienst.');
  for(const option of serviceRoutingControl.querySelectorAll('option'))option.textContent=option.value==='in-out'?t('In/Out','Ein/Aus'):t('Up / down','Auf / ab');
}
function initServices(){
  document.getElementById('services-project-more').addEventListener('click',()=>{const first=serviceUi.projectLimit;serviceUi.projectLimit+=100;renderServiceProjects();document.getElementById('services-project-list').querySelectorAll('button[data-service-project]')[first]?.focus({preventScroll:true});});
  const routingLabel=element('label'),caption=element('span');caption.setAttribute('data-service-routing-label','true');routingLabel.appendChild(caption);
  serviceRoutingControl=element('select');serviceRoutingControl.setAttribute('data-service-routing','true');
  for(const value of ['ordered','in-out']){const option=element('option');option.value=value;serviceRoutingControl.appendChild(option);}
  serviceRoutingControl.value=serviceUi.routing;
  serviceRoutingControl.addEventListener('change',()=>{serviceUi.routing=serviceRoutingControl.value==='in-out'?'in-out':'ordered';renderServices();});
  routingLabel.appendChild(serviceRoutingControl);document.getElementById('services-toolbar').appendChild(routingLabel);syncServiceLanguage();
  for(const kind of ['http_call','grpc_call','event','configured_reference']){const option=element('option',serviceKindLabel(kind));option.value=kind;document.getElementById('services-kind').appendChild(option);}
  const dependencies=serviceModel()?.dependencies||[];
  if(!dependencies.some(e=>e.kind==='configured_http'))serviceUi.kind=dependencies.find(e=>['http_call','grpc_call','event','startup'].includes(e.kind))?.kind||'configured_http';
  document.getElementById('services-kind').addEventListener('change',event=>{serviceUi.kind=event.target.value;serviceUi.edge=null;serviceUi.cycle=null;renderServices();});
  document.getElementById('services-direction').addEventListener('change',event=>{serviceUi.mode=event.target.value;renderServices();});
  document.getElementById('services-cycles-only').addEventListener('click',()=>{serviceUi.cyclesOnly=!serviceUi.cyclesOnly;serviceUi.cycle=null;renderServices();});
  document.getElementById('services-reset').addEventListener('click',()=>{serviceUi.selected=null;serviceUi.project=null;serviceUi.edge=null;serviceUi.cycle=null;serviceUi.cyclesOnly=false;serviceUi.mode='all';renderServices();});
  document.getElementById('services-zoom-in').addEventListener('click',()=>setServiceZoom(serviceUi.zoom*1.25));
  document.getElementById('services-zoom-out').addEventListener('click',()=>setServiceZoom(serviceUi.zoom/1.25));
  document.getElementById('services-zoom-reset').addEventListener('click',()=>setServiceZoom(1));
  document.getElementById('services-fit').addEventListener('click',fitServices);
  document.getElementById('services-tree').addEventListener('wheel',event=>{if(!(event.ctrlKey||event.metaKey))return;event.preventDefault();const unit=event.deltaMode===1?16:event.deltaMode===2?600:1;setServiceZoom(serviceUi.zoom*Math.exp(-Math.max(-1000,Math.min(1000,event.deltaY*unit))*.002));},{passive:false});
  document.addEventListener('keydown',event=>{if(event.key==='Escape')hideServiceTooltip();});
}
