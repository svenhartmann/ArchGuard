'use strict';
// Optional dependency details beside the primary SVG tree. This renderer
// changes neither the analytical cut nor source evidence.
const dependencyPageSize=100;
let dependencyPageKey='',dependencyPages={};

function renderDependencyBrowser() {
  const browser=document.getElementById('dependency-browser');
  if(ui.diagram!=='dependencies')return;
  browser.textContent='';
  const displayed=displayedGraphNodes(),shown=new Set(displayed.map(node=>node.id));
  const byId=new Map(view.nodes.map(node=>[node.id,node]));
  const key=JSON.stringify([viewRevision,ui.diagram,ui.edgeMode,ui.cyclesOnly,ui.selection,packageContext?.name,[...shown],currentFilter()]);
  if(key!==dependencyPageKey){dependencyPageKey=key;dependencyPages={};}
  const header=element('header',undefined,'dependency-browser-heading');browser.appendChild(header);
  const names=new Map();
  for(const node of view.nodes){const name=node.name.split('/').pop();names.set(name,(names.get(name)||0)+1);}
  function shortName(node){const base=node.name.split('/').pop();return names.get(base)>1?node.name:base;}
  function nodeButton(node){
    const action=button(shortName(node),()=>{if(ui.edgeMode==='all')ui.edgeMode='both';selectNode(node.name);},'btn btn-quiet dependency-node-link');
    action.setAttribute('data-dependency-node',node.name);action.setAttribute('title',node.name);
    action.setAttribute('aria-label',t('Inspect {name}','{name} untersuchen',{name:node.name}));return action;
  }
  function openPackage(node){
    const sourceId=node.src??node.id;
    if(node.kind!=='module'||!view.nodes.some(n=>n.src===sourceId)||!childrenOf(work,sourceId).length)return null;
    const action=button(t('Open package','Paket öffnen'),()=>navigateExplorerNode(sourceId),'btn btn-secondary');
    action.setAttribute('data-open-package',node.name);action.setAttribute('aria-label',t('Open package {name}','Paket {name} öffnen',{name:node.name}));return action;
  }
  function priorityBadge(node){
    const review=reviewBySource.get(node.src??node.id);
    if(!ui.highlightPriorities||!review||review.priority==='low')return null;
    const badge=element('span',review.priority==='high'?'P1':'P2','dependency-priority');
    badge.setAttribute('title',reviewPriorityText(review)+'\n'+review.reasons.map(reviewReasonText).join('\n'));
    badge.setAttribute('aria-label',reviewPriorityText(review));return badge;
  }
  function appendEdgeState(parent,edge){
    const key=edge.from+' '+edge.to;
    const state=introducedKeys.has(key)?'New cycle':resolvedKeys.has(key)?'Cycle resolved':cyclicKeys.has(key)?'Cycle':packageContext?.edges.has(key)?'Package cycle context':null;
    if(state){const labels={'New cycle':t('New cycle','Neuer Zyklus'),'Cycle resolved':t('Cycle resolved','Zyklus aufgelöst'),Cycle:t('Cycle','Zyklus'),'Package cycle context':t('Package cycle context','Paketzyklus-Kontext')};const badge=element('span',labels[state],'dependency-cycle-badge');badge.setAttribute('data-cycle-state',state);parent.appendChild(badge);}
  }
  function page(parent,rows,section,render,unit){
    const limit=dependencyPages[section]||dependencyPageSize;
    for(const row of rows.slice(0,limit))parent.appendChild(render(row));
    if(rows.length>limit){
      const footer=element('div',undefined,'dependency-page');
      footer.appendChild(element('p',t('Showing {shown} of {total} {unit}.','{shown} von {total} {unit} angezeigt.',{shown:limit,total:rows.length,unit}),'muted'));
      const more=button(t('Show next {count}','Nächste {count} anzeigen',{count:Math.min(dependencyPageSize,rows.length-limit)}),()=>{dependencyPages[section]=limit+dependencyPageSize;renderDependencyBrowser();});
      more.setAttribute('data-dependency-more',section);footer.appendChild(more);parent.appendChild(footer);
    }
  }
  function sortedEdges(edges){return edges.slice().sort((a,b)=>compareNames(byId.get(a.from).name,byId.get(b.from).name)||compareNames(byId.get(a.to).name,byId.get(b.to).name));}
  function edgeRow(edge,direction){
    const row=element('article',undefined,'dependency-row dependency-'+direction),from=byId.get(edge.from),to=byId.get(edge.to);
    row.setAttribute('data-dependency-row',edge.from+' '+edge.to);
    const route=element('div',undefined,'dependency-route');
    route.appendChild(nodeButton(from));route.appendChild(element('span','→','dependency-route-arrow'));route.appendChild(nodeButton(to));row.appendChild(route);
    const detail=element('div',undefined,'dependency-row-detail');
    const evidence=button(weightLabel(edge)+' '+graphEvidenceUnit()+t(' · Evidence',' · Belege'),()=>selectEdge(edge),'btn btn-quiet dependency-evidence');
    evidence.setAttribute('data-dependency-pair',edge.from+' '+edge.to);
    evidence.setAttribute('aria-label',t('Inspect {source} → {target}','{source} → {target} untersuchen',{source:from.name,target:to.name})+' · '+weightLabel(edge)+' '+graphEvidenceUnit());
    evidence.setAttribute('title',edge.count==null?t('The total is unknown; the shown reference count is a lower bound.','Die Gesamtzahl ist unbekannt; die angezeigte Referenzzahl ist eine Untergrenze.'):t('Exact recorded dependency-reference total; the inspector identifies any sampled evidence.','Exakte Gesamtzahl der erfassten Abhängigkeitsbelege; der Inspektor kennzeichnet Belege aus Stichproben.'));
    detail.appendChild(evidence);appendEdgeState(detail,edge);row.appendChild(detail);return row;
  }
  function section(parent,name,title,edges,empty){
    const box=element('section',undefined,'dependency-section dependency-'+name);
    box.setAttribute('data-dependency-section',name);box.appendChild(element('h3',title+' · '+edges.length));
    if(edges.length)page(box,sortedEdges(edges),name,edge=>edgeRow(edge,name),t('dependencies','Abhängigkeiten'));
    else box.appendChild(element('p',empty,'empty'));
    parent.appendChild(box);return box;
  }
  const scopeEdges=view.edges.filter(edge=>dependencyMatchesSelection(edge.from,edge.to));
  const edges=scopeEdges.filter(edge=>shown.has(edge.from)&&shown.has(edge.to));
  const hidden=scopeEdges.length-edges.length;
  header.appendChild(element('h2',ui.selection?.kind==='node'?t('Dependencies of {name}','Abhängigkeiten von {name}',{name:ui.selection.name}):ui.selection?.kind==='cycle'?t('Cycle dependencies','Zyklusabhängigkeiten'):ui.edgeMode==='all'?t('All dependencies','Alle Abhängigkeiten'):t('Dependencies','Abhängigkeiten')));
  header.appendChild(element('p',t('{count} directed dependencies match this view','{count} gerichtete Abhängigkeiten entsprechen dieser Ansicht',{count:edges.length})+(hidden?t(' · {count} hidden by endpoint display filters. Show files to include their endpoints.',' · {count} durch Endpunktfilter ausgeblendet. Dateien anzeigen, um ihre Endpunkte einzubeziehen.',{count:hidden}):'.')+t(' Each arrow reads source → target.',' Jeder Pfeil bedeutet Quelle → Ziel.'),'muted'));
  const reference=ui.selection?.kind==='node'&&selectedNodeIds.size?selectedNodeIds:null;
  if(ui.edgeMode==='all'||!reference){
    section(browser,'list',ui.edgeMode==='all'?t('All dependencies in this scope','Alle Abhängigkeiten in diesem Bereich'):t('Matching dependencies','Passende Abhängigkeiten'),edges,t('No dependencies match the current selection and filters. Select a package or choose All.','Keine Abhängigkeiten entsprechen der aktuellen Auswahl und den Filtern. Ein Paket oder „Alle“ auswählen.'));return;
  }
  const incoming=[],outgoing=[],internal=[],returns=[];
  for(const edge of edges){
    const from=reference.has(edge.from),to=reference.has(edge.to);
    if(from&&to)internal.push(edge);
    else if(to)incoming.push(edge);
    else if(from)outgoing.push(edge);
    else returns.push(edge);
  }
  const lanes=element('div',undefined,'dependency-lanes');browser.appendChild(lanes);
  section(lanes,'incoming',t('Incoming →','Eingehend →'),incoming,ui.edgeMode==='outgoing'?t('Hidden by the Outgoing filter.','Durch den Filter „Ausgehend“ ausgeblendet.'):t('No incoming dependencies match the current filters.','Keine eingehenden Abhängigkeiten entsprechen den aktuellen Filtern.'));
  const center=element('section',undefined,'dependency-center');center.setAttribute('data-dependency-section','selection');
  center.appendChild(element('div',t('SELECTED','AUSGEWÄHLT'),'dependency-kind'));center.appendChild(element('h3',ui.selection.name));
  const selected=nodeByName(work,ui.selection.name),direct=view.nodes.find(node=>node.name===ui.selection.name);
  center.appendChild(element('p',direct?(direct.kind==='module'?t('Package','Paket'):t('File','Datei')):t('{count} visible nodes represent this package’s contents.','{count} sichtbare Knoten stellen den Inhalt dieses Pakets dar.',{count:reference.size}),'muted'));
  if(direct){const badge=priorityBadge(direct);if(badge)center.appendChild(badge);}
  if(selected){const open=openPackage({...selected,src:selected.id});if(open)center.appendChild(open);}
  center.appendChild(element('p',t('Incoming → this selection → Outgoing. Reciprocal neighbors appear on both sides with separate counts.','Eingehend → diese Auswahl → Ausgehend. Nachbarn mit gegenseitigen Abhängigkeiten erscheinen auf beiden Seiten mit getrennten Anzahlen.'),'muted'));
  lanes.appendChild(center);
  section(lanes,'outgoing',t('→ Outgoing','→ Ausgehend'),outgoing,ui.edgeMode==='incoming'?t('Hidden by the Incoming filter.','Durch den Filter „Eingehend“ ausgeblendet.'):t('No outgoing dependencies match the current filters.','Keine ausgehenden Abhängigkeiten entsprechen den aktuellen Filtern.'));
  if(internal.length)section(browser,'internal',t('Inside the selection','Innerhalb der Auswahl'),internal,'');
  if(returns.length){
    const context=section(browser,'return',t('External cycle return paths','Externe Zyklusrückwege'),returns,'');
    context.appendChild(element('p',t('These outside-to-outside dependencies complete the package-level cycle context. They are not necessarily cycles at the current file detail level.','Diese Abhängigkeiten zwischen externen Knoten vervollständigen den Zykluskontext auf Paketebene. Auf der aktuellen Dateiebene sind sie nicht zwingend zyklisch.'),'muted'));
  }
}
