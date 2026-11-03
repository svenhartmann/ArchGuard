'use strict';
// Pure mirror of archguard-analysis/src/review.rs. Source evidence belongs to
// the original scan; topology and review priorities follow the current view.
function computeReview(full, view, context = {}) {
  const sourceNodes=full.nodes||[],nodes=view.nodes||[],sources=context.sources||{},evidence=new Map((sources.files||[]).map(file=>[file.node_id,file]));
  const fullByName=new Map(sourceNodes.map(node=>[node.name,node])),fullById=new Map(sourceNodes.map(node=>[node.id,node]));
  const children=new Map(),adj=new Map(nodes.map(node=>[node.id,new Set()])),reverse=new Map(nodes.map(node=>[node.id,new Set()]));
  for(const node of sourceNodes){if(!children.has(node.parent))children.set(node.parent,[]);children.get(node.parent).push(node.id);}
  let edgeCount=0;
  for(const edge of view.edges||[])if(adj.has(edge.from)&&adj.has(edge.to)&&!adj.get(edge.from).has(edge.to)){
    adj.get(edge.from).add(edge.to);reverse.get(edge.to).add(edge.from);edgeCount++;
  }
  function reachable(id,edges){const seen=new Set([id]),pending=[id];for(let i=0;i<pending.length;i++)for(const next of edges.get(pending[i])||[])if(!seen.has(next)){seen.add(next);pending.push(next);}seen.delete(id);return seen;}
  function descendants(id){const seen=new Set(),pending=[id];for(let i=0;i<pending.length;i++){const next=pending[i];if(seen.has(next))continue;seen.add(next);pending.push(...children.get(next)||[]);}return seen;}
  function entryAncestor(id){const seen=new Set();while(id!=null&&!seen.has(id)){if(entryPoints.has(id))return true;seen.add(id);id=fullById.get(id)?.parent;}return false;}
  const entryPoints=new Set(context.entry_point_ids||[]),scopeFiles=new Set();
  const git=sources.git||{status:'unavailable',since:'180.days',shallow:false,truncated:false,scanned_commits:0,message:'No Git history supplied'};
  const isFile=node=>node?.kind==='file'&&!node.name.endsWith('/*');
  function sumKnown(ids,key){if(!ids.length)return 0;let sum=0;for(const id of ids){const value=evidence.get(id)?.[key];if(value==null)return null;sum+=value;}return sum;}
  const rows=nodes.map(node=>{
    const source=fullById.get(node.src)||fullByName.get(node.name),members=source?descendants(source.id):new Set();
    const files=[...members].filter(id=>isFile(fullById.get(id)));for(const id of files)scopeFiles.add(id);
    const dependencies=reachable(node.id,adj),dependents=reachable(node.id,reverse),degree=adj.get(node.id).size+reverse.get(node.id).size;
    const mutual=[...dependencies].filter(id=>dependents.has(id)).length,sccSize=mutual?mutual+1:adj.get(node.id).has(node.id)?1:0;
    const changes=git.status==='available'?sumKnown(files,'changes'):null;
    const components={cycle:sccSize?Math.min(40,30+2*(sccSize-1)):0,coupling:Math.min(20,2*degree),impact:Math.min(20,dependents.size),churn:changes===null?0:Math.min(20,changes*Math.min(5,degree))};
    const score=Object.values(components).reduce((sum,value)=>sum+value,0),entryPoint=entryAncestor(source?.id)||[...members].some(id=>entryPoints.has(id));
    const orphan=!reverse.get(node.id).size&&!entryPoint&&files.length?(adj.get(node.id).size?'unreferenced':'isolated'):null;
    const reasons=[];
    if(components.cycle)reasons.push('Cyclic group: '+sccSize+' nodes (+'+components.cycle+')');
    if(components.coupling)reasons.push('Direct coupling: '+reverse.get(node.id).size+' incoming + '+adj.get(node.id).size+' outgoing (+'+components.coupling+')');
    if(components.impact)reasons.push('Transitive dependents: '+dependents.size+' (+'+components.impact+')');
    if(components.churn)reasons.push('Git change touches × coupling: '+changes+' × '+Math.min(5,degree)+' (+'+components.churn+')');
    if(orphan)reasons.push('No incoming imports in this scope; check entry points and dynamic consumers before removing code.');
    return {node_id:node.id,source_id:source?.id??null,name:node.name,kind:node.kind,rank:0,score,priority:score>=40?'high':score>=20?'medium':'low',fan_in:reverse.get(node.id).size,fan_out:adj.get(node.id).size,instability:degree?adj.get(node.id).size/degree:0,scc_size:sccSize,dependencies:dependencies.size,dependents:dependents.size,file_count:files.length,mapped_files:files.filter(id=>evidence.get(id)?.path!=null).length,lines:sumKnown(files,'lines'),changes,entry_point:entryPoint,orphan,components,reasons};
  });
  rows.sort((a,b)=>b.score-a.score||(a.name<b.name?-1:a.name>b.name?1:0)||a.node_id-b.node_id);rows.forEach((row,index)=>{row.rank=index+1;});
  const ccd=rows.reduce((sum,row)=>sum+row.dependencies+1,0),n=rows.length,balanced=(n+1)*Math.log2(n+1)-n,files=[...scopeFiles];
  return {schema_version:1,score_version:'review-priority/v1',summary:{scope_nodes:n,scope_edges:edgeCount,ccd,acd:n?ccd/n:0,nccd:balanced>0?ccd/balanced:0,cyclic_nodes:rows.filter(row=>row.scc_size).length,largest_scc:Math.max(0,...rows.map(row=>row.scc_size)),high_priority:rows.filter(row=>row.priority==='high').length,orphan_candidates:rows.filter(row=>row.orphan).length,source_files:files.length,mapped_files:files.filter(id=>evidence.get(id)?.path!=null).length,lines:sumKnown(files,'lines'),changes:git.status==='available'?sumKnown(files,'changes'):null},nodes:rows,git,entry_points:context.entry_points||[],entry_point_ids:[...entryPoints].sort((a,b)=>a-b),limitations:[
    'Priority is a heuristic review order, not defect probability or measured runtime/build performance.',
    'Orphan candidates have no incoming imports in this scope; public APIs, entry points and dynamic consumers may still use them.',
    'Git changes count file-commit touches, not changed lines or distinct package commits; shallow/truncated history supplies lower bounds.',
    'LOC and Git totals are unknown when any represented source file lacks evidence; no abstractness or Main Sequence Distance is inferred.'
  ]};
}
