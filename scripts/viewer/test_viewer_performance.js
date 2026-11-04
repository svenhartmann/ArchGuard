// Operation-count regressions: preserve behavior without machine-dependent timing gates.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='module')=>({id,name,parent,kind,lang:'ts'});
const fixture={root:'/tmp/performance',nodes:[node(0,'A'),node(1,'A/a.ts',0,'file'),node(2,'B'),node(3,'B/b.ts',2,'file')],edges:[{from:1,to:3,count:1,refs:[{file:'a.ts',line:2,raw:'import B'}]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const app=launch(fixture,{library:false});
app.run(`var treeRenders=0,movesBuilt=0;var originalTreeRender=renderPackageTree,originalMoveSuggestions=packageMoveSuggestions;renderPackageTree=function(){treeRenders++;return originalTreeRender();};packageMoveSuggestions=function(...args){movesBuilt++;return originalMoveSuggestions(...args);};var previousWork=work;var baselineBuilds=0,originalBaseline=baseline;baseline=function(){baselineBuilds++;return originalBaseline();};`);
app.run('renderAll(false)');
assert.equal(app.run('treeRenders'),1,'one UI update must build the visible tree once, not again through the summary');
app.run('setAllTreeBranches(true)');
assert.equal(app.run('work===previousWork'),true,'navigation reuses an unchanged working graph');
assert.equal(app.run('baselineBuilds'),0,'without simulation the current scope is also the baseline; do not clone/reaggregate it');
assert.equal(app.run('movesBuilt'),0,'navigation does not regenerate move suggestions for unchanged source topology');
assert.deepEqual(JSON.parse(app.run('JSON.stringify(view.nodes.map(n=>n.name))')),['A/a.ts','B/b.ts']);
const before=app.run('JSON.stringify([data,work,ops])');
app.run("showTab('calls');treeRenders=0;renderAll(false)");
assert.equal(app.run('treeRenders'),0,'hidden graph must not rebuild on Calls updates');
app.run("ui.simulation=true;recordOp(encodeCut('A/a.ts','B/b.ts'))");
assert.equal(app.run('movesBuilt'),0,'topology changes invalidate move suggestions without computing unused candidates');
assert.equal(app.run('moveSuggestions'),null);
assert.equal(app.run('view.edges.length'),0);
app.run("showTab('graph')");
assert.equal(app.run('treeRenders'),1,'returning to Graph renders the latest simulated model');
assert.equal(app.ids.get('package-tree').querySelectorAll('path[data-tree-relation]').length,0);
app.click('undo-ops');
assert.equal(app.run('movesBuilt'),0,'undo leaves unused move candidates deferred');
assert.equal(app.run('moveSuggestions'),null);
assert.equal(app.run('JSON.stringify([data,work,ops])'),before,'undo restores original evidence and model');
app.run('var previousWork=work;recompute()');
assert.equal(app.run('work===previousWork'),true);
assert.equal(app.run('movesBuilt'),0);
app.run('resetView();treeRenders=0;toggleTreeBranch(0,true)');
assert.equal(app.run('treeRenders'),1,'inline branch navigation renders only its final scope');
app.run('treeRenders=0;navigateExplorerNode(2,true)');
assert.equal(app.run('treeRenders'),1,'Explorer navigation does not rebuild the old graph before the requested scope');
// The start cut looks up every node's depth. Per-node ancestor and child
// scans over the whole node list made opening a large report quadratic.
app.run(`var lookups=0,originalNodeById=nodeById,originalChildrenOf=childrenOf;nodeById=function(...a){lookups++;return originalNodeById(...a);};childrenOf=function(...a){lookups++;return originalChildrenOf(...a);};
var deep={nodes:[]};for(let i=0;i<400;i++)deep.nodes.push({id:i,name:'n'+i,parent:i<4?null:i<40?i%4:i<400?4+i%36:null});deep.nodes.push({id:500,name:'orphan',parent:999},{id:501,name:'loop-a',parent:502},{id:502,name:'loop-b',parent:501});`);
const depths=[0,1,2,3].map(d=>JSON.parse(app.run(`JSON.stringify([...atDepth(deep,${d})].sort((a,b)=>a-b))`)));
assert.equal(app.run('lookups'),0,'depth selection indexes nodes once instead of scanning per node');
const expectedDepth=d=>{const byId=new Map(),parents=new Set();const g=JSON.parse(app.run('JSON.stringify(deep)'));for(const n of g.nodes){byId.set(n.id,n);parents.add(n.parent);}
  return g.nodes.filter(n=>{let depth=0,cur=n.parent;const seen=new Set();while(cur!=null&&!seen.has(cur)){seen.add(cur);depth++;cur=byId.get(cur)?.parent;}return depth===d||depth<d&&!parents.has(n.id);}).map(n=>n.id).sort((a,b)=>a-b);};
depths.forEach((ids,d)=>assert.deepEqual(ids,expectedDepth(d),'depth '+d+' keeps leaves above it, unknown parents and parent loops'));
app.run('nodeById=originalNodeById;childrenOf=originalChildrenOf');
console.log('Viewer work reuse: one visible tree render, no hidden tree rebuilds, navigation cache and simulation/undo invalidation passed');
