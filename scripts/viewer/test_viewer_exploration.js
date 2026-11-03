// Compact exploration: labels and relation endpoints are separate but share rows.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/tmp/exploration',nodes:[{id:0,name:'app',kind:'module',parent:null},{id:1,name:'app/a.ts',kind:'file',parent:0},{id:2,name:'domain',kind:'module',parent:null},{id:3,name:'domain/b.ts',kind:'file',parent:2}],edges:[{from:1,to:3,count:4,refs:[{file:'app/a.ts',line:1,raw:'import B'}]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const app=launch(fixture,{library:false}),tree=app.ids.get('package-tree');
assert.equal(app.run('treeRouting'),'direction','incoming-left/outgoing-right is the default');
const toolbarButtons=Array.from(tree.querySelector('div[data-tree-toolbar]').querySelectorAll('button'));
assert.deepEqual(toolbarButtons.map(b=>b.getAttribute('data-tree-control')),
  ['zoom-out','zoom-in','zoom-reset','zoom-fit','all-expand','all-collapse','sort-alphabetical','sort-connections','routing','reset'],
  'the tree toolbar contains only the supported navigation and arrangement controls');
assert.deepEqual(toolbarButtons.filter(b=>b.getAttribute('data-tree-control')!=='zoom-reset').map(b=>b.textContent),
  ['−','+','⛶','Expand all','Collapse all','Alphabetical','By dependencies','Incoming / Outgoing','Reset tree']);
assert.equal(toolbarButtons.find(b=>b.getAttribute('data-tree-control')==='zoom-fit').getAttribute('aria-label'),'Fit the entire tree in the viewport','the icon-only Fit button keeps a spoken name');
assert.match(toolbarButtons.find(b=>b.getAttribute('data-tree-control')==='zoom-reset').textContent,/^\d+%$/);
const directionToggle=()=>tree.querySelectorAll('button[data-tree-routing]').find(b=>b.getAttribute('data-tree-routing')==='direction');
assert.equal(directionToggle().getAttribute('aria-pressed'),'true','In / Out is active by default');
const nodes=()=>tree.querySelectorAll('g[data-package-node]');
const mirrors=()=>tree.querySelectorAll('g[data-tree-relation]');
assert.equal(mirrors().length,nodes().length*2,'each analytical row has a port on both sides of the tree');
const position=el=>el.getAttribute('transform').match(/translate\(([-\d.]+)[ ,]+([-\d.]+)\)/).slice(1).map(Number);
const axis=mirrors().map(m=>position(m)[0]);assert.equal(new Set(axis).size,2,'relation ports align on two vertical axes');
for(const n of nodes()){
 const p=position(n),shape=n.querySelector('rect[data-tree-row]');
 assert.ok(Number(shape.getAttribute('height'))<=30,'rows stay compact');
 for(const side of ['left','right']){
  const mirror=mirrors().find(m=>m.getAttribute('data-tree-relation')===n.getAttribute('data-tree-source')&&m.getAttribute('data-tree-port-side')===side);
  assert.equal(position(mirror)[1],p[1]);const x=position(mirror)[0],half=Number(shape.getAttribute('width'))/2;
  assert.ok(side==='left'?x<p[0]-half-16:x>p[0]+half+16,'ports sit on their assigned side of the tree');
 }
}
const paths=()=>tree.querySelectorAll('g[data-tree-edge]');
assert.equal(paths().length,1);assert.equal(paths()[0].getAttribute('data-tree-side'),'right','compact dependencies stay beside the tree');
assert.match(paths()[0].querySelector('path').getAttribute('d'),/^M \S+ \S+ L \S+ \S+ C \S+ \S+ \S+ \S+ \S+ \S+ L \S+ \S+$/,'dependencies follow one smooth cubic arc between horizontal endpoint leads');
const before=app.run('JSON.stringify([work,view,ops,currentAcd])');
mirrors()[0].click();assert.equal(app.run('ui.selection.name'),'app','mirrored endpoint selects its tree source');
const positions=JSON.stringify(nodes().map(position));
app.run('setPackageTreeZoom(1.25)');
directionToggle().click();assert.equal(paths()[0].getAttribute('data-tree-side'),'right','optional In/Out retains user-selected outgoing-right convention');
assert.equal(directionToggle().getAttribute('aria-pressed'),'false');
assert.equal(app.document.activeElement,directionToggle(),'keyboard focus follows the rebuilt toggle');
directionToggle().click();
assert.equal(directionToggle().getAttribute('aria-pressed'),'true');
assert.equal(paths()[0].getAttribute('data-tree-side'),'right','a second click restores incoming-left/outgoing-right');
assert.equal(JSON.stringify(nodes().map(position)),positions,'routing changes never move the tree');
assert.equal(app.run('treeZoom'),1.25,'toggling preserves zoom');
assert.equal(app.run('ui.selection.name'),'app','toggling preserves selection');
assert.equal(app.document.activeElement,directionToggle());
const toggle=tree.querySelectorAll('g[data-tree-toggle]').find(g=>g.getAttribute('data-tree-toggle')==='0');
assert.equal(toggle.getAttribute('aria-expanded'),'false');
toggle.click();assert.ok(nodes().some(n=>n.getAttribute('data-package-node')==='app/a.ts'),'disclosure expands inline');
const collapse=tree.querySelectorAll('g[data-tree-toggle]').find(g=>g.getAttribute('data-tree-toggle')==='0');assert.equal(collapse.getAttribute('aria-expanded'),'true');collapse.click();
assert.ok(nodes().some(n=>n.getAttribute('data-package-node')==='app'));
assert.equal(app.run('JSON.stringify([work,view,ops,currentAcd])'),before,'opening and closing a row restores aggregation without changing source analysis');
// An aggregated branch cycle may exist without a cycle between the visible files.
const branch=launch({...fixture,nodes:[...fixture.nodes,{id:4,name:'app/end.ts',kind:'file',parent:0}],edges:[...fixture.edges,{from:3,to:4,count:1,refs:[]}]},{library:false});
branch.run('toggleTreeBranch(0,true)');
assert.equal(branch.run('sccs.length'),0,'the visible files form no fine-grained SCC');
const parent=branch.ids.get('package-tree').querySelectorAll('g[data-tree-source]').find(n=>n.getAttribute('data-tree-source')==='0');
assert.equal(parent.getAttribute('data-tree-branch-cycle'),'true','expanded parent retains its aggregate sibling-cycle decoration');
assert.match(parent.querySelector('title').textContent,/cycle between sibling branches/i);
console.log('Exploration tree: compact rows, aligned relation ports, angular tracks, sorting controls, In/Out, selection and inline disclosure passed');
