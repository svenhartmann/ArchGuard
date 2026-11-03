// Execute the actual embedded pure functions, without reimplementing them.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const js = fs.readFileSync('crates/archguard-report/src/viewer.js', 'utf8');
new vm.Script(js); // Full application syntax check.
function declaration(name) {
  const start = js.indexOf('function ' + name + '(');
  assert.ok(start >= 0, name);
  const firstLineEnd = js.indexOf('\n', start);
  const end = js.slice(start, firstLineEnd).endsWith('}') ? firstLineEnd : js.indexOf('\n}', start) + 2;
  return js.slice(start, end);
}
const context = vm.createContext({});
for (const name of ['esc', 'edgeWeight', 'weightLabel', 'widthFor', 'nodeById', 'ancestorsOf', 'nearestVisible', 'aggregate']) {
  vm.runInContext(declaration(name), context);
}
assert.equal(context.esc('&<>"\''), '&amp;&lt;&gt;&quot;&#39;');
for (const count of [1, 2, 5, 25, 100, 1000]) {
  assert.equal(context.widthFor(count), Math.min(8, 1.5 + Math.log2(count)));
}
const nodes = ['A', 'B', 'C', 'D', 'E'].map((name, id) => ({id, name, parent: null, kind: 'file'}));
const edge = (from, to, count) => ({from, to, count, refs: Array.from({length: Math.min(10, count)}, (_, line) => ({file: nodes[from].name, line, raw: 'import'}))});
const graph = {nodes, edges: [edge(0,1,25), edge(1,0,2), edge(1,2,5), edge(2,0,1), edge(3,4,100)]};
const view = context.aggregate(graph, new Set(nodes.map(n => n.id)));
assert.equal(view.edges.length, 5);
assert.equal(view.edges.find(e => e.from === 0 && e.to === 1).count, 25);
assert.equal(view.edges.find(e => e.from === 1 && e.to === 0).count, 2);
assert.ok(!view.edges.some(e => e.from === 4 && e.to === 3));
assert.equal(context.aggregate(view, new Set(nodes.map(n => n.id))).edges[0].count, 25);
assert.equal(context.weightLabel({refs: Array(10), count: null}), '10+');
// Large trees replay many cuts. Aggregating each edge must not rescan the
// entire source-node array for every parent hop; results still follow each
// fresh cut and the current (possibly simulated) containment graph.
const indexedGraph={nodes:[
  {id:10,name:'A',kind:'module',parent:null},
  {id:101,name:'A/one.ts',kind:'file',parent:10},
  {id:205,name:'A/two.ts',kind:'file',parent:10},
  {id:800,name:'B',kind:'module',parent:null},
  {id:999,name:'B/target.ts',kind:'file',parent:800},
],edges:[
  {from:101,to:999,count:2,refs:[{file:'one.ts',line:1,raw:'import target'}]},
  {from:205,to:999,count:3,refs:[{file:'two.ts',line:7,raw:'import target'}]},
  {from:101,to:101,count:8,refs:[]},
]};
let nodeScans=0;indexedGraph.nodes.find=function(...args){nodeScans++;return Array.prototype.find.apply(this,args);};
const indexedView=context.aggregate(indexedGraph,new Set([10,800]));
assert.equal(indexedView.edges.length,1);assert.equal(indexedView.edges[0].count,5);assert.equal(indexedView.edges[0].refs.length,2);
assert.equal(nodeScans,0,'aggregation indexes source IDs once instead of scanning all nodes at every endpoint/parent hop');
indexedGraph.nodes[2].parent=800;
assert.equal(context.aggregate(indexedGraph,new Set([10,800])).edges[0].count,2,'a new aggregation cannot reuse endpoint mappings from before a simulated move');
assert.equal(context.aggregate(indexedGraph,new Set([101,205,999])).edges.length,2,'the next cut gets independent endpoint mappings and still drops self edges');
for (const name of ['hasPosition', 'nodeBox', 'hitNode', 'curvePoint', 'nodeRectangles', 'arrowGeometry']) {
  vm.runInContext(declaration(name), context);
}
vm.runInContext('const CURVATURE = .25; const ARROW_GAP = 3;', context);
const target = {name:'domain',kind:'module',x:0,y:0};
// A package is nonconvex: approach the tab/notch from the whole circle at
// short and long distances. No arrow may end inside either painted rectangle.
for (let angle=0;angle<360;angle++) for (const radius of [160,500,10000]) {
  const source = {name:'source',x:radius*Math.cos(angle*Math.PI/180),y:radius*Math.sin(angle*Math.PI/180)};
  const arrow = context.arrowGeometry({source,target});
  assert.ok(arrow && Number.isFinite(arrow.x) && Number.isFinite(arrow.y));
  assert.equal(context.hitNode(target,arrow.x,arrow.y),false,'outside package: '+[angle,radius]);
}
console.log('viewer: syntax, escaping, exact weights, directed aggregation and repeated aggregation passed');
console.log('package arrows: 1,080 approaches clip outside the tabbed contour');
for (const name of ['compareNames', 'containmentLayout']) {
  vm.runInContext(declaration(name), context);
}
const treeNode=(id,name,parent=null,kind='module')=>({id,name,parent,kind});
const treeGraph={nodes:[
  treeNode(0,'mid'),treeNode(1,'mid/domain',0),treeNode(2,'mid/domain/one.ts',1,'file'),
  treeNode(3,'mid/utilities-with-a-long-package-name',0),treeNode(4,'mid/utilities-with-a-long-package-name/two.ts',3,'file'),
  treeNode(5,'external'),treeNode(6,'external/index.ts',5,'file'),treeNode(7,'loose.ts',null,'file'),
],edges:[{from:2,to:4},{from:4,to:6},{from:6,to:2}]};
const visibleIds=new Set([2,4,6,7]),originalTree=JSON.stringify(treeGraph);
const layout=context.containmentLayout(treeGraph,visibleIds,'Example project');
const chainNames=['org','org/apache','org/apache/commons','org/apache/commons/collections4'];
const commonsGraph={nodes:chainNames.map((name,i)=>treeNode(1000+i,name,i?999+i:null)).concat(
  Array.from({length:19},(_,i)=>treeNode(1004+i,chainNames[3]+'/package-'+String(i).padStart(2,'0'),1003))
),edges:[]};
const commonsVisible=new Set(commonsGraph.nodes.slice(4).map(n=>n.id));
const commonsLayout=context.containmentLayout(commonsGraph,commonsVisible,'Apache Commons Collections');
const commonsWidth=commonsLayout.bounds.maxX-commonsLayout.bounds.minX;
const commonsHeight=commonsLayout.bounds.maxY-commonsLayout.bounds.minY;
assert.ok(Math.max(commonsWidth/commonsHeight,commonsHeight/commonsWidth)<=3,
  '19 sibling packages must fit compactly instead of forming one horizontal strip');
assert.ok(new Set(commonsLayout.nodes.filter(n=>!n.context).map(n=>n.y)).size>=4,
  'package siblings wrap into several rows');
assert.ok(commonsLayout.nodes.filter(n=>n.context).length<=2,
  'single-child namespace ancestors share a compact path header');
const namespace=commonsLayout.nodes.find(n=>n.context&&!n.project);
assert.equal(JSON.stringify(namespace.sourceIds),JSON.stringify([1000,1001,1002,1003]),
  'compressed context retains every source ancestor in order');
assert.equal(JSON.stringify(namespace.contextNames),JSON.stringify(chainNames));
assert.equal(namespace.name,chainNames[3],'context header shows the full namespace path');
assert.equal(commonsLayout.positions.size,20,'one chain header and all 19 visible packages retain source positions');
for(const id of commonsVisible){
  const card=commonsLayout.nodes.find(n=>n.src===id);
  assert.equal(card.context,false,'visible packages are never compressed');
  assert.equal(JSON.stringify(card.sourceIds),JSON.stringify([id]));
  assert.equal(commonsLayout.edges.find(e=>e.to===id).from,1003,'children attach to their original namespace');
}
assert.equal(JSON.stringify(treeGraph),originalTree,'layout never mutates the analysis graph');
assert.equal(layout.nodes.length,9,'hidden parents and the project remain visible as tree context');
assert.equal(layout.edges.length,8,'each source node has exactly one containment parent');
const layoutById=new Map(layout.nodes.map(n=>[n.src,n]));
assert.equal(layoutById.get(null).project,true);
for(const source of treeGraph.nodes){
  const painted=layoutById.get(source.id);
  assert.equal(painted.context,!visibleIds.has(source.id),'only missing ancestors are context');
  assert.ok([painted.x,painted.y,painted.w,painted.h].every(Number.isFinite));
  const edge=layout.edges.find(e=>e.to===source.id);
  assert.equal(edge.from,source.parent,'tree links use containment, never import direction');
  const parent=layoutById.get(edge.from);
  assert.ok(painted.y-painted.h/2>parent.y+parent.h/2,'children are on a separate level below their parent');
  assert.ok(painted.x-painted.w/2>=layout.bounds.minX&&painted.x+painted.w/2<=layout.bounds.maxX);
  assert.ok(painted.y-painted.h/2>=layout.bounds.minY&&painted.y+painted.h/2<=layout.bounds.maxY);
}
const rootBox=layoutById.get(null);
assert.ok(rootBox.y-rootBox.h/2>=layout.bounds.minY,'Fit includes the project heading');
const roots=[5,0,7].map(id=>layoutById.get(id));
for(let i=1;i<roots.length;i++){
  const before=roots[i-1],after=roots[i],beforeTop=before.y-before.h/2,afterTop=after.y-after.h/2;
  assert.ok(afterTop>beforeTop||(afterTop===beforeTop&&after.x>before.x),'package-first sibling order follows wrapped reading order');
}
function checkLayoutSeparation(tree){
  for(let i=0;i<tree.nodes.length;i++)for(let j=i+1;j<tree.nodes.length;j++){
    const a=tree.nodes[i],b=tree.nodes[j];
    assert.ok(Math.abs(a.x-b.x)>=(a.w+b.w)/2||Math.abs(a.y-b.y)>=(a.h+b.h)/2,'tree cards do not overlap: '+a.name+' / '+b.name);
  }
  const bySource=new Map(tree.nodes.map(n=>[n.src,n]));
  for(const edge of tree.edges){
    const parent=bySource.get(edge.from),child=bySource.get(edge.to);
    assert.ok(child.y-child.h/2>parent.y+parent.h/2,'ancestors precede every descendant row');
    if(edge.railX!==null){
      // A shared outside rail may run beside several rows, but never through
      // any earlier sibling card while approaching a later row.
      for(const card of tree.nodes){
        const railTop=parent.y+parent.h/2+8,railBottom=edge.rowTop-12;
        const intersectsY=card.y+card.h/2>railTop&&card.y-card.h/2<railBottom;
        assert.ok(!intersectsY||edge.railX<=card.x-card.w/2||edge.railX>=card.x+card.w/2,
          'containment rail avoids cards: '+card.name);
      }
    }
  }
}
checkLayoutSeparation(layout);checkLayoutSeparation(commonsLayout);
const positions=tree=>JSON.stringify([...tree.positions].sort((a,b)=>a[0]-b[0]));
const reordered=context.containmentLayout({nodes:treeGraph.nodes.slice().reverse(),edges:[]},new Set([7,6,4,2]),'Example project');
assert.equal(positions(reordered),positions(layout),'input order and dependency cycles cannot affect tree positions');
context.containmentLayout(treeGraph,new Set([0,5,7]),'Example project');
assert.equal(positions(context.containmentLayout(treeGraph,visibleIds,'Example project')),positions(layout),'returning to a scope restores exact coordinates');
assert.equal(positions(context.containmentLayout({nodes:commonsGraph.nodes.slice().reverse(),edges:[]},[...commonsVisible].reverse(),'Apache Commons Collections')),positions(commonsLayout),'wrapped layout is independent of source/set order');
context.containmentLayout(commonsGraph,new Set([1001]),'Apache Commons Collections');
assert.equal(positions(context.containmentLayout(commonsGraph,commonsVisible,'Apache Commons Collections')),positions(commonsLayout),'namespace scope return restores exact coordinates');
const emptyLayout=context.containmentLayout(treeGraph,new Set(),'Example project');
assert.equal(emptyLayout.nodes.length,0);assert.equal(emptyLayout.bounds,null,'empty trees never create invalid camera bounds');
const manyPackages={nodes:Array.from({length:500},(_,id)=>treeNode(id,'package-'+String(id).padStart(3,'0'))),edges:[]};
const wideLayout=context.containmentLayout(manyPackages,new Set(manyPackages.nodes.map(n=>n.id)),'Large project');
assert.equal(wideLayout.nodes.filter(n=>!n.context).length,500);
assert.equal(wideLayout.positions.size,500);
const wideWidth=wideLayout.bounds.maxX-wideLayout.bounds.minX,wideHeight=wideLayout.bounds.maxY-wideLayout.bounds.minY;
assert.ok(Math.max(wideWidth/wideHeight,wideHeight/wideWidth)<=3,'500 siblings remain compact');
checkLayoutSeparation(wideLayout);
assert.equal(positions(context.containmentLayout({nodes:manyPackages.nodes.slice().reverse(),edges:[]},manyPackages.nodes.map(n=>n.id).reverse(),'Large project')),positions(wideLayout));
console.log('tree layout: wrapped subtrees, compressed namespace evidence, bounds, separation, determinism and 500-package spacing passed');
