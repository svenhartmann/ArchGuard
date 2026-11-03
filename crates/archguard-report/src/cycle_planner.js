// Evidence-backed dependency-removal plans for a selected cycle. These are
// assumptions to implement in source, not executable interface refactorings.
// Reuse the viewer's SCC detector; never infer success from an SCC's ordinal ID.
function cycleRepairStats(graph) {
  const components=findSccs(graph),members=new Set(components.flatMap(c=>c.members));
  return {groups:components.length,cyclicNodes:members.size,
    cyclicEdges:components.reduce((sum,c)=>sum+internalEdges(graph,c).length,0),
    largestScc:components.reduce((size,c)=>Math.max(size,c.members.length),0)};
}

function planCycleRepairs(graph, memberNames, options={}) {
  const limit=Math.max(1,Math.min(8192,Math.floor(options.maxExamined||2048)));
  const maxPlans=Math.max(1,Math.min(3,Math.floor(options.maxPlans||3)));
  const scopeNames=[...new Set(memberNames)].sort(compareNames);
  const empty={groups:0,cyclicNodes:0,cyclicEdges:0,largestScc:0};
  const result={scopeNames,before:{...empty},sourceBefore:cycleRepairStats(graph),
    plans:[],rankingLowerBound:false,search:{complete:false,examined:0,limit},error:null};
  const byName=new Map(graph.nodes.map(n=>[n.name,n])),byId=new Map(graph.nodes.map(n=>[n.id,n]));
  const children=new Map();
  for(const n of graph.nodes){if(!children.has(n.parent))children.set(n.parent,[]);children.get(n.parent).push(n.id);}
  const owner=new Map();
  for(let index=0;index<scopeNames.length;index++){
    const node=byName.get(scopeNames[index]);
    if(!node){result.error='Cycle member not found: '+scopeNames[index];return result;}
    const pending=[node.id],seen=new Set();
    while(pending.length){
      const id=pending.pop();if(seen.has(id))continue;seen.add(id);
      if(owner.has(id)){result.error='Cycle member scopes overlap: '+scopeNames[index];return result;}
      owner.set(id,index);pending.push(...(children.get(id)||[]));
    }
  }
  const bundles=new Map();
  for(const edge of graph.edges){
    const from=owner.get(edge.from),to=owner.get(edge.to);
    if(from===undefined||to===undefined||from===to)continue;
    const key=from+' '+to;
    if(!bundles.has(key))bundles.set(key,{from,to,sourceEdges:[]});
    bundles.get(key).sourceEdges.push(edge);
  }
  const edges=[...bundles.values()].sort((a,b)=>a.from-b.from||a.to-b.to);
  const scoped={nodes:scopeNames.map((name,id)=>({id,name})),edges};
  result.before=cycleRepairStats(scoped);
  if(!result.before.cyclicNodes){result.search.complete=true;return result;}
  const sourceFiles=edge=>edge.refs.length?[...new Set(edge.refs.map(r=>r.file))]:[byId.get(edge.from).name];
  for(const edge of edges){
    edge.files=[...new Set(edge.sourceEdges.flatMap(sourceFiles))].sort(compareNames);
    edge.references=edge.sourceEdges.reduce((sum,e)=>sum+edgeWeight(e),0);
    edge.lowerBound=edge.sourceEdges.some(e=>e.count==null);
  }
  const cyclicPairs=new Set(findSccs(scoped).flatMap(c=>internalEdges(scoped,c).map(([a,b])=>a+' '+b)));
  result.rankingLowerBound=edges.some(e=>cyclicPairs.has(e.from+' '+e.to)&&e.lowerBound);

  // Every feedback-edge set must remove an edge from each observed cycle.
  // Branch on one deterministic witness, rather than enumerate arbitrary
  // subsets. Greedy seeds provide complete alternatives before bounded search.
  function witness(remaining) {
    const component=findSccs({nodes:scoped.nodes,edges:remaining})[0];
    if(!component)return [];
    const ids=new Set(component.members),first=remaining.find(e=>ids.has(e.from)&&ids.has(e.to));
    const queue=[first.to],previous=new Map([[first.to,null]]);
    for(let i=0;i<queue.length&&!previous.has(first.from);i++){
      for(const edge of remaining)if(edge.from===queue[i]&&ids.has(edge.to)&&!previous.has(edge.to)){
        previous.set(edge.to,edge);queue.push(edge.to);
      }
    }
    const path=[first];let cursor=first.from;
    while(cursor!==first.to){const edge=previous.get(cursor);path.push(edge);cursor=edge.from;}
    return path.map(e=>edges.indexOf(e));
  }
  function stateFor(cuts) {
    cuts=cuts.slice().sort((a,b)=>a-b);
    return {cuts,key:cuts.join(','),files:[...new Set(cuts.flatMap(i=>edges[i].files))].sort(compareNames),
      references:cuts.reduce((sum,i)=>sum+edges[i].references,0)};
  }
  const compareCost=(a,b)=>a.files.length-b.files.length||a.references-b.references||a.cuts.length-b.cuts.length||compareNames(a.key,b.key);
  const evaluated=new Map(),solutions=new Map();
  function evaluate(state) {
    if(evaluated.has(state.key))return evaluated.get(state.key);
    if(result.search.examined>=limit)return null;
    result.search.examined++;
    const cut=new Set(state.cuts),remaining=edges.filter((_edge,index)=>!cut.has(index));
    state.after=cycleRepairStats({nodes:scoped.nodes,edges:remaining});
    state.witness=state.after.cyclicNodes?witness(remaining):[];
    evaluated.set(state.key,state);
    if(!state.after.cyclicNodes&&state.cuts.length)solutions.set(state.key,state);
    return state;
  }
  const initial=evaluate(stateFor([]));
  const firstBranches=initial.witness.map(index=>stateFor([index])).sort(compareCost);
  for(const seed of firstBranches.slice(0,maxPlans)){
    let current=evaluate(seed);
    while(current?.witness.length){
      const next=current.witness.map(index=>stateFor([...current.cuts,index])).sort(compareCost)[0];
      current=evaluate(next);
    }
    if(!current)break;
    // Greedy orders can contain redundant cuts after other cuts are chosen.
    // Remove them only when another exact SCC pass proves sufficiency.
    for(const index of current.cuts.slice().reverse()){
      const smaller=evaluate(stateFor(current.cuts.filter(i=>i!==index)));
      if(!smaller)break;
      if(!smaller.after.cyclicNodes){solutions.delete(current.key);current=smaller;}
    }
  }
  const queue=[initial],queued=new Set([initial.key]);let truncated=false;
  while(queue.length){
    queue.sort(compareCost);
    const next=queue.shift(),current=evaluate(next);
    if(!current){truncated=true;break;}
    if(!current.witness.length)continue;
    for(const index of current.witness){
      const candidate=stateFor([...current.cuts,index]);
      if(!queued.has(candidate.key)){queued.add(candidate.key);queue.push(candidate);}
    }
    if(queue.length>limit*4){queue.sort(compareCost);queue.length=limit*4;truncated=true;}
  }
  result.search.complete=!truncated&&!queue.length;
  // Supersets do not offer a smaller intervention than an already found plan.
  const complete=[...solutions.values()].filter(p=>![...solutions.values()].some(other=>other.cuts.length<p.cuts.length&&other.cuts.every(i=>p.cuts.includes(i)))).sort(compareCost);
  const partial=[...evaluated.values()].filter(p=>p.cuts.length&&p.after.cyclicEdges<result.before.cyclicEdges)
    .sort((a,b)=>a.after.cyclicNodes-b.after.cyclicNodes||a.after.cyclicEdges-b.after.cyclicEdges||compareCost(a,b));
  for(const state of (complete.length?complete:partial).slice(0,maxPlans)){
    const removed=new Set(state.cuts.flatMap(i=>edges[i].sourceEdges));
    const evidence=[...new Map([...removed].flatMap(e=>e.refs).map(r=>[JSON.stringify([r.file,r.line,r.raw]),{file:r.file,line:r.line,raw:r.raw}])).values()]
      .sort((a,b)=>compareNames(a.file,b.file)||a.line-b.line||compareNames(a.raw,b.raw));
    result.plans.push({key:state.key,ops:state.cuts.map(i=>({cut_edge:{from:scopeNames[edges[i].from],to:scopeNames[edges[i].to]}})),
      complete:state.after.cyclicNodes===0,after:state.after,
      sourceAfter:cycleRepairStats({nodes:graph.nodes,edges:graph.edges.filter(e=>!removed.has(e))}),
      affectedFiles:state.files,referenceCount:state.references,
      referenceLowerBound:state.cuts.some(i=>edges[i].lowerBound),
      evidence:evidence.slice(0,30),evidenceTruncated:evidence.length>30||[...removed].some(e=>e.count==null||e.count>e.refs.length),
      removedEdges:removed.size});
  }
  return result;
}
