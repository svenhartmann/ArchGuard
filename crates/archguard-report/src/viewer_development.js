'use strict';
const startUi={within:null,cyclesOnly:false,limit:24};
function developmentPaths(){return data.developmentPaths||[];}
// Mirrors engine/atlas.rs; aggregation and SCCs use the existing shared algorithms.
function architectureAtlasModel(graph,within=null){
  const index=new Map(graph.nodes.map(n=>[n.id,n])),children=new Map();
  for(const node of graph.nodes){const parent=node.parent??null;if(!children.has(parent))children.set(parent,[]);children.get(parent).push(node.id);}
  let scope=within==null?null:graph.nodes.find(n=>n.name===within)?.id;
  if(within!=null&&scope===undefined)return null;
  let ids=scope!=null&&index.get(scope).kind==='file'?[scope]:(children.get(scope)||[]);
  if(within==null)while(ids.length===1&&index.get(ids[0]).kind==='module'){scope=ids[0];ids=children.get(scope)||[];}
  const cut=new Set(ids),v=aggregate(graph,cut),groups=findSccs(v),cyclic=new Set(groups.flatMap(group=>group.members)),groupOf=new Map(groups.flatMap((group,i)=>group.members.map(id=>[id,i])));
  const owner=new Map();for(const node of graph.nodes){let current=node.id;while(current!=null){if(cut.has(current)){owner.set(node.id,current);break;}current=index.get(current)?.parent;}}
  const areas=v.nodes.map(node=>({name:node.name,kind:node.kind,src:node.src,source_files:0,symbolic_files:0,incoming:0,outgoing:0,external_incoming:0,external_outgoing:0,cyclic:cyclic.has(node.id)}));
  const bySource=new Map(areas.map(n=>[n.src,n]));let external=0;
  for(const node of graph.nodes)if(node.kind==='file'&&owner.has(node.id)){const area=bySource.get(owner.get(node.id));area[node.name.endsWith('/*')?'symbolic_files':'source_files']++;}
  for(const edge of graph.edges){const from=owner.get(edge.from),to=owner.get(edge.to);if(from!==undefined&&to===undefined){bySource.get(from).external_outgoing++;external++;}else if(from===undefined&&to!==undefined){bySource.get(to).external_incoming++;external++;}}
  const connections=v.edges.map(edge=>{areas[edge.from].outgoing++;areas[edge.to].incoming++;return {from:areas[edge.from].name,to:areas[edge.to].name,evidence_count:edge.count??null,cyclic:groupOf.has(edge.from)&&groupOf.get(edge.from)===groupOf.get(edge.to)};});
  return {scope:scope==null?null:index.get(scope).name,status:data.semantic?.status||'imports_only',cycle_groups:groups.length,external_connections:external,areas,connections};
}
function renderStart(){
  if(ui.workspace!=='start')return;
  if(isRepositoryOverview()){document.getElementById('start-atlas').hidden=true;if(startUi.kpis)startUi.kpis.hidden=true;renderRepositoryWorld();return;}
  renderStartKpis();
  const atlas=document.getElementById('start-atlas');hideMetricTooltip(atlas);atlas.textContent='';
  if(renderWorld()){atlas.hidden=true;return;}
  atlas.hidden=false;
  let model=architectureAtlasModel(work,startUi.within);if(!model){startUi.within=null;model=architectureAtlasModel(work);}
  const toolbar=element('div',undefined,'atlas-toolbar');atlas.appendChild(toolbar);
  const back=button(t('Overview','Übersicht'),()=>{startUi.within=null;startUi.limit=24;startUi.cyclesOnly=false;renderStart();},'btn btn-secondary');back.disabled=!startUi.within;toolbar.appendChild(back);
  const label=element('strong',model.scope||rootName);toolbar.appendChild(label);
  const cycle=button(t('Cycles','Zyklen')+' · '+model.cycle_groups,()=>{startUi.cyclesOnly=!startUi.cyclesOnly;startUi.limit=24;renderStart();},'btn btn-secondary');cycle.setAttribute('aria-pressed',String(startUi.cyclesOnly));toolbar.appendChild(cycle);
  toolbar.appendChild(contextHelp(t('Atlas legend','Atlas-Legende'),t('Each tile is an immediate package or file. Color marks observed cycles at this aggregation level, not necessarily file cycles. Tile bars compare source-file counts in this scope. Unrecorded dependencies may be missing. Click packages to drill down; click files to inspect their dependencies.','Jede Kachel ist ein direktes Paket oder eine Datei. Die Farbe markiert beobachtete Zyklen dieser Aggregation, nicht zwingend Dateizyklen. Balken vergleichen Quelldateizahlen im Ausschnitt. Nicht erfasste Abhängigkeiten können fehlen. Pakete öffnen den nächsten Ausschnitt; Dateien öffnen ihre Abhängigkeiten.')));
  if(model.scope){const open=button(t('Open graph','Graph öffnen'),()=>{showTab('graph');revealNode(model.scope);},'btn btn-secondary');toolbar.appendChild(open);}
  if(model.external_connections)atlas.appendChild(explainContext(element('small',t('{count} connections across this scope','{count} Verbindungen über diesen Ausschnitt hinaus',{count:model.external_connections}),'muted'),t('Count of original source connections crossing the selected scope. The tiles and cycle count describe internal connections only.','Anzahl ursprünglicher Quellverbindungen über den gewählten Ausschnitt hinaus. Kacheln und Zykluszahl beschreiben nur interne Verbindungen.')));
  const grid=element('div',undefined,'atlas-grid');atlas.appendChild(grid);
  const rows=model.areas.filter(row=>!startUi.cyclesOnly||row.cyclic).sort((a,b)=>Number(b.cyclic)-Number(a.cyclic)||b.source_files-a.source_files||compareNames(a.name,b.name));
  const maximum=Math.max(1,...model.areas.map(row=>row.source_files));
  for(const row of rows.slice(0,startUi.limit)){
    const tile=button('',()=>{if(row.kind==='module'){startUi.within=row.name;startUi.cyclesOnly=false;startUi.limit=24;renderStart();}else{showTab('graph');revealNode(row.name);}},'atlas-area');
    tile.setAttribute('data-atlas-area',row.name);tile.setAttribute('data-cyclic',String(row.cyclic));
    tile.appendChild(element('strong',row.name.split('/').pop()));
    tile.appendChild(element('span',row.source_files+' '+t('files','Dateien')+' · '+row.incoming+' '+t('in','ein')+' · '+row.outgoing+' '+t('out','aus')));
    if(row.cyclic)tile.appendChild(element('span',t('Cycle','Zyklus'),'badge'));
    if(row.symbolic_files)tile.appendChild(element('span',t('{count} symbolic targets','{count} symbolische Ziele',{count:row.symbolic_files}),'muted'));
    const bar=element('span',undefined,'atlas-size'),fill=element('span');fill.style.width=(row.source_files/maximum*100)+'%';bar.appendChild(fill);tile.appendChild(bar);
    grid.appendChild(explainAction(tile,()=>row.name+' · '+t('{incoming} incoming and {outgoing} outgoing source connections outside this scope.','{incoming} eingehende und {outgoing} ausgehende Quellverbindungen außerhalb dieses Ausschnitts.',{incoming:row.external_incoming,outgoing:row.external_outgoing})));
  }
  if(!rows.length)grid.appendChild(element('p',startUi.cyclesOnly?t('No cycles recorded in this scope.','Keine Zyklen in diesem Ausschnitt erfasst.'):t('No source areas recorded.','Keine Quellbereiche erfasst.'),'empty'));
  if(rows.length>startUi.limit)atlas.appendChild(button(t('Next 24 areas','Nächste 24 Bereiche'),()=>{startUi.limit+=24;renderStart();},'btn btn-secondary'));
}
function bindExplanation(helpId,sourceId){
  const help=document.getElementById(helpId),source=document.getElementById(sourceId);
  explainAction(help,()=>source.children.length?Array.from(source.children).map(node=>node.textContent).join('\n\n'):source.textContent);help.addEventListener('click',()=>{showMetricTooltip(help);metricTooltipPinned=true;});
}
function initDevelopment(){
  for(const tab of document.querySelectorAll('#tabs button'))if(['assessment','review-dashboard'].includes(tab.dataset.tab)){tab.hidden=true;tab.setAttribute('tabindex','-1');}
  for(const [help,source]of [['workspace-info','workspace-description'],['engineering-help','engineering-note'],['services-reference-help','services-reference'],['calls-counting-help','calls-counting-note'],['calls-cycle-help','calls-cycles-coverage']])bindExplanation(help,source);
  const mcp=document.getElementById('workspace-mcp');
  explainAction(mcp,()=>t('Copy the MCP entry for this project and view. The guide links source evidence, change planning and verification tools.','MCP-Einstieg für dieses Projekt und diese Ansicht kopieren. Der Wegweiser verbindet Quellbelege, Änderungsplanung und Prüfwerkzeuge.'));
  mcp.addEventListener('click',()=>copyText(JSON.stringify({name:'development_guide',arguments:{path:data.root,...(ui.workspace==='start'?{}:{view:ui.workspace})}},null,2)));
}

// Project-wide measurements share the dashboard renderer and its evidence links.
function scrollStartWorld(){
  const panel=document.getElementById('tab-start'),world=document.getElementById('start-world');
  const offset=world.getBoundingClientRect().top-panel.getBoundingClientRect().top;
  if(Number.isFinite(offset))panel.scrollTop+=offset;
}
function renderStartKpis(){
  if(typeof renderDashKpis!=='function')return;
  if(!startUi.kpis){
    const panel=document.getElementById('tab-start');
    startUi.kpis=element('section',undefined,'start-kpis');startUi.kpis.setAttribute('aria-label',t('Architecture overview','Architekturübersicht'));
    panel.appendChild(startUi.kpis);
  }
  const home=startUi.kpis;home.textContent='';
  home.appendChild(element('style',`
    .start-kpis{max-width:1600px;margin:24px auto;color:var(--text)}
    .start-kpis-header{display:flex;align-items:center;justify-content:space-between;gap:16px;margin:8px 0 18px}
    .start-kpis h2{font-size:26px;letter-spacing:-.7px;margin:4px 0 8px}.start-kpis-header p{margin:0;color:var(--muted);font-size:12px}
    .start-kpis-actions{display:flex;gap:8px;flex-shrink:0}
    .start-kpis .dash-cards{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px}
    .start-kpis .dash-card{position:relative;display:flex;flex-direction:column;align-items:flex-start;gap:9px;min-width:0;min-height:152px;padding:20px;border:1px solid var(--border);border-radius:16px;background:linear-gradient(135deg,rgba(113,145,220,.09),transparent);overflow:hidden}
    .start-kpis .dash-card strong{font-size:34px;line-height:1.15;letter-spacing:-1px;font-weight:650;overflow-wrap:anywhere}
    .start-kpis .dash-card>span{font-size:13px;font-weight:600}.start-kpis .dash-card-sub{font-size:11px;line-height:1.5;color:var(--muted);overflow-wrap:anywhere}
    .start-kpis .dash-card[role=button]{cursor:pointer}.start-kpis .dash-card[role=button]:hover{border-color:#7897eb;background:rgba(113,145,220,.13)}
    .start-kpis .dash-card:focus-visible{outline:2px solid #7897eb;outline-offset:3px}
    .start-kpis .home-health{order:-1;background:linear-gradient(135deg,rgba(109,136,236,.22),rgba(109,136,236,.04));padding-right:88px}
    .home-health:after{content:'';position:absolute;right:18px;top:24px;width:54px;height:54px;border-radius:50%;background:conic-gradient(var(--score-color,#839bf1) var(--score-angle,0deg),rgba(128,140,170,.18) 0);mask:radial-gradient(transparent 58%,#000 61%)}
    .home-health[data-level=yellow]{--score-color:#d9ac54}.home-health[data-level=green]{--score-color:#5bbd9a}.home-health[data-level=red]{--score-color:#e47b81}
    .start-kpis-profile{margin-top:14px;border:1px solid var(--border);border-radius:12px;padding:12px 16px}.start-kpis-profile>summary{cursor:pointer;font-size:12px;color:var(--muted)}
    .start-kpis-profile-grid{display:grid;grid-template-columns:1fr 1fr;gap:24px;padding-top:12px;min-width:0}.start-kpis-profile-grid>div{min-width:0}.start-kpis-profile svg{max-width:100%}
    @media(max-width:1000px){.start-kpis .dash-cards{grid-template-columns:repeat(2,minmax(0,1fr))}}
    @media(max-width:520px){.start-kpis-header{align-items:flex-start;flex-direction:column}.start-kpis h2{font-size:23px}.start-kpis .dash-card{padding:14px;min-height:135px}.start-kpis .dash-card strong{font-size:27px}.start-kpis .home-health{padding-right:14px}.home-health:after{width:25px;height:25px;top:auto;bottom:14px;right:14px}.start-kpis-profile-grid{grid-template-columns:1fr}}
  `));
  const header=element('div',undefined,'start-kpis-header'),intro=element('div');header.appendChild(intro);
  intro.appendChild(element('h2',t('Architecture at a glance','Architektur im Blick')));
  intro.appendChild(element('p',rootName+' · '+t('Project analysis · independent of the World scope','Projektanalyse · unabhängig vom World-Ausschnitt')));
  const actions=element('div',undefined,'start-kpis-actions');actions.appendChild(button('World View ↑',scrollStartWorld,'btn btn-secondary'));actions.appendChild(button(t('Open graph','Graph öffnen'),()=>showTab('graph'),'btn btn-secondary'));header.appendChild(actions);home.appendChild(header);
  const cards=element('div');home.appendChild(cards);renderDashKpis(cards,data.reviewKpis);
  for(const card of cards.querySelectorAll('div')){
    const label=card.querySelector('span');if(label?.textContent!==t('Health','Gesundheit'))continue;
    card.classList.add('home-health');label.textContent=t('Architecture score · /100','Architektur-Score · /100');
    const health=data.reviewKpis?.health,score=health?.score;
    card.querySelector('strong').textContent=score==null?dashNotMeasured():dashFixed(score,1);
    if(score!=null)card.style.setProperty('--score-angle',Math.max(0,Math.min(100,score))*3.6+'deg');
    const sub=card.querySelector('small');if(sub)sub.textContent=health?.trend==null?t('No comparison snapshot yet','Noch kein Vergleichssnapshot'):(health.trend>=0?'+':'')+dashFixed(health.trend,1)+' '+t('vs. previous snapshot','ggü. vorherigem Snapshot');
  }
  if(data.reviewKpis&&typeof renderDashZones==='function'&&typeof renderDashHealth==='function'){
    const details=element('details',undefined,'start-kpis-profile');details.appendChild(element('summary',t('Architecture profile and history','Architekturprofil und Verlauf')));home.appendChild(details);
    let rendered=false;details.addEventListener('toggle',()=>{if(!details.open||rendered)return;rendered=true;
      const grid=element('div',undefined,'start-kpis-profile-grid'),zones=element('div'),health=element('div');
      zones.appendChild(element('h3',t('Abstraction & stability','Abstraktion & Stabilität')));health.appendChild(element('h3',t('Score history','Score-Verlauf')));grid.appendChild(zones);grid.appendChild(health);details.appendChild(grid);
      renderDashZones(zones,data.reviewKpis);renderDashHealth(health,data.reviewKpis);
    });
  }
}
