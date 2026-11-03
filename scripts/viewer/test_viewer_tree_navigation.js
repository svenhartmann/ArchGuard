// Explorer controls and their graph scope are exercised together against the
// embedded application. This is a state test, not visual browser acceptance.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='module')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to)=>({from,to,count:1,refs:[{file:'source.ts',line:1,raw:'import target'}]});
const fixture={root:'/tmp/tree-navigation',nodes:[
  node(0,'app'),node(1,'app/domain',0),node(2,'app/domain/deep',1),node(3,'app/domain/deep/Entity.ts',2,'file'),
  node(4,'app/domain/Model.ts',1,'file'),node(5,'app/Service.ts',0,'file'),
  node(6,'other'),node(7,'other/Hidden.ts',6,'file'),node(8,'loose.ts',null,'file')
],edges:[edge(5,3),edge(3,7)],view:{depth:0,visible:null},cyclicEdges:[],findings:[]};
const treeNodes=app=>app.ids.get('module-list').querySelectorAll('button[data-node]');
const treeNode=(app,id)=>treeNodes(app).find(button=>button.dataset.node===String(id));
const treeToggle=(app,id)=>app.ids.get('module-list').querySelectorAll('button[data-toggle]').find(button=>button.dataset.toggle===String(id));
const names=app=>Array.from(app.graph.data.nodes,node=>node.name).sort();
const app=launch(fixture);
treeToggle(app,0).click();
assert.deepEqual(names(app),['app/Service.ts','app/domain','other'],'expanding the Explorer package updates the graph and exposes its direct files');
assert.equal(app.run('ui.selection.name'),'app','the expanded package remains the dependency reference');
assert.equal(app.run('ui.showFiles'),false,'explicit navigation does not enable every project file');
assert.equal(app.document.activeElement,treeToggle(app,0),'expansion focuses the fresh disclosure control');
treeNode(app,1).click();
assert.deepEqual(names(app),['app/Service.ts','app/domain/Model.ts','app/domain/deep','other'],'clicking a package row opens its contents');
assert.equal(app.run('ui.selection.name'),'app/domain');
assert.equal(treeToggle(app,1).getAttribute('aria-expanded'),'true');
treeNode(app,2).click();
assert.deepEqual(names(app),['app/Service.ts','app/domain/Model.ts','app/domain/deep/Entity.ts','other']);
treeNode(app,3).click();
assert.equal(app.run('ui.selection.name'),'app/domain/deep/Entity.ts','file rows select the displayed file');
assert.ok(names(app).includes('app/domain/deep/Entity.ts'));
treeToggle(app,1).click();
assert.deepEqual(names(app),['app/Service.ts','app/domain','other'],'collapsing a package restores its card and removes its detailed descendants');
assert.equal(app.run('ui.selection.name'),'app/domain');
treeToggle(app,0).click();
assert.deepEqual(names(app),['app','other'],'collapsing the root returns the package overview');
assert.equal(app.run('ui.selection.name'),'app');
treeNode(app,6).click();
assert.deepEqual(names(app),['app','other/Hidden.ts'],'switching branches shows that branch instead of retaining the previous drill-down');
treeNode(app,8).click();
assert.deepEqual(names(app),['app','loose.ts','other'],'selecting a filtered root file reveals it without opening unrelated packages');
assert.equal(app.run('ui.selection.name'),'loose.ts');

// Navigation derives a fresh path from source parent IDs. Reports initially
// opened below a package must still be able to navigate to that package.
for(const reportView of [{depth:2,visible:null},{depth:4,visible:null},{depth:4,visible:[2,4,5,6,8]}]){
  const detailed=launch({...fixture,view:reportView});
  const initialNames=names(detailed),original=detailed.run('JSON.stringify(data)');
  treeNode(detailed,0).click();
  assert.deepEqual(names(detailed),['app/Service.ts','app/domain','other'],'an ancestor above the initial cut opens its direct contents');
  treeNode(detailed,1).click();
  assert.deepEqual(names(detailed),['app/Service.ts','app/domain/Model.ts','app/domain/deep','other']);
  treeToggle(detailed,0).click();
  assert.deepEqual(names(detailed),['app','other'],'an ancestor above the report cut can collapse back to its card');
  detailed.click('reset-view');
  assert.deepEqual(names(detailed),initialNames,'Reset restores the original report depth or explicit cut');
  assert.equal(detailed.run('JSON.stringify(data)'),original,'navigation never changes the report payload');
}

// The keyboard follows the same navigation as clicks, preserving branch
// expansion on vertical movement and focusing controls from the rebuilt tree.
const keyboard=launch(fixture);
treeNode(keyboard,0).dispatch('keydown',{key:'ArrowRight'});
assert.deepEqual(names(keyboard),['app/Service.ts','app/domain','other']);
assert.equal(keyboard.document.activeElement,treeNode(keyboard,0));
treeNode(keyboard,0).dispatch('keydown',{key:'ArrowRight'});
assert.equal(keyboard.run('ui.selection.name'),'app/domain');
assert.equal(keyboard.document.activeElement,treeNode(keyboard,1));
assert.equal(treeToggle(keyboard,1).getAttribute('aria-expanded'),'false','moving onto a closed child preserves its collapsed state');
treeNode(keyboard,1).dispatch('keydown',{key:'ArrowRight'});
assert.deepEqual(names(keyboard),['app/Service.ts','app/domain/Model.ts','app/domain/deep','other']);
treeNode(keyboard,1).dispatch('keydown',{key:'ArrowDown'});
assert.equal(keyboard.run('ui.selection.name'),'app/domain/deep');
assert.equal(keyboard.document.activeElement,treeNode(keyboard,2));
assert.equal(treeToggle(keyboard,2).getAttribute('aria-expanded'),'false');
treeNode(keyboard,2).dispatch('keydown',{key:'ArrowDown'});
assert.equal(keyboard.run('ui.selection.name'),'app/domain/Model.ts');
assert.equal(keyboard.document.activeElement,treeNode(keyboard,4));
treeNode(keyboard,4).dispatch('keydown',{key:'ArrowUp'});
assert.equal(keyboard.run('ui.selection.name'),'app/domain/deep');
treeNode(keyboard,2).dispatch('keydown',{key:'End'});
assert.equal(keyboard.run('ui.selection.name'),'loose.ts');
assert.deepEqual(names(keyboard),['app','loose.ts','other']);
assert.equal(keyboard.document.activeElement,treeNode(keyboard,8));
treeNode(keyboard,8).dispatch('keydown',{key:'Home'});
assert.equal(keyboard.run('ui.selection.name'),'app');
assert.ok(names(keyboard).includes('app/domain'),'moving to an expanded package retains its open contents');
assert.equal(keyboard.document.activeElement,treeNode(keyboard,0));
treeNode(keyboard,0).dispatch('keydown',{key:'ArrowLeft'});
assert.ok(names(keyboard).includes('app'));
assert.equal(names(keyboard).includes('app/domain'),false);
assert.equal(treeToggle(keyboard,0).getAttribute('aria-expanded'),'false');
treeNode(keyboard,0).dispatch('keydown',{key:'ArrowRight'});
treeNode(keyboard,5).dispatch('keydown',{key:'ArrowLeft'});
assert.equal(keyboard.run('ui.selection.name'),'app','Left on a file navigates to its parent package');
assert.equal(keyboard.document.activeElement,treeNode(keyboard,0));

// A repeated selection refreshes the reference without resetting a user's
// camera or rebuilding graph data. The active workspace follows navigation.
const stable=launch(fixture);
treeNode(stable,0).click();
stable.graph.zoom=.42;
const stableUpdates=stable.graph.updates,stableCenters=stable.graph.centers.length;
const stablePositions=stable.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))');
treeNode(stable,0).click();
assert.equal(stable.graph.updates,stableUpdates,'reselecting the same opened package does not rebuild the graph');
assert.equal(stable.graph.zoom,.42,'reselecting the same opened package preserves zoom');
assert.equal(stable.graph.centers.length,stableCenters,'reselecting the same opened package preserves the camera position');
assert.equal(stable.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))'),stablePositions);
for(const tab of ['dsm','calls']){
  stable.run('showTab('+JSON.stringify(tab)+')');
  treeNode(stable,0).click();
  assert.equal(stable.run('ui.workspace'),'graph','Explorer navigation activates Graph from '+tab);
  assert.equal(stable.ids.get('tab-graph').classList.contains('panel-hidden'),false);
  assert.equal(stable.graph.updates,stableUpdates,'returning to an unchanged graph does not rebuild it');
}

// Search reveals matching paths temporarily. Typing alone does not navigate;
// selecting its deep result reveals that file through every real ancestor.
const search=launch(fixture),searchInitial=names(search);
search.ids.get('filter').value='Entity.ts';search.run('applySearch()');
assert.deepEqual(names(search),searchInitial,'typing a search only filters the Explorer');
treeNode(search,3).click();
assert.equal(search.run('ui.selection.name'),'app/domain/deep/Entity.ts');
assert.ok(names(search).includes('app/domain/deep/Entity.ts'),'a search result hidden behind collapsed branches is visible after selection');
assert.equal(search.run('ui.showFiles'),false);
search.click('clear-search');
assert.ok(treeNode(search,3),'selected file keeps its ancestor path open after clearing search');

// A stale focused cycle cannot trap navigation, while the genuine external
// return path must remain visible after opening its package and descendants.
const cyclic=launch({...fixture,edges:[edge(3,7),edge(7,8),edge(8,3)],findings:[
  {id:'scc-path',kind:'cycle',severity:'error',nodes:['app','other','loose.ts'],edge_count:3,suggested_cuts:[]}
]});
cyclic.run('focusCycle(cyclesForView()[0])');
assert.notEqual(cyclic.run('ui.cycle'),null);
treeNode(cyclic,0).click();
assert.equal(cyclic.run('ui.cycle'),null,'Explorer navigation clears explicit cycle isolation');
assert.equal(cyclic.run('isolated'),null);
treeNode(cyclic,1).click();treeNode(cyclic,2).click();
const shownPairs=Array.from(cyclic.graph.data.links).filter(cyclic.graph.linkVisibility).map(link=>link.source.name+' → '+link.target.name).sort();
assert.deepEqual(shownPairs,['app/domain/deep/Entity.ts → other','loose.ts → app/domain/deep/Entity.ts','other → loose.ts'],'the complete outside cycle, including its outside-to-outside hop, survives Explorer navigation');

// Navigation only changes view state, including when a simulated dependency
// removal is pending. Source evidence and operations survive branch changes.
const simulated=launch(fixture);
simulated.click('simulation-mode');simulated.run("recordOp(encodeCut('app/Service.ts','app/domain/deep/Entity.ts'))");
assert.equal(simulated.run('ops.length'),1);
const simulationModel=simulated.run('JSON.stringify([data.nodes,data.edges,work,ops])');
treeNode(simulated,0).click();treeNode(simulated,1).click();treeNode(simulated,2).click();
treeToggle(simulated,0).click();treeNode(simulated,6).click();
assert.equal(simulated.run('JSON.stringify([data.nodes,data.edges,work,ops])'),simulationModel,'tree navigation preserves the working graph and pending simulation');
assert.equal(simulated.run('window.archguardCanReload()'),false);

// The navigation path also works before deferred graph initialization, and
// keeps the Explorer usable when the optional drawing library is unavailable.
const deferred=launch(fixture,{deferGraphData:true});
treeNode(deferred,0).click();treeNode(deferred,1).click();
deferred.graph.flushLayout();
assert.deepEqual(names(deferred),['app/Service.ts','app/domain/Model.ts','app/domain/deep','other']);
assert.ok(deferred.graph.data.nodes.every(node=>Number.isFinite(node.x)&&Number.isFinite(node.y)));
assert.ok(Number.isFinite(deferred.graph.zoom)&&deferred.graph.zoom>0,'the opened scope receives a finite fitted zoom');
assert.ok(deferred.graph.centers.at(-1).every(Number.isFinite),'the fitted camera uses finite coordinates');
const missing=launch(fixture,{library:false});
missing.run("showTab('dsm')");treeNode(missing,0).click();
assert.equal(missing.run('ui.workspace'),'graph');
assert.equal(missing.run('ui.selection.name'),'app');
assert.deepEqual(JSON.parse(missing.run('JSON.stringify(view.nodes.map(n=>n.name).sort())')),['app/Service.ts','app/domain','loose.ts','other']);
assert.equal(missing.ids.get('graph-state').textContent,'','offline tree navigation does not claim an unused library failed');

// On a narrow screen the Explorer is deliberately opened as an overlay. Tree
// navigation must keep it usable instead of opening the Inspector over it.
const compact=launch(fixture,{width:768});
compact.click('toggle-explorer');
assert.equal(compact.ids.get('main').classList.contains('explorer-closed'),false);
const previousRow=treeNode(compact,0);
previousRow.click();
assert.notEqual(treeNode(compact,0),previousRow,'navigation replaces the rendered Explorer controls');
assert.equal(compact.document.activeElement,treeNode(compact,0),'compact navigation focuses the rebuilt row');
assert.equal(compact.ids.get('main').classList.contains('explorer-closed'),false,'compact navigation keeps the Explorer open');
assert.equal(compact.ids.get('toggle-explorer').getAttribute('aria-expanded'),'true');
assert.equal(compact.ids.get('main').classList.contains('inspector-closed'),true,'compact navigation leaves room for the active Explorer');
assert.deepEqual(names(compact),['app/Service.ts','app/domain','other']);
treeNode(compact,0).dispatch('keydown',{key:'ArrowDown'});
assert.equal(compact.document.activeElement,treeNode(compact,1));
assert.equal(compact.ids.get('main').classList.contains('explorer-closed'),false);
treeToggle(compact,0).click();
assert.equal(compact.document.activeElement,treeToggle(compact,0));
assert.equal(compact.ids.get('main').classList.contains('explorer-closed'),false);

// Direction and cycle filters survive row navigation and collapsing; their
// actual links must follow the newly selected package, not a stale reference.
const filterFixture={...fixture,edges:[edge(3,7),edge(7,3),edge(4,5)],findings:[
  {id:'scc-filter',kind:'cycle',severity:'error',nodes:['app','other'],edge_count:2,suggested_cuts:[]}
]};
for(const mode of ['incoming','outgoing','both','all']){
  const filtered=launch(filterFixture);
  treeNode(filtered,0).click();
  filtered.ids.get('edge-modes').querySelectorAll('button').find(button=>button.dataset.edges===mode).click();
  filtered.click('toggle-cycles');
  function expectReference(reference,endpoint){
    assert.equal(filtered.run('ui.edgeMode'),mode,'navigation preserves '+mode);
    assert.equal(filtered.run('ui.cyclesOnly'),true,'navigation preserves Cycles only');
    assert.equal(filtered.run('edgeReference().name'),reference,'the filter reference follows the current tree selection');
    assert.equal(filtered.run('ui.selection.name'),reference);
    const pairs=Array.from(filtered.graph.data.links).filter(filtered.graph.linkVisibility).map(link=>link.source.name+' → '+link.target.name).sort();
    const expected=[];
    if(mode!=='outgoing')expected.push('other → '+endpoint);
    if(mode!=='incoming')expected.push(endpoint+' → other');
    assert.deepEqual(pairs,expected.sort(),'direction and cycle filtering uses the selected package contents');
  }
  expectReference('app','app/domain');
  treeNode(filtered,1).click();
  expectReference('app/domain','app/domain/deep');
  treeToggle(filtered,1).click();
  expectReference('app/domain','app/domain');
  treeToggle(filtered,0).click();
  expectReference('app','app');
}

// Returning to roots through the Explorer leaves an empty expansion trail,
// but its scope differs from a deep report. Cycle lists and baseline metrics
// must both use that root cut instead of the report's original file findings.
const detailedCycles={...fixture,view:{depth:4,visible:null},edges:[edge(3,7),edge(7,3),edge(4,5),edge(5,4)],findings:[
  {id:'file-external',kind:'cycle',severity:'error',nodes:['app/domain/deep/Entity.ts','other/Hidden.ts'],edge_count:2,suggested_cuts:[]},
  {id:'file-internal',kind:'cycle',severity:'error',nodes:['app/domain/Model.ts','app/Service.ts'],edge_count:2,suggested_cuts:[]}
]};
const scopedCycles=launch(detailedCycles);
assert.equal(scopedCycles.run('cyclesForView().length'),2);
treeNode(scopedCycles,0).click();
assert.equal(scopedCycles.run('currentAcd'),2.5,'opening app yields the four-node cut with a three-node SCC');
assert.equal(scopedCycles.run('baselineStats.acd'),2.5,'baseline uses the same opened-package cut');
assert.equal(scopedCycles.run('baselineStats.cycles'),1);
treeToggle(scopedCycles,0).click();
assert.equal(scopedCycles.run('trail.length'),0);
assert.deepEqual(JSON.parse(scopedCycles.run('JSON.stringify(cyclesForView().map(c=>c.nodes.slice().sort()))')),[['app','other']],'root navigation derives its cycle rather than reusing stale file findings');
assert.equal(scopedCycles.run('currentAcd'),5/3,'the root cut has three nodes, two of them mutually reachable');
assert.equal(scopedCycles.run('baselineStats.acd'),5/3);
assert.equal(scopedCycles.run('baselineStats.cycles'),1);
scopedCycles.click('simulation-mode');
scopedCycles.run("recordOp(encodeCut('app/domain/deep/Entity.ts','other/Hidden.ts'))");
assert.equal(scopedCycles.run('currentAcd'),4/3,'the simulated root cut loses one direction');
assert.equal(scopedCycles.run('sccs.length'),0);
assert.equal(scopedCycles.run('baselineStats.acd'),5/3,'the original root cut remains the simulation baseline');
assert.equal(scopedCycles.run('baselineStats.cycles'),1);
assert.equal(scopedCycles.run('resolvedKeys.size'),1,'the remaining root edge is recognized as a resolved cycle edge');
assert.equal(scopedCycles.run('introducedKeys.size'),0);
console.log('Viewer Explorer navigation tests passed');
