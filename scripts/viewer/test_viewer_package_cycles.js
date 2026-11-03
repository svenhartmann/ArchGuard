// Package navigation keeps cycle evidence across sibling/ancestor boundaries.
// Execute the actual viewer with the shared deterministic DOM/ForceGraph doubles.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to,count=1)=>({from,to,count,refs:[{file:'source-'+from+'.ts',line:from+1,raw:'import target_'+to+';'}]});
const nodes=[
  node(0,'root',null,'module'),node(1,'root/A',0,'module'),node(2,'root/A/inner',1,'module'),
  node(3,'root/A/inner/Start.ts',2),node(4,'root/A/inner/End.ts',2),
  node(5,'root/A/sibling',1,'module'),node(6,'root/A/sibling/Sibling.ts',5),node(7,'root/A/Parent.ts',1),
  node(8,'root/B',0,'module'),node(9,'root/B/External.ts',8),node(10,'root/Parent.ts',0),
  node(11,'root/Unrelated',0,'module'),node(12,'root/Unrelated/U.ts',11),
  node(13,'root/Other',0,'module'),node(14,'root/Other/O.ts',13),node(15,'root/Loose.ts',0),
];
const finding=(id,names)=>({id,kind:'cycle',severity:'error',nodes:names,edge_count:names.length,suggested_cuts:[]});
const fixture={root:'/tmp/package-cycles',nodes,
  edges:[edge(3,6,3),edge(6,10,5),edge(10,9,7),edge(9,3,11),edge(3,4,13),edge(12,14,17),edge(14,12,19),edge(15,9,23)],
  findings:[finding('scc-main',['root/A','root/B','root/Parent.ts']),finding('scc-unrelated',['root/Unrelated','root/Other'])],
  cyclicEdges:[],view:{depth:1,visible:null},
};
const names=app=>Array.from(app.graph.data.nodes,n=>n.name).sort();
const pair=l=>l.source.name+' → '+l.target.name;
const visibleLinks=app=>Array.from(app.graph.data.links).filter(app.graph.linkVisibility);
const visiblePairs=app=>visibleLinks(app).map(pair).sort();
const mode=(app,value)=>app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===value).click();
function openInner(app){app.run("expandNode('root/A');expandNode('root/A/inner')");}
const mainCycle=[
  'root/A/inner/Start.ts → root/A/sibling',
  'root/A/sibling → root/Parent.ts',
  'root/Parent.ts → root/B',
  'root/B → root/A/inner/Start.ts',
].sort();
const app=launch(fixture);openInner(app);
for(const expected of mainCycle)assert.ok(visiblePairs(app).includes(expected),'opening a package retains its complete external cycle: '+expected);
assert.ok(names(app).includes('root/Parent.ts'),'a cycle reveals its file endpoint on the parent level');
assert.equal(names(app).includes('root/Loose.ts'),false,'unrelated parent-level files remain hidden');
assert.equal(visiblePairs(app).some(p=>p.includes('root/Unrelated')||p.includes('root/Other')),false,'an unrelated cycle does not clutter the opened package');
assert.equal(app.graph.linkColor(visibleLinks(app).find(l=>l.source.name==='root/B')),app.run('C.cycle'),'the complete incoming cycle edge stays red without a child selection');
assert.equal(app.graph.linkColor(visibleLinks(app).find(l=>l.target.name==='root/A/sibling')),app.run('C.cycle'),'the complete outgoing cycle edge stays red without a child selection');
assert.match(app.ids.get('edge-legend').textContent,/Inside: inner.*Incoming.*Outgoing/,'the legend identifies the reference package');

// Revealing a cycle is display-only: directed totals and source references
// survive aggregation unchanged, including the two outside-to-outside hops.
for(const [expected,index] of [[mainCycle.find(p=>p.startsWith('root/A/inner/Start')),0],[mainCycle.find(p=>p.startsWith('root/A/sibling')),1],[mainCycle.find(p=>p.startsWith('root/Parent')),2],[mainCycle.find(p=>p.startsWith('root/B')),3]]){
  const link=visibleLinks(app).find(l=>pair(l)===expected);
  assert.equal(link.count,fixture.edges[index].count);
  assert.equal(JSON.stringify(link.refs),JSON.stringify(fixture.edges[index].refs));
}
const analysis=app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])');
app.run("selectNode('root/A/inner/Start.ts')");
for(const expected of mainCycle)assert.ok(visiblePairs(app).includes(expected),'selecting a child keeps the full external cycle in Both: '+expected);
const incoming=visibleLinks(app).find(l=>l.source.name==='root/B');
const outgoing=visibleLinks(app).find(l=>l.target.name==='root/A/sibling');
assert.equal(app.graph.linkColor(incoming),app.run('C.cycle'),'cycle color overrides the selected child\'s incoming direction color');
assert.equal(app.graph.linkColor(outgoing),app.run('C.cycle'),'cycle color overrides the selected child\'s outgoing direction color');
mode(app,'incoming');
assert.deepEqual(visiblePairs(app),['root/B → root/A/inner/Start.ts'],'Incoming obeys the explicitly selected child');
mode(app,'outgoing');
assert.deepEqual(visiblePairs(app),['root/A/inner/Start.ts → root/A/inner/End.ts','root/A/inner/Start.ts → root/A/sibling'],'Outgoing obeys the explicitly selected child');
mode(app,'all');
assert.ok(visiblePairs(app).includes('root/Unrelated → root/Other'),'All still includes unrelated visible dependencies');
mode(app,'both');
assert.equal(app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),analysis,'selection and display filters do not alter analytical scope or evidence');

// A deliberately focused cycle is authoritative over the navigation context.
app.run("focusCycle(cyclesForView().find(c=>c.nodes.includes('root/Unrelated')))");
assert.deepEqual(names(app),['root/Other','root/Unrelated']);
assert.deepEqual(visiblePairs(app),['root/Other → root/Unrelated','root/Unrelated → root/Other']);
app.click('cycles-reset');
for(const expected of mainCycle)assert.ok(visiblePairs(app).includes(expected),'leaving cycle focus restores the opened package context');

// Derived external-file visibility follows the simulated graph rather than
// becoming a permanent manual reveal after the first cycle is shown.
const simulated=launch(fixture);openInner(simulated);
const originalData=simulated.run('JSON.stringify(data)');
simulated.click('simulation-mode');
simulated.run("recordOp(encodeCut('root/Parent.ts','root/B'))");
assert.equal(names(simulated).includes('root/Parent.ts'),false,'cutting the return path hides an external file no longer needed by the cycle');
assert.equal(simulated.run('ops.length'),1);
assert.equal(simulated.run('JSON.stringify(data)'),originalData,'simulation and context preserve original source evidence');
simulated.click('undo-ops');
assert.ok(names(simulated).includes('root/Parent.ts'));
for(const expected of mainCycle)assert.ok(visiblePairs(simulated).includes(expected),'undo restores the full cycle and its external files');
simulated.ids.get('breadcrumbs').querySelectorAll('button[data-node]').find(b=>b.dataset.node==='1').click();
assert.ok(names(simulated).includes('root/A/inner'));
assert.equal(names(simulated).includes('root/A/inner/Start.ts'),false);
assert.ok(visiblePairs(simulated).includes('root/B → root/A/inner'),'breadcrumb collapse recomputes the parent package cycle scope');
simulated.ids.get('breadcrumbs').querySelectorAll('button')[0].click();
assert.equal(names(simulated).includes('root/Parent.ts'),false,'Reset clears derived external context');
assert.equal(simulated.run('edgeReference()'),null,'Reset clears the direction reference');
assert.equal(simulated.run('ui.edgeMode'),'both','the direction chosen by opening a package survives Reset as a pending filter');
assert.deepEqual(visiblePairs(simulated),['root/B → root/A','root/Other → root/Unrelated','root/Unrelated → root/Other'],'a pending direction keeps the overview connections visible until a new selection');
assert.equal(simulated.run('JSON.stringify(data)'),originalData);

// Two architecture cycles cease to be SCCs at file resolution: Start and End
// are distinct and have no return path. Keep the genuine dependency paths as
// clearly labeled package-level context, not as invented file-level cycles.
const coarseFixture={...fixture,edges:[edge(3,6,3),edge(6,4,5),edge(3,9,7),edge(9,7,11)],findings:[finding('scc-ancestor',['root/A','root/B'])]};
const coarse=launch(coarseFixture);openInner(coarse);
const coarsePairs=['root/A/inner/Start.ts → root/A/sibling','root/A/sibling → root/A/inner/End.ts','root/A/inner/Start.ts → root/B','root/B → root/A/Parent.ts'].sort();
assert.deepEqual(visiblePairs(coarse),coarsePairs,'both immediate and higher ancestor package-cycle paths remain visible');
assert.equal(coarse.run('sccs.length'),0,'package context does not manufacture finer-grained SCCs');
assert.equal(coarse.run('cyclicKeys.size'),0,'package context does not mark its acyclic fine edges as cyclic');
assert.equal(coarse.graph.data.nodes.some(n=>n.inCycle),false);
assert.equal(coarse.run('findSccs(work).length'),0,'the original files remain acyclic');
function assertContextColors(app,selectedChild=false){
  for(const link of visibleLinks(app)){
    const color=app.run('C.'+(link.source.name.endsWith('Start.ts')?'outgoing':!selectedChild&&link.target.name.endsWith('End.ts')?'incoming':'edge'));
    assert.equal(app.graph.linkColor(link),color,'acyclic package context keeps its direction color: '+pair(link));
    assert.deepEqual(Array.from(app.graph.linkLineDash(link)),[6,4],'network distinguishes coarser package context with dashes');
    assert.match(app.graph.linkLabel(link).textContent,/Package-level cycle context.*not cyclic in the current detail scope/);
    const key=link.source.id+' '+link.target.id;
    const group=app.ids.get('package-tree').querySelectorAll('g[data-tree-edge]').find(g=>g.getAttribute('data-tree-edge')===key);
    const line=group.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-line');
    assert.equal(line.getAttribute('stroke'),color,'tree line uses the same direction color');
    assert.equal(line.getAttribute('stroke-dasharray'),'6 4','tree distinguishes coarser package context with dashes');
    assert.equal(group.querySelector('polygon').getAttribute('fill'),color,'arrowhead and line agree');
    group.dispatch('pointerenter',{clientX:100,clientY:100});
    const tip=app.ids.get('graph-context-tooltip');
    assert.equal(tip.hidden,false);
    assert.match(tip.textContent,/Package-level cycle context.*not cyclic in the current detail scope/);
    assert.ok(tip.textContent.includes(link.refs[0].raw),'context qualification retains exact import evidence');
    group.dispatch('pointerleave');
  }
}
assertContextColors(coarse);
// True cycles in the first fixture keep solid red lines, including external hops.
for(const link of visibleLinks(app).filter(link=>app.graph.linkColor(link)===app.run('C.cycle'))){
  assert.equal(app.graph.linkLineDash(link),null,'real current-scope cycles stay solid');
}
const hint=coarse.ids.get('graph-hint').textContent;
assert.match(hint,/package-level/i);assert.match(hint,/cycle/i);assert.ok(hint.includes('root/A/inner'),'the hint identifies the package being inspected');
coarse.run("selectNode('root/A/inner/Start.ts')");
assert.deepEqual(visiblePairs(coarse),coarsePairs,'child selection also retains coarse cycle return paths in Both');
assertContextColors(coarse,true);
mode(coarse,'incoming');assert.deepEqual(visiblePairs(coarse),[],'coarse package context does not invent incoming dependencies to Start');
mode(coarse,'outgoing');assert.equal(visibleLinks(coarse).length,2);assertContextColors(coarse,true);
mode(coarse,'both');coarse.run('ui.cyclesOnly=true;renderGraphControls()');
assert.deepEqual(visiblePairs(coarse),coarsePairs,'Cycles only retains context paths without claiming file cycles');assertContextColors(coarse,true);
coarse.run('ui.cyclesOnly=false;renderGraphControls()');
assert.match(coarse.ids.get('cycles-panel').textContent,/Package-level cycle/,'the cycle list distinguishes coarse context from current SCCs');
for(const [member,expected] of [
  ['root/A/inner',['root/A/inner/Start.ts → root/A/sibling','root/A/sibling → root/A/inner/End.ts']],
  ['root/A',coarsePairs],
]){
  coarse.run('focusCycle(cyclesForView().find(c=>c.origin==="package"&&c.nodes.includes('+JSON.stringify(member)+')))');
  assert.notEqual(coarse.run('ui.cycle'),null,'focusing a coarse cycle must retain its identity after recomputation');
  assert.deepEqual(visiblePairs(coarse),expected.slice().sort());
  assert.match(coarse.ids.get('sidebar').textContent,/coarser package level/);
  assert.equal(coarse.run('sccs.length'),0);
  assert.equal(coarse.run('cyclicKeys.size'),0);
  coarse.click('cycles-reset');
  assert.deepEqual(visiblePairs(coarse),coarsePairs,'leaving coarse-cycle focus restores the full package context');
}

const unrelatedAncestor=launch({...coarseFixture,edges:[edge(7,9),edge(9,6)]});openInner(unrelatedAncestor);
assert.deepEqual(visiblePairs(unrelatedAncestor),[],'a parent-level cycle with no participating child in this package stays hidden');

// Selecting an already expanded ancestor must retain its descendant scope.
// Otherwise reconcileSelection drops P and silently filters against P/A instead.
const ancestorFixture={root:'/tmp/ancestor-reference',nodes:[
  node(0,'P',null,'module'),node(1,'P/A',0,'module'),node(2,'P/A/a.ts',1),
  node(3,'P/Parent.ts',0),node(4,'B',null,'module'),node(5,'B/b.ts',4),
],edges:[edge(3,5,3),edge(5,3,5)],findings:[finding('scc-parent',['P','B'])],cyclicEdges:[],view:{depth:0,visible:null}};
const ancestor=launch(ancestorFixture);
ancestor.run("expandNode('P');expandNode('P/A')");
const ancestorAnalysis=ancestor.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])');
const ancestorPositions=ancestor.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))');
const ancestorZoom=ancestor.graph.zoom,ancestorUpdates=ancestor.graph.updates;
ancestor.run("revealNode('P')");
assert.equal(ancestor.run('ui.selection?.name'),'P','the explicitly selected expanded ancestor remains the reference');
assert.equal(ancestor.run('ui.selection.kind'),'node');
assert.equal(ancestor.ids.get('edge-reference').textContent,'Reference: P (package contents)');
for(const [value,count] of [['incoming',1],['outgoing',1],['both',2],['all',2]]){
  const button=ancestor.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===value);
  assert.equal(button.disabled,false,'expanded ancestors enable direction filters');
  assert.equal(Number(button.textContent.match(/\((\d+)\)$/)[1]),count,'the button previews the visible edge count for '+value);
}
assert.equal(ancestor.ids.get('sidebar').querySelectorAll('h2')[0].textContent,'P');
assert.equal(ancestor.ids.get('module-list').querySelectorAll('button[data-node]').find(b=>b.dataset.node==='0').getAttribute('aria-pressed'),'true','Explorer marks the selected ancestor');
mode(ancestor,'incoming');assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts']);
mode(ancestor,'outgoing');assert.deepEqual(visiblePairs(ancestor),['P/Parent.ts → B']);
mode(ancestor,'both');assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts','P/Parent.ts → B']);
assert.equal(ancestor.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),ancestorAnalysis,'selecting an ancestor does not alter analysis or simulation');
assert.equal(ancestor.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))'),ancestorPositions);
assert.equal(ancestor.graph.zoom,ancestorZoom,'selecting an already represented ancestor preserves zoom');
assert.equal(ancestor.graph.updates,ancestorUpdates,'ancestor selection does not rebuild unchanged graph data');
ancestor.run("selectNode('P/A/a.ts')");
assert.deepEqual(visiblePairs(ancestor),[],'selecting a real leaf narrows the ordinary dependencies');
ancestor.run("revealNode('P')");
assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts','P/Parent.ts → B'],'selecting the ancestor again restores its reference');

// A report may start below the selected package without an expansion trail.
// The initial depth must not make the ancestor impossible to select.
const initialDetail=launch({...ancestorFixture,view:{depth:2,visible:null}});
const initialDetailAnalysis=initialDetail.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])');
const initialDetailPositions=initialDetail.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))');
const initialDetailZoom=initialDetail.graph.zoom,initialDetailUpdates=initialDetail.graph.updates;
initialDetail.run("revealNode('P')");
assert.equal(initialDetail.run('ui.selection?.name'),'P','an ancestor of the initial cut remains selectable without trail entries');
assert.equal(initialDetail.run('ui.selection.kind'),'node');
assert.equal(initialDetail.ids.get('sidebar').querySelectorAll('h2')[0].textContent,'P');
assert.equal(initialDetail.ids.get('module-list').querySelectorAll('button[data-node]').find(b=>b.dataset.node==='0').getAttribute('aria-pressed'),'true');
mode(initialDetail,'incoming');assert.deepEqual(visiblePairs(initialDetail),['B/b.ts → P/Parent.ts']);
mode(initialDetail,'outgoing');assert.deepEqual(visiblePairs(initialDetail),['P/Parent.ts → B/b.ts']);
mode(initialDetail,'both');assert.deepEqual(visiblePairs(initialDetail),['B/b.ts → P/Parent.ts','P/Parent.ts → B/b.ts']);
assert.equal(initialDetail.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),initialDetailAnalysis);
assert.equal(initialDetail.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))'),initialDetailPositions);
assert.equal(initialDetail.graph.zoom,initialDetailZoom);
assert.equal(initialDetail.graph.updates,initialDetailUpdates);

// An ancestor selection includes descendants beyond a deliberately focused
// cycle and therefore must leave that isolation instead of remaining trapped.
const ancestorFocus=launch({...ancestorFixture,view:{depth:2,visible:null},findings:[finding('scc-file',['P/Parent.ts','B/b.ts'])]});
ancestorFocus.run('focusCycle(cyclesForView()[0])');mode(ancestorFocus,'both');
assert.deepEqual(names(ancestorFocus),['B/b.ts','P/Parent.ts']);
for(const clearSelection of [()=>ancestorFocus.graph.onBackgroundClick(),()=>ancestorFocus.document.events.keydown[0]({key:'Escape'})]){
  ancestorFocus.run('focusCycle(cyclesForView()[0])');
  clearSelection();
  assert.notEqual(ancestorFocus.run('ui.cycle'),null,'clearing item selection keeps explicit cycle focus');
  assert.equal(ancestorFocus.run('ui.edgeMode'),'both','clearing selection preserves the chosen direction mode');
  assert.deepEqual(visiblePairs(ancestorFocus),['B/b.ts → P/Parent.ts','P/Parent.ts → B/b.ts'],'cycle focus remains the direction reference after background/Escape');
  assert.match(ancestorFocus.ids.get('edge-reference').textContent,/Focused cycle/);
  assert.ok(ancestorFocus.ids.get('edge-modes').querySelectorAll('button').every(b=>!b.disabled));
}
ancestorFocus.run("selectNode('P')");
assert.equal(ancestorFocus.run('ui.selection?.name'),'P');
assert.equal(ancestorFocus.run('ui.cycle'),null,'an ancestor with descendants outside the focused cycle exits cycle isolation');
assert.equal(ancestorFocus.run('isolated'),null);
assert.ok(names(ancestorFocus).includes('P/A/a.ts'));
assert.deepEqual(visiblePairs(ancestorFocus),['B/b.ts → P/Parent.ts','P/Parent.ts → B/b.ts']);

// Keep the same ancestor reference while its represented members and edges
// change through simulation; never keep dense IDs from a previous graph.
ancestor.click('simulation-mode');
ancestor.run("recordOp(encodeCut('P/Parent.ts','B/b.ts'))");
assert.equal(ancestor.run('ui.selection?.name'),'P');
assert.equal(ancestor.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges==='outgoing').textContent,'Outgoing (0)','a simulation updates the count even before that direction is selected');
mode(ancestor,'incoming');assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts']);
mode(ancestor,'outgoing');assert.deepEqual(visiblePairs(ancestor),[]);
mode(ancestor,'both');assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts']);
ancestor.click('undo-ops');
assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts','P/Parent.ts → B']);
ancestor.run("recordOp(encodeMove('P/Parent.ts','B'))");
assert.equal(ancestor.run('ui.selection?.name'),'P','moving a child out does not discard its still-represented parent');
assert.deepEqual(visiblePairs(ancestor),[],'moved children cease to belong to the selected ancestor scope');
ancestor.click('undo-ops');
assert.deepEqual(visiblePairs(ancestor),['B → P/Parent.ts','P/Parent.ts → B']);
assert.equal(ancestor.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),ancestorAnalysis,'undo restores the original model and ancestor reference');
ancestor.run("recordOp(encodeMove('P','B'))");
assert.notEqual(ancestor.run('ui.selection?.name'),'P','a renamed ancestor cannot leave the obsolete name selected');
assert.equal(ancestor.run('ui.selection===null||!!nodeByName(work,ui.selection.name)'),true,'any surviving selection names a current node');

// Without a reference, All remains usable and a direction may be chosen first:
// it stays pending and cannot hide the displayed overview connections.
const noReference=launch(ancestorFixture);
assert.equal(noReference.ids.get('edge-reference').textContent,'Select a package or file for incoming/outgoing dependencies.');
for(const button of noReference.ids.get('edge-modes').querySelectorAll('button')){
  assert.equal(button.disabled,false,'direction buttons remain usable before selecting a reference');
}
mode(noReference,'all');
assert.deepEqual(visiblePairs(noReference),['B → P','P → B']);
for(const value of ['incoming','outgoing','both']){
  mode(noReference,value);
  assert.equal(noReference.run('ui.edgeMode'),value,'a direction can be chosen before its reference');
  assert.equal(noReference.ids.get('edge-reference').textContent,'Select a package or file to apply the chosen direction.');
  assert.deepEqual(visiblePairs(noReference),['B → P','P → B'],'a pending direction preserves the overview connections');
}
noReference.run("selectNode('P')");
assert.match(noReference.ids.get('edge-reference').textContent,/Reference: P/);
assert.ok(noReference.ids.get('edge-modes').querySelectorAll('button').every(b=>!b.disabled));
mode(noReference,'incoming');assert.deepEqual(visiblePairs(noReference),['B → P']);
mode(noReference,'all');noReference.graph.onBackgroundClick();
assert.equal(noReference.ids.get('edge-reference').textContent,'Select a package or file for incoming/outgoing dependencies.');
mode(noReference,'both');assert.equal(noReference.run('ui.edgeMode'),'both');
assert.deepEqual(visiblePairs(noReference),['B → P','P → B'],'a pending direction after clearing the selection preserves the overview connections');

// Undo/Clear deliberately leave cycle isolation. They must also discard its
// selection, even when the restored graph has a cycle with the same members.
for(const action of ['undo-ops','clear-ops']){
  const restored=launch(fixture);openInner(restored);restored.click('simulation-mode');
  restored.run("recordOp(encodeCut('root/A/inner/Start.ts','root/A/inner/End.ts'));focusCycle(cyclesForView().find(c=>c.origin==='current'&&c.nodes.includes('root/A/inner/Start.ts')))");
  assert.equal(restored.run('ui.selection.kind'),'cycle');
  restored.click(action);
  assert.equal(restored.run('ops.length'),0);
  assert.equal(restored.run('ui.cycle'),null);
  assert.notEqual(restored.run('ui.selection?.kind'),'cycle','leaving focus through '+action+' discards the stale cycle selection');
  assert.equal(restored.run('ui.edgeMode'),'both');
  for(const expected of mainCycle)assert.ok(visiblePairs(restored).includes(expected),action+' restores the package reference and complete cycle path');
  assert.match(restored.ids.get('edge-reference').textContent,/Reference: root\/A\/inner/);
  assert.ok(restored.ids.get('edge-modes').querySelectorAll('button').every(b=>!b.disabled));
}

console.log('package cycle context: complete external paths, sibling/parent files, coarse-cycle honesty, filters, focus, simulation, navigation and exact evidence passed');
