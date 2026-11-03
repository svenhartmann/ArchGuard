// Candidate search executes the same source included by the standalone viewer.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const plannerPath='crates/archguard-report/src/cycle_planner.js';
assert.ok(fs.existsSync(plannerPath),'cycle repair planner implementation is missing');
const viewer=fs.readFileSync('crates/archguard-report/src/viewer_scc.js','utf8')+'\n'+fs.readFileSync('crates/archguard-report/src/viewer.js','utf8');
function declaration(name){
  const start=viewer.indexOf('function '+name+'('),lineEnd=viewer.indexOf('\n',start);
  assert.ok(start>=0,'existing graph helper '+name);
  return viewer.slice(start,viewer.slice(start,lineEnd).endsWith('}')?lineEnd:viewer.indexOf('\n}',start)+2);
}
const context=vm.createContext({});
for(const name of ['findSccs','internalEdges','compareNames','edgeWeight','nodeByName','nodeById','childrenOf','subtree','applyOp'])vm.runInContext(declaration(name),context);
vm.runInContext(fs.readFileSync(plannerPath,'utf8'),context);
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind});
const edge=(from,to,count=1,file='file-'+from+'.ts')=>({from,to,count,refs:[{file,line:1,raw:'import target_'+to+';'}]});
function plan(graph,names,options){
  context.input=graph;context.names=names;context.options=options;
  return JSON.parse(vm.runInContext('JSON.stringify(planCycleRepairs(input,names,options))',context));
}
const triangle={nodes:['A','B','C','X','Y'].map((name,id)=>node(id,name)),edges:[edge(0,1,5),edge(1,2,1),edge(2,0,3),edge(3,4),edge(4,3)]};
const original=JSON.stringify(triangle),result=plan(triangle,['A','B','C']);
assert.equal(result.error,null);
assert.equal(result.before.cyclicNodes,3);
assert.equal(result.before.cyclicEdges,3);
assert.equal(result.sourceBefore.cyclicNodes,5);
assert.equal(result.plans.length,3,'a triangle offers all three single-dependency alternatives');
assert.deepEqual(result.plans.map(p=>p.ops[0].cut_edge),[{from:'B',to:'C'},{from:'C',to:'A'},{from:'A',to:'B'}]);
for(const p of result.plans){
  assert.equal(p.complete,true);assert.equal(p.after.cyclicNodes,0);
  assert.equal(p.sourceAfter.cyclicNodes,2,'an unrelated source cycle remains intact');
  assert.equal(p.affectedFiles.length,1);
  assert.equal(p.removedEdges,1);
}
assert.equal(JSON.stringify(triangle),original,'search never mutates the source graph');
assert.deepEqual(plan({...triangle,nodes:triangle.nodes.slice().reverse(),edges:triangle.edges.slice().reverse()},['C','A','B']),result,'input ordering does not change ranked plans or search results');

// Two cycles sharing one importer need multiple cuts; source-file count is the
// first cost axis, not a claim about developer hours or removed SCC count.
const twoCycles={nodes:['A','B','C'].map((name,id)=>node(id,name)),edges:[edge(0,1,8,'A.ts'),edge(1,0,1,'B.ts'),edge(0,2,9,'A.ts'),edge(2,0,1,'C.ts')]};
const two=plan(twoCycles,['A','B','C']);
assert.ok(two.plans.length>=2);
assert.deepEqual(two.plans[0].affectedFiles,['A.ts']);
assert.equal(two.plans[0].ops.length,2);
assert.deepEqual(two.plans[0].ops,[{cut_edge:{from:'A',to:'B'}},{cut_edge:{from:'A',to:'C'}}]);
assert.equal(two.plans[0].after.cyclicEdges,0);
assert.ok(two.plans.every(p=>p.complete&&p.ops.length===2),'one cut cannot resolve both remaining loops');

// Package cycle evidence is real, although distinct files have no return path.
const coarse={nodes:[node(0,'P',null,'module'),node(1,'P/Start.ts',0),node(2,'P/End.ts',0),node(3,'Q',null,'module'),node(4,'Q/Other.ts',3)],edges:[edge(1,4,12,'P/Start.ts'),edge(4,2,2,'Q/Other.ts')]};
const coarsePlan=plan(coarse,['P','Q']);
assert.equal(coarsePlan.before.cyclicNodes,2);
assert.equal(coarsePlan.sourceBefore.cyclicNodes,0);
assert.ok(coarsePlan.plans.every(p=>p.complete&&p.after.cyclicNodes===0&&p.sourceAfter.cyclicNodes===0));
assert.equal(coarsePlan.plans[0].referenceCount,2);
assert.deepEqual(coarsePlan.plans[0].affectedFiles,['Q/Other.ts']);
assert.match(plan(coarse,['P','P/Start.ts']).error,/overlap/i);
assert.match(plan(coarse,['Missing','Q']).error,/not found/i);
assert.equal(plan({nodes:triangle.nodes,edges:[edge(0,1)]},['A','B']).plans.length,0);

const unknown=plan({...triangle,edges:triangle.edges.map(e=>({...e,count:null}))},['A','B','C']);
assert.equal(unknown.rankingLowerBound,true);
assert.ok(unknown.plans.every(p=>p.referenceLowerBound));
assert.ok(unknown.plans.every(p=>p.evidence.length===1));
const bounded=plan(twoCycles,['A','B','C'],{maxExamined:1});
assert.equal(bounded.search.complete,false,'search bounds are disclosed instead of claiming a minimum');
assert.ok(bounded.search.examined<=1);
assert.ok(bounded.plans.every(p=>p.complete=== (p.after.cyclicNodes===0)));

// Cross-check an independently exhaustive small graph, including a chord and
// two cycles. The ranking prioritizes distinct source files before references.
const exhaustive={nodes:['A','B','C','D'].map((name,id)=>node(id,name)),edges:[edge(0,1,2),edge(0,2,3),edge(1,2,5),edge(2,0,7),edge(2,3,11),edge(3,1,13)]};
const alternatives=[];
for(let mask=1;mask<1<<exhaustive.edges.length;mask++){
  const cuts=exhaustive.edges.filter((_e,index)=>mask&(1<<index));
  context.probe={nodes:exhaustive.nodes,edges:exhaustive.edges.filter((_e,index)=>!(mask&(1<<index)))};
  if(vm.runInContext('findSccs(probe).length',context))continue;
  alternatives.push({mask,cuts,files:new Set(cuts.flatMap(e=>e.refs.map(r=>r.file))).size,refs:cuts.reduce((sum,e)=>sum+e.count,0),key:exhaustive.edges.map((_e,index)=>index).filter(index=>mask&(1<<index)).join(',')});
}
const minimal=alternatives.filter(a=>!alternatives.some(b=>b.mask!==a.mask&&(a.mask&b.mask)===b.mask));
minimal.sort((a,b)=>a.files-b.files||a.refs-b.refs||a.cuts.length-b.cuts.length||a.key.localeCompare(b.key));
const searched=plan(exhaustive,['A','B','C','D']);
assert.equal(searched.search.complete,true,'small bounded search exhausts all witness branches');
assert.deepEqual(searched.plans.map(p=>p.key),minimal.slice(0,3).map(p=>p.key),'ranking agrees with independent exhaustive cut-set enumeration');

// The projected plan must remove exactly the real source edges its exported
// SimOps remove, including several importing files behind one package arrow.
const bundled={nodes:[...coarse.nodes,node(5,'P/Second.ts',0)],edges:[edge(1,4,100,'P/Start.ts'),edge(5,4,200,'P/Second.ts'),edge(4,2,2,'Q/Other.ts')]};
const bundledResult=plan(bundled,['P','Q']);
const packageCut=bundledResult.plans.find(p=>p.ops[0].cut_edge.from==='P');
assert.equal(packageCut.removedEdges,2);
assert.equal(packageCut.referenceCount,300,'exact totals are not replaced by evidence sample sizes');
assert.equal(packageCut.evidenceTruncated,true);
assert.deepEqual(packageCut.affectedFiles,['P/Second.ts','P/Start.ts']);
for(const [graph,names] of [[triangle,['A','B','C']],[twoCycles,['A','B','C']],[coarse,['P','Q']],[bundled,['P','Q']]]){
  for(const p of plan(graph,names).plans){
    context.probe=JSON.parse(JSON.stringify(graph));context.operations=p.ops;
    vm.runInContext('for(const op of operations)applyOp(probe,op)',context);
    assert.equal(graph.edges.length-context.probe.edges.length,p.removedEdges);
    const sourceStats=JSON.parse(vm.runInContext('JSON.stringify(cycleRepairStats(probe))',context));
    assert.deepEqual(sourceStats,p.sourceAfter,'exported SimOps reproduce the exact reported source-cycle effect');
  }
}
console.log('cycle planner: exact effects, ranked alternatives, multi-cut cycles, coarse/file distinction, bounds and deterministic evidence passed');
