'use strict';
function diffTargetKey(report) { return report.target_graph ? 'target_graph' : 'staged_graph'; }
function diffLabels(report) {
 const branch=report.target==='ref', short=ref=>ref.replace(/^refs\/(heads|remotes)\//,''), target=branch?short(report.target_ref):'Git-Index · gestagt';
 return {
  target,
  heading:branch?'Architekturvergleich der Branches':'Architektur im Staging',
  patch:branch?'Quelltext-Diff der Branches':'Gestagter Quelltext-Diff',
  revision:`${report.base_ref} (${report.base_commit.slice(0,10)}) → ${branch?`${target} (${report.target_commit.slice(0,10)})`:`Index ${report.index_fingerprint.slice(0,10)}`} · Regeln: ${report.policy_source==='explicit'?'explizite Konfiguration':branch?'aus dem Ziel-Branch':'aus dem Index'}`,
  explanation:branch?'Verglichen werden die beiden Branch-Spitzen zum Analysezeitpunkt, nicht ihr gemeinsamer Vorfahr. Die Commit-IDs stehen oben. Beide Seiten verwenden dieselben Regeln aus dem Ziel-Branch (oder der expliziten Konfiguration) und dieselbe Anordnung. Checkout, Index und Arbeitsdateien bleiben unverändert und fließen nicht in den Vergleich ein.':'Links steht der Commit, rechts der vollständige Git-Index. Auch teilweise gestagte Dateien werden exakt aus dem Index gelesen. Beide Seiten verwenden dieselben Regeln und dieselbe Anordnung. Nicht gestagte und unversionierte Dateien sind nicht enthalten.'
 };
}
function diffUnion(report) {
  const nodes = new Map();
  for (const [key, side] of [['base_graph','before'],[diffTargetKey(report),'after']]) {
    const names = new Map(report[key].nodes.map(n => [n.id,n.name]));
    for (const n of report[key].nodes) {
      if (!nodes.has(n.name)) nodes.set(n.name,{name:n.name,kind:n.kind,parent:n.parent==null?null:names.get(n.parent),before:false,after:false});
      nodes.get(n.name)[side]=true;
      nodes.get(n.name)[side+'_cycle']=!!n.in_cycle;
      nodes.get(n.name)[side+'_files']=n.files;
    }
  }
  for(const n of nodes.values())if(nodes.has(n.parent))nodes.get(n.parent).has_children=true;
  return nodes;
}
function buildDiffRows(report, expanded, changesOnly, search) {
  const nodes=diffUnion(report), changed=new Set([...report.added_nodes,...report.removed_nodes]);
  for(const e of [...report.added_edges,...report.removed_edges,...report.changed_edges]) { changed.add(e.from);changed.add(e.to); }
  for(const e of [...report.cycle_changes?.added_edges||[],...report.cycle_changes?.removed_edges||[]]) {changed.add(e.from);changed.add(e.to);}
  const included=new Set(changed);
  for(const key of ['base_graph',diffTargetKey(report)]) {
    const names=new Map(report[key].nodes.map(n=>[n.id,n.name]));
    for(const e of report[key].edges) if(changed.has(names.get(e.from)) || changed.has(names.get(e.to))) { included.add(names.get(e.from));included.add(names.get(e.to)); }
  }
  const q=search.trim().toLowerCase();
  const keep=new Set([...nodes.keys()].filter(name=>(!changesOnly||included.has(name))&&(!q||name.toLowerCase().includes(q))));
  for(const name of [...keep]) { let parent=nodes.get(name).parent;const seen=new Set();while(parent && nodes.has(parent) && !seen.has(parent)) {seen.add(parent);keep.add(parent);parent=nodes.get(parent).parent;} }
  const children=new Map();
  for(const n of nodes.values()) {const p=nodes.has(n.parent)?n.parent:null;if(!children.has(p))children.set(p,[]);children.get(p).push(n);}
  for(const values of children.values()) values.sort((a,b)=>(a.kind===b.kind?0:a.kind==='module'?-1:1)||a.name.localeCompare(b.name));
  const rows=[];
  function visit(parent,depth) { for(const n of children.get(parent)||[]) {if(!keep.has(n.name))continue;rows.push({...n,depth,changed:changed.has(n.name)});if(expanded.has(n.name)||q)visit(n.name,depth+1);} }
  visit(null,0);return rows;
}
function visibleDiffEdges(report,key,rows) {
  const graph=report[key], names=new Map(graph.nodes.map(n=>[n.id,n.name])), all=diffUnion(report), shown=new Set(rows.map(n=>n.name));
  function ancestor(name) {const seen=new Set();while(name&&!shown.has(name)&&!seen.has(name)){seen.add(name);name=all.get(name)?.parent;}return shown.has(name)?name:null;}
  const changes=new Map();for(const [list,status] of [[report.added_edges,'added'],[report.removed_edges,'removed'],[report.changed_edges,'changed']]) for(const e of list)changes.set(JSON.stringify([e.from,e.to]),status);
  const cycleChanges=new Set([...report.cycle_changes?.added_edges||[],...report.cycle_changes?.removed_edges||[]].map(e=>JSON.stringify([e.from,e.to])));
  const edges=new Map();for(const e of graph.edges) {
    const from=ancestor(names.get(e.from)),to=ancestor(names.get(e.to));if(!from||!to||from===to)continue;
    const k=JSON.stringify([from,to]), change=changes.get(JSON.stringify([names.get(e.from),names.get(e.to)]))||'unchanged';
    if(!edges.has(k))edges.set(k,{from,to,change,refs:[],count:0,cyclic:false,cycle_changed:false});const edge=edges.get(k);edge.refs.push(...e.refs);if(change!=='unchanged')edge.change=change;
    edge.count=edge.count===null||e.count==null?null:edge.count+e.count;
    edge.cyclic ||=!!e.cyclic;edge.cycle_changed ||=cycleChanges.has(JSON.stringify([names.get(e.from),names.get(e.to)]));
  }return [...edges.values()];
}
if(typeof module!=='undefined')module.exports={buildDiffRows,visibleDiffEdges,diffLabels};
if(typeof document!=='undefined') {
 const original=JSON.parse(document.getElementById('architecture-diff-data').textContent), $=id=>document.getElementById(id);
 const targetKey=diffTargetKey(original),labels=diffLabels(original),cache=new Map();
 let report,full,union,expanded=new Set(),selected=null,scale=1,syncing=false;
 function text(tag,value,className) {const el=document.createElement(tag);el.textContent=value;if(className)el.className=className;return el;}
 function prepare() {
  const level=$('diff-level').value,depth=$('diff-depth').value,key=JSON.stringify([level,depth]);
  if(!cache.has(key)){if(cache.size>=4)cache.delete(cache.keys().next().value);cache.set(key,prepareDiffView(original,level,depth==='direct'?'direct':Number(depth)));}
  full=cache.get(key);report=filterDiffCycles(full,$('diff-cycles').value);union=diffUnion(report);
  if(level==='packages')expanded=new Set();
  else for(const n of union.values())if(n.kind==='module')expanded.add(n.name);
  $('diff-depth').disabled=level!=='packages';$('open').hidden=level==='packages';$('close').hidden=level==='packages';
  $('summary').replaceChildren();
  for(const [value,label,status] of [[full.added_edges.length,'neue Beziehungen','added'],[full.removed_edges.length,'entfernte Beziehungen','removed'],[full.changed_edges.length,'geänderte Belege','changed']])$('summary').append(text('span',`${value} ${label}`,status));
  $('cycle-status').textContent=`Zyklusgruppen: ${full.base_graph.cycle_groups.length} → ${full[targetKey].cycle_groups.length} · ${full.cycle_changes.added_edges.length} Beziehungen neu im Zyklus · ${full.cycle_changes.removed_edges.length} nicht mehr zyklisch`;
  for(const [key,id]of [['base_graph','base-coverage'],[targetKey,'staged-coverage']]) {
   const g=full[key],c=g.semantic,files=original[key].nodes.filter(n=>n.kind==='file').length;
   $(id).textContent=`${level==='packages'?g.nodes.length+' Pakete · ':''}${files} Dateien · ${g.edges.length} Beziehungen · Compiler: ${{ready:'vollständig',partial:'teilweise',unavailable:'nicht verfügbar'}[c?.status]||'unbekannt'}`;
  }
  render();
 }
 $('revision').textContent=labels.revision;
 $('heading').textContent=labels.heading;document.title='ArchGuard · '+labels.heading;
 $('target-title').textContent=labels.target;$('patch-title').textContent=labels.patch;
 $('comparison-explanation').textContent=labels.explanation;
 $('base-title').textContent=original.base_ref.replace(/^refs\/(heads|remotes)\//,'');
 $('patch').textContent=original.source_diff||'Keine Quelltextänderungen zwischen diesen Ständen.';
 for(const message of original.limitations)$('limits').append(text('p',message));
 $('findings-title').textContent=`Regelbefunde · ${original.new_findings.length} neu · ${original.resolved_findings.length} behoben`;
 for(const [list,label]of [[original.new_findings,'Neu'],[original.resolved_findings,'Behoben']])for(const f of list)$('findings').append(text('p',`${label}: ${f.kind} · ${f.nodes.join(' → ')}${f.message?' · '+f.message:''}`));
 const maxDepth=[original.base_graph,original[targetKey]].reduce((max,g)=>g.nodes.reduce((depth,n)=>n.kind==='module'?Math.max(depth,n.name.split('/').length):depth,max),0);
 for(let i=0;i<Math.min(maxDepth,32);i++){const option=text('option',`Ebene ${i+1}`);option.value=String(i);$('diff-depth').append(option);}
 function details(rows) {
  const pane=$('selection');pane.replaceChildren();if(!selected){pane.textContent='Paket oder Datei auswählen, um die Beziehungen und Quellbelege zu sehen.';return;}
  pane.append(text('strong',selected));const copy=text('button','Pfad kopieren');copy.style.marginLeft='12px';copy.onclick=async()=>{try{await navigator.clipboard.writeText(selected);copy.textContent='Kopiert';}catch{copy.textContent='Pfad oben markieren';}};pane.append(copy);
  for(const [key,label]of [['base_graph',original.base_ref],[targetKey,labels.target]])for(const e of visibleDiffEdges(report,key,rows).filter(e=>e.from===selected||e.to===selected)) {
   const box=text('details','', 'evidence');box.append(text('summary',`${label}: ${e.from} → ${e.to} · ${e.count==null?'mind. '+e.refs.length:e.count} Verwendungen${e.cyclic?' · Zyklus':''}`,e.change));
   let loaded=false;box.ontoggle=()=>{if(!box.open||loaded)return;loaded=true;let offset=0;const pre=text('pre',''),more=text('button','Weitere Belege');box.append(pre,more);function next(){pre.textContent+=e.refs.slice(offset,offset+100).map(r=>`${r.file}:${r.line}\n${r.raw}\n\n`).join('');offset+=100;more.hidden=offset>=e.refs.length;more.textContent=`Weitere Belege (${Math.max(0,e.refs.length-offset)} verbleibend)`;}more.onclick=next;next();};pane.append(box);
  }
 }
 function render() {
  const positions=[$('before').scrollTop,$('before').scrollLeft];let hiddenLinks=0;
  const rows=buildDiffRows(report,expanded,$('changes').checked,$('search').value), index=new Map(rows.map((r,i)=>[r.name,i]));
  $('empty').textContent=rows.length?'':$('changes').checked?'Keine Architekturänderungen in dieser Auswahl. Alle Knoten sind über den Filter erreichbar.':'Keine Knoten in dieser Auswahl.';
  if(selected&&!index.has(selected))selected=null;
  for(const [side,key,id] of [['before','base_graph','before'],['after',targetKey,'after']]) {
    const pane=$(id);pane.replaceChildren();const width=Math.max(400,pane.clientWidth)*scale, height=Math.max(240,rows.length*50+24)*scale;
    const drawing=text('div','', 'drawing');drawing.style.width=width+'px';drawing.style.height=height+'px';pane.append(drawing);
    const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('width',width);svg.setAttribute('height',height);drawing.append(svg);
    const ns=svg.namespaceURI,defs=document.createElementNS(ns,'defs');svg.append(defs);
    for(const status of ['added','removed','changed','unchanged','cycle']) { const m=document.createElementNS(ns,'marker');m.id=`${id}-${status}`;for(const [k,v]of Object.entries({viewBox:'0 0 10 10',refX:'9',refY:'5',markerWidth:'6',markerHeight:'6',orient:'auto-start-reverse'}))m.setAttribute(k,v);const p=document.createElementNS(ns,'path');p.setAttribute('d','M 0 0 L 10 5 L 0 10 z');p.setAttribute('fill',status==='cycle'?'var(--cycle)':status==='unchanged'?'var(--muted)':`var(--${status==='added'?'add':status==='removed'?'remove':'changed'})`);m.append(p);defs.append(m); }
    let links=visibleDiffEdges(report,key,rows).filter(e=>selected?(e.from===selected||e.to===selected):(e.change!=='unchanged'||e.cycle_changed||!$('changes').checked||$('diff-cycles').value!=='all'));
    const limit=selected?200:80;if(links.length>limit){hiddenLinks+=links.length-limit;links=links.slice(0,limit);}
    links.forEach((e,i)=>{const y1=(index.get(e.from)*50+32)*scale,y2=(index.get(e.to)*50+32)*scale,x=width-138*scale,reach=Math.min(118,28+(i%12)*8)*scale;const p=document.createElementNS(ns,'path');p.setAttribute('d',`M ${x} ${y1} C ${x+reach} ${y1}, ${x+reach} ${y2}, ${x} ${y2}`);p.setAttribute('stroke',e.change==='unchanged'?(e.cyclic?'var(--cycle)':'var(--muted)'):`var(--${e.change==='added'?'add':e.change==='removed'?'remove':'changed'})`);p.setAttribute('stroke-width','1.6');p.setAttribute('fill','none');p.setAttribute('marker-end',`url(#${id}-${e.change==='unchanged'&&e.cyclic?'cycle':e.change})`);svg.append(p);});
    rows.forEach((r,i)=>{const status=!r.before?'added':!r.after?'removed':r.changed?'changed':'';const row=text('div','',`row ${r[side]?'':'missing'} ${status} ${r.name===selected?'selected':''}`);row.style.top=(12+i*50)*scale+'px';row.style.height=40*scale+'px';row.style.fontSize=13*scale+'px';row.style.paddingLeft=(8+r.depth*14)*scale+'px';row.style.width=(width-150*scale)+'px';row.title=r.name;row.tabIndex=0;row.setAttribute('role','button');row.setAttribute('aria-label',r.name+(r[side]?'':' · hier nicht vorhanden'));row.onclick=()=>{selected=r.name;render();};row.onkeydown=e=>{if(e.target===row&&(e.key==='Enter'||e.key===' ')){e.preventDefault();selected=r.name;render();}};if(r.has_children){const toggle=text('button',expanded.has(r.name)?'▾':'▸','toggle');toggle.setAttribute('aria-label',`${expanded.has(r.name)?'Schließen':'Öffnen'}: ${r.name}`);toggle.setAttribute('aria-expanded',expanded.has(r.name));toggle.onclick=e=>{e.stopPropagation();expanded.has(r.name)?expanded.delete(r.name):expanded.add(r.name);render();};row.append(toggle);}row.append(text('span',`${status==='added'?'+ ':status==='removed'?'− ':status==='changed'?'~ ':''}${report.level==='packages'?r.name:r.name.split('/').pop()}`,'name'));if(r[side+'_cycle'])row.append(text('span',!r[(side==='before'?'after':'before')+'_cycle']?(side==='after'?'Zyklus neu':'Zyklus entfällt'):'Zyklus','cycle-badge'));drawing.append(row);});
    pane.scrollTop=positions[0];pane.scrollLeft=positions[1];
  }
  $('edge-limit').textContent=hiddenLinks?`${hiddenLinks} weitere Linien ausgeblendet. Paket auswählen oder die Suche eingrenzen; die Quellbelege bleiben vollständig erreichbar.`:'';
  details(rows);$('zoom').textContent=Math.round(scale*100)+' %';
 }
 for(const [id,other]of [['before','after'],['after','before']])$(id).onscroll=()=>{if(syncing)return;syncing=true;$(other).scrollTop=$(id).scrollTop;$(other).scrollLeft=$(id).scrollLeft;syncing=false;};
 $('search').oninput=render;$('changes').onchange=render;
 for(const id of ['diff-level','diff-depth','diff-cycles'])$(id).onchange=()=>{selected=null;if(id==='diff-cycles'&&$('diff-cycles').value!=='all')$('changes').checked=false;prepare();};
 $('open').onclick=()=>{for(const n of union.values())if(n.kind==='module')expanded.add(n.name);render();};$('close').onclick=()=>{expanded.clear();render();};
 $('minus').onclick=()=>{scale=Math.max(.6,scale-.1);render();};$('plus').onclick=()=>{scale=Math.min(1.8,scale+.1);render();};$('zoom').onclick=()=>{scale=1;render();};window.addEventListener('resize',render);prepare();
}
