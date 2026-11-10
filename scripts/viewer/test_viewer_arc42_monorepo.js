const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
// Two services wrap the same package chain (de/x), a third root is a plain directory (ADR 0010).
const nodes=[],edges=[];let id=0;
const add=(name,parent,kind='module')=>{nodes.push({id,name,kind,parent,lang:kind==='file'?'java':undefined});return id++;};
for(const service of ['svc-a','svc-b']){
  const root=add(service,null),de=add(service+'/de',root),x=add(service+'/de/x',de);
  const client=add(service+'/de/x/client',x),model=add(service+'/de/x/model',x);
  const a=add(service+'/de/x/client/C.java',client,'file'),b=add(service+'/de/x/model/M.java',model,'file');
  edges.push({from:a,to:b,count:1,refs:[]});
}
const tools=add('tools',null),one=add('tools/one',tools),two=add('tools/two',tools);add('tools/one/a.py',one,'file');add('tools/two/b.py',two,'file');
const fixture={root:'/repo',nodes,edges,view:{depth:0,visible:null},findings:[],cyclicEdges:[],insights:{arc42:{chapters:[]}},services:{services:[],dependencies:[],projects:[],cycles:[],complete:true,diagnostics:[]},calls:{status:'ready',classes:[],calls:[]}};
const app=launch(fixture,{library:false});
const overview=JSON.parse(app.run('JSON.stringify(arc42PackageProjection())'));
assert.deepEqual(overview.allNodes.map(n=>n.name).sort(),['svc-a','svc-b','tools'],'level 1: one block per top-level directory of a monorepo');
const a=JSON.parse(app.run("JSON.stringify(arc42PackageProjection('0'))"));
assert.deepEqual(a.allNodes.map(n=>n.label).sort(),['client','model'],'level 2: the service packages below the shared wrapper chain');
const levels=JSON.parse(app.run("JSON.stringify([...arc42BuildingLevels('0').entries.values()].map(e=>[e.node.name,e.level]))"));
assert.ok(levels.some(([n,l])=>n==='svc-a/de/x/client'&&l===3),'level 3 skips the wrapper chain too');
console.log('arc42 monorepo: services as level-1 blocks, wrapper chains skipped below them');
