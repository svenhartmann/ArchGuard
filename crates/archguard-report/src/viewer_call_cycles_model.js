'use strict';
// Full compiler snapshot, independent of UI filters. Class-level aggregation can
// form a cycle even when no closed method-call path exists. SCCs describe groups;
// one witness per group avoids enumerating exponentially many elementary cycles.
function computeCallCycles(calls,classes,methods) {
  const classEdges=new Map(),methodEdges=new Map(),siteKeys=new Map();
  let unresolved_targets=0,missing_methods=0;
  const compare=(a,b)=>a<b?-1:a>b?1:0;
  function add(edges,from,to,call) {
    const key=JSON.stringify([from,to]);
    let edge=edges.get(key);
    if(!edge){edge={from,to,calls:[]};edges.set(key,edge);}
    edge.calls.push(call);
  }
  function siteKey(call) {
    if(!siteKeys.has(call))siteKeys.set(call,JSON.stringify(Object.keys(call).sort().map(key=>[key,call[key]])));
    return siteKeys.get(call);
  }
  function compareSites(a,b) {
    return compare(a.file||'',b.file||'')||(a.line||0)-(b.line||0)||(a.column||0)-(b.column||0)||compare(siteKey(a),siteKey(b));
  }
  for(const call of calls) {
    if(call.resolution==='unresolved')unresolved_targets++;
    if(call.resolution!=='project')continue;
    const from=classes.has(call.from_class)?call.from_class:null;
    const to=classes.has(call.to_class)?call.to_class:null;
    const fromMethod=methods.get(call.from_method),toMethod=methods.get(call.to_method);
    const fromId=from!==null&&fromMethod?.class_id===from?call.from_method:null;
    const toId=to!==null&&toMethod?.class_id===to?call.to_method:null;
    if(from!==null&&to!==null&&from!==to)add(classEdges,from,to,call);
    if(fromId!==null&&toId!==null)add(methodEdges,fromId,toId,call);
    else missing_methods++;
  }
  function witness(group) {
    const adjacency=new Map(group.members.map(id=>[id,[]]));
    for(const edge of group.edges)adjacency.get(edge.from).push(edge.to);
    const start=group.members[0];
    if(group.members.length===1)return [start,start];
    // Prefer an inter-method edge over a self-loop for a multi-member SCC.
    const next=adjacency.get(start).find(id=>id!==start);
    const parents=new Map([[next,null]]),queue=[next];
    for(let i=0;i<queue.length&&!parents.has(start);i++) {
      for(const target of adjacency.get(queue[i])) {
        if(parents.has(target))continue;
        parents.set(target,queue[i]);queue.push(target);
        if(target===start)break;
      }
    }
    const path=[];
    for(let id=start;id!==null;id=parents.get(id))path.push(id);
    return [start,...path.reverse()];
  }
  function groups(kind,index) {
    const edges=[...index.values()],ids=new Set();
    for(const edge of edges){ids.add(edge.from);ids.add(edge.to);}
    // Existing iterative Tarjan implementation also supports exact string IDs.
    const nodes=[...ids].sort(compare).map(id=>({id,name:id}));
    const result=findSccs({nodes,edges}).map(component=>{
      const members=component.members.slice().sort(compare);
      return {id:JSON.stringify([kind,members]),kind,members,edges:[],witness:[],call_sites:0};
    });
    // Assign internal edges once, rather than scanning the graph for every SCC.
    const owner=new Map();
    for(const group of result)for(const id of group.members)owner.set(id,group);
    for(const edge of edges) {
      const group=owner.get(edge.from);
      if(!group||owner.get(edge.to)!==group)continue;
      edge.calls.sort(compareSites);group.edges.push(edge);group.call_sites+=edge.calls.length;
    }
    for(const group of result) {
      group.edges.sort((a,b)=>compare(a.from,b.from)||compare(a.to,b.to));
      group.witness=witness(group);
    }
    return result.sort((a,b)=>compare(a.members[0],b.members[0]));
  }
  return {classes:groups('class',classEdges),methods:groups('method',methodEdges),unresolved_targets,missing_methods};
}
