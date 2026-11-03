'use strict';
// Small, deterministic diagrams share the report's graph and never infer links.
let detailGraphSequence=0;
function detailLabels(names){
  const parts=names.map(name=>String(name).split('/'));let shared=0;
  while(parts.length&&parts.every(p=>p.length>shared+1&&p[shared]===parts[0][shared]))shared++;
  return {prefix:parts.length?parts[0].slice(0,shared).join('/'):'',labels:new Map(names.map((name,i)=>[name,parts[i].slice(shared).join('/')]))};
}
function detailHeading(container,names){
  const labels=detailLabels(names);container.appendChild(element('h2',names.map(n=>labels.labels.get(n)).join(' → ')));
  if(labels.prefix){const context=element('details',undefined,'context-note');context.appendChild(element('summary',t('Package path','Paketpfad')));context.appendChild(element('p',labels.prefix,'inspector-full-name'));container.appendChild(context);}
}
function renderDetailGraph(container,{nodes,edges,label,onNode,onEdge,focusId,maxNodes=8,maxEdges=16}){
  const shown=nodes.slice(0,maxNodes),ids=new Set(shown.map(n=>n.id)),links=edges.filter(e=>ids.has(e.from)&&ids.has(e.to)).slice(0,maxEdges),labels=detailLabels(shown.map(n=>n.name));
  const figure=element('figure',undefined,'detail-graph');figure.setAttribute('data-detail-graph','true');figure.appendChild(element('figcaption',label));
  if(!shown.length){figure.appendChild(element('p',t('No recorded nodes.','Keine erfassten Knoten.'),'muted'));container.appendChild(figure);return figure;}
  // Condense cycles for the layout only. Rows follow the directed dependency order.
  const groups=findSccs({nodes:shown,edges:links}).map(s=>s.members),covered=new Set(groups.flat());
  for(const n of shown)if(!covered.has(n.id))groups.push([n.id]);
  const groupOf=new Map();groups.forEach((g,i)=>g.forEach(id=>groupOf.set(id,i)));
  const next=groups.map(()=>new Set()),degree=groups.map(()=>0),rank=groups.map(()=>0);
  for(const e of links){const a=groupOf.get(e.from),b=groupOf.get(e.to);if(a!==b&&!next[a].has(b)){next[a].add(b);degree[b]++;}}
  const queue=degree.map((d,i)=>d===0?i:null).filter(i=>i!==null);
  for(let i=0;i<queue.length;i++)for(const child of next[queue[i]]){rank[child]=Math.max(rank[child],rank[queue[i]]+1);if(--degree[child]===0)queue.push(child);}
  const ordered=[...shown].sort((a,b)=>rank[groupOf.get(a.id)]-rank[groupOf.get(b.id)]),indexOf=new Map(ordered.map((n,i)=>[n.id,i]));
  const side=e=>indexOf.get(e.from)<indexOf.get(e.to)?'left':'right';
  const colors={incoming:'var(--c-incoming)',outgoing:'var(--c-outgoing)',cycle:'var(--c-cycle)',dependency:'var(--c-incoming)'};
  const kind=e=>side(e)==='right'&&groupOf.get(e.from)===groupOf.get(e.to)&&(e.from===e.to||groups[groupOf.get(e.from)].length>1)?'cycle':e.from===focusId?'outgoing':e.to===focusId?'incoming':'dependency';
  const legend=element('div',undefined,'detail-graph-legend');
  for(const type of new Set(links.map(kind))){const item=element('span',({incoming:t('Incoming','Eingehend'),outgoing:t('Outgoing','Ausgehend'),cycle:t('Cycle','Zyklus'),dependency:t('Dependency','Abhängigkeit')})[type]);item.style.color=colors[type];legend.appendChild(item);}if(links.length)figure.appendChild(legend);
  if(labels.prefix)figure.appendChild(element('p',labels.prefix,'detail-graph-prefix'));
  if(links.length)figure.appendChild(element('p',t('Left ↓ · Right ↑','Links ↓ · Rechts ↑'),'detail-graph-routing'));
  const board=element('div',undefined,'detail-graph-board'),rows=element('div',undefined,'detail-graph-rows');
  board.setAttribute('data-detail-relations',String(links.length>0));rows.setAttribute('data-detail-rows','true');figure.appendChild(board);
  const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');board.appendChild(svg);board.appendChild(rows);
  const svgNode=(tag,attrs)=>{const n=document.createElementNS('http://www.w3.org/2000/svg',tag);for(const [k,v]of Object.entries(attrs))n.setAttribute(k,String(v));return n;};
  const markerId='detail-arrow-'+(++detailGraphSequence),defs=svgNode('defs',{});svg.appendChild(defs);
  for(const type of new Set(links.map(kind))){const marker=svgNode('marker',{id:markerId+'-'+type,viewBox:'0 0 10 10',refX:9,refY:5,markerWidth:6,markerHeight:6,markerUnits:'userSpaceOnUse',orient:'auto'});marker.appendChild(svgNode('path',{d:'M 0 0 L 10 5 L 0 10 z',fill:colors[type]}));defs.appendChild(marker);}
  const controls=new Map(),mirrors=new Map();
  const iconPath=n=>n.kind==='module'||n.kind==='project'?'M 3 5 H 8 L 10 7 H 17 V 16 H 3 Z':n.kind==='file'?'M 5 2 H 12 L 16 6 V 18 H 5 Z M 12 2 V 6 H 16':'M 4 4 H 16 V 16 H 4 Z';
  for(const n of ordered){
    const relative=labels.labels.get(n.name),parts=relative.split('/'),base=parts.pop(),parent=parts.join('/'),control=onNode?button('',()=>onNode(n),'detail-graph-node'):element('span',undefined,'detail-graph-node');
    control.setAttribute('data-detail-node',String(n.id));control.setAttribute('data-detail-kind',n.kind||'node');control.setAttribute('data-detail-focus',String(n.id===focusId));control.setAttribute('aria-label',n.name);control.setAttribute('title',n.name);
    const icon=svgNode('svg',{viewBox:'0 0 20 20','aria-hidden':'true','data-detail-icon':'true'});icon.appendChild(svgNode('path',{d:iconPath(n),class:'tree-row-icon'}));control.appendChild(icon);
    const text=element('span',undefined,'detail-graph-label');text.setAttribute('data-detail-label','true');text.appendChild(element('strong',base));if(parent)text.appendChild(element('small',parent));control.appendChild(text);rows.appendChild(control);controls.set(n.id,control);
  }
  // Mirror the main tree: labels stay separate from a shared relation axis.
  // Downward dependencies curve left, upward returns (and self-loops) right.
  const routes=links.map((edge,index)=>({edge,index,side:side(edge)})).sort((a,b)=>Math.abs(indexOf.get(a.edge.from)-indexOf.get(a.edge.to))-Math.abs(indexOf.get(b.edge.from)-indexOf.get(b.edge.to))||a.index-b.index);
  const axis=svgNode('line',{class:'tree-relation-axis','aria-hidden':'true'});svg.appendChild(axis);
  for(const r of routes){
    const e=r.edge,type=kind(e),from=shown.find(n=>n.id===e.from),to=shown.find(n=>n.id===e.to),group=svgNode('g',{class:'detail-graph-edge','data-detail-edge-control':JSON.stringify([e.from,e.to]),'aria-label':from.name+' → '+to.name,...(onEdge?{tabindex:0,role:'button'}:{})});
    group.appendChild(svgNode('title',{}));group.querySelector('title').textContent=from.name+' → '+to.name;
    r.hit=svgNode('path',{class:'detail-graph-hit'});r.line=svgNode('path',{class:'detail-graph-link',stroke:colors[type],'vector-effect':'non-scaling-stroke','marker-end':'url(#'+markerId+'-'+type+')','data-detail-edge':JSON.stringify([e.from,e.to]),'data-detail-side':r.side});group.appendChild(r.hit);group.appendChild(r.line);
    if(onEdge){group.addEventListener('click',()=>onEdge(e));group.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation();onEdge(e);}});}
    const highlight=active=>{svg.classList.toggle('edge-hover',active);group.classList.toggle('is-hovered',active);for(const id of [e.from,e.to]){controls.get(id).classList.toggle('is-related',active);mirrors.get(id)?.classList.toggle('is-related',active);}};group.addEventListener('pointerenter',()=>highlight(true));group.addEventListener('pointerleave',()=>highlight(false));group.addEventListener('focus',()=>highlight(true));group.addEventListener('blur',()=>highlight(false));svg.appendChild(group);
  }
  if(links.length)for(const n of ordered){
    const mirror=svgNode('g',{class:'detail-graph-relation','data-detail-relation':String(n.id),'data-detail-kind':n.kind||'node','data-detail-focus':String(n.id===focusId),...(onNode?{tabindex:0,role:'button','aria-label':n.name}:{'aria-hidden':'true'})});
    mirror.appendChild(svgNode('rect',{x:-8,y:-10,width:16,height:20,rx:3}));mirror.appendChild(svgNode('path',{d:iconPath(n),transform:'translate(-10 -10)',class:'tree-row-icon'}));svg.appendChild(mirror);mirrors.set(n.id,mirror);
    if(onNode){mirror.addEventListener('click',()=>onNode(n));mirror.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation();onNode(n);}});}
    const highlight=active=>{controls.get(n.id).classList.toggle('is-related',active);mirror.classList.toggle('is-related',active);};
    for(const control of [controls.get(n.id),mirror]){control.addEventListener('pointerenter',()=>highlight(true));control.addEventListener('pointerleave',()=>highlight(false));control.addEventListener('focus',()=>highlight(true));control.addEventListener('blur',()=>highlight(false));}
  }
  function draw(){
    const bounds=board.getBoundingClientRect(),width=board.clientWidth||bounds.width||280,positions=new Map();
    for(const [i,n]of ordered.entries()){const rect=controls.get(n.id).getBoundingClientRect(),measured=Number.isFinite(rect.top)&&Number.isFinite(bounds.top)&&rect.height>0;positions.set(n.id,measured?{right:rect.right-bounds.left-(board.clientLeft||0),top:rect.top-bounds.top-(board.clientTop||0),height:rect.height}:{right:10+(width-20)*(links.length?0.54:1),top:10+i*62,height:52});}
    const height=board.clientHeight||Math.max(...[...positions.values()].map(p=>p.top+p.height))+10;svg.setAttribute('viewBox','0 0 '+width+' '+height);svg.setAttribute('width',width);svg.setAttribute('height',height);
    const labelRight=Math.max(...[...positions.values()].map(p=>p.right)),gutter=Math.max(16,(width-labelRight-24)/2),relationX=labelRight+12+gutter;
    for(const [key,value]of Object.entries({x1:relationX,x2:relationX,y1:10,y2:height-10}))axis.setAttribute(key,value);axis.style.display=links.length?'':'none';
    const center=id=>{const p=positions.get(id);return p.top+p.height/2;};
    for(const [id,mirror]of mirrors)mirror.setAttribute('transform',`translate(${relationX} ${center(id)})`);
    routes.forEach((r,lane)=>{
      const sign=r.side==='left'?-1:1,y1=center(r.edge.from)+3,y2=center(r.edge.to)-3,reach=Math.min(gutter,18+Math.sqrt(Math.abs(y2-y1))*4+lane*2),x=relationX+sign*9,lead=relationX+sign*12,outer=relationX+sign*reach;
      const d=`M ${x} ${y1} L ${lead} ${y1} C ${outer} ${y1} ${outer} ${y2} ${lead} ${y2} L ${x} ${y2}`;r.line.setAttribute('d',d);r.hit.setAttribute('d',d);
    });
  }
  const connections=element('div',undefined,'detail-graph-connections');connections.setAttribute('data-detail-connections','true');
  for(const e of links){const from=shown.find(n=>n.id===e.from),to=shown.find(n=>n.id===e.to),text=labels.labels.get(from.name)+' → '+labels.labels.get(to.name),control=onEdge?button(text,()=>onEdge(e),'btn btn-quiet'):element('span',text);control.setAttribute('data-detail-connection','true');control.setAttribute('aria-label',from.name+' → '+to.name);connections.appendChild(control);}
  if(links.length){const detail=element('details',undefined,'detail-graph-evidence');detail.setAttribute('data-detail-connections-panel','true');detail.appendChild(element('summary',plural(links.length,'{count} connection','{count} connections','{count} Verbindung','{count} Verbindungen')));detail.appendChild(connections);figure.appendChild(detail);}else figure.appendChild(element('p',t('No recorded internal connections.','Keine erfassten internen Verbindungen.'),'muted'));
  if(shown.length<nodes.length||links.length<edges.length)figure.appendChild(element('p',t('Excerpt: {shown} / {total} nodes · {edges} / {all} connections.','Ausschnitt: {shown} / {total} Knoten · {edges} / {all} Verbindungen.',{shown:shown.length,total:nodes.length,edges:links.length,all:edges.length}),'muted'));
  container.appendChild(figure);draw();
  const resize=new ResizeObserver(()=>{if(!board.isConnected){resize.disconnect();return;}draw();});resize.observe(board);
  return figure;
}
function detailScopeGraph(names){return aggregate(work,new Set(names.map(name=>nodeByName(work,name)?.id).filter(id=>id!==undefined)));}
function detailFileConnections(from,to){
  const a=nodeByName(work,from),b=nodeByName(work,to);if(!a||!b)return {forward:[],reverse:[],files:{nodes:[],edges:[]}};
  const left=subtree(work,a.id),right=subtree(work,b.id),inside=new Set([...left,...right]),files=work.nodes.filter(n=>n.kind==='file'&&inside.has(n.id)),ids=new Set(files.map(n=>n.id));
  return {forward:work.edges.filter(e=>left.has(e.from)&&right.has(e.to)),reverse:work.edges.filter(e=>right.has(e.from)&&left.has(e.to)),files:{nodes:files,edges:work.edges.filter(e=>ids.has(e.from)&&ids.has(e.to))}};
}
// Source-set attribution uses explicit path conventions, never type/file names.
// Incomplete reference samples cannot establish an exclusive source-set category.
function cycleSourceScope(edge){
  const refs=edge.refs||[];if(!refs.length||edge.count==null||edge.count!==refs.length)return 'unknown';
  const kinds=new Set(refs.map(ref=>{
    const parts=String(ref.file||'').replaceAll('\\','/').replace(/^\.\//,'').split('/'),roots=new Set();
    for(let i=0;i<parts.length-1;i++)if(i===0||parts[i-1]==='src'){
      if(parts[i]==='main')roots.add('production');
      if(['test','testFixtures','integrationTest','androidTest'].includes(parts[i]))roots.add('test');
    }
    if(roots.size)return roots.size===1?[...roots][0]:'unknown';
    return parts.some(p=>['test','tests','__tests__','spec','specs'].includes(p))?'test':'unknown';
  }));
  if(kinds.has('unknown'))return 'unknown';return kinds.size===1?[...kinds][0]:'mixed';
}
function packageCycleExplanation(from,to){
  const connection=detailFileConnections(from,to),byId=new Map(work.nodes.map(n=>[n.id,n]));
  const real=id=>{const n=byId.get(id);return n?.kind==='file'&&!n.name.endsWith('/*');};
  const unique=edges=>{const pairs=new Map();for(const e of edges){const key=JSON.stringify([e.from,e.to]),old=pairs.get(key);if(old)pairs.set(key,{...old,count:old.count==null||e.count==null?null:old.count+e.count,refs:[...old.refs,...e.refs]});else pairs.set(key,e);}return [...pairs.values()];};
  const directions=[[from,to,connection.forward],[to,from,connection.reverse]].map(([a,b,edges])=>{
    const counts={production:0,test:0,mixed:0,unknown:0},pairs=unique(edges);for(const e of pairs)counts[real(e.from)&&real(e.to)?cycleSourceScope(e):'unknown']++;
    return {from:a,to:b,counts,edges:pairs};
  });
  const priority={production:0,unknown:1,mixed:2,test:3},incoming=new Map();
  for(const e of work.edges)if(cycleSourceScope(e)==='production')incoming.set(e.to,(incoming.get(e.to)||0)+1);
  const compare=(a,b)=>priority[cycleSourceScope(a)]-priority[cycleSourceScope(b)]||compareNames(byId.get(a.from)?.name||'',byId.get(b.from)?.name||'')||compareNames(byId.get(a.to)?.name||'',byId.get(b.to)?.name||'');
  let best=null;
  for(const [index,direction]of directions.entries()){
    const returns=new Map();for(const e of directions[1-index].edges.filter(e=>real(e.from)&&real(e.to)).sort(compare)){if(!returns.has(e.from))returns.set(e.from,[]);returns.get(e.from).push(e);}
    for(const first of direction.edges){if(!real(first.from)||!real(first.to))continue;const second=returns.get(first.to)?.[0];if(!second)continue;
      const score=[priority[cycleSourceScope(first)]+priority[cycleSourceScope(second)],-(incoming.get(first.from)||0),byId.get(first.to).name.split('/').length],key=[first.from,first.to,second.to].map(id=>byId.get(id).name).join('\n');
      if(!best||score[0]<best.score[0]||score[0]===best.score[0]&&(score[1]<best.score[1]||score[1]===best.score[1]&&(score[2]<best.score[2]||score[2]===best.score[2]&&compareNames(key,best.key)<0)))best={first,second,score,key,index};
    }
  }
  const witness=best?{connected:true,closed:best.first.from===best.second.to,nodes:[...new Set([best.first.from,best.first.to,best.second.to])],edges:[best.first,best.second],start:directions[best.index].from,middle:directions[best.index].to}:null;
  if(!witness){const edges=directions.map(d=>d.edges.filter(e=>real(e.from)&&real(e.to)).sort(compare)[0]).filter(Boolean);return {directions,witness:{connected:false,closed:false,nodes:[...new Set(edges.flatMap(e=>[e.from,e.to]))],edges},fileCycles:findSccs(connection.files).length};}
  return {directions,witness,fileCycles:findSccs(connection.files).length};
}
function renderPackageCycleExplanation(container,from,to){
  const model=packageCycleExplanation(from,to),witness=model.witness,labels=detailLabels([from,to]),names=new Map(work.nodes.map(n=>[n.id,n])),section=element('section',undefined,'cycle-explanation');section.setAttribute('data-cycle-explanation','true');container.appendChild(section);
  const a=labels.labels.get(witness.start),b=labels.labels.get(witness.middle),typed=witness.nodes.every(id=>names.get(id)?.name.endsWith('.java'));
  const why=witness.connected?(witness.closed?t('Package cycle: the example also returns to the same file.','Paketzyklus: Das Beispiel führt auch zur selben Datei zurück.'):typed?t('Package cycle: {a} uses {b}, while {b} depends on other types in {a}.','Paketzyklus: {a} nutzt {b}. Gleichzeitig liegen von {b} benötigte Typen in {a}.',{a,b}):t('Package cycle: {a} uses {b}, while {b} depends on other files in {a}.','Paketzyklus: {a} nutzt {b}. Gleichzeitig liegen von {b} benötigte Dateien in {a}.',{a,b})):t('Package cycle: the two directions come from separate source connections.','Paketzyklus: Die beiden Richtungen stammen aus getrennten Quellverbindungen.');
  section.appendChild(element('p',(ops.length?t('Simulated structure · ','Simulierte Struktur · '):'')+why,'detail-verdict'));
  let evidence=null;
  if(witness.edges.length){
    const owner=id=>subtree(work,nodeByName(work,from).id).has(id)?from:to;
    const figure=renderDetailGraph(section,{nodes:witness.nodes.map(id=>names.get(id)),edges:witness.edges,label:witness.connected?t('One recorded example','Ein belegtes Beispiel'):t('Separate recorded examples','Getrennte belegte Beispiele'),onNode:n=>revealNode(n.name),onEdge:e=>selectEdgeByName(owner(e.from),owner(e.to))});
    figure.setAttribute('data-cycle-witness','true');
    section.appendChild(element('small',witness.connected?(witness.closed?t('The chain closes at the same file.','Die Kette schließt sich an derselben Datei.'):t('The return reaches a different file in the same package. This example is not a closed file cycle.','Die Rückrichtung erreicht eine andere Datei desselben Pakets. Dieses Beispiel ist kein geschlossener Dateizyklus.')):t('These arrows do not form a connected file path.','Diese Pfeile bilden keinen zusammenhängenden Dateipfad.'),'muted'));
    evidence=element('details',undefined,'context-note');evidence.setAttribute('data-witness-evidence','true');evidence.appendChild(element('summary',t('Evidence for this example','Belege zum Beispiel')));
    for(const e of witness.edges)for(const r of (e.refs||[]).slice(0,3))evidence.appendChild(element('pre',r.file+':'+r.line+' '+r.raw));
  }else section.appendChild(element('p',t('No complete file example is available in this report.','Dieser Bericht enthält kein vollständiges Dateibeispiel.'),'muted'));
  const scopes=['production','test'];for(const key of ['mixed','unknown'])if(model.directions.some(d=>d.counts[key]))scopes.push(key);
  const scopeLabels={production:t('Production','Produktionscode'),test:t('Tests','Tests'),mixed:t('Mixed','Gemischt'),unknown:t('Unknown','Unbekannt')},table=element('table',undefined,'cycle-counts');table.setAttribute('data-cycle-counts','true');table.appendChild(element('caption',t('Recorded connections by source path','Erfasste Verbindungen nach Quellpfad')));
  const head=element('thead'),header=element('tr');for(const label of [t('Direction','Richtung'),...scopes.map(k=>scopeLabels[k])]){const th=element('th',label);th.setAttribute('scope','col');header.appendChild(th);}head.appendChild(header);table.appendChild(head);
  const body=element('tbody');for(const [index,d]of model.directions.entries()){const row=element('tr'),label=element('th'),action=button(labels.labels.get(d.from)+' → '+labels.labels.get(d.to),()=>selectEdgeByName(d.from,d.to),'btn btn-quiet');action.setAttribute('data-package-direction',index?'reverse':'forward');action.setAttribute('aria-label',t('Inspect {direction}','{direction} untersuchen',{direction:d.from+' → '+d.to}));label.setAttribute('scope','row');label.appendChild(action);row.appendChild(label);for(const scope of scopes){const value=element('td',String(d.counts[scope]));value.setAttribute('data-source-scope',scope);row.appendChild(value);}body.appendChild(row);}table.appendChild(body);section.appendChild(table);
  section.appendChild(contextHelp(t('Connection counts','Verbindungszahlen'),t('Each directed source pair counts once, not once per import or call site. Production/test attribution uses main/test source paths and explicit test directories. Other paths, missing references and incomplete samples stay unknown. Mixed means evidence from both production and tests. These are path conventions, not verified build source sets.','Jedes gerichtete Quellpaar zählt einmal, nicht jede Import- oder Aufrufstelle. Die Zuordnung nutzt main/test-Quellpfade und ausdrückliche Testverzeichnisse. Andere Pfade, fehlende Belege und unvollständige Stichproben bleiben unbekannt. Gemischt bedeutet Belege aus Produktions- und Testcode. Dies sind Pfadkonventionen, keine verifizierten Build-Quellmengen.')));
  if(evidence)section.appendChild(evidence);
  section.appendChild(element('p',model.fileCycles?t('{count} file-level cycle groups within these packages.','{count} Zyklusgruppen auf Dateiebene innerhalb dieser Pakete.',{count:model.fileCycles}):t('No file-level cycle within these packages in the recorded graph.','Kein Dateizyklus innerhalb dieser Pakete im erfassten Graphen.'),'muted'));
  if(graphCycleCoverageWarning())section.appendChild(element('small',graphCycleCoverageWarning(),'muted'));
}
function renderDependencyDetailGraph(container,from,to){
  const graph=detailScopeGraph([from,to]),connection=detailFileConnections(from,to),isPackage=[from,to].some(name=>nodeByName(work,name)?.kind==='module');
  if(isPackage&&connection.forward.length&&connection.reverse.length)renderPackageCycleExplanation(container,from,to);
  else renderDetailGraph(container,{...graph,label:ops.length?t('Simulated dependencies','Simulierte Abhängigkeiten'):t('Observed dependencies','Beobachtete Abhängigkeiten'),onNode:n=>selectNode(n.name,true),onEdge:e=>selectEdgeByName(graph.nodes.find(n=>n.id===e.from).name,graph.nodes.find(n=>n.id===e.to).name)});
  if(!isPackage)return;
  const detail=element('details',undefined,'inspector-section');detail.setAttribute('data-file-connections','true');detail.appendChild(element('summary',plural(connection.forward.length,'{count} source connection','{count} source connections','{count} Quellverbindung','{count} Quellverbindungen')));container.appendChild(detail);
  const list=element('div'),preview=element('div');detail.appendChild(preview);detail.appendChild(list);let shown=0;
  const labels=detailLabels(connection.files.nodes.map(n=>n.name)),more=button(t('Show next 20','Nächste 20 anzeigen'),()=>fill(),'btn btn-quiet');detail.appendChild(more);
  function fill(){for(const e of connection.forward.slice(shown,shown+20)){const a=nodeById(work,e.from),b=nodeById(work,e.to);const control=button(labels.labels.get(a.name)+' → '+labels.labels.get(b.name),()=>{preview.textContent='';renderDetailGraph(preview,{nodes:[a,b],edges:[e],label:t('Source connection','Quellverbindung'),onNode:n=>revealNode(n.name)});for(const r of e.refs.slice(0,5))preview.appendChild(element('pre',r.file+':'+r.line+' '+r.raw));},'btn btn-quiet detail-file-pair');list.appendChild(control);}shown+=20;more.hidden=shown>=connection.forward.length;}
  fill();
}
function selectEdgeByName(from,to){const edge=view.edges.find(e=>nodeById(view,e.from).name===from&&nodeById(view,e.to).name===to);if(edge)selectEdge(edge);}
function renderNodeDetailGraph(container,node,incoming,outgoing){
  const ids=new Set([node.id]);const edges=[];for(const e of [...incoming,...outgoing]){if(ids.size>=8&&!ids.has(e.from)&&!ids.has(e.to))continue;ids.add(e.from);ids.add(e.to);edges.push(e);}
  if(!edges.length)return;
  renderDetailGraph(container,{nodes:[node,...view.nodes.filter(n=>n.id!==node.id&&ids.has(n.id))],edges,focusId:node.id,label:t('Dependencies around this selection','Abhängigkeiten dieser Auswahl'),onNode:n=>selectNode(n.name,true),onEdge:selectEdge});
}
