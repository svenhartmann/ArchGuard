// Cycles-only is an additional graph display filter, not an analysis scope.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to,count=1)=>({from,to,count,refs:[{file:'source-'+from+'.ts',line:from+1,raw:'import target_'+to+';'}]});
const report=(nodes,edges)=>({root:'/tmp/cycles-only',nodes,edges,findings:[],cyclicEdges:[],view:{depth:0,visible:null}});
const fixture=report(['A','B','C','D','E','F'].map((name,id)=>node(id,name)),[
  edge(0,1,25),edge(1,0,2),edge(2,3,5),edge(3,2,7),
  edge(0,2,11),edge(4,0,13),edge(0,5,17),
]);
const pair=l=>l.source.name+' → '+l.target.name;
const links=app=>Array.from(app.graph.data.links).filter(app.graph.linkVisibility);
const pairs=app=>links(app).map(pair).sort();
const modeButton=(app,value)=>app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===value);
const mode=(app,value)=>modeButton(app,value).click();
const analysis=app=>app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])');
const positions=app=>app.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))');
const app=launch(fixture);
const toggle=app.document.getElementById('toggle-cycles');
assert.equal(toggle.getAttribute('aria-pressed'),'false','cycle filtering is opt-in');
app.run("selectNode('A')");
assert.equal(pairs(app).length,5,'ordinary Both includes cyclic and acyclic connections');
const before=analysis(app),layout=positions(app),updates=app.graph.updates,zoom=app.graph.zoom;
toggle.click();
assert.equal(toggle.getAttribute('aria-pressed'),'true');
assert.equal(app.run('ui.edgeMode'),'both','the toggle preserves the direction selection');
const cyclicPairs=['A → B','B → A','C → D','D → C'];
for(const [value,expected] of [
  ['incoming',['B → A']],['outgoing',['A → B']],['both',['A → B','B → A']],['all',cyclicPairs],
]){
  assert.equal(Number(modeButton(app,value).textContent.match(/\((\d+)\)$/)[1]),expected.length,'direction count includes cycles-only filtering: '+value);
  mode(app,value);assert.deepEqual(pairs(app),expected,'cycles-only intersects '+value);
  assert.match(app.ids.get('graph-visibility').textContent,new RegExp(expected.length+' / 7 dependencies shown'));
}
assert.equal(pairs(app).includes('A → C'),false,'a bridge between two cyclic groups is not a cycle edge');
assert.equal(analysis(app),before,'filtering preserves graph evidence, SCCs, metrics and simulation');
assert.equal(positions(app),layout,'filtering preserves every node position');
assert.equal(app.graph.updates,updates,'filtering does not rebuild graph data');
assert.equal(app.graph.zoom,zoom,'filtering preserves zoom');
assert.equal(links(app).find(l=>pair(l)==='A → B').count,25,'reference totals remain exact');
toggle.click();assert.equal(toggle.getAttribute('aria-pressed'),'false');
assert.equal(pairs(app).length,7,'disabling the filter restores ordinary All');
toggle.click();app.run("selectNode('E')");mode(app,'outgoing');
assert.deepEqual(pairs(app),[],'a reference with no cyclic outgoing edge has an empty result');
assert.match(app.ids.get('graph-hint').textContent,/cycl/i,'the empty result explains the active cycle filter');

// A surviving edge from a broken cycle may be marked resolved, but must no
// longer pass the current cycles-only filter. Undo restores the full cycle.
mode(app,'all');app.click('simulation-mode');
app.run("recordOp(encodeCut('B','A'))");
assert.deepEqual(pairs(app),['C → D','D → C']);
assert.equal(toggle.getAttribute('aria-pressed'),'true');
app.click('undo-ops');assert.deepEqual(pairs(app),cyclicPairs);
app.run("showTab('dsm');showTab('graph');resetView()");
assert.equal(toggle.getAttribute('aria-pressed'),'true','the display preference survives navigation/reset');
assert.deepEqual(pairs(app),cyclicPairs);

// Hidden cyclic file endpoints are counted after applying the cycle filter;
// a noncyclic edge to another hidden file must not inflate that hint.
const hidden=launch(report([
  node(0,'A',null,'module'),node(1,'A/a.ts',0),node(2,'B',null,'module'),node(3,'B/b.ts',2),
  node(4,'Hidden.ts'),node(5,'Loose.ts'),
],[edge(1,3),edge(3,1),edge(1,4),edge(4,1),edge(1,5)]));
hidden.click('toggle-cycles');
assert.deepEqual(pairs(hidden),['A → B','B → A']);
assert.match(hidden.ids.get('graph-hint').textContent,/2 related dependencies are hidden by the file filter/);
hidden.run("selectNode('A')");
assert.deepEqual(pairs(hidden),['A → B','A → Hidden.ts','B → A','Hidden.ts → A'],'selection reveals related cyclic file endpoints');
hidden.click('toggle-files');
assert.deepEqual(pairs(hidden),['A → B','A → Hidden.ts','B → A','Hidden.ts → A']);
assert.doesNotMatch(hidden.ids.get('graph-hint').textContent,/related dependencies are hidden/);

// Package-level cycles can disappear at file resolution. Preserve their
// explicitly labelled context, without relabelling fine edges or metrics.
const coarse=launch(report([
  node(0,'A',null,'module'),node(1,'A/Start.ts',0),node(2,'A/End.ts',0),
  node(3,'B',null,'module'),node(4,'B/External.ts',3),
  node(5,'C',null,'module'),node(6,'C/Loose.ts',5),
],[edge(1,4),edge(4,2),edge(1,6)]));
coarse.run("expandNode('A')");
assert.equal(coarse.run('sccs.length'),0);
assert.equal(coarse.run('cyclicKeys.size'),0);
const coarseBefore=analysis(coarse);
coarse.click('toggle-cycles');
assert.deepEqual(pairs(coarse),['A/Start.ts → B','B → A/End.ts'],'cycles-only retains package-level return paths');
assert.match(coarse.ids.get('graph-hint').textContent,/package-level/i);
assert.equal(analysis(coarse),coarseBefore,'context does not invent file-level cycles');
coarse.run("focusCycle(cyclesForView().find(c=>c.origin==='package'))");
assert.deepEqual(pairs(coarse),['A/Start.ts → B','B → A/End.ts'],'explicit package-cycle focus survives cycle filtering');
assert.equal(coarse.run('cyclicKeys.size'),0);
coarse.click('cycles-reset');mode(coarse,'incoming');
assert.deepEqual(pairs(coarse),['B → A/End.ts']);
mode(coarse,'outgoing');assert.deepEqual(pairs(coarse),['A/Start.ts → B']);
console.log('cycles-only: direction intersections, counts, stable layout, simulation/undo, hidden files and package-cycle context passed');
