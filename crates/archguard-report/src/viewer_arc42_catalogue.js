'use strict';
// arc42 diagram catalogue (ADR 0006) in the web reader. The generated sections of shown
// diagrams are cut from the chapter text and drawn here instead of showing their source.
// A static page cannot write archguard.toml: a tick only changes this view and yields
// the [arc42] snippet to copy. Mirrors architecture_ui/catalogue.rs (desktop).
const arc42MapIds={dependencies:'building-blocks',calls:'runtime-calls',cycles:'cycles'};
// One colour and label per relation kind, as architecture_ui/figure.rs (desktop).
const arc42RelationStyles={
  startup:{color:['--muted','#8a99ad'],dashed:true,label:()=>t('starts first','startet zuerst')},
  http:{color:['--c-node','#2563EB'],label:()=>'HTTP'},
  grpc:{color:['--c-incoming','#0369A1'],label:()=>'gRPC'},
  event:{color:['--c-outgoing','#7C3AED'],label:()=>t('event','Ereignis')},
  reference:{color:['--c-edge','#64748B'],dashed:true,label:()=>t('reference','Verweis')},
  data:{color:['--c-resolved','#1E8E3E'],label:()=>t('uses','nutzt')}
};
function arc42Catalogue(){const list=data.insights?.arc42?.diagrams;return Array.isArray(list)&&list.length?list:null;}
function arc42CatalogueEntry(id){return arc42Catalogue()?.find(d=>d.id===id)||null;}
function arc42Shown(d){return architectureUi.arc42Ticks?.[d.id]??d.shown!==false;}
function arc42HiddenIds(){return (arc42Catalogue()||[]).filter(d=>!arc42Shown(d)).map(d=>d.id);}
function arc42TicksChanged(){return (arc42Catalogue()||[]).some(d=>arc42Shown(d)!==(d.shown!==false));}
function arc42Snippet(){return '[arc42]\nhidden_diagrams = ['+arc42HiddenIds().map(id=>JSON.stringify(id)).join(', ')+']';}
function arc42ThemeToken(name,fallback){const css=typeof getComputedStyle==='function'?getComputedStyle(document.documentElement):null;return css?.getPropertyValue(name).trim()||fallback;}

// The section exactly as arc42_diagrams::section writes it: heading, note, one fenced block.
function arc42FindSection(markdown,d){
  const head='### '+d.title+'\n\n'+d.note+'\n\n```',start=markdown.indexOf(head);if(start<0)return null;
  const fence=markdown.slice(start+head.length).match(/^(\w+)\n/);if(!fence)return null;
  const body=start+head.length+fence[0].length,close=markdown.slice(body).match(/^```[ \t]*(?:\n|$)/m);if(!close)return null;
  let end=body+close.index+close[0].length;if(markdown[end]==='\n')end++;
  return {start,end,lang:fence[1],source:markdown.slice(body,body+close.index),text:markdown.slice(start,end)};
}
function arc42StripCatalogue(markdown,number){
  for(const d of arc42Catalogue()||[]){if(d.chapter!==number)continue;const found=arc42FindSection(markdown,d);if(found)markdown=markdown.slice(0,found.start)+markdown.slice(found.end);}
  return markdown;
}
function arc42SectionOf(d){const chapter=data.insights.arc42.chapters?.find(c=>c.number===d.chapter);return chapter?arc42FindSection(String(chapter.markdown||''),d):null;}

// The neutral catalogue model as the reader's graph model.
function arc42CatalogueModel(d){
  const kinds={package:'package',remainder:'group',container:'container',database:'database',external:'external'};
  return {c4:d.kind!=='packages',kind:d.id,scope:d.title,unit:d.unit,
    nodes:(d.elements||[]).map(e=>({id:e.id,name:e.name,label:e.name,kind:kinds[e.kind]||'package',technology:e.technology,group:e.group,cycle:!!e.cycle,detail:[e.technology,e.group].filter(Boolean).join(' · ')})),
    edges:(d.relations||[]).map(r=>({from:r.from,to:r.to,kind:r.kind,count:r.count,cycle:!!r.cycle}))};
}
function arc42C4Type(node){
  const what={container:t('Container','Container'),database:t('Database','Datenbank'),external:t('External container','Externer Container')}[node.kind];
  return what&&(node.technology?what+' · '+node.technology:what);
}
function arc42RelationText(edge){return arc42RelationStyles[edge.kind]?.label()??String(edge.count??1);}
function arc42RelationColor(edge){const style=arc42RelationStyles[edge.kind];return style?arc42ThemeToken(...style.color):null;}

// Graphviz model of a C4 diagram: cylinders for databases, dashed external containers,
// one dashed cluster per group, one colour per relation kind.
function arc42C4Graph(model,selected){
  const ids=new Map(model.nodes.map((n,i)=>[n.id,'n'+i])),muted=arc42ThemeToken('--muted','#b6c4d8'),accent=arc42ThemeToken('--accent','#92b6ef'),dim=arc42ThemeToken('--c-dim','#45556c');
  const near=e=>!selected||e.from===selected||e.to===selected;
  const nodes=model.nodes.map((n,i)=>({name:ids.get(n.id),group:n.group,attributes:{id:'arc42-standard-node-'+i,shape:n.kind==='database'?'cylinder':'box',style:n.kind==='external'?'filled,dashed':'filled,rounded',label:n.name+'\n['+arc42C4Type(n)+']',tooltip:n.name,color:selected===n.id?accent:muted,penwidth:selected===n.id?'3':'1.4'}}));
  const edges=model.edges.filter(e=>ids.has(e.from)&&ids.has(e.to)).map((e,i)=>({tail:ids.get(e.from),head:ids.get(e.to),attributes:{id:'arc42-standard-edge-'+i,xlabel:arc42RelationText(e)+' · '+(e.count??1),tooltip:arc42RelationText(e)+' · '+(e.count??1)+' '+(model.unit||''),color:near(e)?arc42RelationColor(e)||accent:dim,style:arc42RelationStyles[e.kind]?.dashed?'dashed':'solid',penwidth:selected&&near(e)?'2.5':'1.3'}}));
  const groups=[...new Set(nodes.map(n=>n.group).filter(Boolean))];
  const subgraphs=groups.map((g,i)=>({name:'cluster_'+i,graphAttributes:{label:g,style:'dashed',color:muted,fontcolor:muted,labelloc:'t'},nodes:nodes.filter(n=>n.group===g).map(({name,attributes})=>({name,attributes}))}));
  return {directed:true,graphAttributes:{rankdir:'LR',label:'',bgcolor:'transparent',pad:'.3',nodesep:'.55',ranksep:'1',splines:'spline',fontname:'Arial',fontcolor:muted},nodeAttributes:{fontname:'Arial',fontsize:'12',margin:'.18,.14',fillcolor:arc42ThemeToken('--panel','#1c2b42'),fontcolor:arc42ThemeToken('--text','#ecf2fa')},edgeAttributes:{fontname:'Arial',fontsize:'10',fontcolor:muted,arrowsize:'.7'},
    nodes:nodes.filter(n=>!n.group).map(({name,attributes})=>({name,attributes})),edges,subgraphs};
}
// C4-PlantUML export of the drawn diagram; `label` escapes like arc42PlantUml.
function arc42C4PlantUml(model,label){
  const ids=new Map(model.nodes.map((n,i)=>[n.id,'n'+i])),lines=['@startuml','!include <C4/C4_Deployment>'];
  const shape={database:'ContainerDb',external:'Container_Ext'},groups=[...new Set(model.nodes.map(n=>n.group??null))];
  groups.forEach((g,i)=>{
    if(g!=null)lines.push('Deployment_Node(g'+i+', "'+label(g)+'", "Docker Compose") {');
    for(const n of model.nodes.filter(n=>(n.group??null)===g))lines.push((shape[n.kind]||'Container')+'('+ids.get(n.id)+', "'+label(n.name)+'", "'+label(n.technology||'')+'")');
    if(g!=null)lines.push('}');
  });
  for(const e of model.edges)if(ids.has(e.from)&&ids.has(e.to))lines.push('Rel('+ids.get(e.from)+', '+ids.get(e.to)+', "'+label(arc42RelationText(e))+'", "'+label((e.count??1)+' '+(model.unit||''))+'")');
  lines.push('@enduml');return lines.join('\n');
}

// Ticks: hidden diagrams collapse to one line with their tick.
function arc42Tick(parent,d){
  const label=element('label',undefined,'arc42-tick'),box=element('input');box.type='checkbox';box.checked=arc42Shown(d);box.setAttribute('data-arc42-tick',d.id);
  box.setAttribute('aria-label',t('{title} in documentation','{title} in der Dokumentation',{title:d.title}));
  box.addEventListener('change',()=>arc42Toggle(d.id,!!box.checked));label.appendChild(box);label.appendChild(element('span',t('In documentation','In der Dokumentation')));parent.appendChild(label);
}
function arc42Toggle(id,shown){
  (architectureUi.arc42Ticks??={})[id]=shown;
  const tab=document.getElementById('tab-architecture'),top=tab.scrollTop;renderArchitecture();tab.scrollTop=top;
  [...document.getElementById('architecture-content').querySelectorAll('input[data-arc42-tick]')].find(n=>n.getAttribute('data-arc42-tick')===id)?.focus({preventScroll:true});
}
// One catalogue figure: the tick, then `draw` when shown; without an entry just `draw`.
function arc42Figure(parent,d,draw){
  if(!d)return draw(parent);
  const shell=element('section',undefined,'arc42-figure');shell.setAttribute('data-arc42-diagram',d.id);parent.appendChild(shell);
  const head=element('div',undefined,'arc42-figure-head');shell.appendChild(head);
  if(arc42Shown(d)){arc42Tick(head,d);return draw(shell);}
  shell.setAttribute('data-arc42-hidden','true');head.appendChild(element('span',d.title+' · '+t('not in the documentation','nicht in der Dokumentation'),'arc42-map-note'));arc42Tick(head,d);return true;
}
function arc42SnippetBox(parent,always=false){
  if(!always&&!arc42TicksChanged())return;
  const box=element('div',undefined,'arc42-snippet'),text=arc42Snippet();parent.appendChild(box);
  box.appendChild(element('p',t('This page cannot write archguard.toml. To keep this choice, put the following into the [arc42] section of archguard.toml and run archguard doc --arc42 again.','Diese Seite kann archguard.toml nicht schreiben. Um diese Auswahl zu behalten, folgendes in den Abschnitt [arc42] von archguard.toml übernehmen und archguard doc --arc42 erneut ausführen.'),'arc42-map-note'));
  const pre=element('pre');pre.setAttribute('data-arc42-snippet','true');pre.appendChild(element('code',text));box.appendChild(pre);
  box.appendChild(button(t('Copy','Kopieren'),()=>copyText(text),'btn btn-secondary'));
}

// A C4 or package diagram drawn from the catalogue model, with legend and relation list.
function arc42DrawCatalogue(parent,d){
  const model=arc42CatalogueModel(d),shell=element('div',undefined,'arc42-visual');parent.appendChild(shell);
  shell.appendChild(element('h3',d.title));shell.appendChild(element('p',d.note,'arc42-map-note'));
  if(!model.nodes.length){shell.appendChild(element('p',t('This diagram has no elements.','Dieses Diagramm hat keine Elemente.')));return true;}
  if(model.c4)shell.appendChild(element('p',t('Blue: HTTP · cyan: gRPC · violet: events · green: datastore use · grey dashed: startup order · light grey dashed: configured reference. Counts are configuration evidence.','Blau: HTTP · Cyan: gRPC · Violett: Ereignisse · Grün: Datenbanknutzung · grau gestrichelt: Startreihenfolge · hellgrau gestrichelt: Konfigurationsverweis. Anzahlen sind Konfigurationsbelege.'),'arc42-map-note'));
  arc42Graph(shell,model,{title:d.title,all:true});
  const names=new Map(model.nodes.map(n=>[n.id,n.name])),list=element('details',undefined,'arc42-relations');list.appendChild(element('summary',t('Relationships and counts','Beziehungen und Anzahl')+' · '+model.edges.length));
  for(const e of model.edges)list.appendChild(element('p',names.get(e.from)+' → '+names.get(e.to)+' · '+(model.c4?arc42RelationText(e)+' · ':'')+(e.count??1)+' '+(model.unit||'')));
  shell.appendChild(list);return true;
}
// The chapter's catalogue diagrams the derived maps did not already draw.
function renderArc42CatalogueFigures(parent,number,drawn){
  let any=false;
  for(const d of arc42Catalogue()||[])if(d.chapter===number&&!drawn.has(d.id))any=arc42Figure(parent,d,host=>arc42DrawCatalogue(host,d))||any;
  return any;
}

// The view below the chapters: the whole catalogue, hidden ones included.
function renderArc42DiagramView(article){
  article.appendChild(element('h2',t('Diagrams','Diagramme')));
  article.appendChild(element('p',t('All diagrams the documentation can show. Ticked ones are written into their chapter of arc42.md as Mermaid or PlantUML.','Alle Diagramme, die die Dokumentation zeigen kann. Angehakte werden als Mermaid oder PlantUML in ihr Kapitel von arc42.md geschrieben.'),'arc42-map-note'));
  arc42SnippetBox(article,true);
  for(const d of arc42Catalogue()||[])arc42DiagramEntry(article,d);
}
function arc42DiagramEntry(parent,d){
  const shell=element('section',undefined,'arc42-figure');shell.setAttribute('data-arc42-diagram',d.id);if(!arc42Shown(d))shell.setAttribute('data-arc42-hidden','true');parent.appendChild(shell);
  const head=element('div',undefined,'arc42-figure-head');head.appendChild(element('strong',t('Chapter {n}','Kapitel {n}',{n:d.chapter})+' · '+d.id));arc42Tick(head,d);shell.appendChild(head);
  arc42DrawCatalogue(shell,d);
  const found=arc42SectionOf(d),source=element('details',undefined,'arc42-source');shell.appendChild(source);
  source.appendChild(element('summary',found?(found.lang==='plantuml'?t('PlantUML source','PlantUML-Quelltext'):t('Mermaid source','Mermaid-Quelltext')):t('Source','Quelltext')));
  if(found){const pre=element('pre');pre.setAttribute('data-arc42-diagram-source',d.id);pre.appendChild(element('code',found.source));source.appendChild(pre);}
  else source.appendChild(element('p',t('This diagram is not in the generated document, so its Mermaid/PlantUML source is not part of this report. It appears in the chapter after the diagram is enabled in archguard.toml and archguard doc --arc42 runs again.','Dieses Diagramm steht nicht im erzeugten Dokument, daher enthält dieser Bericht seinen Mermaid-/PlantUML-Quelltext nicht. Er erscheint im Kapitel, nachdem das Diagramm in archguard.toml aktiviert und archguard doc --arc42 erneut ausgeführt wurde.'),'arc42-map-note'));
}
