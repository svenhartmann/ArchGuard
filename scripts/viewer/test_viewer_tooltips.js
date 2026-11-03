// Execute the viewer's pointer handlers with the shared deterministic DOM and
// ForceGraph doubles. Native canvas hit-map coverage is a separate check.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const rootName='org/example/<img src=x onerror=alert(1)>';
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const fixture={root:'/tmp/tooltip-example',nodes:[
  node(0,rootName,null,'module'),node(1,rootName+'/A',0,'module'),
  node(2,rootName+'/A/Start.ts',1),node(3,rootName+'/B',0,'module'),node(4,rootName+'/B/End.ts',3),
],edges:[{from:2,to:4,count:3,refs:[{file:'<img src=x onerror=alert(2)>.ts',line:7,raw:'import "<script>";'}]}],findings:[],cyclicEdges:[],view:{depth:1,visible:null}};
const app=launch(fixture);
app.click('diagram-network');
const tip=app.document.getElementById('graph-context-tooltip');
const canvas=app.ids.get('graph').querySelector('canvas');
function contextHeader(){return JSON.parse(app.run('JSON.stringify(graphLayout.nodes.find(n=>n.context&&!n.project))'));}
function hoverHeader(transform={x:0,y:0,scale:1}){
  const header=contextHeader();assert.ok(header,'fixture has a drawn package context header');
  app.graph.screenTransform=transform;
  canvas.dispatch('pointermove',{offsetX:header.x*transform.scale+transform.x,offsetY:header.y*transform.scale+transform.y,buttons:0});
  assert.equal(tip.hidden,false,'hovering a drawn context header displays its tooltip');
  assert.ok(tip.textContent.includes(header.name),'tooltip includes the full context package name');
  assert.equal(tip.querySelectorAll('img').length,0,'source package names stay literal text');
  return header;
}
const tooltipMarkup=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8').match(/<div\b[^>]*\bid="graph-context-tooltip"[^>]*>/);
assert.ok(tooltipMarkup,'the context tooltip is part of the standalone report');
assert.match(tooltipMarkup[0],/\brole="tooltip"/);
assert.match(tooltipMarkup[0],/\bhidden\b/,'the tooltip starts hidden before application initialization');
hoverHeader();
assert.ok(tip.textContent.includes(rootName));
canvas.dispatch('pointerleave');
assert.equal(tip.hidden,true,'leaving the canvas clears the context tooltip');
hoverHeader({x:320,y:140,scale:.4});
canvas.dispatch('pointermove',{offsetX:790,offsetY:590,buttons:0});
assert.equal(tip.hidden,true,'moving away from a context header clears the tooltip');
hoverHeader();canvas.dispatch('pointerdown');
assert.equal(tip.hidden,true,'starting a drag clears the tooltip');
const dragged=contextHeader();
canvas.dispatch('pointermove',{offsetX:dragged.x,offsetY:dragged.y,buttons:1});
assert.equal(tip.hidden,true,'dragging over a header does not reopen its tooltip');
hoverHeader();app.graph.onZoom();
assert.equal(tip.hidden,true,'zooming or panning clears the old tooltip position');
hoverHeader();for(const callback of app.document.events.keydown)callback({key:'Escape'});
assert.equal(tip.hidden,true,'Escape dismisses the tooltip');
hoverHeader();app.run('setWidth()');
assert.equal(tip.hidden,true,'resizing clears the old tooltip position');

// The panel uses measured bounds to stay inside the visible graph when a
// header is near its right/bottom edge, including after zoom and pan.
tip.getBoundingClientRect=()=>({width:220,height:80});
const header=contextHeader();
hoverHeader({x:790-header.x*.5,y:590-header.y*.5,scale:.5});
assert.ok(Number.parseFloat(tip.style.left)>=8);
assert.ok(Number.parseFloat(tip.style.left)+220<=792);
assert.ok(Number.parseFloat(tip.style.top)>=8);
assert.ok(Number.parseFloat(tip.style.top)+80<=592);

// A context overlay must not cover ordinary package/edge tooltips or survive
// changes to the graph whose former headers may no longer exist.
hoverHeader();
const card=app.graph.data.nodes[0],transform=app.graph.screenTransform;
canvas.dispatch('pointermove',{offsetX:card.x*transform.scale+transform.x,offsetY:card.y*transform.scale+transform.y,buttons:0});
assert.equal(tip.hidden,true,'ordinary graph cards retain the library tooltip');
for(const action of ['renderGraph()','resetView()',"showTab('dsm')"]){
  app.run("showTab('graph');setDiagram('network')");hoverHeader();app.run(action);
  assert.equal(tip.hidden,true,'graph change clears tooltip: '+action);
}

// Ordinary node and edge labels continue using text nodes rather than parsing
// repository-controlled names or import references as HTML.
app.run("showTab('graph')");
const nodeTip=app.graph.nodeLabel(app.graph.data.nodes[0]);
assert.ok(nodeTip.textContent.includes(rootName));
assert.equal(nodeTip.querySelectorAll('img').length,0);
const edgeTip=app.graph.linkLabel(app.graph.data.links[0]);
assert.match(edgeTip.textContent,/3 import references/);
assert.match(edgeTip.textContent,/<img src=x onerror=alert\(2\)>/);
assert.match(edgeTip.textContent,/<script>/);
assert.equal(edgeTip.querySelectorAll('img').length,0);
assert.equal(edgeTip.querySelectorAll('script').length,0);
console.log('viewer tooltips: context hover, zoom/pan, leave/navigation cleanup and safe labels passed');
