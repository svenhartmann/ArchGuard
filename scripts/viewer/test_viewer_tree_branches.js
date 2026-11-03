// Inline exploration changes one branch without navigating away from its siblings.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='module')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to,count=1)=>({from,to,count,refs:[{file:'source-'+from+'.ts',line:from+1,raw:'import target;'}]});
const fixture={root:'/tmp/branches',nodes:[
  node(0,'A'),node(1,'A/inner',0),node(2,'A/inner/start.ts',1,'file'),node(3,'A/inner/end.ts',1,'file'),node(4,'A/direct.ts',0,'file'),
  node(5,'B'),node(6,'B/b.ts',5,'file'),node(7,'C'),node(8,'C/c.ts',7,'file'),node(9,'Outside'),node(10,'Outside/o.ts',9,'file'),
],edges:[edge(2,6,3),edge(6,8,5),edge(8,3,7),edge(4,6,11)],view:{depth:0,visible:null},cyclicEdges:[],findings:[]};
const scope=app=>JSON.parse(app.run('JSON.stringify(view.nodes.map(n=>n.name).sort())'));
const toggle=(app,id,opened)=>app.run('toggleTreeBranch('+id+(opened===undefined?'':','+opened)+')');
const names=app=>JSON.parse(app.run('JSON.stringify(displayedGraphNodes().map(n=>n.name).sort())'));
const pairs=app=>JSON.parse(app.run("JSON.stringify(view.edges.filter(e=>dependencyMatchesSelection(e.from,e.to)).map(e=>view.nodes[e.from].name+' → '+view.nodes[e.to].name).sort())"));
const app=launch(fixture,{library:false});
assert.equal(app.run('typeof toggleTreeBranch'),'function','the central exploration tree needs a branch-preserving disclosure API');
const baseline=app.run('JSON.stringify([data,work,ops])');
app.run('treeZoom=.5');
assert.equal(toggle(app,0,true),true);
assert.deepEqual(scope(app),['A/direct.ts','A/inner','B','C','Outside']);
assert.ok(names(app).includes('A/direct.ts'),'opening a package explicitly reveals its direct file children');
assert.equal(toggle(app,5,true),true);
assert.deepEqual(scope(app),['A/direct.ts','A/inner','B/b.ts','C','Outside'],'opening B preserves the already expanded A branch');
assert.equal(toggle(app,1,true),true);
assert.deepEqual(scope(app),['A/direct.ts','A/inner/end.ts','A/inner/start.ts','B/b.ts','C','Outside']);
assert.equal(app.run('treeZoom'),.5);
assert.equal(app.run('ui.selection.name'),'A/inner');
assert.ok(app.ids.get('breadcrumbs').textContent.endsWith('A › inner'));
assert.equal(toggle(app,0,false),true);
assert.deepEqual(scope(app),['A','B/b.ts','C','Outside'],'collapsing A does not collapse B');
assert.equal(app.run('ui.selection.name'),'A');
assert.equal(app.run("view.edges.find(e=>view.nodes[e.from].name==='A'&&view.nodes[e.to].name==='B/b.ts').count"),14,'collapse sums the original import totals without recounting the evidence sample');
assert.equal(app.run("view.edges.find(e=>view.nodes[e.from].name==='A'&&view.nodes[e.to].name==='B/b.ts').refs.length"),2,'collapse retains the original two source sites');
assert.ok(app.ids.get('breadcrumbs').textContent.endsWith('A'),'inline collapse location wins over another expanded sibling');
assert.equal(app.run('explorerOpen.has(0)||explorerOpen.has(1)'),false);
assert.equal(app.run('explorerOpen.has(5)'),true);
assert.equal(app.run('treeZoom'),.5);
const previous=app.run('JSON.stringify([visible,view,trail,treeZoom])');
assert.equal(toggle(app,0,false),false,'an already collapsed branch is an exact no-op');
assert.equal(toggle(app,10,true),false,'files do not expand');
assert.equal(toggle(app,999,true),false,'unknown source ids do not mutate the scope');
assert.equal(app.run('JSON.stringify([visible,view,trail,treeZoom])'),previous);
assert.equal(app.run('JSON.stringify([data,work,ops])'),baseline,'disclosures never change graph data or simulation operations');
assert.equal(toggle(app,0,true),true);
assert.deepEqual(scope(app),['A/direct.ts','A/inner','B/b.ts','C','Outside'],'reopening A shows direct children without reopening its previously collapsed descendants');
app.run('resetView()');assert.deepEqual(scope(app),['A','B','C','Outside']);assert.equal(app.run('treeLocation'),null);

// Context ancestors above the report cut can collapse without bringing in
// unrelated roots or siblings omitted by an explicit focused report.
for(const view of [{depth:3,visible:null},{depth:3,visible:[2,6]}]){
 const detailed=launch({...fixture,view},{library:false}),initial=scope(detailed),source=detailed.run('JSON.stringify(data)');
 assert.equal(toggle(detailed,0,false),true);
 assert.deepEqual(scope(detailed),view.visible?['A','B/b.ts']:['A','B/b.ts','C/c.ts','Outside/o.ts']);
 assert.equal(toggle(detailed,0,true),true);
 assert.deepEqual(scope(detailed),view.visible?['A/direct.ts','A/inner','B/b.ts']:['A/direct.ts','A/inner','B/b.ts','C/c.ts','Outside/o.ts']);
 assert.equal(detailed.run('baselineStats.acd'),detailed.run('currentAcd'),'baseline uses the same explicitly coarsened cut');
 detailed.run('resetView()');assert.deepEqual(scope(detailed),initial);assert.equal(detailed.run('JSON.stringify(data)'),source);
 toggle(detailed,0,false);detailed.run('navigateExplorerNode(5)');
 assert.deepEqual(scope(detailed),['A','B/b.ts','C','Outside'],'Explorer keeps its established navigation semantics after inline toggles');
}

// A coarsened report cut derives current SCCs even with an empty expansion trail.
const coarsened=launch({...fixture,view:{depth:3,visible:null}},{library:false});
for(const id of [0,5,7])toggle(coarsened,id,false);
assert.equal(coarsened.run('trail.length'),0);
assert.equal(coarsened.run('sccs.length'),1);
assert.equal(coarsened.run('cyclesForView().length'),1,'collapse cannot reuse stale initial file findings');
const location=coarsened.run('treeLocation');coarsened.run("expandNode('missing')");
assert.equal(coarsened.run('treeLocation'),location,'invalid navigation does not discard the inline location');
coarsened.run("revealNode('Outside/o.ts')");
assert.ok(coarsened.ids.get('breadcrumbs').textContent.endsWith('Outside › o.ts'),'explicit reveal leaves the previous inline location');

// With multiple branches open, selecting either branch keeps the complete
// package-level cycle including outside-to-outside return edges.
const cycles=launch(fixture,{library:false});toggle(cycles,0,true);toggle(cycles,1,true);toggle(cycles,5,true);
const cyclePairs=['A/inner/start.ts → B/b.ts','A/direct.ts → B/b.ts','B/b.ts → C','C → A/inner/end.ts'].sort();
for(const name of ['A','B','A/inner']){
 const analysis=cycles.run('JSON.stringify([work,view,trail,sccs,currentAcd,ops])');
 cycles.run('selectNode('+JSON.stringify(name)+')');
 assert.equal(cycles.run('packageContext.name'),name);
 assert.deepEqual(pairs(cycles),cyclePairs,'selected '+name+' owns the outer cycle context across every open branch');
 assert.equal(cycles.run('JSON.stringify([work,view,trail,sccs,currentAcd,ops])'),analysis,'changing the context reference does not rebuild analysis');
}
cycles.run("selectEdge(view.edges.find(e=>view.nodes[e.from].name==='B/b.ts'))");
assert.equal(cycles.run('ui.selection.reference'),'A/inner');assert.deepEqual(pairs(cycles),cyclePairs);
cycles.click('toggle-cycles');assert.deepEqual(pairs(cycles),cyclePairs);
assert.equal(cycles.run('sccs.length'),0,'a projected package-cycle is not mislabeled as an actual file-level SCC');

// Collapsing a branch must not inherit the last other branch's cycle context.
const unrelated=launch({...fixture,edges:[edge(6,8),edge(8,6)]},{library:false});
toggle(unrelated,0,true);toggle(unrelated,5,true);toggle(unrelated,0,false);
assert.equal(unrelated.run('packageContext'),null,"a collapsed reference has no other branch's expansion context");
assert.deepEqual(pairs(unrelated),[],'Both does not show an unrelated B/C cycle when collapsed A is selected');

// Pending simulation and source-id-based expansions survive collapse and Undo.
const simulated=launch(fixture,{library:false});toggle(simulated,0,true);toggle(simulated,5,true);
simulated.click('simulation-mode');simulated.run("recordOp(encodeCut('B/b.ts','C/c.ts'))");
const model=simulated.run('JSON.stringify([data,work,ops])');toggle(simulated,0,false);toggle(simulated,0,true);
assert.equal(simulated.run('JSON.stringify([data,work,ops])'),model);
assert.ok(scope(simulated).includes('B/b.ts'));
simulated.click('undo-ops');assert.ok(scope(simulated).includes('B/b.ts'));assert.equal(simulated.run('ops.length'),0);
assert.ok(pairs(simulated).includes('B/b.ts → C'),'undo restores the full package-cycle return path');
// Source IDs, rather than paths, keep the right branches open across a move.
const moved=launch(fixture,{library:false});toggle(moved,0,true);toggle(moved,5,true);toggle(moved,1,true);
moved.click('simulation-mode');moved.run("recordOp(encodeMove('A/inner','B'))");
assert.ok(scope(moved).includes('B/inner/start.ts'));
assert.ok(scope(moved).includes('B/b.ts'));
toggle(moved,0,false);
assert.ok(scope(moved).includes('B/inner/start.ts'),'collapsing A cannot close a subtree moved into B');
moved.click('undo-ops');
assert.deepEqual(scope(moved),['A','B/b.ts','C','Outside'],"Undo respects the user's collapsed A while preserving expanded B");
console.log('Inline tree branches: independent expansion, initial-cut collapse, aggregation, context reference, zoom, source immutability and simulation passed');
