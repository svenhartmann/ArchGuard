// Coarse package history matters even when another branch opened first.
// Skip impossible context cuts before aggregation, without dropping that history.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[
  {id:0,name:'A',kind:'module',parent:null},
  {id:1,name:'A/Start.ts',kind:'file',parent:0},
  {id:2,name:'A/End.ts',kind:'file',parent:0},
  {id:3,name:'B',kind:'module',parent:null},
  {id:4,name:'B/Start.ts',kind:'file',parent:3},
  {id:5,name:'B/End.ts',kind:'file',parent:3},
  {id:6,name:'C',kind:'module',parent:null},
  {id:7,name:'C/Leaf.ts',kind:'file',parent:6},
];
const fixture={root:'/tmp/context-scaling',nodes,edges:[{from:1,to:5,count:2,refs:[{file:'Start.ts',line:1,raw:'import B.End'}]},{from:4,to:2,count:3,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
for(const order of [[3,0,6],[0,3,6]]){
  const app=launch(fixture,{library:false});
  for(const id of order)app.run('toggleTreeBranch('+id+',true)');
  app.run("selectNode('A')");
  assert.equal(app.run('sccs.length'),0,'the detailed files have no return path');
  assert.equal(app.run('packageContext.edges.size'),2,'both real dependencies of the coarse A/B cycle stay visible regardless of opening order');
  assert.equal(app.run('packageContext.levels.length'),1);
  const before=app.run('JSON.stringify([work,view,packageContext.levels,[...packageContext.edges]])');
  app.run('globalThis.contextAggregations=0;globalThis.contextAggregate=aggregate;aggregate=function(...args){contextAggregations++;return contextAggregate(...args)};');
  app.run("selectNode('A')");
  assert.equal(app.run('contextAggregations'),order[0]===3?2:1,'cuts without A or one of its ancestors cannot contribute context and are skipped before aggregation');
  assert.equal(app.run('JSON.stringify([work,view,packageContext.levels,[...packageContext.edges]])'),before,'the context optimization preserves analysis, evidence and package-cycle paths');
}
console.log('Package context scaling: irrelevant cuts skipped, sibling-before-ancestor history and exact dependency evidence preserved.');
