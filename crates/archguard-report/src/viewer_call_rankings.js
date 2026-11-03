'use strict';
// Input is the already filtered source-call snapshot, before an element filter.
// Count the same records as the evidence/tree views, never runtime executions.
function computeCallRankings(calls,classes,methods) {
  const classRows=new Map(),methodRows=new Map();
  let unresolved_targets=0,missing_callers=0,missing_targets=0;
  function addSite(rows,from,to) {
    function add(id,direction) {
      if(id===null)return null;
      let row=rows.get(id);
      if(!row){row={id,incoming:0,outgoing:0,total:0};rows.set(id,row);}
      row[direction]++;return row;
    }
    const outgoing=add(from,'outgoing'),incoming=add(to,'incoming');
    // One record may be both incoming and outgoing (for example, recursion).
    if(outgoing)outgoing.total++;
    if(incoming&&incoming!==outgoing)incoming.total++;
  }
  for(const call of calls) {
    const from=classes.has(call.from_class)?call.from_class:null,to=classes.has(call.to_class)?call.to_class:null;
    addSite(classRows,from,to);
    const fromMethod=methods.get(call.from_method),toMethod=methods.get(call.to_method);
    const fromId=from!==null&&fromMethod?.class_id===from?call.from_method:null;
    const toId=to!==null&&toMethod?.class_id===to?call.to_method:null;
    addSite(methodRows,fromId,toId);
    if(fromId===null)missing_callers++;
    if(call.resolution==='project'&&toId===null)missing_targets++;
    if(call.resolution==='unresolved')unresolved_targets++;
  }
  const order=(a,b)=>b.incoming-a.incoming||(a.id<b.id?-1:a.id>b.id?1:0);
  return {classes:[...classRows.values()].sort(order),methods:[...methodRows.values()].sort(order),unresolved_targets,missing_callers,missing_targets};
}
