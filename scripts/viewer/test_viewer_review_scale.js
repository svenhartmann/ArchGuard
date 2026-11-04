// Review priorities on large views: identical results to per-node traversal,
// without quadratic set work. Counts operations, not machine-dependent time.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('crates/archguard-report/src/viewer_review.js','utf8');
let adds=0;
class CountingSet extends Set{add(value){adds++;return super.add(value);}}
const compute=vm.runInNewContext(source+'\ncomputeReview;',{Set:CountingSet});
// Reference semantics of review.rs: transitive dependencies/dependents per
// node (excluding itself) and the size of its cyclic group.
function reference(view){
  const out=new Map(view.nodes.map(n=>[n.id,new Set()])),inc=new Map(view.nodes.map(n=>[n.id,new Set()]));
  for(const e of view.edges)if(out.has(e.from)&&out.has(e.to)){out.get(e.from).add(e.to);inc.get(e.to).add(e.from);}
  const reach=(id,edges)=>{const seen=new Set([id]),todo=[id];for(let i=0;i<todo.length;i++)for(const next of edges.get(todo[i]))if(!seen.has(next)){seen.add(next);todo.push(next);}seen.delete(id);return seen;};
  return new Map(view.nodes.map(n=>{const down=reach(n.id,out),up=reach(n.id,inc),mutual=[...down].filter(id=>up.has(id)).length;return [n.id,{dependencies:down.size,dependents:up.size,scc_size:mutual?mutual+1:out.get(n.id).has(n.id)?1:0}];}));
}
let seed=7;const random=()=>(seed=(seed*1103515245+12345)%2147483648)/2147483648;
for(let round=0;round<40;round++){
  const count=2+Math.floor(random()*40),nodes=Array.from({length:count},(_,id)=>({id,src:id,name:'n'+id,kind:'file',parent:null}));
  const edges=Array.from({length:Math.floor(random()*count*3)},()=>({from:Math.floor(random()*count),to:Math.floor(random()*count)}));
  if(round%5===0)edges.push({from:999,to:0});
  const view={nodes,edges},expected=reference(view),review=compute({nodes,edges},view,{});
  for(const row of review.nodes)assert.deepEqual({dependencies:row.dependencies,dependents:row.dependents,scc_size:row.scc_size},expected.get(row.node_id),'round '+round+' node '+row.node_id);
}
// One large cycle feeding a long chain: per-node traversal adds every
// reachable node for every node (millions of set insertions).
const size=3000,nodes=Array.from({length:size},(_,id)=>({id,src:id,name:'n'+id,kind:'file',parent:null}));
const edges=[...Array.from({length:size-1},(_,i)=>({from:i,to:i+1})),{from:1499,to:0}];
adds=0;const review=compute({nodes,edges},{nodes,edges},{});
assert.equal(review.nodes.find(r=>r.node_id===0).scc_size,1500);assert.equal(review.nodes.find(r=>r.node_id===0).dependencies,size-1);assert.equal(review.nodes.find(r=>r.node_id===2999).dependents,size-1);
assert.ok(adds<40*size,`review set insertions stay near linear: ${adds} for ${size} nodes`);
console.log(`Review scale: 40 random graphs match per-node traversal; ${adds} set insertions for ${size} nodes`);
