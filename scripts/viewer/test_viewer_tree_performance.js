// Dense trees retain their complete model while mounting only the visible scene.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const geometry=vm.createContext({});vm.runInContext(fs.readFileSync('crates/archguard-report/src/viewer_tree_layout.js','utf8'),geometry);
const intervals=Array.from({length:2500},(_,i)=>({start:(i*37)%5000,end:(i*37)%5000+(i*97)%1800})).sort((a,b)=>a.start-b.start||a.end-b.end),ends=[];
const expected=intervals.map(edge=>{let lane=ends.findIndex(end=>end+2<edge.start);if(lane<0)lane=ends.length;ends[lane]=edge.end;return lane;});
assert.equal(geometry.packageTreeAssignLanes(intervals),ends.length);assert.deepEqual(intervals.map(e=>e.lane),expected,'heap allocation preserves exact lowest-free-lane geometry');

const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to)=>({from,to,count:1,refs:[{file:'source.ts',line:7,raw:'import target'}]});
// A mixed cut contains visible packages and hidden direct files. Selecting a
// package must reveal its file endpoints, otherwise both directions look empty.
const mixed=launch({root:'/mixed',nodes:[node(0,'pkg',null,'module'),node(1,'pkg/a.ts',0),node(2,'in.ts'),node(3,'out.ts')],edges:[edge(2,1),edge(1,3)],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
mixed.run("selectNode('pkg')");
for(const direction of ['incoming','outgoing']){
 mixed.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===direction).click();
 assert.equal(mixed.ids.get('package-tree').querySelectorAll('g[data-tree-edge]').length,1,'selected package dependencies must include hidden file endpoints in '+direction);
}
const count=700,nodes=Array.from({length:count},(_,i)=>node(i,'pkg/File-'+String(i).padStart(4,'0')+'.ts',count));nodes.push(node(count,'pkg',null,'module'));
const edges=Array.from({length:count*8},(_,i)=>edge(i%count,((i%count)+1+Math.floor(i/count)*37)%count));
const app=launch({root:'/dense',nodes,edges,findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false}),tree=app.ids.get('package-tree');
app.run('setAllTreeBranches(true)');
const source=app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])');
const rows=()=>tree.querySelectorAll('g[data-tree-source]'),svg=()=>tree.querySelector('svg');
assert.equal(svg().getAttribute('data-tree-windowed'),'true');assert.ok(rows().length<100,'only viewport rows and overscan are mounted');
assert.equal(Number(svg().getAttribute('data-tree-edge-count')),edges.length,'all relations remain in the graph');
assert.equal(app.run('exportSnapshot().edges.length'),edges.length,'JSON exports retain offscreen connections');
const firstRows=rows().map(n=>n.getAttribute('data-tree-source')).join();tree.scrollTop=9000;app.run('renderPackageTree()');
assert.ok(rows().length<100);assert.notEqual(rows().map(n=>n.getAttribute('data-tree-source')).join(),firstRows,'scrolling mounts new rows');
app.run('var layoutBuilds=0,originalPackageLayout=packageTreeLayout;packageTreeLayout=function(...args){layoutBuilds++;return originalPackageLayout(...args);};');
app.run("selectNode('pkg/File-0350.ts')");
tree.scrollTop=0;app.run("renderPackageTree();selectNode('pkg/File-0350.ts')");assert.ok(tree.scrollTop>0,'explicit reselection reveals an offscreen selected row');
for(const direction of ['incoming','outgoing','both','all'])app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===direction).click();
const clipped=app.run('treeClippedPath([{x:0,y:0},{x:10,y:0},{x:100,y:0},{x:100,y:100},{x:10,y:100},{x:0,y:100}],25,75)');
assert.match(clipped,/^M .* C /,'windowed routes retain their cubic shape');
const coords=clipped.match(/-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/g).map(Number);
assert.ok(Math.abs(coords[1]-25)<1e-6&&Math.abs(coords.at(-1)-75)<1e-6,'transit geometry is clipped to viewport bounds');
assert.equal(app.run('treeClippedPath([{x:0,y:0},{x:10,y:0},{x:100,y:0},{x:100,y:100},{x:10,y:100},{x:0,y:100}],125,175)').trim(),'');
assert.equal(app.run('layoutBuilds'),0,'selection and direction filters reuse geometry');
assert.equal(Number(svg().getAttribute('data-tree-edge-count')),edges.length);
const scroll=tree.scrollTop;
const full=JSON.parse(app.run('JSON.stringify(withFullPackageTree(()=>({rows:document.getElementById("package-tree").querySelectorAll("g[data-tree-source]").length,edges:document.getElementById("package-tree").querySelectorAll("g[data-tree-edge]").length})))'));
assert.equal(full.rows,count+2);assert.equal(full.edges,edges.length,'full SVG export keeps every relationship');
assert.equal(tree.scrollTop,scroll);assert.equal(svg().getAttribute('data-tree-windowed'),'true');assert.ok(rows().length<100);
assert.equal(app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),source,'windowing never changes graph evidence or metrics');
// Opening unrelated sibling packages cannot change the selected package's SCC.
// Keep the historical A/B return path without rebuilding every sibling cut.
const historyNodes=[node(0,'A',null,'module'),node(1,'A/start.ts',0),node(2,'A/end.ts',0),node(3,'B',null,'module'),node(4,'B/start.ts',3),node(5,'B/end.ts',3)];
for(let i=0;i<160;i++){historyNodes.push(node(6+i*2,'Other'+i,null,'module'),node(7+i*2,'Other'+i+'/file.ts',6+i*2));}
const history=launch({root:'/history',nodes:historyNodes,edges:[edge(1,5),edge(4,2)],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
history.run('trail=[...work.nodes.filter(n=>n.kind==="module"&&n.name!=="A").map(n=>n.id),0];ui.selection=null;treeLocation="project";recompute();var contextBuilds=0,originalAggregate=aggregate;aggregate=function(...args){contextBuilds++;return originalAggregate(...args);};selectNode("A");');
assert.equal(history.run('packageContext.edges.size'),2);assert.equal(history.run('packageContext.levels.length'),1);
assert.ok(history.run('contextBuilds')<=2,'unrelated sibling expansions do not rebuild unchanged package-cycle context');
console.log('Dense tree: bounded viewport rows, geometry reuse, complete exports and visible selected-file endpoints passed');
