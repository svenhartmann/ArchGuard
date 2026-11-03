// Breadcrumb clicks use the real embedded application and the shared DOM/
// ForceGraph doubles. These assertions cover navigation, not visual acceptance.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='module')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to)=>({from,to,count:1,refs:[{file:'source.ts',line:1,raw:'import target'}]});
const fixture={root:'/tmp/breadcrumb-project',nodes:[
  node(0,'app'),node(1,'app/domain',0),node(2,'app/domain/deep',1),node(3,'app/domain/deep/Entity.ts',2,'file'),
  node(4,'app/domain/Model.ts',1,'file'),node(5,'app/Service.ts',0,'file'),
  node(6,'other'),node(7,'other/Hidden.ts',6,'file'),node(8,'loose.ts',null,'file')
],edges:[edge(5,3),edge(3,7)],view:{depth:0,visible:null},cyclicEdges:[],findings:[]};
const treeNode=(app,id)=>app.ids.get('module-list').querySelectorAll('button[data-node]').find(button=>button.dataset.node===String(id));
const treeToggle=(app,id)=>app.ids.get('module-list').querySelectorAll('button[data-toggle]').find(button=>button.dataset.toggle===String(id));
const crumbs=app=>Array.from(app.ids.get('breadcrumbs').querySelectorAll('button'));
const crumb=(app,id)=>crumbs(app).find(button=>button.dataset.node===String(id));
const names=app=>Array.from(app.graph.data.nodes,node=>node.name).sort();
function expectPath(app,ids,labels){
  const buttons=crumbs(app);
  assert.deepEqual(buttons.map(button=>button.textContent),['breadcrumb-project',...labels],'breadcrumb uses compact names along the actual source hierarchy');
  assert.deepEqual(buttons.slice(1).map(button=>Number(button.dataset.node)),ids,'breadcrumb identity uses stable source IDs');
  for(const button of buttons.slice(1)){
    const fullName=app.run('nodeById(work,'+Number(button.dataset.node)+').name');
    assert.ok(button.title.includes(fullName),'breadcrumb hover exposes the full path');
    assert.ok(button.getAttribute('aria-label').includes(fullName),'breadcrumb accessible label exposes the full path');
  }
  assert.equal(buttons.at(-1).getAttribute('aria-current'),'location','the final path item identifies the current location');
  assert.ok(buttons.slice(0,-1).every(button=>button.getAttribute('aria-current')===null),'only the current location has aria-current');
}
const app=launch(fixture);
for(const id of [0,1,2,3])treeNode(app,id).click();
expectPath(app,[0,1,2,3],['app','domain','deep','Entity.ts']);
const oldAppCrumb=crumb(app,0);
crumbs(app).find(button=>button.textContent==='app').click();
assert.deepEqual(names(app),['app/Service.ts','app/domain','other'],'an ancestor breadcrumb opens that package, instead of collapsing it to a card');
assert.equal(app.run('ui.selection.name'),'app','breadcrumb navigation selects the same package as the tree');
assert.equal(treeToggle(app,0).getAttribute('aria-expanded'),'true');
assert.equal(treeToggle(app,1).getAttribute('aria-expanded'),'false','ancestor navigation collapses the deeper Explorer path');
assert.equal(treeNode(app,2),undefined);
assert.equal(app.run('explorerOpen.has(2)'),false,'hidden descendants do not retain the abandoned breadcrumb expansion');
assert.notEqual(crumb(app,0),oldAppCrumb);
assert.equal(app.document.activeElement,crumb(app,0),'breadcrumb navigation focuses the newly rendered ancestor crumb');
expectPath(app,[0],['app']);
treeNode(app,1).click();treeNode(app,2).click();treeNode(app,3).click();
crumb(app,1).click();
assert.deepEqual(names(app),['app/Service.ts','app/domain/Model.ts','app/domain/deep','other']);
assert.equal(app.run('ui.selection.name'),'app/domain');
assert.equal(treeToggle(app,1).getAttribute('aria-expanded'),'true');
assert.equal(treeToggle(app,2).getAttribute('aria-expanded'),'false');
expectPath(app,[0,1],['app','domain']);

// A selected collapsed package still has a location. Its breadcrumb is a
// path, even though the package itself is not an expansion-trail entry.
treeToggle(app,1).click();
assert.equal(app.run('ui.selection.name'),'app/domain');
assert.ok(names(app).includes('app/domain'));
expectPath(app,[0,1],['app','domain']);
crumb(app,1).click();
assert.ok(names(app).includes('app/domain/deep'),'clicking a collapsed package breadcrumb opens its contents');
assert.equal(treeToggle(app,1).getAttribute('aria-expanded'),'true');

// Clicking an already current package or file changes neither graph data nor
// camera. Focus returns to its fresh breadcrumb instead of a detached button.
for(const selectedIds of [[0,1],[0,1,2,3]]){
  const current=launch(fixture);
  for(const id of selectedIds)treeNode(current,id).click();
  current.graph.zoom=.37;
  const updates=current.graph.updates,centers=current.graph.centers.length;
  const positions=current.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))');
  const selected=selectedIds.at(-1),previous=crumb(current,selected);
  previous.click();
  assert.equal(current.graph.updates,updates,'repeated current breadcrumb navigation does not rebuild graph data');
  assert.equal(current.graph.zoom,.37,'repeated current breadcrumb navigation preserves zoom');
  assert.equal(current.graph.centers.length,centers,'repeated current breadcrumb navigation preserves the camera');
  assert.equal(current.run('JSON.stringify(renderedNodes.map(n=>[n.src,n.x,n.y,n.fx,n.fy]))'),positions);
  assert.equal(current.document.activeElement,crumb(current,selected));
}

// Initial detail reports have no expansion history. Source parent IDs still
// provide a complete path when a file is selected directly in the graph.
for(const reportView of [{depth:4,visible:null},{depth:3,visible:[2,4,5,6,8]}]){
  const detailed=launch({...fixture,view:reportView}),initial=names(detailed);
  if(reportView.visible)detailed.graph.onNodeClick(detailed.graph.data.nodes.find(node=>node.name==='app/domain/deep'));
  else detailed.graph.onNodeClick(detailed.graph.data.nodes.find(node=>node.name==='app/domain/deep/Entity.ts'));
  assert.equal(detailed.run('trail.length'),0,'selection does not invent expansion history');
  expectPath(detailed,reportView.visible?[0,1,2]:[0,1,2,3],reportView.visible?['app','domain','deep']:['app','domain','deep','Entity.ts']);
  crumb(detailed,0).click();
  assert.deepEqual(names(detailed),['app/Service.ts','app/domain','other'],'an ancestor above the original report depth is navigable');
  detailed.run("showTab('dsm')");
  crumbs(detailed)[0].click();
  assert.deepEqual(names(detailed),initial,'project breadcrumb restores the original report cut');
  assert.equal(detailed.run('ui.selection'),null);
  assert.equal(detailed.run('ui.workspace'),'graph');
  assert.equal(detailed.run('explorerOpen.size'),0,'project navigation resets the Explorer expansion');
  expectPath(detailed,[],[]);
  assert.equal(detailed.document.activeElement,crumbs(detailed)[0],'project breadcrumb receives focus after reset');
}

// Breadcrumb labels and ancestry follow the simulated source IDs through a
// package move, and return to the original names and parents after undo.
const moved=launch(fixture);
moved.click('simulation-mode');moved.run("recordOp(encodeMove('app/domain','other'))");
assert.equal(moved.run('ops.length'),1);
for(const id of [6,1,2,3])treeNode(moved,id).click();
expectPath(moved,[6,1,2,3],['other','domain','deep','Entity.ts']);
assert.equal(crumb(moved,1).title,'other/domain');
crumb(moved,6).click();
assert.deepEqual(names(moved),['app','other/Hidden.ts','other/domain']);
assert.equal(moved.run('ui.selection.name'),'other');
moved.click('undo-ops');
for(const id of [0,1,2,3])treeNode(moved,id).click();
expectPath(moved,[0,1,2,3],['app','domain','deep','Entity.ts']);
assert.equal(crumb(moved,1).title,'app/domain');
assert.equal(moved.run('ops.length'),0);

// Changing breadcrumb scope preserves source evidence, pending operations
// and user display filters, including a return to the project overview.
const preserved=launch(fixture);
preserved.click('simulation-mode');preserved.run("recordOp(encodeCut('app/Service.ts','app/domain/deep/Entity.ts'))");
for(const id of [0,1,2,3])treeNode(preserved,id).click();
preserved.ids.get('edge-modes').querySelectorAll('button').find(button=>button.dataset.edges==='incoming').click();
preserved.click('toggle-cycles');
const originalModel=preserved.run('JSON.stringify([data,work,ops])');
crumb(preserved,0).click();
assert.equal(preserved.run('ui.edgeMode'),'incoming');
assert.equal(preserved.run('ui.cyclesOnly'),true);
assert.equal(preserved.run('ui.selection.name'),'app');
assert.equal(preserved.run('JSON.stringify([data,work,ops])'),originalModel,'ancestor navigation preserves source evidence and the working simulation');
crumbs(preserved)[0].click();
assert.equal(preserved.run('ui.edgeMode'),'incoming');
assert.equal(preserved.run('ui.cyclesOnly'),true);
assert.equal(preserved.run('JSON.stringify([data,work,ops])'),originalModel,'project navigation preserves source evidence and pending operations');
assert.equal(preserved.run('window.archguardCanReload()'),false);
console.log('Viewer breadcrumb navigation tests passed');
