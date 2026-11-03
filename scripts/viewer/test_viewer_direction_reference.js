// Inspecting a visible dependency keeps its direction reference and geometry.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to)=>({from,to,count:1,refs:[{file:'source.ts',line:1,raw:'import target;'}]});
const fixture={root:'/tmp/reference',nodes:['A','B','C','D','E'].map((name,id)=>node(id,name)),edges:[edge(0,1),edge(2,0),edge(1,3),edge(4,3)],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const mode=(app,value)=>app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===value).click();
const select=(app,from,to)=>app.run('selectEdge(view.edges.find(e=>view.nodes[e.from].name==='+JSON.stringify(from)+'&&view.nodes[e.to].name==='+JSON.stringify(to)+'))');
const pairs=app=>JSON.parse(app.run("JSON.stringify(view.edges.filter(e=>dependencyMatchesSelection(e.from,e.to)).map(e=>view.nodes[e.from].name+' → '+view.nodes[e.to].name).sort())"));
const paths=app=>app.ids.get('package-tree').querySelectorAll('g[data-tree-edge]').map(group=>{
  const path=group.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-line');
  return [group.getAttribute('data-tree-edge'),path.getAttribute('d'),path.getAttribute('stroke'),group.querySelector('polygon').getAttribute('fill')];
});
const reference=app=>JSON.parse(app.run('JSON.stringify(directionReference()&&{name:directionReference().name,ids:[...directionReference().ids]})'));
for(const direction of ['both','incoming','outgoing']){
  const app=launch(fixture);app.run("selectNode('A')");mode(app,direction);
  const beforePairs=pairs(app),beforePaths=paths(app),analysis=app.run('JSON.stringify([work,view,ops,trail,currentAcd])');
  select(app,...(direction==='incoming'?['C','A']:['A','B']));
  assert.equal(app.run('ui.selection.reference'),'A','inspecting a visible dependency retains the selected node as its direction reference');
  assert.deepEqual(reference(app),{name:'A',ids:[0]});
  assert.deepEqual(pairs(app),beforePairs,'an evidence click keeps the current direction filter scope');
  assert.deepEqual(paths(app),beforePaths,'an evidence click preserves sides, routed paths and direction colors');
  assert.equal(app.run('JSON.stringify([work,view,ops,trail,currentAcd])'),analysis,'reference retention does not mutate the analysis');
  assert.match(app.ids.get('edge-reference').textContent,/A/);
  assert.match(app.ids.get('edge-legend').textContent,/Reference: A/);
  mode(app,'both');select(app,'C','A');assert.equal(reference(app).name,'A','inspecting another edge keeps the same reference');
  app.run("selectNode('B')");assert.equal(reference(app).name,'B','a new node selection replaces the direction reference');
  app.run('resetView()');assert.equal(reference(app),null,'reset removes the retained reference with the selection');
}

// Matrix/other explicit selections can target an edge outside the current
// filter. They retain the existing endpoint-pair fallback, so the edge is shown.
const outside=launch(fixture);outside.run("selectNode('A')");mode(outside,'both');
assert.equal(pairs(outside).includes('E → D'),false);select(outside,'E','D');
assert.equal(outside.run('ui.selection.reference'),undefined);assert.equal(reference(outside),null);
assert.equal(outside.run('edgeReference().name'),'E ↔ D');assert.ok(pairs(outside).includes('E → D'));
outside.run("selectNode('A')");mode(outside,'incoming');select(outside,'A','B');
assert.equal(reference(outside),null,'an edge outside the explicit direction filter does not retain a hiding reference');
assert.ok(pairs(outside).includes('A → B'));

const packages={...fixture,nodes:[node(0,'0Before',null,'module'),node(1,'0Before/one.ts',0),node(2,'0Before/two.ts',0),node(3,'A',null,'module'),node(4,'A/Start.ts',3),node(5,'A/End.ts',3),node(6,'B',null,'module'),node(7,'B/Other.ts',6),node(8,'C',null,'module'),node(9,'C/Other.ts',8)],edges:[edge(4,7),edge(7,9),edge(9,5)]};
const opened=launch(packages);opened.run("expandNode('A')");mode(opened,'both');
assert.equal(opened.run('ui.selection'),null,'fixture uses the implicit opened-package reference');
const originalPairs=pairs(opened),originalPaths=paths(opened);
select(opened,'A/Start.ts','B');assert.equal(opened.run('ui.selection.reference'),'A');
assert.deepEqual(reference(opened),{name:'A',ids:[1,2]},'an opened package projects to its visible descendants');
assert.deepEqual(pairs(opened),originalPairs,'external package-cycle return paths survive inspection');
assert.deepEqual(paths(opened),originalPaths);
opened.run("expandNode('0Before')");
assert.deepEqual(reference(opened),{name:'A',ids:[2,3]},'reference projection follows new dense IDs instead of caching old IDs');
assert.equal(opened.run('ui.selection.reference'),'A','opening another package does not rewrite the inspected edge reference');
const traversals=opened.run('(()=>{const original=childrenOf;let visits=0;childrenOf=(graph,id)=>{visits++;return original(graph,id);};try{for(let i=0;i<100;i++)directionReference();return visits;}finally{childrenOf=original;}})()');
assert.ok(traversals<=packages.nodes.length,'repeated edge painting must not retraverse the entire package for each color lookup');
opened.click('simulation-mode');opened.run("recordOp(encodeMove('A/End.ts','0Before'))");
assert.deepEqual(reference(opened),{name:'A',ids:[3]},'simulation recomputes the reference from current package membership');
opened.click('undo-ops');assert.deepEqual(reference(opened),{name:'A',ids:[2,3]});
opened.run("ui.selection.reference='missing/package'");assert.equal(reference(opened),null,'a missing reference cannot alias an unrelated dense ID');
assert.equal(opened.run('edgeReference().name'),'A/Start.ts ↔ B');

const cycle=launch({...fixture,edges:[...fixture.edges,edge(1,0)]});cycle.run("selectNode('A')");mode(cycle,'both');cycle.click('toggle-cycles');
const cyclePairs=pairs(cycle),cyclePaths=paths(cycle);select(cycle,'A','B');
assert.deepEqual(pairs(cycle),cyclePairs);assert.deepEqual(paths(cycle),cyclePaths,'cycle filtering and completely red cycle edges survive evidence inspection');
assert.equal(cycle.run('ui.cyclesOnly'),true);
console.log('Direction reference: stable edge inspection, filter/color/path preservation, external return paths, dense-ID projection, simulation/undo and explicit outside-filter fallback passed');

// Direction can be chosen before the reference, without blanking the graph.
const pending=launch(fixture);
for(const direction of ['incoming','outgoing','both']){
 pending.run('resetView()');
 const control=pending.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===direction);
 assert.equal(control.disabled,false,'direction buttons remain usable before selecting a node');
 mode(pending,direction);
 assert.equal(pending.run('ui.edgeMode'),direction);
 assert.equal(pairs(pending).length,fixture.edges.length,'pending direction preserves graph context');
 assert.match(pending.ids.get('edge-reference').textContent,/Select a package or file|Paket oder Datei auswählen/);
 pending.run("selectNode('A')");
 assert.deepEqual(pairs(pending),direction==='incoming'?['C → A']:direction==='outgoing'?['A → B']:['A → B','C → A']);
 assert.equal(pending.run('ui.edgeMode'),direction,'node selection applies the chosen direction');
}
console.log('Direction preselection: clickable controls, visible context, selection guidance and applied incoming/outgoing filters passed');

// Expanded packages contain internal edges: these are neither incoming nor outgoing.
const boundaryFixture={...fixture,nodes:[node(0,'app',null,'module'),node(1,'app/A.ts',0),node(2,'app/B.ts',0),node(3,'outside.ts')],edges:[edge(1,2),edge(3,1),edge(2,3)]};
const boundary=launch(boundaryFixture);
boundary.run("expandNode('app');selectNode('app')");
mode(boundary,'incoming');assert.deepEqual(pairs(boundary),['outside.ts → app/A.ts'],'incoming excludes internal package edges');
mode(boundary,'outgoing');assert.deepEqual(pairs(boundary),['app/B.ts → outside.ts'],'outgoing excludes internal package edges');
mode(boundary,'both');assert.ok(pairs(boundary).includes('app/A.ts → app/B.ts'),'both retains internal package context');
const internalOnly=launch({...boundaryFixture,edges:[edge(1,2)]});
internalOnly.run("expandNode('app');selectNode('app')");
for(const direction of ['incoming','outgoing']){mode(internalOnly,direction);assert.deepEqual(pairs(internalOnly),[],'internal-only packages have no boundary dependencies');}
mode(internalOnly,'both');assert.deepEqual(pairs(internalOnly),['app/A.ts → app/B.ts']);
console.log('Package boundaries: internal edges excluded from in/out and preserved in both; empty boundaries remain empty');
