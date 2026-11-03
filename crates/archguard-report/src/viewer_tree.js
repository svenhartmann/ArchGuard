'use strict';
// The main diagram uses a scrollable SVG tree at readable text size. All
// routing is computed before direction filters, so filtering a selection cannot
// move cards or reroute its remaining edges.
let treeLastClick=null,treeZoom=1,treePositionKey='',treeRouting='direction',treeSort='connections',treeViewportWidth=0;
const TREE_MIN_ZOOM=.01,TREE_MAX_ZOOM=3;
// Geometry belongs to the graph cut, not to its selection or scroll position.
let treeModelCache=null,treeWindowState=null,treeViewportPending=false,treeForceFull=false,treeLastReference=null,treeRevealName=null;
function treeViewportRange(container,top=container.scrollTop||0) {
  const height=container.clientHeight||container.getBoundingClientRect().height||600;
  const header=container.querySelector('div[data-tree-toolbar]')?.getBoundingClientRect().height||40;
  const bucket=Math.floor(Math.max(0,top/treeZoom)/360)*360;
  return {from:Math.max(0,bucket-360),to:bucket+Math.max(120,height-header)/treeZoom+720};
}
function scheduleTreeViewport() {
  if(!treeWindowState||treeViewportPending)return;
  treeViewportPending=true;
  const schedule=typeof requestAnimationFrame==='function'?requestAnimationFrame:setTimeout;
  schedule(()=>{
    treeViewportPending=false;if(!treeWindowState||ui.workspace!=='graph'||ui.diagram!=='overview')return;
    const range=treeViewportRange(document.getElementById('package-tree'));
    if(range.from!==treeWindowState.from||range.to!==treeWindowState.to||treeZoom!==treeWindowState.zoom)renderPackageTree();
  });
}
document.getElementById('package-tree').addEventListener('scroll',scheduleTreeViewport,{passive:true});
function withFullPackageTree(callback) {
  const tree=document.getElementById('package-tree'),top=tree.scrollTop,left=tree.scrollLeft;
  treeForceFull=true;
  try{renderPackageTree();return callback();}
  finally{treeForceFull=false;tree.scrollTop=top;tree.scrollLeft=left;renderPackageTree();}
}
function packageTreeModel(shown,width) {
  const key=shown.map(n=>n.id).join(','),cached=treeModelCache;
  if(cached?.view===view&&cached.work===work&&cached.key===key&&cached.width===width&&cached.sort===treeSort)return cached;
  const byId=new Map(shown.map(n=>[n.id,n])),bySource=new Map(shown.map(n=>[n.src,n]));
  const links=view.edges.filter(e=>byId.has(e.from)&&byId.has(e.to)),degree=new Map(view.nodes.map(n=>[n.id,{incoming:0,outgoing:0}]));
  for(const edge of view.edges){degree.get(edge.from).outgoing++;degree.get(edge.to).incoming++;}
  const layout=packageTreeLayout(containmentLayout(work,shown.map(n=>n.src),rootName),links.map(e=>({from:byId.get(e.from).src,to:byId.get(e.to).src,key:e.from+' '+e.to})),{width,order:treeSort,tracks:true});
  const routes=new Map(layout.routes.map(route=>[route.key,route]));
  treeModelCache={view,work,key,width,sort:treeSort,byId,bySource,links,degree,layout,routes,sources:new Map(work.nodes.map(n=>[n.id,n])),parents:new Set(work.nodes.map(n=>n.parent)),cycleNodes:new Set(sccs.flatMap(c=>c.members))};
  return treeModelCache;
}
function treeDirectedRoute(route,side,width) {
  if(side!=='left')return route;
  if(!route.left){const points=route.points.map(p=>({x:width-p.x,y:p.y}));route.left={...route,side:'left',points,hitPoints:null,path:treeDependencyPath(points)};}
  return route.left;
}
// Clip transit strokes to the viewport. Their complete routes remain in the
// model for hit testing and full SVG/JSON exports, with no relationship limit.
function treeClippedPath(points,from,to) {
  const [source,lead,c1,c2,tail,target]=points,parts=[];
  const line=(a,b)=>{if(a.y>=from&&a.y<=to)parts.push(`M ${a.x} ${a.y} L ${b.x} ${b.y}`);};
  line(source,lead);
  const low=Math.max(from,Math.min(lead.y,tail.y)),high=Math.min(to,Math.max(lead.y,tail.y));
  if(low<=high){
    // Monotone Y permits bounded clipping without changing the curve shape.
    const atY=y=>{if(y===lead.y)return 0;if(y===tail.y)return 1;let lo=0,hi=1;for(let i=0;i<32;i++){const mid=(lo+hi)/2;if((treeCurvePoint(points,mid).y<y)===(tail.y>lead.y))lo=mid;else hi=mid;}return (lo+hi)/2;};
    const a=atY(lead.y<tail.y?low:high),b=atY(lead.y<tail.y?high:low),start=treeCurvePoint(points,a),end=treeCurvePoint(points,b);
    const tangent=t=>{const u=1-t;return {x:u*u*(c1.x-lead.x)+2*u*t*(c2.x-c1.x)+t*t*(tail.x-c2.x),y:u*u*(c1.y-lead.y)+2*u*t*(c2.y-c1.y)+t*t*(tail.y-c2.y)};};
    const first=tangent(a),last=tangent(b),span=b-a;
    parts.push(`M ${start.x} ${start.y} C ${start.x+first.x*span} ${start.y+first.y*span} ${end.x-last.x*span} ${end.y-last.y*span} ${end.x} ${end.y}`);
  }
  line(tail,target);return parts.join(' ')+' ';
}
function treeRouteHitPoints(route){
  if(!route.hitPoints){route.hitPoints=[route.points[0],route.points[1]];for(let i=1;i<=32;i++)route.hitPoints.push(treeCurvePoint(route.points,i/32));route.hitPoints.push(route.points.at(-1));}
  return route.hitPoints;
}

function treeCycleColorHelp(directional=true){return directional?t('Red marks only right-hand cyclic edges; left-hand edges keep their direction color.','Nur rechte zyklische Kanten sind rot; linke Kanten behalten ihre Richtungsfarbe.'):t('Red marks cycle edges in both directions.','Rot markiert Zykluskanten in beiden Richtungen.');}
function treeCycleDescription(side,directional) {
  return t('Cyclic dependency. ','Zyklische Abhängigkeit. ')+(side==='right'?(directional?t('Right-hand cycle edge highlighted red.','Rechte zyklische Kante rot hervorgehoben.'):t('Cycle edge highlighted red.','Zykluskante rot hervorgehoben.')):t('Left-hand edge keeps its direction color; right-hand cycle edges are red.','Die linke Kante behält ihre Richtungsfarbe; rechte zyklische Kanten sind rot.'));
}
function treeAvailableWidth(container) {
  const width=container.clientWidth||container.getBoundingClientRect().width;
  return Number.isFinite(width)&&width>0?Math.floor(width):0;
}
function updateTreeZoomControls() {
  const tree=document.getElementById('package-tree');
  for(const b of tree.querySelectorAll('button[data-tree-zoom]')){
    const action=b.getAttribute('data-tree-zoom');
    b.disabled=action==='in'?treeZoom>=TREE_MAX_ZOOM:action==='out'?treeZoom<=TREE_MIN_ZOOM:false;
  }
  const scale=tree.querySelector('output[data-tree-scale]');if(scale)scale.textContent=Math.round(treeZoom*100)+'%';
}
function setPackageTreeZoom(value,anchor) {
  const tree=document.getElementById('package-tree'),svg=tree.querySelector('svg');
  if(!svg||!Number.isFinite(value))return;
  const bounds=tree.getBoundingClientRect(),header=tree.querySelector('div[data-tree-toolbar]')?.getBoundingClientRect().height||0;
  const width=tree.clientWidth||bounds.width,height=tree.clientHeight||bounds.height;
  const x=anchor?.x??width/2,y=anchor?.y??(header+Math.max(0,height-header)/2);
  const zoom=Math.max(TREE_MIN_ZOOM,Math.min(TREE_MAX_ZOOM,value)),ratio=zoom/treeZoom;
  const left=Math.max(0,((tree.scrollLeft||0)+x)*ratio-x),top=Math.max(0,((tree.scrollTop||0)+y-header)*ratio-y+header);
  const dimensions=svg.getAttribute('viewBox').split(' ').map(Number);
  treeZoom=zoom;svg.setAttribute('width',dimensions[2]*zoom);svg.setAttribute('height',dimensions[3]*zoom);
  tree.scrollLeft=left;tree.scrollTop=top;hideContextTooltip();updateTreeZoomControls();scheduleTreeViewport();
}
function fitPackageTree() {
  const tree=document.getElementById('package-tree'),svg=tree.querySelector('svg');if(!svg)return;
  const bounds=tree.getBoundingClientRect(),header=tree.querySelector('div[data-tree-toolbar]')?.getBoundingClientRect().height||0;
  const dimensions=svg.getAttribute('viewBox').split(' ').map(Number);
  setPackageTreeZoom(Math.min(Math.max(1,(tree.clientWidth||bounds.width)-24)/dimensions[2],Math.max(1,(tree.clientHeight||bounds.height)-header-24)/dimensions[3]));
  tree.scrollLeft=0;tree.scrollTop=0;
}
document.getElementById('package-tree').addEventListener('wheel',event=>{
  if(!(event.ctrlKey||event.metaKey)||ui.workspace!=='graph'||ui.diagram!=='overview')return;
  const tree=document.getElementById('package-tree');if(!tree.querySelector('svg'))return;
  event.preventDefault();
  const bounds=tree.getBoundingClientRect(),unit=event.deltaMode===1?16:event.deltaMode===2?bounds.height:1;
  setPackageTreeZoom(treeZoom*Math.exp(-Math.max(-1000,Math.min(1000,event.deltaY*unit))*.002),{x:event.clientX-(bounds.left||0),y:event.clientY-(bounds.top||0)});
},{passive:false});
function resetPackageTree() {
  ui.edgeMode='all';ui.edgeModeAutomatic=true;treeLastClick=null;treeZoom=1;treePositionKey='';treeRouting='direction';treeSort='connections';
  const tree=document.getElementById('package-tree');tree.scrollTop=0;tree.scrollLeft=0;
  resetView();
}
function treeSvg(tag, attributes={}, text) {
  const node=document.createElementNS('http://www.w3.org/2000/svg',tag);
  for(const [key,value] of Object.entries(attributes))node.setAttribute(key,String(value));
  if(text!==undefined)node.textContent=text;
  return node;
}
// Keep diagram-specific styling with the SVG so it also travels with exports.
// Scope it to this view: Services and detail diagrams share some tree classes.
const TREE_DIAGRAM_STYLE=`
  .package-tree-diagram .tree-column-label { fill: var(--muted); font: 600 10px system-ui; letter-spacing: .08em; pointer-events: none; }
  .package-tree-diagram .tree-branch-band { fill: var(--panel); fill-opacity: .45; stroke: none; }
  .package-tree-diagram .package-tree-hierarchy { stroke-width: 1; }
  .package-tree-diagram .tree-row-background { fill: var(--panel); stroke: var(--border); stroke-width: .6; }
  .package-tree-diagram .tree-context .tree-row-background { fill: transparent; stroke: none; }
  .package-tree-diagram .tree-row.in-cycle .tree-row-background { fill: var(--panel); fill-opacity: 1; }
  .package-tree-diagram .tree-row.is-selected .tree-row-background,
  .package-tree-diagram .tree-row:hover .tree-row-background,
  .package-tree-diagram .tree-row:focus .tree-row-background { fill: var(--panel-2); stroke: var(--accent); stroke-width: 1.5; fill-opacity: 1; }
  .package-tree-diagram .tree-row.is-related .tree-node-name { fill: var(--accent); }
  .package-tree-diagram .tree-row-icon { fill: none; stroke: var(--muted); stroke-linejoin: round; }
  .package-tree-diagram .tree-package .tree-node-name { font-weight: 600; }
  .package-tree-diagram .tree-row-guide { stroke: var(--border); stroke-width: 1; stroke-dasharray: 2 5; pointer-events: none; }
  .package-tree-diagram .tree-row-guide.is-related { stroke: var(--accent); stroke-dasharray: none; opacity: .5; }
  .package-tree-diagram .tree-relation-background { fill: var(--panel); stroke: var(--muted); stroke-width: 1.2; }
  .package-tree-diagram .tree-port-dot { fill: var(--muted); pointer-events: none; }
  .package-tree-diagram .in-cycle .tree-port-dot,
  .package-tree-diagram .tree-cycle-mark { fill: var(--c-cycle); pointer-events: none; }
  .package-tree-diagram .tree-relation-node.is-selected .tree-relation-background,
  .package-tree-diagram .tree-relation-node.is-related .tree-relation-background,
  .package-tree-diagram .tree-relation-node:hover .tree-relation-background,
  .package-tree-diagram .tree-relation-node:focus .tree-relation-background { stroke: var(--accent); stroke-width: 2; }
  .package-tree-diagram .tree-disclosure rect { fill: transparent; stroke: none; }
  .package-tree-diagram .tree-disclosure path { stroke: var(--muted); stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .package-tree-diagram .tree-disclosure:hover rect,
  .package-tree-diagram .tree-disclosure:focus rect { fill: var(--panel-2); stroke: var(--accent); stroke-width: 1; }
  .package-tree-diagram .tree-edge-line { stroke-linecap: round; }
  .package-tree-diagram.tree-node-hover .package-tree-edge,
  .package-tree-diagram.tree-node-hover .tree-transit-edges { opacity: .12; }
  .package-tree-diagram.edge-hover .tree-transit-edges { opacity: .15; }
  .package-tree-diagram.tree-node-hover .package-tree-edge.is-related,
  .package-tree-diagram.tree-node-hover .package-tree-edge.is-selected { opacity: 1; }
  .package-tree-diagram .package-tree-edge.is-related .tree-edge-line { stroke-width: 2.5; }
`;
function treeTooltip(target, lines) {
  target.appendChild(treeSvg('title',{},lines.join('\n')));
  function show(event){
    const tip=document.getElementById('graph-context-tooltip'),area=document.getElementById('tab-graph').getBoundingClientRect();
    tip.textContent='';tip.appendChild(tooltipEl(lines));tip.hidden=false;
    const bounds=target.getBoundingClientRect(),x=(event.clientX??bounds.right??0)-(area.left||0),y=(event.clientY??bounds.top??0)-(area.top||0),box=tip.getBoundingClientRect();
    tip.style.left=Math.max(8,Math.min(x+12,area.width-box.width-8))+'px';
    tip.style.top=Math.max(8,Math.min(y+12,area.height-box.height-8))+'px';
  }
  target.addEventListener('pointerenter',show);target.addEventListener('focus',show);
  target.addEventListener('pointerleave',hideContextTooltip);target.addEventListener('blur',hideContextTooltip);
}
// Keep the interacted row anchored when expansion or selection changes layout.
// A raw scroll offset cannot preserve position when preceding rows are reordered.
function preserveTreeRowPosition(source,action,attribute='data-tree-source') {
  const container=document.getElementById('package-tree'),find=()=>[...container.querySelectorAll('g['+attribute+']')].find(n=>n.getAttribute(attribute)===String(source));
  const row=()=>[...container.querySelectorAll('g[data-tree-source]')].find(n=>n.getAttribute('data-tree-source')===String(source));
  const before=row()?.getBoundingClientRect();
  treeRevealName=nodeById(work,source)?.name||null;
  action();
  const target=find();if(!target)return;
  const after=row()?.getBoundingClientRect();
  if(before&&Number.isFinite(before.top)&&Number.isFinite(after?.top))container.scrollTop+=after.top-before.top;
  if(before&&Number.isFinite(before.left)&&Number.isFinite(after?.left))container.scrollLeft+=after.left-before.left;
  target.focus({preventScroll:true});
}
// Keep secondary controls available without covering the graph with permanent buttons.
const TREE_TOOLBAR_STYLE=`
  #package-tree .package-tree-legend { gap: 8px; padding: 8px 12px; background: var(--panel); }
  #package-tree .package-tree-legend > .package-tree-zoom { display: inline-flex; gap: 0; border: 1px solid var(--border); border-radius: 10px; padding: 2px; background: var(--panel-2); }
  #package-tree .package-tree-legend > .package-tree-zoom button { min-width: 32px; min-height: 32px; padding: 4px 8px; border: 0; background: transparent; box-shadow: none; border-radius: 7px; }
  #package-tree .package-tree-legend > .package-tree-zoom button:hover { background: var(--panel); }
  #package-tree .tree-options-details > summary { list-style: none; min-height: 36px; display: flex; align-items: center; gap: 10px; padding: 0 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--panel-2); color: var(--text); }
  #package-tree .tree-options-details > summary::after { content: '⌄'; color: var(--muted); }
  #package-tree .tree-options-details > summary::-webkit-details-marker { display: none; }
  #package-tree .tree-options-details[open] > summary { border-color: var(--accent); }
  #package-tree .tree-options-content { left: 12px; top: calc(100% + 6px); width: 320px; max-width: calc(100% - 24px); max-height: min(65vh, 540px); overflow-y: auto; padding: 16px; border-radius: 14px; box-sizing: border-box; gap: 10px; }
  #package-tree .tree-options-content .package-tree-zoom { display: flex; flex-wrap: wrap; gap: 6px; }
  #package-tree .tree-options-content button { min-height: 34px; }
  #package-tree .tree-menu-label { margin-top: 4px; color: var(--muted); font-size: 11px; }
  #package-tree .tree-options-content > details { padding-top: 10px; border-top: 1px solid var(--border); }
  #package-tree .tree-options-content > details > .tree-legend-content { position: static; width: auto; max-width: none; padding: 4px 0; border: 0; box-shadow: none; background: transparent; }
  #package-tree .tree-options-content > details p { margin: 8px 0; }
  #package-tree .tree-options-content [data-tree-control=reset] { margin-top: 6px; }
  @media (max-width: 700px) {
    #package-tree .package-tree-legend { z-index: 5; }
    #package-tree .tree-options-content { position: fixed; left: 12px; right: 12px; top: auto; bottom: 12px; width: auto; max-width: none; max-height: min(65dvh, 540px); }
  }
  @media (pointer: coarse) { #package-tree .package-tree-legend button, #package-tree .tree-options-details > summary { min-height: 44px; min-width: 44px; } }
`;
document.addEventListener('pointerdown',event=>{
  const menu=document.getElementById('package-tree').querySelector('details[data-tree-options]');
  if(menu?.open&&!menu.contains(event.target))menu.open=false;
},true);
function renderPackageTree() {
  const container=document.getElementById('package-tree');
  // File visibility and detailed counts belong with their existing disclosures.
  const displayContent=document.getElementById('graph-display-menu').querySelector('div'),files=document.getElementById('toggle-files');
  if(displayContent&&files.parentElement!==displayContent)displayContent.appendChild(files);
  const scopeDetails=document.getElementById('graph-scope-details'),counts=document.getElementById('graph-status-brief');
  if(counts.parentElement!==scopeDetails)scopeDetails.appendChild(counts);
  // Preserve native scroll while changing direction, colors or selection.
  let top=container.scrollTop;const left=container.scrollLeft;
  const focusedToggle=document.activeElement?.getAttribute('data-tree-toggle');
  const focusedSource=document.activeElement?.getAttribute('data-tree-source')??focusedToggle;
  const focusedControl=document.activeElement?.getAttribute('data-tree-control');
  const legendOpen=container.querySelector('details[data-tree-legend]')?.open||false;
  const optionsOpen=container.querySelector('details[data-tree-options]')?.open||false;
  const width=treeAvailableWidth(container),resized=width!==treeViewportWidth;treeViewportWidth=width;
  let range=treeViewportRange(container);
  container.textContent='';
  container.appendChild(element('style',TREE_TOOLBAR_STYLE));
  const shown=displayedGraphNodes(),model=packageTreeModel(shown,treeViewportWidth),{byId,bySource,links,degree,layout}=model;
  const reference=directionReference(),windowed=!treeForceFull&&layout.nodes.length>300;
  if(windowed&&(treeRevealName!==null||reference?.name!==treeLastReference)){
    const anchor=layout.nodes.find(n=>n.name===(treeRevealName||reference?.name)),height=container.clientHeight||container.getBoundingClientRect().height||600;
    if(anchor&&(anchor.y*treeZoom<top+30||anchor.y*treeZoom>top+height-100)){top=Math.max(0,anchor.y*treeZoom-height/3);range=treeViewportRange(container,top);}
  }
  treeLastReference=reference?.name||null;treeRevealName=null;
  const compactScene=windowed&&treeZoom<.35;
  const inWindow=y=>!windowed||y>=range.from-14&&y<=range.to+14;
  const visibleRows=windowed?layout.nodes.filter(n=>inWindow(n.y)||String(n.src)===focusedSource):layout.nodes;
  treeWindowState=windowed?{...range,zoom:treeZoom}:null;
  const positionKey=JSON.stringify(layout.nodes.map(n=>[n.src,n.x,n.y,n.w,n.h])),layoutChanged=positionKey!==treePositionKey;
  treePositionKey=positionKey;
  const legend=element('div',undefined,'package-tree-legend');
  legend.setAttribute('data-tree-toolbar','true');
  const zoomControls=element('div',undefined,'package-tree-zoom');zoomControls.setAttribute('role','group');zoomControls.setAttribute('aria-label',t('Tree zoom','Baum-Zoom'));
  for(const [action,label,description,handler] of [
    ['out','−',t('Zoom out','Verkleinern'),()=>setPackageTreeZoom(treeZoom/1.25)],
    ['in','+',t('Zoom in','Vergrößern'),()=>setPackageTreeZoom(treeZoom*1.25)],
    ['reset','100%',t('Reset zoom to 100%','Zoom auf 100 % zurücksetzen'),()=>setPackageTreeZoom(1)],
    ['fit','⛶',t('Fit the entire tree in the viewport','Den gesamten Baum in den sichtbaren Bereich einpassen'),fitPackageTree],
  ]){
    const b=button(action==='reset'?undefined:label,handler,'btn btn-secondary');b.setAttribute('data-tree-zoom',action);b.setAttribute('data-tree-control','zoom-'+action);b.setAttribute('aria-label',description);b.title=description;
    if(action==='reset'){
      const scale=element('output',undefined,'tree-scale');scale.setAttribute('data-tree-scale','true');scale.setAttribute('aria-label',t('Current tree zoom','Aktueller Baum-Zoom'));b.appendChild(scale);
    }
    zoomControls.appendChild(b);
  }
  legend.appendChild(zoomControls);
  if(typeof patternHighlight!=='undefined'&&patternHighlight.id){legend.appendChild(element('span',t('Pattern evidence highlighted','Musterbelege markiert')));legend.appendChild(button(t('Clear pattern highlight','Mustermarkierung löschen'),clearPatternHighlight,'btn btn-secondary'));}
  const bulk=treeBulkState(),branches=element('div',undefined,'package-tree-zoom');branches.setAttribute('role','group');branches.setAttribute('aria-label',t('All tree branches','Alle Baumzweige'));
  for(const [action,label] of [['expand',t('Expand all','Alle öffnen')],['collapse',t('Collapse all','Alle schließen')]]){
    const b=button(label,()=>{setAllTreeBranches(action==='expand');container.querySelectorAll('button[data-tree-all]').forEach(next=>{if(next.getAttribute('data-tree-all')!==(action)&&!next.disabled)next.focus({preventScroll:true});});},'btn btn-secondary');
    b.setAttribute('data-tree-all',action);b.setAttribute('data-tree-control','all-'+action);b.disabled=!bulk[action];b.title=action==='expand'?t('Expand all packages in this report; clear search and synchronize Explorer.','Alle Pakete dieses Berichts öffnen; Suche leeren und Explorer synchronisieren.'):t('Collapse all packages in this report; clear search and synchronize Explorer.','Alle Pakete dieses Berichts schließen; Suche leeren und Explorer synchronisieren.');branches.appendChild(b);
  }

  const sortControls=element('div',undefined,'package-tree-zoom');sortControls.setAttribute('role','group');sortControls.setAttribute('aria-label',t('Tree sorting','Baumsortierung'));
  for(const [order,label] of [['alphabetical',t('Alphabetical','Alphabetisch')],['connections',t('By dependencies','Nach Beziehungen')]]){
    const b=button(label,()=>{treeSort=order;renderPackageTree();[...container.querySelectorAll('button[data-tree-sort]')].find(next=>next.getAttribute('data-tree-sort')===order)?.focus({preventScroll:true});},'btn '+(treeSort===order?'btn-primary':'btn-secondary'));
    b.setAttribute('data-tree-sort',order);b.setAttribute('data-tree-control','sort-'+order);b.setAttribute('aria-pressed',String(treeSort===order));
    b.title=order==='connections'?t('Arrange sibling branches by their dependencies to reduce crossings. Package containment stays unchanged.','Geschwisterzweige anhand ihrer Abhängigkeiten anordnen, um Kreuzungen zu verringern. Die Pakethierarchie bleibt erhalten.'):t('Sort sibling packages and files alphabetically.','Benachbarte Pakete und Dateien alphabetisch sortieren.');sortControls.appendChild(b);
  }

  const options=element('details',undefined,'tree-legend-details tree-options-details');options.setAttribute('data-tree-options','true');options.open=optionsOpen;
  const optionsSummary=element('summary',t('View','Ansicht'));optionsSummary.setAttribute('data-tree-control','options');options.appendChild(optionsSummary);
  options.addEventListener('keydown',event=>{
    if(event.key!=='Escape'||!options.open)return;
    event.preventDefault();event.stopPropagation();options.open=false;optionsSummary.focus({preventScroll:true});
  });
  const optionsContent=element('div',undefined,'tree-legend-content tree-options-content');
  optionsContent.appendChild(element('strong',t('Branches','Zweige'),'tree-menu-label'));optionsContent.appendChild(branches);
  optionsContent.appendChild(element('strong',t('Order','Sortierung'),'tree-menu-label'));optionsContent.appendChild(sortControls);
  optionsContent.appendChild(element('strong',t('Connections','Verbindungen'),'tree-menu-label'));
  if(connectionCountNotice()){
    const coverage=element('span',t('* Incomplete ⓘ','* Unvollständig ⓘ'));coverage.setAttribute('data-connection-coverage','true');
    optionsContent.appendChild(explainMetric(coverage,'dependencies',connectionCountNotice()+' '+graphCoverageDescription()));
  }

  const routingControls=element('div',undefined,'package-tree-zoom');routingControls.setAttribute('role','group');routingControls.setAttribute('aria-label',t('Relation arrangement','Anordnung der Beziehungen'));
  const directionToggle=button(t('Incoming / Outgoing','Eingehend / Ausgehend'),()=>{
    treeRouting=treeRouting==='direction'?'ordered':'direction';renderPackageTree();
    container.querySelector('button[data-tree-routing]')?.focus({preventScroll:true});
  },'btn btn-secondary');
  directionToggle.setAttribute('data-tree-routing','direction');directionToggle.setAttribute('data-tree-control','routing');directionToggle.setAttribute('aria-pressed',String(treeRouting==='direction'));
  directionToggle.title=t('Show incoming dependencies to the selected package or file left of the tree; outgoing dependencies and cycles stay right. Turn off to place all connections on the right.','Eingehende Abhängigkeiten zum ausgewählten Paket oder zur Datei links vom Baum anzeigen; ausgehende Abhängigkeiten und Zyklen bleiben rechts. Ausschalten ordnet alle Verbindungen rechts an.');routingControls.appendChild(directionToggle);
  optionsContent.appendChild(routingControls);
  const help=element('details',undefined,'tree-legend-details');help.setAttribute('data-tree-legend','true');help.open=legendOpen;

  const helpSummary=element('summary',t('Legend','Legende'));helpSummary.setAttribute('data-tree-control','legend');help.appendChild(helpSummary);
  const helpContent=element('div',undefined,'tree-legend-content');
  helpContent.appendChild(element('p',treeRouting==='ordered'?t('Dependencies beside the tree; arrowheads point to the target.','Abhängigkeiten neben dem Baum; Pfeilspitzen zeigen zum Ziel.'):t('Selected package or file: incoming left of the tree · outgoing and cycles right','Ausgewähltes Paket oder Datei: eingehend links vom Baum · ausgehend und Zyklen rechts')));
  helpContent.appendChild(element('p',t('Incoming blue · Outgoing violet · Cycles red','Eingehend blau · Ausgehend violett · Zyklen rot')));
  helpContent.appendChild(element('p',treeCycleColorHelp(false)));
  helpContent.appendChild(element('p',t('Red dots: packages or files involved in a cycle. Hover or focus a row to trace its visible dependencies.','Rote Punkte: Pakete oder Dateien mit Zyklusbeteiligung. Eine Zeile mit der Maus oder Tastatur fokussieren, um ihre sichtbaren Beziehungen hervorzuheben.')));
  helpContent.appendChild(element('p',t('Dashed: coarser package-level cycle context; direction colors are retained.','Gestrichelt: Zykluskontext auf gröberer Paketebene; Richtungsfarben bleiben erhalten.')));
  helpContent.appendChild(element('p',t('A cycle is a closed directed dependency path. A cycle group contains nodes that can all reach each other; it may contain several cycles.','Ein Zyklus ist ein geschlossener gerichteter Abhängigkeitspfad. Eine Zyklusgruppe enthält Knoten, die sich gegenseitig erreichen können; sie kann mehrere Zyklen enthalten.')));
  help.appendChild(helpContent);optionsContent.appendChild(help);
  const reset=button(t('Reset tree','Baum zurücksetzen'),resetPackageTree,'btn btn-secondary');reset.setAttribute('data-tree-control','reset');optionsContent.appendChild(reset);
  options.appendChild(optionsContent);legend.appendChild(options);container.appendChild(legend);
  if(focusedControl!=null)[...legend.querySelectorAll('button[data-tree-control]'),...legend.querySelectorAll('summary[data-tree-control]')].find(control=>control.getAttribute('data-tree-control')===focusedControl&&!control.disabled)?.focus({preventScroll:true});
  if(!layout.nodes.length){container.appendChild(element('p',t('No nodes in this scope.','Keine Knoten in diesem Bereich.'),'empty'));return;}
  const svg=treeSvg('svg',{class:'package-tree-diagram',width:layout.width*treeZoom,height:layout.height*treeZoom,viewBox:'0 0 '+layout.width+' '+layout.height,role:'group','aria-label':t('Package hierarchy and directed dependencies','Pakethierarchie und gerichtete Abhängigkeiten')});
  svg.setAttribute('data-tree-windowed',String(windowed));
  svg.setAttribute('data-tree-edge-count',String(links.filter(e=>dependencyMatchesSelection(e.from,e.to)).length));
  svg.appendChild(treeSvg('style',{},TREE_DIAGRAM_STYLE));
  const first=layout.nodes[0],labelLeft=first.x-first.w/2;
  svg.appendChild(treeSvg('text',{x:labelLeft+10,y:14,class:'tree-column-label','aria-hidden':'true'},t('PACKAGES','PAKETE')));
  if(treeRouting==='direction')svg.appendChild(treeSvg('text',{x:layout.incomingX/2,y:14,'text-anchor':'middle',class:'tree-column-label','aria-hidden':'true'},t('INCOMING','EINGEHEND')));
  svg.appendChild(treeSvg('text',{x:(layout.relationX+layout.width)/2,y:14,'text-anchor':'middle',class:'tree-column-label','aria-hidden':'true'},treeRouting==='direction'?t('OUTGOING · CYCLES','AUSGEHEND · ZYKLEN'):t('DEPENDENCIES','ABHÄNGIGKEITEN')));
  const rowTargets=new Map(),edgeTargets=[];
  function emphasize(source){
    const active=source!==undefined,related=new Set(active?[source]:[]);
    svg.classList.toggle('tree-node-hover',active);
    for(const edge of edgeTargets){
      const matches=active&&(edge.from===source||edge.to===source);
      edge.group.classList.toggle('is-related',matches);
      if(matches){related.add(edge.from);related.add(edge.to);}
    }
    for(const [id,targets] of rowTargets)for(const target of targets)target.classList.toggle('is-related',related.has(id));
  }
  function bindEmphasis(target,source){
    target.addEventListener('pointerenter',()=>emphasize(source));
    target.addEventListener('pointerleave',()=>emphasize());
    target.addEventListener('focus',()=>emphasize(source));
    target.addEventListener('blur',()=>emphasize());
  }
  for(const node of layout.nodes){
    if(!inWindow(node.y)&&!node.context)continue;
    rowTargets.set(node.src,[]);
    if(node.context&&!node.project){
      const index=layout.nodes.indexOf(node);let end=index+1;
      while(end<layout.nodes.length&&layout.nodes[end].depth>node.depth)end++;
      const bottom=layout.nodes[end-1].y+layout.nodes[end-1].h/2;
      if(!windowed||bottom>=range.from&&node.y-node.h/2<=range.to)svg.appendChild(treeSvg('rect',{x:node.x-node.w/2+(node.indent||0),y:node.y-node.h/2,width:node.w-(node.indent||0),height:bottom-(node.y-node.h/2),rx:8,class:'tree-branch-band','aria-hidden':'true'}));
    }
    if(bySource.has(node.src)&&inWindow(node.y)){
      const guides=[[node.x+node.w/2+6,node.relationX-10]];
      if(treeRouting==='direction')guides.push([node.incomingX+10,node.x-node.w/2-6]);
      for(const [x1,x2] of guides){
        const guide=treeSvg('line',{x1,y1:node.y,x2,y2:node.y,class:'tree-row-guide','aria-hidden':'true'});
        svg.appendChild(guide);rowTargets.get(node.src).push(guide);
      }
    }
  }
  for(const edge of layout.hierarchy)if(!windowed||edge.points[0].y<=range.to&&edge.points.at(-1).y>=range.from)svg.appendChild(treeSvg('path',{d:roundedTreePath(edge.points,6),class:'package-tree-hierarchy','data-tree-hierarchy':String(edge.to),'aria-hidden':'true'}));
  const transit=[],batches=new Map(),transitLayer=treeSvg('g',{class:'tree-transit-edges'});
  if(windowed)svg.appendChild(transitLayer);
  for(const edge of links){
    if(!dependencyMatchesSelection(edge.from,edge.to))continue;
    const key=edge.from+' '+edge.to,base=model.routes.get(key);if(!base)continue;
    if(windowed&&Math.max(base.points[0].y,base.points.at(-1).y)<range.from||windowed&&Math.min(base.points[0].y,base.points.at(-1).y)>range.to)continue;
    const side=treeRouting==='direction'&&!cyclicKeys.has(key)&&reference?.ids.has(edge.to)&&!reference.ids.has(edge.from)?'left':'right';
    const route=treeDirectedRoute(base,side,layout.width);
    const source=byId.get(edge.from),target=byId.get(edge.to),link={...edge,source,target};
    if(windowed&&selectedLinkKey!==key&&(compactScene||!inWindow(route.points[0].y)&&!inWindow(route.points.at(-1).y))){
      const stroke=linkStrokeColor(link,route.side),dash=isPackageContextOnly(link)?'6 4':'',width=Math.min(3,widthFor(edgeWeight(edge))*.55),batchKey=[stroke,dash,width].join('|');
      if(!batches.has(batchKey))batches.set(batchKey,{stroke,dash,width,paths:[]});
      batches.get(batchKey).paths.push(treeClippedPath(route.points,range.from,range.to));transit.push({edge,route,link});continue;
    }
    const active=selectedLinkKey===key,group=treeSvg('g',{class:'package-tree-edge'+(active?' is-selected':''),'data-tree-edge':key,'data-tree-side':route.side,tabindex:0,role:'button','aria-label':source.name+' → '+target.name+' · '+weightLabel(edge)+' '+graphEvidenceUnit()});
    edgeTargets.push({group,from:source.src,to:target.src});
    const d=route.path;
    // Scale the hit stroke with the diagram. A fixed screen-space width
    // would cover adjacent lanes when the user fits or zooms out the tree.
    group.appendChild(treeSvg('path',{d,class:'tree-edge-hit',fill:'none',stroke:'transparent','stroke-width':8,'pointer-events':'stroke'}));
    const contextOnly=isPackageContextOnly(link);
    group.appendChild(treeSvg('path',{d,class:'tree-edge-line',fill:'none',stroke:linkStrokeColor(link,route.side),'stroke-width':Math.min(3,widthFor(edgeWeight(edge))*.55),'pointer-events':'none',...(contextOnly?{'stroke-dasharray':'6 4'}:{})}));
    const tip=route.points[route.points.length-1],prior=route.points[route.points.length-2],angle=Math.atan2(tip.y-prior.y,tip.x-prior.x)*180/Math.PI;
    group.appendChild(treeSvg('polygon',{points:'0,0 -9,-4.5 -9,4.5',transform:'translate('+tip.x+' '+tip.y+') rotate('+angle+')',fill:linkArrowColor(link,route.side),'data-tree-arrow':key,'pointer-events':'none'}));
    const incoming=reference?.ids.has(edge.to)&&!reference.ids.has(edge.from),outgoing=reference?.ids.has(edge.from)&&!reference.ids.has(edge.to);
    const routing=treeRouting==='ordered'?t('Directed dependency; the arrowhead marks the target.','Gerichtete Abhängigkeit; die Pfeilspitze markiert das Ziel.'):(incoming?t('Incoming to {name}.','Eingehend nach {name}.',{name:reference.name}):outgoing?t('Outgoing from {name}.','Ausgehend von {name}.',{name:reference.name}):t('Context connection.','Kontextbeziehung.'))+' '+(route.side==='left'?t('Left of the tree.','Links vom Baum.'):t('Right of the tree.','Rechts vom Baum.'));
    treeTooltip(group,[source.name+' → '+target.name,routing,...(isCyclic(link)?[t('Cyclic dependency. Cycle edges always appear on the right in red, including incoming edges.','Zyklische Abhängigkeit. Zykluskanten erscheinen immer rechts in Rot, auch eingehende Kanten.')]:[]),...(contextOnly?[packageCycleContextHelp()]:[]),weightLabel(edge)+' '+graphEvidenceUnit(),...edge.refs.slice(0,10).map(r=>r.file+':'+r.line+'  '+r.raw)]);
    group.addEventListener('click',()=>selectEdge(edge));
    group.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation?.();selectEdge(edge);if(event.key==='Enter')openInspector(true);}});
    group.addEventListener('pointerenter',()=>{svg.classList.add('edge-hover');group.classList.add('is-hovered');});
    group.addEventListener('pointerleave',()=>{svg.classList.remove('edge-hover');group.classList.remove('is-hovered');});
    svg.appendChild(group);
  }
  if(transit.length){
    const layer=transitLayer;layer.setAttribute('data-tree-transit',transit.length);
    for(const batch of batches.values())layer.appendChild(treeSvg('path',{d:batch.paths.join(' '),fill:'none',stroke:batch.stroke,'stroke-width':batch.width,...(batch.dash?{'stroke-dasharray':batch.dash}:{}),'pointer-events':'stroke'}));
    // Transit edges still open their source evidence. Hit testing is linear in
    // crossing edges only and runs on demand, without thousands of DOM targets.
    layer.addEventListener('click',event=>{
      const box=svg.getBoundingClientRect(),point={x:(event.clientX-box.left)/treeZoom,y:(event.clientY-box.top)/treeZoom};let nearest=null,best=64/(treeZoom*treeZoom);
      for(const candidate of transit)for(let i=1,points=treeRouteHitPoints(candidate.route);i<points.length;i++){
        const a=points[i-1],b=points[i],dx=b.x-a.x,dy=b.y-a.y,t=Math.max(0,Math.min(1,((point.x-a.x)*dx+(point.y-a.y)*dy)/(dx*dx+dy*dy||1)));
        const distance=(point.x-a.x-t*dx)**2+(point.y-a.y-t*dy)**2;
        if(distance<best){best=distance;nearest=candidate;}
      }
      if(nearest)selectEdge(nearest.edge);
    });
  }
  const iconPath=(kind,x,y)=>kind==='module'||kind==='project'?'M '+x+' '+(y-7)+' l 6 3.5 v 7 l -6 3.5 -6 -3.5 v -7 Z M '+(x-6)+' '+(y-3.5)+' l 6 3.5 6 -3.5 M '+x+' '+y+' v 7':'M '+(x-5)+' '+(y-7)+' h 7 l 3 3 v 11 h -10 Z';
  for(const node of visibleRows){
    if(!rowTargets.has(node.src))rowTargets.set(node.src,[]);
    const actual=bySource.get(node.src),isPackage=node.kind==='module',selected=ui.selection?.kind==='node'&&ui.selection.name===node.name;
    const review=actual&&reviewBySource.get(node.src),priority=ui.highlightPriorities&&review&&review.priority!=='low';
    const qualityCount=node.src==null?0:qualityNodeCount(node.src);
    const branchCycle=!!node.siblingCycle,inCycle=branchCycle||(actual&&model.cycleNodes.has(actual.id));
    const source=node.src===null?null:model.sources.get(node.src),hasChildren=source&&model.parents.has(node.src);
    const expanded=!!hasChildren&&!visible.has(node.src);
    const group=treeSvg('g',{transform:'translate('+node.x+' '+node.y+')',class:'package-tree-node tree-row'+(node.context?' tree-context':'')+(isPackage?' tree-package':'')+(selected?' is-selected':'')+(inCycle?' in-cycle':''),tabindex:0,role:'button','aria-label':node.name+(node.context?t(' · hierarchy context',' · Hierarchiekontext'):''),'aria-pressed':selected});
    group.setAttribute('data-tree-source',node.src===null?'project':String(node.src));
    if(actual)group.setAttribute('data-package-node',actual.name);
    if(branchCycle)group.setAttribute('data-tree-branch-cycle','true');
    if(typeof patternNodeMarked==='function'&&patternNodeMarked(node))group.setAttribute('data-pattern-node','true');
    if(hasChildren)group.setAttribute('aria-expanded',String(expanded));
    if(currentFilter()&&!node.name.toLowerCase().includes(currentFilter()))group.classList.add('search-dimmed');
    const x=-node.w/2,y=-node.h/2,indent=node.indent||0,iconX=x+indent+27,labelX=iconX+13;
    const lines=[node.project?(data.root||node.name):node.name,node.context?t('Hierarchy context','Hierarchiekontext'):isPackage?t('Package','Paket')+(review?t(' · {count} files',' · {count} Dateien',{count:review.file_count}):''):t('File','Datei')];
    if(compactScene){
      group.appendChild(treeSvg('rect',{x,y,width:node.w,height:node.h,rx:7,class:'tree-row-background','data-tree-row':'true'}));
      if(isPackage)group.appendChild(treeSvg('text',{x:labelX,y:4,class:'tree-node-name'},node.name.split('/').pop()));
    }else{
    group.appendChild(treeSvg('rect',{x,y,width:node.w,height:node.h,rx:7,class:'tree-row-background','data-tree-row':'true'}));
    group.appendChild(treeSvg('path',{d:iconPath(node.kind,iconX,0),class:'tree-row-icon'}));
    if(inCycle)group.appendChild(treeSvg('circle',{cx:iconX+6,cy:-6,r:2.5,class:'tree-cycle-mark','aria-hidden':'true'}));
    const base=node.project?node.name:node.context?node.name:node.name.split('/').pop();
    const labelRight=node.w/2-(actual?124:10)-(priority?30:0)-(qualityCount?40:0),limit=Math.max(4,Math.floor((labelRight-labelX)/6.6)),label=base.length>limit?base.slice(0,limit-1)+'…':base;
    const clipId='tree-label-'+(node.src===null?'project':node.src),clip=treeSvg('clipPath',{id:clipId});
    clip.appendChild(treeSvg('rect',{x:labelX,y:-14,width:Math.max(1,labelRight-labelX),height:28}));group.appendChild(clip);
    group.appendChild(treeSvg('text',{x:labelX,y:4,class:'tree-node-name','clip-path':'url(#'+clipId+')'},label));
    if(actual){
      const counts=degree.get(actual.id);
      group.appendChild(treeSvg('text',{x:node.w/2-116,y:4,class:'tree-direction-count',fill:C.incoming,'data-tree-incoming':counts.incoming},t('In {count}','Ein {count}',{count:connectionCountLabel(counts.incoming)})));
      group.appendChild(treeSvg('text',{x:node.w/2-8,y:4,'text-anchor':'end',class:'tree-direction-count',fill:C.outgoing,'data-tree-outgoing':counts.outgoing},t('Out {count}','Aus {count}',{count:connectionCountLabel(counts.outgoing)})));
      lines.push(t('{incoming} incoming · {outgoing} outgoing connections in the analysis scope, including hidden endpoints. ','{incoming} eingehende · {outgoing} ausgehende Beziehungen im Analysebereich, einschließlich ausgeblendeter Endpunkte. ',counts)+metricHelp('incoming',false)+' '+metricHelp('outgoing',false));
      if(connectionCountNotice())lines.push(connectionCountNotice());
    }
    if(priority){
      const badge=treeSvg('g',{'data-tree-review-priority':review.priority});
      badge.appendChild(treeSvg('rect',{x:node.w/2-153,y:-9,width:25,height:18,rx:3,class:'tree-priority-shape'}));
      badge.appendChild(treeSvg('text',{x:node.w/2-149,y:4,class:'tree-priority-text'},review.priority==='high'?'P1':'P2'));group.appendChild(badge);
      lines.push(reviewPriorityText(review),...review.reasons.map(reviewReasonText));
    }
    if(branchCycle)lines.push(t('Cycle between sibling branches: aggregated dependencies, not necessarily a cycle between individual files.','Zyklus zwischen benachbarten Zweigen: aggregierte Abhängigkeiten, nicht zwingend ein Zyklus zwischen einzelnen Dateien.'));
    else if(inCycle)lines.push(t('Part of a dependency cycle in the current scope.','Teil eines Abhängigkeitszyklus im aktuellen Ausschnitt.'));
    if(qualityCount){
      const badge=treeSvg('g',{'data-tree-quality-count':qualityCount}),x=node.w/2-(actual?124:10)-(priority?30:0)-38;
      badge.appendChild(treeSvg('rect',{x,y:-9,width:34,height:18,rx:3,class:'tree-priority-shape'}));
      badge.appendChild(treeSvg('text',{x:x+3,y:4,class:'tree-priority-text'},qualityBadgeText(qualityCount)));group.appendChild(badge);
      lines.push(qualityBadgeDescription(qualityCount));group.setAttribute('aria-label',group.getAttribute('aria-label')+' · '+qualityBadgeDescription(qualityCount));
    }
    if(hasChildren)lines.push(t('Use the chevron or Arrow Right / Left to expand or collapse this branch. Double-click to navigate into this package.','Mit dem Aufklapppfeil oder den Pfeiltasten rechts / links diesen Zweig öffnen oder schließen. Per Doppelklick in dieses Paket navigieren.'));
    }
    treeTooltip(group,lines);
    rowTargets.get(node.src).push(group);if(actual)bindEmphasis(group,node.src);
    function select(){if(node.project)resetPackageTree();else preserveTreeRowPosition(node.src,()=>selectNode(node.name));}
    function open(){if(node.project)resetPackageTree();else if(hasChildren)navigateExplorerNode(node.src);}
    function toggle(opened,disclosure=false){
      treeLastClick=null;
      preserveTreeRowPosition(node.src,()=>toggleTreeBranch(node.src,opened),disclosure?'data-tree-toggle':'data-tree-source');
    }
    if(hasChildren&&!compactScene){
      const disclosure=treeSvg('g',{transform:'translate('+(x+indent+10)+' 0)',class:'tree-disclosure','data-tree-toggle':String(node.src),tabindex:0,role:'button','aria-label':expanded?t('Collapse {name}','{name} schließen',{name:node.name}):t('Expand {name}','{name} öffnen',{name:node.name}),'aria-expanded':String(expanded)});
      disclosure.appendChild(treeSvg('rect',{x:-8,y:-10,width:16,height:20,rx:4}));
      disclosure.appendChild(treeSvg('path',{d:expanded?'M -3 -1.5 L 0 1.5 L 3 -1.5':'M -1.5 -3 L 1.5 0 L -1.5 3'}));
      disclosure.addEventListener('click',event=>{event.stopPropagation?.();toggle(!expanded,true);});
      disclosure.addEventListener('dblclick',event=>event.stopPropagation?.());
      disclosure.addEventListener('keydown',event=>{if(['Enter',' ','ArrowLeft','ArrowRight'].includes(event.key)){event.preventDefault();event.stopPropagation?.();toggle(event.key==='ArrowRight'?true:event.key==='ArrowLeft'?false:!expanded,true);}});
      group.appendChild(disclosure);
    }
    group.addEventListener('click',event=>{
      const twice=Number.isFinite(event.timeStamp)&&treeLastClick?.src===node.src&&event.timeStamp-treeLastClick.time>=0&&event.timeStamp-treeLastClick.time<450;
      treeLastClick=Number.isFinite(event.timeStamp)?{src:node.src,time:event.timeStamp}:null;
      if(twice&&isPackage){treeLastClick=null;open();}else select();
    });
    group.addEventListener('dblclick',open);
    group.addEventListener('keydown',event=>{if(windowed&&['ArrowUp','ArrowDown','Home','End'].includes(event.key)){
      event.preventDefault();const index=layout.nodes.indexOf(node),next=layout.nodes[event.key==='Home'?0:event.key==='End'?layout.nodes.length-1:Math.max(0,Math.min(layout.nodes.length-1,index+(event.key==='ArrowUp'?-1:1)))];
      container.scrollTop=Math.max(0,next.y*treeZoom-100);renderPackageTree();
      [...container.querySelectorAll('g[data-tree-source]')].find(n=>n.getAttribute('data-tree-source')===(next.src===null?'project':String(next.src)))?.focus({preventScroll:true});
    }else if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation?.();select();if(event.key==='Enter'&&!node.project)openInspector(true);}else if(hasChildren&&(event.key==='ArrowRight'||event.key==='ArrowLeft')){event.preventDefault();toggle(event.key==='ArrowRight');}});
    svg.appendChild(group);
    if(actual&&!compactScene)for(const side of treeRouting==='direction'?['left','right']:['right']){
      const portX=side==='left'?node.incomingX:node.relationX;
      const portLabel=treeRouting==='ordered'?t('Relations for {name}','Beziehungen für {name}',{name:node.name}):side==='left'?t('Incoming dependencies to {name}','Eingehende Abhängigkeiten zu {name}',{name:node.name}):t('Outgoing dependencies and cycles for {name}','Ausgehende Abhängigkeiten und Zyklen für {name}',{name:node.name});
      const mirror=treeSvg('g',{transform:'translate('+portX+' '+node.y+')',class:'tree-relation-node'+(selected?' is-selected':'')+(inCycle&&side==='right'?' in-cycle':''),'data-tree-relation':String(node.src),'data-tree-port-side':side,tabindex:0,role:'button','aria-label':portLabel,'aria-pressed':selected});
      // The port keeps the existing +/-8px edge anchors, with more hit room
      // than its visible outline. No duplicate package icon is needed here.
      mirror.appendChild(treeSvg('rect',{x:-10,y:-12,width:20,height:24,fill:'transparent',stroke:'none'}));
      mirror.appendChild(treeSvg('circle',{cx:0,cy:0,r:7.5,class:'tree-relation-background'}));
      mirror.appendChild(treeSvg('circle',{cx:0,cy:0,r:2.5,class:'tree-port-dot'}));
      rowTargets.get(node.src).push(mirror);bindEmphasis(mirror,node.src);
      treeTooltip(mirror,lines);mirror.addEventListener('click',select);mirror.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation?.();select();if(event.key==='Enter'&&!node.project)openInspector(true);}});svg.appendChild(mirror);
    }
  }
  container.appendChild(svg);
  if(focusedSource!=null){const attribute=focusedToggle!=null?'data-tree-toggle':'data-tree-source';[...container.querySelectorAll('g['+attribute+']')].find(n=>n.getAttribute(attribute)===focusedSource)?.focus({preventScroll:true});}
  let nextLeft=left||0;
  if(layoutChanged&&!resized){
    // A dense left gutter can be wider than the viewport. Reveal the package
    // column on scope changes, while preserving deliberate scrolling on filters.
    const anchor=layout.nodes.find(n=>n.name===reference?.name)||layout.nodes.find(n=>!n.context)||layout.nodes[0];
    const width=container.clientWidth||container.getBoundingClientRect().width;
    if(anchor&&width>0&&((anchor.x-anchor.w/2)*treeZoom<nextLeft||(anchor.x+anchor.w/2)*treeZoom>nextLeft+width))nextLeft=Math.max(0,anchor.x*treeZoom-width/2);
  }
  container.scrollTop=top;container.scrollLeft=nextLeft;updateTreeZoomControls();
}
