// DOM/ForceGraph test doubles exercise the actual embedded application.
// These are deterministic state tests, not browser/rendering acceptance.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),os=require('node:os');
const js=['viewer_scc.js','viewer_i18n.js','cycle_planner.js','viewer_review.js','viewer_quality_metrics.js','viewer_quality_findings.js','viewer_detail_graphs.js','viewer_class_heatmap.js','viewer_engineering_quality.js','viewer_dependencies.js','viewer_tree_layout.js','viewer_tree.js','viewer_panels.js','viewer_services.js','viewer_workspace.js','viewer_export.js','viewer_patterns.js','viewer_architecture.js','viewer_assessment.js','viewer_calls_tree.js','viewer_call_rankings.js','viewer_method_metrics.js','viewer_call_rankings_ui.js','viewer_call_cycles_model.js','viewer_call_cycles_ui.js','viewer_security.js','viewer_repository.js','viewer_world_model.js','viewer_world.js','viewer_development.js','viewer.js','viewer_scenarios.js'].map(name=>fs.readFileSync('crates/archguard-report/src/'+name,'utf8')).join('\n');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
class Element {
  constructor(tag,document){this.tagName=tag.toUpperCase();this.document=document;this.children=[];this.attributes={};this.dataset={};this.events={};this.style={setProperty:(k,v)=>this.style[k]=v};this._text='';this.value='';this.hidden=false;this.disabled=false;this.open=false;this.className='';}
  get classList(){const owner=this;return {add(...names){const set=new Set(owner.className.split(' ').filter(Boolean));names.forEach(n=>set.add(n));owner.className=[...set].join(' ');},remove(...names){owner.className=owner.className.split(' ').filter(n=>!names.includes(n)).join(' ');},contains(name){return owner.className.split(' ').includes(name);},toggle(name,force){const add=force===undefined?!this.contains(name):force;if(add)this.add(name);else this.remove(name);return add;}};}
  set textContent(value){this._text=String(value);this.children=[];}
  get textContent(){return this._text+this.children.map(c=>c.textContent).join('');}
  get childNodes(){return this.children;}
  set innerHTML(value){throw new Error('Unexpected HTML sink: '+value);}
  appendChild(child){this.children.push(child);child.parentElement=this;return child;}
  remove(){if(this.parentElement){this.parentElement.children=this.parentElement.children.filter(child=>child!==this);this.parentElement=null;}}
  setAttribute(key,value){this.attributes[key]=String(value);if(key==='class')this.className=String(value);if(key.startsWith('data-'))this.dataset[key.slice(5)]=String(value);}
  getAttribute(key){return this.attributes[key]??null;}
  removeAttribute(key){delete this.attributes[key];if(key.startsWith('data-'))delete this.dataset[key.slice(5)];}
  contains(node){for(let current=node;current;current=current.parentElement)if(current===this)return true;return false;}
  addEventListener(type,callback){(this.events[type]??=[]).push(callback);}
  dispatch(type,event={}){for(const callback of this.events[type]||[])callback({target:this,stopPropagation(){},preventDefault(){},...event});}
  click(){if(!this.disabled)this.dispatch('click');}
  focus(){this.document.activeElement=this;}
  showModal(){this.open=true;}
  close(){this.open=false;}
  get isConnected(){return true;}
  getBoundingClientRect(){return this.classList.contains('panel-hidden')?{width:0,height:0}:{width:800,height:600};}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  querySelectorAll(selector){const [tag,attr]=selector.split('[');const matches=[];for(const child of this.children){if(child.tagName===tag.toUpperCase()&&(!attr||child.getAttribute(attr.slice(0,-1))!==null||attr==='data-edge]'&&child.dataset.edge!==undefined))matches.push(child);matches.push(...child.querySelectorAll(selector));}return matches;}
}
function launch(data,{library=true,width=1440,reducedMotion=false,deferGraphData=false,storage,location,live=false}={}){
  const ids=new Map(),document={events:{},activeElement:null};
  document.createElement=tag=>new Element(tag,document);document.createTextNode=text=>{const e=document.createElement('#text');e.textContent=text;return e;};
  document.createElementNS=(namespace,tag)=>{assert.equal(namespace,'http://www.w3.org/2000/svg');return document.createElement(tag);};
  document.documentElement=document.createElement('html');
  document.head=document.createElement('head');
  for(const match of html.matchAll(/<([a-z]+)[^>]*\bid="([^"]+)"[^>]*>/g))ids.set(match[2],document.createElement(match[1]));
  document.getElementById=id=>{assert.ok(ids.has(id),'missing HTML element '+id);return ids.get(id);};
  const decode=value=>value.replace(/&(amp|quot|lt|gt|#39);/g,(_,name)=>({amp:'&',quot:'"',lt:'<',gt:'>','#39':"'"}[name]));
  const staticNodes=[];
  for(const match of html.matchAll(/<([a-z]+)([^>]*)>([^<]*)/g)){
    const attributes=Object.fromEntries([...match[2].matchAll(/([\w:-]+)="([^"]*)"/g)].map(value=>[value[1],decode(value[2])]));
    if(!Object.keys(attributes).some(key=>key.startsWith('data-i18n-')))continue;
    const node=attributes.id?document.getElementById(attributes.id):document.createElement(match[1]);
    for(const [key,value] of Object.entries(attributes))node.setAttribute(key,value);
    node.textContent=decode(match[3]);staticNodes.push(node);
  }
  document.getElementById('archguard-data').textContent=JSON.stringify(data);
  document.getElementById('graph').appendChild(document.createElement('canvas'));
  for(const [group,attr,values] of [['tabs','tab',['start','graph','dsm','calls','services','architecture','security','quality','assessment']],['explorer-tabs','explorer',['modules','cycles','review']],['theme-switch','mode',['light','dark','system']],['edge-modes','edges',['incoming','outgoing','both','all']]])for(const value of values){const b=staticNodes.find(node=>node.getAttribute('data-'+attr)===value)||document.createElement('button');b.setAttribute('data-'+attr,value);document.getElementById(group).appendChild(b);}
  document.querySelectorAll=selector=>{if(selector.startsWith('[data-i18n-'))return staticNodes.filter(node=>node.getAttribute(selector.slice(1,-1))!==null);const [id,...rest]=selector.split(' ');return document.getElementById(id.slice(1)).querySelectorAll(rest.join(' '));};
  document.addEventListener=(type,callback)=>(document.events[type]??=[]).push(callback);
  const graph={updates:0,zoom:1,forces:{},downloads:[],fits:[],centers:[]},force={strength(){return this;},distance(){return this;}};
  // Real force-graph resolves endpoints and assigns coordinates in a deferred
  // update. Tests can retain that intermediate state until this explicit tick.
  graph.flushLayout=()=>{
    graph.data.nodes.forEach((n,i)=>{n.x??=(i%10)*200;n.y??=Math.floor(i/10)*100;});
    // force-graph delegates ID endpoint resolution to d3's link force. With
    // that force removed, the application must supply node objects itself.
    if(graph.forces.link!==null)graph.data.links.forEach(l=>{
      l.source=graph.data.nodes.find(n=>n.id===(typeof l.source==='object'?l.source.id:l.source));
      l.target=graph.data.nodes.find(n=>n.id===(typeof l.target==='object'?l.target.id:l.target));
    });
    if(graph.onEngineTick)graph.onEngineTick();
  };
  let proxy;proxy=new Proxy(function(){},{get(_target,name){return (...args)=>{
    if(name==='d3Force'){if(args.length===1)return force;graph.forces[args[0]]=args[1];return proxy;}
    if(name==='zoom'&&!args.length)return graph.zoom;
    if(name==='centerAt'){graph.centers.push(args);return proxy;}
    if(name==='screen2GraphCoords'){
      const t=graph.screenTransform||{x:0,y:0,scale:1};
      return {x:(args[0]-t.x)/t.scale,y:(args[1]-t.y)/t.scale};
    }
    if(name==='zoomToFit'){
      assert.ok(graph.data.nodes.length&&graph.data.nodes.every(n=>Number.isFinite(n.x)&&Number.isFinite(n.y)),'cannot fit a graph before its node positions exist');
      graph.fits.push(args);return proxy;
    }
    if(name==='graphData'){
      graph.updates++;graph.data=args[0];
      if(!deferGraphData)graph.flushLayout();
    }else graph[name]=args[0];return proxy;
  };}});
  const window={innerWidth:width,matchMedia:query=>({matches:query.includes('reduced-motion')&&reducedMotion,addEventListener(){}}),addEventListener(){},archguardLiveViewer:live,...(location?{location}:{})};
  document.getElementById('main').getBoundingClientRect=()=>({width:window.innerWidth,height:600,left:0,top:0});
  const context=vm.createContext({document,window,navigator:{},localStorage:storage||{getItem(){throw Error('blocked');},setItem(){throw Error('blocked');}},getComputedStyle:()=>({getPropertyValue:()=>''}),console,Blob,URL:class extends (require('node:url').URL){static createObjectURL(blob){graph.downloads.push(blob);return 'blob:test';}static revokeObjectURL(){}},setTimeout:callback=>callback,clearTimeout(){},setInterval(){},clearInterval(){},ResizeObserver:class{observe(){}},...(library?{ForceGraph:()=>()=>proxy}:{})});
  const start=performance.now();vm.runInContext(js,context);const loadMs=performance.now()-start;
  return {context,document,graph,ids,staticNodes,loadMs,installLibrary:()=>{context.ForceGraph=()=>()=>proxy;},run:code=>vm.runInContext(code,context),click:id=>ids.get(id).click()};
}
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to,count=1)=>({from,to,count,refs:[{file:'<img src=x onerror=alert(1)>.ts',line:1,raw:'import "<&>";'}]});
const fixture={root:'/tmp/example',nodes:[node(0,'A'),node(1,'B'),node(2,'C'),node(3,'D'),node(4,'E')],edges:[edge(0,1,25),edge(1,0,2),edge(1,2,5),edge(2,0),edge(3,4,100)],view:{depth:0,visible:null},cyclicEdges:[],findings:[{id:'scc-1',kind:'cycle',severity:'error',nodes:['A','B','C'],edge_count:4,suggested_cuts:[],sampling:{complete:false,searched_from:'A',explored_edges:8}}]};
const app=launch(fixture);
const summaryMetrics=app.ids.get('summary').children.filter(n=>n.className==='metric');
assert.equal(summaryMetrics.length,4);
for(const metric of summaryMetrics){
  assert.ok(metric.getAttribute('data-metric-description'),'every summary metric has an explanation');
  assert.equal(metric.getAttribute('tabindex'),'0','metric explanations are keyboard reachable');
  metric.focus();metric.dispatch('focus');
  assert.equal(metric.getAttribute('aria-describedby'),'metric-tooltip');
  assert.equal(app.ids.get('metric-tooltip').textContent,metric.getAttribute('data-metric-description'),'assistive technology receives the visible explanation');
}
const acdMetric=summaryMetrics.find(n=>n.textContent.includes('ACD'));
assert.match(acdMetric.getAttribute('data-metric-description'),/Average Component Dependency/);
assert.match(acdMetric.getAttribute('data-metric-description'),/direct or indirect.*including.*itself/);
assert.match(acdMetric.getAttribute('data-metric-description'),/2\.40/,'the tooltip explains the actual displayed scope value');
assert.match(summaryMetrics[1].getAttribute('data-metric-description'),/Multiple import references.*one connection/);
app.run("selectNode('A')");
const nodeMetrics=app.ids.get('sidebar').querySelectorAll('div').filter(n=>n.getAttribute('data-metric-help'));
assert.equal(nodeMetrics.length,3,'incoming, outgoing and instability each explain their own meaning');
assert.ok(nodeMetrics.some(n=>/not runtime reliability/.test(n.getAttribute('data-metric-description'))));
app.graph.onBackgroundClick();
assert.ok(app.graph.data.links.every(l=>l.source&&typeof l.source==='object'&&l.target&&typeof l.target==='object'),'disabled link force still leaves real node endpoints for drawing and selection');
// Canvas callbacks can run between graphData() and the library's deferred
// endpoint/coordinate initialization. They must leave that frame untouched.
const paint=[];
const paintContext=new Proxy({}, {get(_target,method){return (...args)=>{
  assert.ok(args.every(arg=>typeof arg!=='number'||Number.isFinite(arg)),'finite canvas arguments for '+String(method));
  paint.push(method);
};}});
for(const endpoints of [
  {source:0,target:1},
  {source:{id:0,name:'A'},target:{id:1,name:'B'}},
  {source:{id:0,name:'A',x:0,y:0},target:{id:1,name:'B',x:NaN,y:20}},
])app.graph.linkCanvasObject({...endpoints,count:25,refs:[]},paintContext,1);
assert.equal(paint.length,0,'unresolved links do not paint or throw');
app.graph.nodeCanvasObject({id:0,name:'A'},paintContext,1);
app.graph.nodePointerAreaPaint({id:0,name:'A'},'#000',paintContext);
assert.equal(paint.length,0,'unpositioned nodes do not paint');
app.run("ui.edgeMode='all'");
app.graph.linkCanvasObject({source:{id:0,name:'A',x:0,y:0},target:{id:1,name:'B',x:200,y:0},count:25,refs:[]},paintContext,1);
assert.ok(paint.includes('fill'),'a resolved directed link still paints its arrow');
app.run("ui.edgeMode='both'");
assert.equal(app.run('storedTheme()'),'system');app.run("applyTheme('dark')");assert.equal(app.document.documentElement.getAttribute('data-theme'),'dark');
assert.equal(app.run('ops.length'),0);
assert.equal(app.run('window.archguardCanReload()'),true);
app.run('setInspectorOpen(false)'); // An edge selection alone still keeps closed details closed.
const updates=app.graph.updates;
app.graph.onLinkClick(app.graph.data.links[0]);
assert.equal(app.run('ops.length'),0,'browsing edge cannot cut');
assert.match(app.ids.get('sidebar').textContent,/A → B/);assert.match(app.ids.get('sidebar').textContent,/25 import references/);
assert.match(app.ids.get('sidebar').textContent,/<img src=x onerror=alert\(1\)>/);
assert.equal(app.graph.updates,updates,'selection preserves simulation/layout');
assert.notEqual(app.document.activeElement,app.ids.get('sidebar'),'selection does not transfer focus to hidden details');
assert.equal(app.ids.get('main').classList.contains('inspector-closed'),true);
const tooltip=app.graph.linkLabel(app.graph.data.links[0]);assert.equal(tooltip.tagName,'DIV');assert.match(tooltip.textContent,/<img/);assert.equal(tooltip.querySelectorAll('img').length,0);
app.graph.onLinkClick(app.graph.data.links[1]);assert.match(app.ids.get('sidebar').textContent,/B → A/);assert.match(app.ids.get('sidebar').textContent,/2 import references/);
app.run("copyText('x')");assert.match(app.ids.get('feedback').textContent,/unavailable/);
app.run("showTab('dsm')");assert.equal(app.ids.get('dsm-tab').querySelectorAll('button[data-edge]').length,5);
const cells=app.ids.get('dsm-tab').querySelectorAll('button[data-edge]');cells.find(c=>c.dataset.edge==='0 1').click();assert.match(app.ids.get('sidebar').textContent,/A → B/);
app.run("showTab('graph')");assert.equal(app.graph.updates,updates);
assert.equal(app.graph.enableNodeDrag,false,'the containment layout does not allow accidental dragging');
assert.equal(app.graph.onNodeDragEnd,undefined,'simulation uses explicit confirmed move actions');
assert.equal(app.run('ops.length'),0);
app.click('simulation-mode');app.run("askSimulation(encodeCut('A','B'),'Cut A → B?')");assert.equal(app.ids.get('action-dialog').open,true);app.click('cancel-action');assert.equal(app.run('ops.length'),0);
app.ids.get('sidebar').querySelectorAll('button').find(b=>b.textContent==='Propose cut').focus();
app.run("askSimulation(encodeCut('A','B'),'Cut A → B?')");app.click('confirm-action');assert.equal(app.run('ops.length'),1);assert.equal(app.run('sccs.length'),0);
assert.equal(app.document.activeElement,app.ids.get('sidebar'),'confirmation focuses the rebuilt Inspector');
assert.equal(app.run('window.archguardCanReload()'),false,'live updates preserve pending simulation');
app.click('simulation-mode');assert.equal(app.run('ops.length'),1,'leaving Simulation preserves changes');assert.match(app.ids.get('simulation-summary').textContent,/Preserved/);
app.click('undo-ops');assert.equal(app.run('ops.length'),0);assert.equal(app.run('sccs.length'),1);
app.run('focusCycle(cyclesForView()[0])');assert.equal(app.run('renderedNodes.length'),3);assert.match(app.ids.get('sidebar').textContent,/incomplete/);app.click('cycles-reset');assert.equal(app.run('renderedNodes.length'),5);
app.run("selectNode('A'); recordOp(encodeCut('A','B'))");assert.equal(app.run('ops.length'),0);
const retainedSelection=app.run('JSON.stringify(ui.selection)');app.document.events.keydown[0]({key:'Escape'});
assert.equal(app.run('JSON.stringify(ui.selection)'),retainedSelection,'first Escape closes details and retains selection');
app.document.events.keydown[0]({key:'Escape'});assert.equal(app.run('ui.selection'),null);
const selectedExplorerRows=()=>app.ids.get('module-list').querySelectorAll('button[data-node]').filter(b=>b.getAttribute('aria-pressed')==='true');
assert.equal(selectedExplorerRows().length,0,'Escape clears the Explorer selection indicator too');
app.run("selectNode('A')");const selectionRow=selectedExplorerRows()[0];
app.graph.onLinkClick(app.graph.data.links[0]);assert.equal(selectedExplorerRows().length,0,'edge selection clears the old node highlight');
assert.ok(app.ids.get('module-list').querySelectorAll('button[data-node]').includes(selectionRow),'selection refresh preserves existing Explorer controls');
app.run("selectNode('A')");app.graph.onBackgroundClick();assert.equal(selectedExplorerRows().length,0,'background click clears the Explorer selection indicator');
app.ids.get('filter').value='unknown';app.run('applySearch()');assert.match(app.ids.get('module-list').textContent,/No matching/);app.click('clear-search');assert.equal(app.ids.get('filter').value,'');
const missing=launch(fixture,{library:false,width:768});assert.equal(missing.ids.get('graph-state').textContent,'','the offline tree does not depend on the optional network library');missing.run("showTab('dsm')");assert.equal(missing.ids.get('dsm-tab').querySelectorAll('button[data-edge]').length,5);
const empty=launch({...fixture,nodes:[],edges:[],findings:[]});assert.match(empty.ids.get('module-list').textContent,/No source files/);
// Stable node selection cannot accidentally move to a different dense ID after zoom.
const hierarchy=launch({...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/a',0),node(2,'app/b',0),node(3,'other')],edges:[edge(1,3),edge(3,2)],findings:[]});
hierarchy.run("selectNode('other');expandNode('app')");assert.equal(hierarchy.run('nodeById(view,selectedId).name'),'other');hierarchy.run('resetView()');assert.equal(hierarchy.run('ui.selection'),null);
// A real double click must expose package contents even when the overview's
// file filter is off and another package remains visible.
const navigation=launch({...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/domain',0,'module'),node(2,'app/domain/Entity.ts',1),node(3,'app/Service.ts',0),node(4,'other',null,'module'),node(5,'other/Hidden.ts',4),node(6,'loose.ts')],edges:[edge(3,2),edge(2,5)],findings:[]});
navigation.graph.screenTransform={x:135,y:90,scale:.65};
const navigationCanvas=navigation.ids.get('graph').querySelector('canvas');
function doubleClickAt(x,y){
  const t=navigation.graph.screenTransform;
  navigationCanvas.dispatch('dblclick',{offsetX:x*t.scale+t.x,offsetY:y*t.scale+t.y,clientX:x*t.scale+t.x+400,clientY:y*t.scale+t.y+200});
}
const navigationNames=()=>Array.from(navigation.graph.data.nodes,n=>n.name).sort();
const appCard=navigation.graph.data.nodes.find(n=>n.name==='app');
navigation.graph.onNodeClick(appCard);
assert.equal(navigation.run('ui.selection.name'),'app','single click still selects without expanding');
assert.equal(navigation.run('trail.length'),0);
const appBox=navigation.run('nodeBox(renderedNodes.find(n=>n.name==="app"))');
doubleClickAt(appCard.x+appBox.w/2-4,appCard.y-appBox.h/2+4);
doubleClickAt(10000,10000);
assert.equal(navigation.run('trail.length'),0,'background and the package-tab cutout do not navigate');
navigation.click('simulation-mode');navigation.run("recordOp(encodeCut('app/Service.ts','app/domain/Entity.ts'))");
const navigationModel=navigation.run('JSON.stringify([data.nodes,data.edges,work,ops])');
doubleClickAt(appCard.x,appCard.y);
assert.deepEqual(navigationNames(),['app/Service.ts','app/domain','other'],'opening a package reveals direct files without revealing unrelated files');
assert.equal(navigation.run('ui.showFiles'),false,'opening one package keeps the overview file preference');
assert.equal(navigation.run('trail.join()'),'0');
assert.equal(navigation.run('explorerOpen.has(0)'),true,'the opened package is expanded in the Explorer too');
const domainCard=navigation.graph.data.nodes.find(n=>n.name==='app/domain');
doubleClickAt(domainCard.x,domainCard.y);
assert.deepEqual(navigationNames(),['app/Service.ts','app/domain/Entity.ts','other']);
assert.equal(navigation.run('trail.join()'),'0,1');
assert.equal(navigation.run('explorerOpen.has(1)'),true);
const fileCard=navigation.graph.data.nodes.find(n=>n.name==='app/domain/Entity.ts');
doubleClickAt(fileCard.x,fileCard.y);
assert.equal(navigation.run('trail.join()'),'0,1','double-clicking a file never expands the tree');
navigation.ids.get('breadcrumbs').querySelectorAll('button[data-node]').find(b=>b.dataset.node==='0').click();
assert.deepEqual(navigationNames(),['app/Service.ts','app/domain','other'],'parent breadcrumb opens that package and restores the previous level');
assert.equal(navigation.run('ui.selection.name'),'app');
navigation.ids.get('breadcrumbs').querySelectorAll('button')[0].click();
assert.deepEqual(navigationNames(),['app','other']);
assert.equal(navigation.run('trail.length'),0);
assert.equal(navigation.run('JSON.stringify([data.nodes,data.edges,work,ops])'),navigationModel,'navigation preserves source data and pending simulation operations');
const deferred=launch({...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/a',0),node(2,'app/b',0),node(3,'other')],edges:[edge(1,3),edge(3,2)],findings:[]},{deferGraphData:true});
assert.equal(deferred.graph.centers.length,0,'initial fitting waits until force-graph has finished its automatic zoom update');
deferred.graph.flushLayout();
const beforeExpandFits=deferred.graph.centers.length;
deferred.run("expandNode('app')");
assert.equal(deferred.graph.centers.length,beforeExpandFits,'scope fitting also waits for the deferred library update');
assert.ok(deferred.graph.data.nodes.every(n=>Number.isFinite(n.x)&&n.x===n.fx&&n.y===n.fy));
deferred.graph.zoom=3; // The real library may set 4/cbrt(nodeCount) here.
deferred.graph.flushLayout();
const expandedFits=deferred.graph.centers.length;
assert.equal(expandedFits,beforeExpandFits+1,'the first initialized tick applies one final fit');
assert.ok(deferred.graph.zoom<3,'the final fit replaces the library automatic zoom');
deferred.graph.flushLayout();
assert.equal(deferred.graph.centers.length,expandedFits,'later ticks do not restart the camera');
const noNodes=launch({...fixture,nodes:[],edges:[],findings:[]});noNodes.run('fitView()');assert.equal(noNodes.graph.centers.length,0,'an empty view never fits invalid bounds');
const hiddenGraph=launch({...fixture,initialTab:'dsm'},{deferGraphData:true});
hiddenGraph.graph.flushLayout();
assert.equal(hiddenGraph.graph.centers.length,0,'a hidden graph must not consume its pending Fit with a zero viewport');
assert.equal(hiddenGraph.run('pendingFit'),true);
hiddenGraph.run("showTab('graph')");
assert.equal(hiddenGraph.graph.centers.length,1,'showing Graph completes the pending Fit');
assert.ok(hiddenGraph.graph.zoom>.1);
const hiddenZoom=hiddenGraph.graph.zoom;hiddenGraph.run("showTab('dsm');fitView()");
assert.equal(hiddenGraph.graph.zoom,hiddenZoom,'Fit in hidden Matrix workspace preserves the current camera');
hiddenGraph.run("showTab('graph')");assert.equal(hiddenGraph.graph.centers.length,2);
const scope=launch({...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/a',0),node(2,'app/b',0)],edges:[edge(1,2),edge(2,1)],findings:[]});
assert.equal(scope.run('baselineStats.cycles'),0);scope.run("expandNode('app')");assert.equal(scope.run('baselineStats.cycles'),1,'before/after uses the current expansion scope');
scope.click('simulation-mode');scope.run("recordOp(encodeCut('app/a','app/b'))");assert.equal(scope.run('baselineStats.cycles'),1);assert.equal(scope.run('sccs.length'),0);
scope.run("recordOp(encodeMove('app','app/a'))");assert.equal(scope.run('ops.length'),1);assert.match(scope.ids.get('feedback').textContent,/not a module/);
scope.click('clear-ops');assert.equal(scope.run('sccs.length'),1);assert.equal(scope.run('ops.length'),0);
const groups=launch({...fixture,nodes:Array.from({length:6},(_,i)=>node(i,String.fromCharCode(65+i))),edges:[edge(0,1),edge(1,0),edge(2,3),edge(3,2),edge(4,5),edge(5,4)],findings:[['A','B'],['C','D'],['E','F']].map((nodes,i)=>({id:'scc-'+i,kind:'cycle',nodes,edge_count:2,suggested_cuts:[]})),initialTab:'cycles'},{width:768,reducedMotion:true});
assert.equal(groups.run('ui.explorer'),'cycles');assert.equal(groups.ids.get('main').classList.contains('explorer-closed'),false);assert.equal(groups.graph.cooldownTicks,1);
for(let i=0;i<3;i++){groups.run('focusCycle(cyclesForView()['+i+'])');assert.equal(groups.run('renderedNodes.map(n=>n.name).join()'),[['A','B'],['C','D'],['E','F']][i].join());assert.equal(groups.ids.get('cycles-panel').children.length,3);groups.click('cycles-reset');}
const ordered=launch({...fixture,nodes:[node(0,'a'),node(1,'Z'),node(2,'_')],edges:[],findings:[]});
assert.equal(ordered.run('matrixOrder().map(n=>n.name).join()'),'Z,_,a','ordering follows Rust name ordering, not locale collation');
// Directed geometry uses the same curve for both direction, with mirrored routing.
app.run("globalThis.ab={source:{x:0,y:0,name:'A'},target:{x:200,y:0,name:'B'}};globalThis.ba={source:ab.target,target:ab.source};");
assert.equal(app.run('curvePoint(ab,.5).y'),-app.run('curvePoint(ba,.5).y'));
assert.equal(app.run('hitNode(ab.target,arrowGeometry(ab).x,arrowGeometry(ab).y)'),false);
for(const count of [1,2,5,25,100]){assert.equal(app.graph.linkWidth({count,refs:[]}),Math.min(8,1.5+Math.log2(count)));assert.equal(app.graph.linkCurvature({count}),.25);}
const geometry=[];app.graph.nodePointerAreaPaint({name:'A',kind:'file',x:10,y:20},'#000',{beginPath(){},roundRect(...args){geometry.push(args);},fill(){}});
assert.deepEqual(geometry[0],[-34,4,88,32,8]);
// Package silhouette is shared by drawing and picking, including its tab and
// empty upper-right cutout. The outline, rather than its box, clips arrows.
app.run("globalThis.packageNode={id:90,name:'domain',shortLabel:'domain',kind:'module',x:0,y:0,fileCount:3,packageCount:1};");
assert.equal(app.run('hitNode(packageNode,0,0)'),true);
assert.equal(app.run('hitNode(packageNode,-nodeBox(packageNode).w/2+4,-nodeBox(packageNode).h/2+4)'),true);
assert.equal(app.run('hitNode(packageNode,nodeBox(packageNode).w/2-4,-nodeBox(packageNode).h/2+4)'),false,'tab cutout must not be clickable');
assert.equal(app.run('hitNode(packageNode,nodeBox(packageNode).w/2+1,0)'),false);
function canvasRecorder(){
  const calls=[],properties={};
  const context=new Proxy(properties,{get(target,name){
    if(name in target)return target[name];
    if(name==='measureText')return text=>({width:text.length*(parseFloat(target.font)||12)*.55});
    return (...args)=>{assert.ok(args.every(arg=>typeof arg!=='number'||Number.isFinite(arg)));calls.push([name,...args]);};
  }});
  return {calls,context};
}
// Direction must be readable along a long edge, not only as a tiny endpoint
// triangle. Screen-space size stays constant across zoom levels.
app.run("ui.edgeMode='all';selectedLinkKey=null");
const directedFrom={id:900,name:'from',kind:'module',x:0,y:0};
const directedTo={id:901,name:'to',kind:'module',x:900,y:0};
const directed={source:directedFrom,target:directedTo,count:1,refs:[]};
for(const scale of [.3,.65,1,2]){
  const recorded=canvasRecorder();app.graph.linkCanvasObject(directed,recorded.context,scale);
  assert.equal(recorded.calls.filter(c=>c[0]==='fill').length,2,'long edges paint an endpoint arrow and an on-curve direction marker');
  assert.equal(recorded.calls.filter(c=>c[0]==='stroke').length,2,'each arrow has a contrast outline');
  const wings=recorded.calls.filter(c=>c[0]==='lineTo');
  assert.ok(wings.every(c=>Math.abs(c[1])*scale>=10),'weak dependencies still have at least10px long arrowheads');
  const midpoint=app.run('curvePoint({source:{x:0,y:0},target:{x:900,y:0}},.5)');
  assert.ok(recorded.calls.some(c=>c[0]==='translate'&&c[1]===midpoint.x&&c[2]===midpoint.y),'direction marker follows the drawn curve');
}
const reversePaint=canvasRecorder();app.graph.linkCanvasObject({...directed,source:directedTo,target:directedFrom},reversePaint.context,1);
assert.ok(reversePaint.calls.some(c=>c[0]==='rotate'&&Math.abs(Math.abs(c[1])-Math.PI)<1e-8),'reverse direction points back along its own curve');
const shortPaint=canvasRecorder();app.graph.linkCanvasObject({...directed,target:{...directedTo,x:200}},shortPaint.context,1);
assert.equal(shortPaint.calls.filter(c=>c[0]==='fill').length,1,'short edges do not crowd cards with a second arrow');
app.run("selectedLinkKey='900 901'");
const labeledArrow=canvasRecorder();app.graph.linkCanvasObject(directed,labeledArrow.context,.65);
const weightPosition=labeledArrow.calls.find(c=>c[0]==='fillText');
assert.ok(weightPosition[3]<app.run('curvePoint({source:{x:0,y:0},target:{x:900,y:0}},.5).y')-14/.65,'weight label clears the middle arrow');
app.run('selectedLinkKey=null');
// Larger outlined arrows must not lose their wings behind package cards,
// including oblique approaches to the package tab and at overview zoom.
for(const scale of [.3,1,2])for(const distance of [220,600])for(let degrees=0;degrees<360;degrees+=5){
  const source={...directedFrom,x:Math.cos(degrees*Math.PI/180)*distance,y:Math.sin(degrees*Math.PI/180)*distance};
  const target={...directedTo,x:0,y:0},recorder=canvasRecorder();
  app.graph.linkCanvasObject({source,target,count:100,refs:[]},recorder.context,scale);
  const tips=recorder.calls.filter(c=>c[0]==='translate'),angles=recorder.calls.filter(c=>c[0]==='rotate');
  if(distance===600)assert.equal(tips.length,2,'long visible curves retain both direction markers: '+[scale,distance,degrees]);
  for(let i=0;i<tips.length;i++){
    const angle=angles[i][1],size=14/scale;
    for(const [u,v] of [[0,0],[-size,-size*.6],[-size,size*.6],[-size,0],[-size*.5,-size*.3],[-size*.5,size*.3]]){
      const x=tips[i][1]+u*Math.cos(angle)-v*Math.sin(angle),y=tips[i][2]+u*Math.sin(angle)+v*Math.cos(angle);
      app.context.arrowProbe={source,target,x,y};
      assert.equal(app.run('hitNode(arrowProbe.source,arrowProbe.x,arrowProbe.y)||hitNode(arrowProbe.target,arrowProbe.x,arrowProbe.y)'),false,'complete arrow stays outside both cards');
    }
  }
}
app.run("ui.edgeMode='both'");
const packagePaint=canvasRecorder(),packageHit=canvasRecorder();
app.graph.nodeCanvasObject(app.run('packageNode'),packagePaint.context,1);
app.graph.nodePointerAreaPaint(app.run('packageNode'),'#000',packageHit.context);
const firstOutline=calls=>calls.slice(0,calls.findIndex(c=>c[0]==='fill')).filter(c=>['beginPath','moveTo','lineTo','closePath'].includes(c[0]));
assert.deepEqual(firstOutline(packagePaint.calls),firstOutline(packageHit.calls));
assert.ok(firstOutline(packagePaint.calls).filter(c=>c[0]==='lineTo').length>=5,'package has a tabbed silhouette');
assert.ok(packagePaint.calls.some(c=>c[0]==='fillText'&&c[1]==='domain'));
assert.ok(packagePaint.calls.some(c=>c[0]==='fillText'&&String(c[1]).includes('3 files')));
const overviewPaint=canvasRecorder();app.graph.nodeCanvasObject(app.run('packageNode'),overviewPaint.context,.3);
assert.ok(overviewPaint.calls.some(c=>c[0]==='fillText'&&String(c[1]).includes('domain')),'package name remains visible at overview zoom');
for(const [x,y] of [[-300,0],[300,0],[0,-300],[0,300],[-200,-300],[200,-300],[-364,-400]]){
  app.run('globalThis.packageLink={source:{name:"source",x:'+x+',y:'+y+'},target:packageNode};');
  assert.equal(app.run('hitNode(packageNode,arrowGeometry(packageLink).x,arrowGeometry(packageLink).y)'),false,'arrow stays outside package for '+[x,y]);
}
const packages=launch({...fixture,nodes:[node(0,'domain',null,'module'),node(1,'domain/entities',0,'module'),node(2,'domain/entities/A.ts',1),node(3,'domain/B.ts',0),node(4,'unrelated.ts')],edges:[],findings:[]});
assert.equal(packages.run('renderedNodes.find(n=>n.name==="domain").fileCount'),2,'package counts include nested files only');
assert.equal(packages.run('renderedNodes.find(n=>n.name==="domain").packageCount'),1);
const readableFixture={...fixture,nodes:[node(0,'A',null,'module'),node(1,'A/a.ts',0),node(2,'B',null,'module'),node(3,'B/b.ts',2),node(4,'loose.ts')],edges:[edge(1,3,25),edge(3,1,2),edge(1,4,3),edge(4,3,4)],findings:[{id:'cycle-all',kind:'cycle',nodes:['A','B','loose.ts'],edge_count:4,suggested_cuts:[]}]};
const readable=launch(readableFixture);
const visibleDependencies=app=>app.graph.data.links.filter(app.graph.linkVisibility);
const edgeMode=(app,mode)=>app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===mode).click();
assert.equal(readable.run('view.nodes.length'),3);assert.equal(readable.graph.data.nodes.length,2,'a mixed cut starts with packages, not every direct file');
assert.equal(visibleDependencies(readable).length,2,'the default tree includes arrows between visible packages before selection');
assert.match(readable.ids.get('graph-hint').textContent,/2 related dependenc.*hidden.*file/);
assert.match(readable.ids.get('toggle-files').textContent,/1 hidden/);
assert.match(readable.ids.get('graph-visibility').textContent,/2.*3.*analysis scope/);
const analysisBefore=readable.run('JSON.stringify([view,sccs,currentAcd,work.edges])'),readableUpdates=readable.graph.updates,readableZoom=readable.graph.zoom;
readable.run("selectNode('A')");assert.equal(visibleDependencies(readable).length,3,'selection reveals its direct file endpoint');
assert.doesNotMatch(readable.ids.get('graph-hint').textContent,/related dependenc.*hidden.*file/);
edgeMode(readable,'incoming');assert.equal(visibleDependencies(readable).length,1);assert.equal(visibleDependencies(readable)[0].source.name,'B');
edgeMode(readable,'outgoing');assert.deepEqual(Array.from(visibleDependencies(readable),l=>l.target.name).sort(),['B','loose.ts']);
assert.equal(readable.graph.updates,readableUpdates+1,'revealing an endpoint updates graphData once; direction filters reuse it');assert.equal(readable.graph.zoom,readableZoom);
const suppressed=readable.graph.data.links.find(l=>l.source.name==='B'),suppressedPaint=canvasRecorder();readable.graph.linkCanvasObject(suppressed,suppressedPaint.context,1);
assert.equal(suppressedPaint.calls.length,0,'custom arrows obey the same filter as dependency lines');
edgeMode(readable,'all');readable.graph.onBackgroundClick();assert.equal(visibleDependencies(readable).length,4,'revealed endpoints stay visible until explicitly hidden');
readable.click('toggle-files');assert.equal(readable.graph.data.nodes.length,3);assert.equal(visibleDependencies(readable).length,4);
assert.equal(readable.run('JSON.stringify([view,sccs,currentAcd,work.edges])'),analysisBefore,'display filters never change the analytical cut, SCCs, metrics or evidence');
readable.click('toggle-files');assert.equal(readable.graph.data.nodes.length,2);
readable.graph.zoom=.72;readable.run("revealNode('loose.ts')");assert.ok(readable.graph.data.nodes.some(n=>n.name==='loose.ts'),'Explorer selection reveals a hidden file');
assert.equal(readable.graph.zoom,.72,'revealing a hidden file preserves zoom');
const revealedUpdates=readable.graph.updates;readable.run("selectNode('A')");assert.equal(readable.graph.updates,revealedUpdates,'revealed files remain stable when selecting an already visible package');
readable.click('reset-view');assert.equal(readable.graph.data.nodes.length,2,'Reset view clears files revealed through previous selections');
const edgeReveal=launch(readableFixture);edgeReveal.graph.zoom=.63;edgeReveal.run("selectEdge(view.edges.find(e=>nodeById(view,e.to).name==='loose.ts'))");
assert.ok(edgeReveal.graph.data.nodes.some(n=>n.name==='loose.ts'),'Matrix/Inspector edge selection reveals both endpoints');assert.equal(edgeReveal.graph.zoom,.63);
assert.ok(visibleDependencies(edgeReveal).some(l=>l.target.name==='loose.ts'));
const deferredReveal=launch(readableFixture,{deferGraphData:true});deferredReveal.graph.flushLayout();deferredReveal.graph.zoom=1;
deferredReveal.run("selectNode('loose.ts')");deferredReveal.graph.zoom=3;deferredReveal.graph.flushLayout();
assert.equal(deferredReveal.graph.zoom,1,'hidden-file reveal restores exact zoom1 after the library deferred automatic zoom');
const readableCycle=launch(readableFixture);readableCycle.run('focusCycle(cyclesForView()[0])');
assert.equal(readableCycle.graph.data.nodes.length,3,'cycle focus overrides the file filter for every member');assert.equal(visibleDependencies(readableCycle).length,4);
const outsideCycleFixture={...readableFixture,nodes:[...readableFixture.nodes,node(5,'outside.ts')],edges:[...readableFixture.edges,edge(3,5)]};
const outsideCycleEdge=launch(outsideCycleFixture);outsideCycleEdge.run("focusCycle(cyclesForView()[0]);selectEdge(view.edges.find(e=>nodeById(view,e.to).name==='outside.ts'))");
assert.equal(outsideCycleEdge.run('ui.cycle'),null,'selecting a Matrix edge outside the focused cycle exits that cycle scope');
assert.equal(outsideCycleEdge.run('isolated'),null);assert.equal(outsideCycleEdge.run('ui.selection.kind'),'edge');
assert.ok(outsideCycleEdge.graph.data.nodes.some(n=>n.name==='outside.ts'));assert.ok(visibleDependencies(outsideCycleEdge).some(l=>l.target.name==='outside.ts'));
assert.equal(outsideCycleEdge.ids.get('cycles-panel').querySelectorAll('button').some(b=>b.getAttribute('aria-pressed')==='true'),false,'the old cycle indicator is cleared');
const outsideCycleNode=launch(outsideCycleFixture);outsideCycleNode.run("focusCycle(cyclesForView()[0]);selectNode('outside.ts')");
assert.equal(outsideCycleNode.run('ui.cycle'),null);assert.equal(outsideCycleNode.run('ui.selection.name'),'outside.ts');
assert.ok(outsideCycleNode.graph.data.nodes.some(n=>n.name==='outside.ts'),'Inspector neighbor selection also reveals nodes outside the old cycle');
const filesOnly=launch(fixture);assert.equal(filesOnly.graph.data.nodes.length,5);assert.equal(visibleDependencies(filesOnly).length,5,'the default tree also shows import arrows in file-only scopes');assert.equal(filesOnly.ids.get('toggle-files').disabled,true,'file-only cuts never become empty through the package filter');
const independentFiles=launch({...fixture,edges:[],findings:[]});assert.equal(independentFiles.ids.get('graph-state').hidden,true,'empty dependency hints never cover otherwise visible cards');assert.match(independentFiles.ids.get('graph-hint').textContent,/No dependencies in this analysis scope/);
const renumberedPackages=launch({...fixture,nodes:[node(0,'A.ts'),node(1,'B',null,'module'),node(2,'C',null,'module')],edges:[],findings:[]});
renumberedPackages.click('simulation-mode');renumberedPackages.run("recordOp(encodeMove('A.ts','B'));selectNode('B')");
assert.equal(renumberedPackages.run('renderedNodes.find(n=>n.name==="B").id'),renumberedPackages.run('selectedId'),'hidden-file moves may renumber unchanged package names; cached dense IDs must follow the analytical cut');
const hiddenCycle=launch({...readableFixture,edges:[edge(1,4),edge(4,3),edge(3,1)],findings:[]});
assert.equal(hiddenCycle.run('renderedNodes.every(n=>n.inCycle)'),true);
const hiddenCycleUpdates=hiddenCycle.graph.updates;hiddenCycle.click('simulation-mode');hiddenCycle.run("recordOp(encodeCut('A','loose.ts'))");
assert.equal(hiddenCycle.run('renderedNodes.some(n=>n.inCycle)'),false,'a cut through a hidden file clears cycle badges even when shown endpoints and links stay unchanged');
assert.equal(hiddenCycle.graph.updates,hiddenCycleUpdates,'cycle metadata refresh does not reset an unchanged visible graph');
const fixedTree=launch({...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/domain',0,'module'),node(2,'app/domain/model.ts',1),node(3,'app/api.ts',0),node(4,'other')],edges:[edge(2,4),edge(4,2)],findings:[]});
fixedTree.run("expandNode('app');expandNode('app/domain')");
assert.deepEqual(JSON.parse(fixedTree.run('JSON.stringify(graphLayout.nodes.filter(n=>n.context&&!n.project).flatMap(n=>n.sourceIds).sort((a,b)=>a-b))')),[0,1],'compact context headers retain every real ancestor source ID');
assert.equal(fixedTree.run('view.nodes.length'),3,'context ancestors do not enter the analytical cut');
assert.equal(fixedTree.graph.data.nodes.length,3);assert.equal(fixedTree.graph.data.links.length,2);
assert.equal(fixedTree.run('sccs.length'),1,'dependency cycles remain valid on a containment tree');
assert.ok(fixedTree.graph.data.nodes.every(n=>n.x===n.fx&&n.y===n.fy));
for(const force of ['charge','link','center'])assert.equal(fixedTree.graph.forces[force],null,'disable the '+force+' force');
const fixedPositions=fixedTree.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y]))');
fixedTree.run("selectNode('app/domain/model.ts');applyTheme('dark');document.getElementById('filter').value='model';applySearch()");
assert.equal(fixedTree.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y]))'),fixedPositions,'selection, theme and search never rearrange the tree');
fixedTree.run("selectNode('app/domain/model.ts',true)");
assert.equal(fixedTree.graph.centers.at(-1)[2],0,'Explorer and neighbor focus center instantly without a camera animation');
const contextPaint=canvasRecorder();fixedTree.graph.onRenderFramePre(contextPaint.context,1);
assert.ok(contextPaint.calls.some(c=>c[0]==='fillText'&&String(c[1]).includes('app')),'expanded ancestors are actually painted');
assert.ok(contextPaint.calls.some(c=>c[0]==='lineTo'),'orthogonal containment paths are painted separately');
fixedTree.run('fitView()');
const [centerX,centerY]=fixedTree.graph.centers.at(-1),fitBounds=fixedTree.run('graphLayout.bounds');
assert.equal(centerX,(fitBounds.minX+fitBounds.maxX)/2);assert.equal(centerY,(fitBounds.minY+fitBounds.maxY)/2);
assert.ok((fitBounds.maxX-fitBounds.minX)*fixedTree.graph.zoom<=680+.001);
assert.ok((fitBounds.maxY-fitBounds.minY)*fixedTree.graph.zoom<=480+.001,'fitting includes project root and ancestor cards');
// The legacy grouping heuristic remains available in a closed advanced panel.
// It is not presented as an automatic cycle repair in the Explorer.
const suggestionFixture={...fixture,nodes:[node(0,'A',null,'module'),node(1,'A/a.ts',0),node(2,'B',null,'module'),node(3,'B/b.ts',2),node(4,'C',null,'module'),node(5,'C/c.ts',4),node(6,'D',null,'module'),node(7,'D/d.ts',6)],edges:[edge(1,3,25),edge(3,1,2),edge(1,5,3)],findings:[]};
const suggestions=launch(suggestionFixture);suggestions.run("selectNode('A')");
assert.match(suggestions.ids.get('sidebar').textContent,/Suggested package move/);
assert.equal(suggestions.run('moveSuggestions.get(0).target'),'B');
assert.equal(suggestions.run('moveSuggestions.get(0).total'),27,'full totals rank candidates, not capped evidence samples');
assert.equal(suggestions.run('moveSuggestions.get(0).outgoing'),25);
assert.equal(suggestions.run('moveSuggestions.get(0).incoming'),2);
assert.equal(suggestions.run('moveSuggestions.has(6)'),false,'an unrelated package has no invented recommendation');
assert.doesNotMatch(suggestions.ids.get('module-list').textContent,/Suggested move/);
assert.equal(suggestions.ids.get('sidebar').querySelectorAll('details').find(node=>node.className==='expert-simulation').open,false);
assert.match(suggestions.ids.get('sidebar').textContent,/25 outgoing.*2 incoming/);
assert.match(suggestions.ids.get('sidebar').textContent,/sibling packages/);
assert.match(suggestions.ids.get('sidebar').textContent,/Imports and file-level cycles stay unchanged/);
assert.equal(suggestions.run('ui.simulation'),false);
const suggestedAction=()=>suggestions.ids.get('sidebar').querySelectorAll('button').find(b=>b.textContent==='Simulate suggested move');
const originalImports=suggestions.run('JSON.stringify(work.edges)'),originalFileCycles=suggestions.run('findSccs(work).length');
suggestedAction().click();
assert.equal(suggestions.run('ui.simulation'),true);
assert.equal(suggestions.ids.get('action-dialog').open,true);
assert.deepEqual(JSON.parse(suggestions.run('JSON.stringify(ui.pending)')),{move_node:{node:'A',new_parent:'B'}});
assert.equal(suggestions.run('ops.length'),0,'a suggestion opens confirmation before applying an operation');
assert.equal(suggestions.ids.get('sidebar').querySelector('select').value,'B','the manual move control starts with the recommendation');
suggestions.click('cancel-action');
assert.equal(suggestions.run('ops.length'),0);assert.equal(suggestions.run('JSON.stringify(work.edges)'),originalImports);
suggestedAction().click();suggestions.click('confirm-action');
assert.deepEqual(JSON.parse(suggestions.run('JSON.stringify(ops)')),[{move_node:{node:'A',new_parent:'B'}}]);
assert.equal(suggestions.run('nodeById(work,0).name'),'B/A');
assert.equal(suggestions.run('JSON.stringify(work.edges)'),originalImports,'moving a package does not change any import evidence');
assert.equal(suggestions.run('findSccs(work).length'),originalFileCycles,'grouping cannot resolve a file dependency cycle');
suggestions.run("selectNode('C')"); // request candidates after the move cleared the old selection
assert.equal(suggestions.run('moveSuggestions.has(0)'),false,'the moved package no longer has its old siblings');
assert.equal(suggestions.run('moveSuggestions.get(4).target'),'B','recommendations follow changed package ownership');
suggestions.click('undo-ops');assert.equal(suggestions.run('moveSuggestions.get(0).target'),'B');
suggestions.run("recordOp(encodeCut('A','B'));recordOp(encodeCut('B','A'));recordOp(encodeCut('A','C'))");
assert.equal(suggestions.run('moveSuggestions.has(0)'),false,'cut evidence must invalidate a previous recommendation');
suggestions.run("selectNode('A')");assert.match(suggestions.ids.get('sidebar').textContent,/No supported move among sibling packages/);
assert.equal(suggestions.ids.get('sidebar').querySelectorAll('button').some(b=>b.textContent==='Simulate suggested move'),false);
suggestions.click('clear-ops');assert.equal(suggestions.run('moveSuggestions.get(0).target'),'B');
const tiedSuggestions=launch({...suggestionFixture,edges:[edge(1,5,5),edge(1,3,5)]});
tiedSuggestions.run("selectNode('A')");
assert.equal(tiedSuggestions.run('moveSuggestions.get(0).target'),'B','equal evidence is resolved by stable path order');
const unknownSuggestions=launch({...suggestionFixture,edges:[edge(1,3,null)]});unknownSuggestions.run("selectNode('A')");
assert.equal(unknownSuggestions.run('moveSuggestions.get(0).lowerBound'),true);
assert.match(unknownSuggestions.ids.get('sidebar').textContent,/at least 1/);
assert.match(unknownSuggestions.ids.get('sidebar').textContent,/lower bounds/);
assert.doesNotMatch(unknownSuggestions.ids.get('module-list').textContent,/Suggested move/);
const nestedSuggestions=launch({...fixture,nodes:[node(0,'root',null,'module'),node(1,'root/A',0,'module'),node(2,'root/A/a.ts',1),node(3,'root/B',0,'module'),node(4,'root/B/b.ts',3),node(5,'root/A/nested',1,'module'),node(6,'root/A/nested/n.ts',5),node(7,'outside',null,'module'),node(8,'outside/o.ts',7)],edges:[edge(2,4,3),edge(6,2,2000),edge(2,8,1000)],findings:[]});
nestedSuggestions.run("revealNode('root/A')");
assert.equal(nestedSuggestions.run('moveSuggestions.get(1).target'),'root/B','only siblings compete; parent, descendants and unrelated scopes do not inflate affinity');
assert.equal(nestedSuggestions.run('moveSuggestions.get(1).total'),3,'imports within the source subtree do not support a move');
nestedSuggestions.run("revealNode('root/A')");nestedSuggestions.click('simulation-mode');
const targets=nestedSuggestions.ids.get('sidebar').querySelector('select').children.map(option=>option.value);
assert.ok(!targets.includes('root/A')&&!targets.includes('root/A/nested')&&!targets.includes('root'),'manual targets exclude self, descendants and the unchanged parent');
assert.equal(nestedSuggestions.ids.get('sidebar').querySelector('select').value,'root/B');
const conflictingSuggestion=launch({...suggestionFixture,nodes:[...suggestionFixture.nodes,node(8,'B/A',2,'module')]});
conflictingSuggestion.run("selectNode('A')");
assert.equal(conflictingSuggestion.run('moveSuggestions.get(0).target'),'C','a stronger but invalid name-collision move is excluded');
const uncertainRanking=launch({...suggestionFixture,edges:[edge(1,3,25),edge(1,5,null)]});uncertainRanking.run("selectNode('A')");
assert.equal(uncertainRanking.run('moveSuggestions.get(0).lowerBound'),false);
assert.equal(uncertainRanking.run('moveSuggestions.get(0).rankingLowerBound'),true);
assert.match(uncertainRanking.ids.get('sidebar').textContent,/candidates are ranked using sample lower bounds/);
// IDE-style hierarchy follows source IDs/parents and synchronizes disclosure
// with the graph. Path text is only a label, never the structural source.
const treeFixture={...fixture,nodes:[node(0,'src',null,'module'),node(1,'src/zeta.ts',0),node(2,'src/domain',0,'module'),node(3,'src/domain/model.ts',2),node(4,'README.md'),node(5,'lib',null,'module'),node(6,'lib/index.ts',5),node(7,'unrelated/path.ts',2)],edges:[],findings:[]};
const tree=launch(treeFixture);
function treeNodes(app){return app.ids.get('module-list').querySelectorAll('button[data-node]');}
function treeNode(app,id){return treeNodes(app).find(b=>b.dataset.node===String(id));}
function treeToggle(app,id){return app.ids.get('module-list').querySelectorAll('button[data-toggle]').find(b=>b.dataset.toggle===String(id));}
function treeIds(app){return treeNodes(app).map(b=>Number(b.dataset.node));}
assert.equal(tree.ids.get('module-list').tagName,'UL','the hierarchy uses nested list semantics');
assert.deepEqual(treeIds(tree),[5,0,4],'package roots precede file roots, with deterministic sorting');
const treeUpdates=tree.graph.updates,oldToggle=treeToggle(tree,0);oldToggle.click();
assert.deepEqual(treeIds(tree),[5,0,2,1,4]);
assert.equal(treeToggle(tree,0).getAttribute('aria-expanded'),'true');
assert.notEqual(treeToggle(tree,0),oldToggle);assert.equal(tree.document.activeElement,treeToggle(tree,0),'disclosure restores focus to its replacement button');
assert.equal(tree.graph.updates,treeUpdates+1,'expanding Explorer updates the graph scope and layout');
assert.deepEqual(Array.from(tree.graph.data.nodes,n=>n.name).sort(),['lib','src/domain','src/zeta.ts']);
assert.equal(tree.run('ops.length'),0);assert.equal(tree.run('ui.selection.name'),'src');
assert.equal(treeNode(tree,2).querySelectorAll('span').find(s=>s.className==='explorer-name').textContent,'domain');
assert.equal(treeNode(tree,2).title,'src/domain');
treeToggle(tree,2).click();assert.deepEqual(treeIds(tree),[5,0,2,3,7,1,4]);
assert.equal(treeNode(tree,7).parentElement.parentElement.parentElement.getAttribute('id'),'explorer-children-2','parent ID determines placement even when path text differs');
assert.equal(treeNode(tree,3).getAttribute('aria-expanded'),null,'file selection is not an expandable control');
assert.equal(treeToggle(tree,3),undefined);
treeNode(tree,2).click();assert.equal(tree.run('ui.selection.name'),'src/domain');
treeToggle(tree,0).click();assert.equal(treeNode(tree,2),undefined,'a selected branch can still be collapsed manually');
tree.run("revealNode('src/domain/model.ts')");
assert.equal(treeToggle(tree,0).getAttribute('aria-expanded'),'true');assert.equal(treeToggle(tree,2).getAttribute('aria-expanded'),'true');
assert.equal(treeNode(tree,3).getAttribute('aria-pressed'),'true','graph selection opens its ancestors in Explorer');
treeToggle(tree,0).click();treeToggle(tree,5).click();
const manualOpen=tree.run('JSON.stringify([...explorerOpen].sort())');
tree.ids.get('filter').value='model.ts';tree.run('applySearch()');
assert.deepEqual(treeIds(tree),[0,2,3],'search includes ancestors of matching descendants only');
assert.equal(treeToggle(tree,0).disabled,true,'search expansion is temporary and explained by a disabled disclosure');
assert.equal(tree.run('JSON.stringify([...explorerOpen].sort())'),manualOpen);
assert.match(tree.ids.get('search-count').textContent,/1 result/);
tree.click('clear-search');assert.equal(tree.run('JSON.stringify([...explorerOpen].sort())'),manualOpen);
assert.deepEqual(treeIds(tree),[5,6,0,4],'search reset restores the manually opened branches');
treeToggle(tree,0).click();tree.click('simulation-mode');tree.run("recordOp(encodeMove('src/domain','lib'))");
assert.equal(treeNode(tree,2).parentElement.parentElement.parentElement.getAttribute('id'),'explorer-children-5');
assert.equal(treeToggle(tree,2).getAttribute('aria-expanded'),'true','open state follows stable IDs across a move');
assert.equal(treeNode(tree,3).title,'lib/domain/model.ts');
tree.click('undo-ops');assert.equal(treeNode(tree,2).parentElement.parentElement.parentElement.getAttribute('id'),'explorer-children-0');
assert.equal(treeNode(tree,3).title,'src/domain/model.ts');
const keyboardTree=launch(treeFixture);treeToggle(keyboardTree,0).click();
treeNode(keyboardTree,0).dispatch('keydown',{key:'ArrowDown'});assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,2));
treeNode(keyboardTree,2).dispatch('keydown',{key:'ArrowRight'});assert.equal(treeToggle(keyboardTree,2).getAttribute('aria-expanded'),'true');assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,2));
treeNode(keyboardTree,2).dispatch('keydown',{key:'ArrowRight'});assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,3));
treeNode(keyboardTree,3).dispatch('keydown',{key:'ArrowLeft'});assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,2));
treeNode(keyboardTree,2).dispatch('keydown',{key:'ArrowLeft'});assert.equal(treeToggle(keyboardTree,2).getAttribute('aria-expanded'),'false');assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,2));
treeNode(keyboardTree,2).dispatch('keydown',{key:'ArrowLeft'});assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,0));
treeNode(keyboardTree,0).dispatch('keydown',{key:'Home'});assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,5));
treeNode(keyboardTree,5).dispatch('keydown',{key:'End'});assert.equal(keyboardTree.document.activeElement,treeNode(keyboardTree,4));
assert.equal(keyboardTree.run('ui.selection.name'),'README.md','arrow navigation selects the focused tree entry in the graph');
assert.equal(keyboardTree.run('ops.length'),0,'tree navigation never creates simulation operations');
const manyTreeNodes=Array.from({length:500},(_,i)=>[node(2*i,'p'+String(i).padStart(3,'0'),null,'module'),node(2*i+1,'p'+String(i).padStart(3,'0')+'/index.ts',2*i)]).flat();
const boundedTree=launch({...fixture,nodes:manyTreeNodes,edges:[],findings:[]});
assert.equal(treeNodes(boundedTree).length,100);
treeToggle(boundedTree,0).click();assert.equal(treeNodes(boundedTree).length,100,'the row budget includes expanded descendants');
boundedTree.run("selectNode('p400')");assert.equal(treeNode(boundedTree,800).getAttribute('aria-pressed'),'true','selection beyond the first page remains visible within the row budget');
assert.equal(treeNodes(boundedTree).length,100);
boundedTree.ids.get('module-list').querySelectorAll('button').find(b=>b.textContent.startsWith('Show next 100')).click();
assert.equal(treeNodes(boundedTree).length,200);
// Real querySelectorAll returns an iterable NodeList, not an Array with find.
function useNativeCollections(app){
  const list=app.ids.get('module-list'),query=list.querySelectorAll.bind(list);
  list.querySelectorAll=selector=>{const nodes=query(selector);return {length:nodes.length,item:i=>nodes[i]||null,forEach:callback=>nodes.forEach(callback),[Symbol.iterator]:()=>nodes[Symbol.iterator]()};};
  return query;
}
const nativeTree=launch(treeFixture),nativeToggle=treeToggle(nativeTree,0),nativeQuery=useNativeCollections(nativeTree);
nativeToggle.click();assert.equal(nativeTree.document.activeElement,nativeQuery('button[data-toggle]').find(b=>b.dataset.toggle==='0'));
const nativeMore=boundedTree.ids.get('module-list').querySelectorAll('button').find(b=>b.textContent.startsWith('Show next 100'));
const boundedQuery=useNativeCollections(boundedTree);nativeMore.click();
assert.equal(boundedQuery('button[data-node]').length,300);
assert.ok(boundedQuery('button[data-node]').includes(boundedTree.document.activeElement),'pagination focuses a newly rendered row with native collection behavior');
// A move inside collapsed packages changes their contents without changing the
// visible nodes/edges. Refresh metadata without disturbing their layout.
const movedContents=launch({...fixture,nodes:[node(0,'a',null,'module'),node(1,'a/x.ts',0),node(2,'b',null,'module')],edges:[],findings:[]});
const contentUpdates=movedContents.graph.updates;movedContents.click('simulation-mode');movedContents.run("recordOp(encodeMove('a/x.ts','b'))");
assert.equal(movedContents.run('renderedNodes.find(n=>n.name==="a").fileCount'),0);
assert.equal(movedContents.run('renderedNodes.find(n=>n.name==="b").fileCount'),1);
assert.equal(movedContents.graph.updates,contentUpdates,'metadata refresh preserves the graph layout');
movedContents.click('undo-ops');assert.equal(movedContents.run('renderedNodes.find(n=>n.name==="a").fileCount'),1);assert.equal(movedContents.run('renderedNodes.find(n=>n.name==="b").fileCount'),0);
assert.equal(movedContents.graph.updates,contentUpdates);
const stableTree=launch({...fixture,nodes:[node(0,'A'),node(1,'B')],edges:[],findings:[]});
assert.ok(stableTree.run('Math.max(Math.abs(renderedNodes[0].x-renderedNodes[1].x)-(nodeBox(renderedNodes[0]).w+nodeBox(renderedNodes[1]).w)/2,Math.abs(renderedNodes[0].y-renderedNodes[1].y)-(nodeBox(renderedNodes[0]).h+nodeBox(renderedNodes[1]).h)/2)>=24'),'compact rows or columns leave at least 24px between actual card bounds');
assert.equal(stableTree.graph.forces['rect-collision'],undefined,'the containment tree needs no collision simulation');
// Deterministic large graph: hidden matrix stays unbuilt; lists and active matrix are bounded.
const largeNodes=Array.from({length:500},(_,i)=>node(i,'module-'+String(i).padStart(3,'0'))),largeEdges=[];
for(let i=0;i<500;i++)for(let j=1;j<=4;j++)largeEdges.push(edge(i,(i+j)%500));
const large=launch({...fixture,nodes:largeNodes,edges:largeEdges,findings:[]});assert.equal(large.ids.get('dsm-tab').children.length,0);
assert.ok(large.ids.get('module-list').children.length<=101);
const initial=large.graph.updates,start=performance.now();large.run("selectNode('module-100');document.getElementById('filter').value='module-10';applySearch()");const interactionMs=performance.now()-start;
assert.equal(large.graph.updates,initial);large.run("showTab('dsm')");assert.ok(large.ids.get('dsm-tab').querySelectorAll('button').length<=1600);
const firstTable=large.ids.get('dsm-tab').children[0];large.run("selectNode('module-101')");assert.equal(large.ids.get('dsm-tab').children[0],firstTable,'matrix selection updates in place');large.click('matrix-next');assert.match(large.ids.get('matrix-page').textContent,/columns 41–80/);
const stableSnapshot=large.run('JSON.stringify(renderedNodes.map(n=>[n.x,n.y,n.fx,n.fy]))');
for(let tick=0;tick<120;tick++)large.graph.flushLayout();
assert.equal(large.run('JSON.stringify(renderedNodes.map(n=>[n.x,n.y,n.fx,n.fy]))'),stableSnapshot,'120 library ticks leave all fixed positions unchanged');
// Without cycles fewer packages share the screen, so basename collisions are
// rare and package cards used to collapse to bare fragments like "core".
// A package card must always paint the full package path; only files compact.
const acyclicPackages=launch({...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/api',0,'module'),node(2,'app/api/todos.py',1),node(3,'app/core',0,'module'),node(4,'app/core/settings.py',3),node(5,'libs',null,'module'),node(6,'libs/api',5,'module'),node(7,'libs/api/client.py',6)],edges:[],findings:[]});
acyclicPackages.run("expandNode('app');expandNode('libs')");
const coreCard=acyclicPackages.graph.data.nodes.find(n=>n.name==='app/core');
assert.ok(coreCard,'app/core is visible after opening app');
const corePaint=canvasRecorder();acyclicPackages.graph.nodeCanvasObject(coreCard,corePaint.context,1);
assert.ok(corePaint.calls.some(c=>c[0]==='fillText'&&c[1]==='app/core'),'a package card names the full package even without basename collisions');
assert.ok(corePaint.calls.some(c=>c[0]==='fillText'&&c[1]==='PACKAGE'),'the package tab remains');
acyclicPackages.run("expandNode('app/core')");
const settingsCard=acyclicPackages.graph.data.nodes.find(n=>n.name==='app/core/settings.py');
const settingsPaint=canvasRecorder();acyclicPackages.graph.nodeCanvasObject(settingsCard,settingsPaint.context,1);
assert.ok(settingsPaint.calls.some(c=>c[0]==='fillText'&&c[1]==='settings.py'),'files keep their compact basename');
// Deep zoom over a package continues into the package: crossing the drill-in
// zoom threshold opens it like a double-click and fits the revealed contents.
const drillFixture={...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/domain',0,'module'),node(2,'app/domain/Entity.ts',1),node(3,'app/Service.ts',0),node(4,'other',null,'module'),node(5,'other/Hidden.ts',4)],edges:[edge(3,2)],findings:[]};
const drill=launch(drillFixture);
const drillCanvas=drill.ids.get('graph').querySelector('canvas');
const drillNames=()=>Array.from(drill.graph.data.nodes,n=>n.name).sort();
const drillUpdates=drill.graph.updates;
drill.graph.zoom=9;
drillCanvas.dispatch('pointermove',{offsetX:400,offsetY:300});
drill.graph.screenTransform={x:400,y:300,scale:9};
const drillAppCard=drill.graph.data.nodes.find(n=>n.name==='app');
drillCanvas.dispatch('pointermove',{offsetX:drillAppCard.x*9+400,offsetY:drillAppCard.y*9+300});
drill.graph.onZoom({transform:{k:9}});
assert.notEqual(drill.graph.updates,drillUpdates,'crossing the zoom threshold over a package opens it');
assert.ok(drillNames().includes('app/domain'),'the opened package reveals its contents');
assert.equal(drill.run('trail.join()'),'0');
// Repeated zoom frames past the threshold must not open the next level unasked.
const drillRevealed=drill.graph.updates;
drill.graph.onZoom({transform:{k:12}});
assert.equal(drill.graph.updates,drillRevealed,'staying above the threshold does not drill again');
// Zooming back out below the threshold arms the drill-in for the next package.
drill.graph.zoom=3;drill.graph.onZoom({transform:{k:3}});
const drillDomainCard=drill.graph.data.nodes.find(n=>n.name==='app/domain');
drill.graph.zoom=9;
if(drillDomainCard){
  drill.graph.screenTransform={x:400,y:300,scale:9};
  drillCanvas.dispatch('pointermove',{offsetX:drillDomainCard.x*9+400,offsetY:drillDomainCard.y*9+300});
  drill.graph.onZoom({transform:{k:9}});
  assert.ok(drillNames().includes('app/domain/Entity.ts'),'after zooming out, a new zoom-in opens the package under the pointer');
}
// No package under the pointer: zooming past the threshold changes nothing.
const drillEmpty=launch(drillFixture);
const emptyCanvas=drillEmpty.ids.get('graph').querySelector('canvas');
const emptyUpdates=drillEmpty.graph.updates;
drillEmpty.graph.zoom=9;
emptyCanvas.dispatch('pointermove',{offsetX:5,y:5});
drillEmpty.graph.screenTransform={x:0,y:0,scale:9};
emptyCanvas.dispatch('pointermove',{offsetX:10000,offsetY:10000});
drillEmpty.graph.onZoom({transform:{k:9}});
assert.equal(drillEmpty.graph.updates,emptyUpdates,'zoom without a package under the pointer never navigates');
// The workspace can be toggled into and out of the browser fullscreen.
const fullscreen=launch(fixture);
const fullscreenButton=fullscreen.ids.get('toggle-fullscreen');
assert.ok(fullscreenButton,'the graph controls offer a fullscreen toggle');
let fullscreenEntered=0,fullscreenExited=0;
const fullscreenRoot=fullscreen.document.documentElement;
fullscreenRoot.requestFullscreen=()=>{fullscreenEntered++;fullscreen.document.fullscreenElement=fullscreenRoot;return Promise.resolve();};
fullscreen.document.exitFullscreen=()=>{fullscreenExited++;fullscreen.document.fullscreenElement=null;return Promise.resolve();};
fullscreenButton.click();assert.equal(fullscreenEntered,1);assert.equal(fullscreenButton.getAttribute('aria-pressed'),'true');
fullscreenButton.click();assert.equal(fullscreenExited,1);assert.equal(fullscreenButton.getAttribute('aria-pressed'),'false');
const noFullscreen=launch(fixture);
const noFsRoot=noFullscreen.document.documentElement;
noFsRoot.requestFullscreen=undefined;noFullscreen.document.exitFullscreen=undefined;
noFullscreen.ids.get('toggle-fullscreen').click();
assert.match(noFullscreen.ids.get('feedback').textContent,/fullscreen|vollbild/i,'a missing fullscreen API is reported, not thrown');
const exported=launch({...fixture,nodes:[node(0,'A.ts'),node(1,'B.ts')],edges:[edge(0,1)],findings:[]});exported.click('simulation-mode');exported.run("recordOp(encodeCut('A.ts','B.ts'))");exported.click('export-ops');
exported.graph.downloads[0].text().then(text=>{assert.deepEqual(JSON.parse(text),[{cut_edge:{from:'A.ts',to:'B.ts'}}]);if(process.env.VIEWER_EXPORT_PATH)fs.writeFileSync(process.env.VIEWER_EXPORT_PATH,text);}).catch(error=>{console.error(error);process.exitCode=1;});
console.log(JSON.stringify({result:'viewer state tests passed',runtime:process.version,platform:process.platform,cpu:os.cpus()[0].model,smallLoadMs:app.loadMs,large500nodes2000edgesLoadMs:large.loadMs,searchSelectionMs:interactionMs,scope:'Node + DOM/ForceGraph test doubles; not browser paint timings'},null,2));
