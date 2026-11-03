'use strict';
const diffSccs=typeof module!=='undefined'?require('./viewer_scc.js').findSccs:findSccs;
function packageDiffGraph(graph,depth='direct') {
 const nodes=new Map(graph.nodes.map(n=>[n.id,n])),owners=new Map(),packages=new Map();
 for(const file of graph.nodes.filter(n=>n.kind==='file')) {
  const chain=[],seen=new Set();let parent=nodes.get(file.parent);
  while(parent&&!seen.has(parent.id)){seen.add(parent.id);if(parent.kind==='module')chain.unshift(parent);parent=nodes.get(parent.parent);}
  const owner=depth==='direct'?chain.at(-1):chain[Math.min(Number(depth),chain.length-1)];
  // Source package paths are relative; '/' uniquely represents root-level files.
  const key=owner?'module:'+owner.name:'root:',name=owner?owner.name:'/';
  owners.set(file.id,key);
  if(!packages.has(key))packages.set(key,{id:key,name,kind:'module',parent:null,files:0});
  packages.get(key).files++;
 }
 const edges=new Map();
 for(const e of graph.edges) {
  const from=owners.get(e.from),to=owners.get(e.to);if(from===undefined||to===undefined||from===to)continue;
  const key=JSON.stringify([from,to]);
  if(!edges.has(key))edges.set(key,{from,to,count:0,refs:[],evidence:new Map()});
  const merged=edges.get(key);
  merged.count=merged.count===null||e.count==null?null:merged.count+e.count;
  for(const r of e.refs||[])merged.evidence.set(JSON.stringify([r.file,r.line,r.raw]),r);
 }
 const result=[...edges.values()].map(({evidence,...e})=>({...e,refs:[...evidence.entries()].sort(([a],[b])=>a.localeCompare(b)).map(([,r])=>r)}));
 result.sort((a,b)=>String(a.from).localeCompare(String(b.from))||String(a.to).localeCompare(String(b.to)));
 return {...graph,nodes:[...packages.values()].sort((a,b)=>a.name.localeCompare(b.name)),edges:result};
}
function annotateDiffCycles(graph) {
 const groups=diffSccs(graph),owner=new Map();
 for(const group of groups)for(const id of group.members)owner.set(id,group);
 return {...graph,nodes:graph.nodes.map(n=>({...n,in_cycle:owner.has(n.id)})),cycle_groups:groups,edges:graph.edges.map(e=>({...e,cyclic:owner.has(e.from)&&owner.get(e.from)===owner.get(e.to)}))};
}
function diffNamedEdges(graph) {
 const names=new Map(graph.nodes.map(n=>[n.id,n.name]));
 return new Map(graph.edges.map(e=>{const from=names.get(e.from),to=names.get(e.to);return [JSON.stringify([from,to]),{...e,from,to}];}));
}
function prepareDiffView(report,level='packages',depth='direct') {
 const targetKey=report.target_graph?'target_graph':'staged_graph';
 const project=graph=>annotateDiffCycles(level==='packages'?packageDiffGraph(graph,depth):graph);
 const before=project(report.base_graph),after=project(report[targetKey]);
 const old=diffNamedEdges(before),next=diffNamedEdges(after),added_edges=[],removed_edges=[],changed_edges=[];
 const signature=e=>JSON.stringify([e.count??null,(e.refs||[]).map(r=>JSON.stringify([r.file,r.line,r.raw])).sort()]);
 for(const key of new Set([...old.keys(),...next.keys()])) {
  const a=old.get(key),b=next.get(key),edge={from:(b||a).from,to:(b||a).to,before:a?.refs||[],after:b?.refs||[]};
  if(!a)added_edges.push(edge);else if(!b)removed_edges.push(edge);else if(signature(a)!==signature(b))changed_edges.push(edge);
 }
 const oldNodes=new Set(before.nodes.map(n=>n.name)),newNodes=new Set(after.nodes.map(n=>n.name));
 const cycle_changes={added_edges:[],removed_edges:[]};
 for(const key of new Set([...old.keys(),...next.keys()])) {
  const a=old.get(key),b=next.get(key);
  if(!!a?.cyclic===!!b?.cyclic)continue;
  cycle_changes[b?.cyclic?'added_edges':'removed_edges'].push({from:(b||a).from,to:(b||a).to});
 }
 return {...report,level,base_graph:before,[targetKey]:after,
  added_nodes:[...newNodes].filter(n=>!oldNodes.has(n)),removed_nodes:[...oldNodes].filter(n=>!newNodes.has(n)),
  added_edges,removed_edges,changed_edges,cycle_changes};
}
function filterDiffCycles(report,mode='all') {
 if(mode==='all')return report;
 const targetKey=report.target_graph?'target_graph':'staged_graph';
 const keys=['base_graph',targetKey],allowed=new Set(),filtered=new Map();
 for(const key of keys) {
  const graph=report[key],edges=graph.edges.filter(e=>mode==='cycles'?e.cyclic:!e.cyclic),ids=new Set();
  for(const e of edges){ids.add(e.from);ids.add(e.to);}
  const cyclicNodes=new Set(graph.cycle_groups.flatMap(g=>g.members));
  for(const n of graph.nodes)if(ids.has(n.id)||(mode==='acyclic'&&!cyclicNodes.has(n.id)))allowed.add(n.name);
  filtered.set(key,edges);
 }
 const result={...report};
 // Retain nodes on both sides even if only one side has matching edges.
 for(const key of keys)result[key]={...report[key],nodes:report[key].nodes.filter(n=>allowed.has(n.name)),edges:filtered.get(key)};
 return result;
}
if(typeof module!=='undefined')module.exports={prepareDiffView,filterDiffCycles,packageDiffGraph};
