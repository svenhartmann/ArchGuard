'use strict';
const worldUi={within:null,homeScope:null,history:[],lens:'problems',kpi:'complexity',camera:{x:0,y:0,zoom:1,rotation:0},selection:null,model:null,layout:null,hits:[],bound:false,hover:null,limit:20,query:'',flow:0};
const WORLD_TONES={flagged:'--c-cycle',neutral:'--accent',unknown:'--muted',covered:'--c-resolved',dependency:'--c-resolved',low:'--m-low',medium:'--m-medium',high:'--m-high',info:'--accent'};
function worldComplexityColor(value){return `hsl(${Math.max(15,175-Math.log2(value+1)*24)} 48% 55%)`;}
// The Explore lens colours surfaces by one KPI at a time. Class metrics come
// from the imported code metrics joined on the compiler class id — the same
// rows the class heatmap rates, so the language-cohort colours match exactly.
const WORLD_CLASS_KPIS=['cbo','rfc','dit','noc','wmc','lcom4'];
const worldKpiCache={source:null,rows:new Map()};
function worldKpiRows(){const source=engineeringData('code_metrics');if(worldKpiCache.source!==source){worldKpiCache.source=source;worldKpiCache.rows=new Map((source?.classes||[]).map(row=>[row.class_id,row]));}return worldKpiCache.rows;}
function worldKpiRow(entity){return entity?.class_id?worldKpiRows().get(entity.class_id)||null:null;}
function worldKpiActive(){return worldUi.lens==='complexity'&&WORLD_CLASS_KPIS.includes(worldUi.kpi);}
function worldKpiLabel(){return WORLD_CLASS_KPIS.includes(worldUi.kpi)?worldUi.kpi.toUpperCase():t('Complexity','Komplexität');}
function worldEntityValue(entity){if(!worldKpiActive())return Number.isFinite(entity.max_complexity)?entity.max_complexity:null;const row=worldKpiRow(entity);return row?classMetricValue(row,worldUi.kpi):null;}
// Help for the metric selector: the shared tooltip provider re-evaluates this
// on every show, so it always describes the currently selected KPI.
function worldKpiHelpText(){return WORLD_CLASS_KPIS.includes(worldUi.kpi)?metricHelp(worldUi.kpi)+' '+t('Colours compare with measured classes of the same source language exactly as in the class heatmap: below 50% low, from 50% elevated, from 80% high; unmeasured stays gray. Grouped surfaces use their highest value; heights normalise to the highest value in this view.','Die Farben vergleichen mit gemessenen Klassen derselben Quellsprache wie in der Klassen-Heatmap: unter 50% niedrig, ab 50% erhöht, ab 80% hoch; nicht Gemessenes bleibt grau. Gebündelte Flächen verwenden ihren höchsten Wert; die Höhen sind auf den höchsten Wert dieser Ansicht normiert.'):t('Observed maximum cyclomatic method complexity, logarithmically colored from teal to orange; grouped surfaces use the largest recorded value and heights are log-scaled. The scale is continuous, not a good/bad threshold.','Erfasste maximale zyklomatische Methodenkomplexität, logarithmisch von Türkis nach Orange eingefärbt; gebündelte Flächen verwenden den größten erfassten Wert und die Höhen sind logarithmiert. Die Skala ist kontinuierlich, keine Gut-/Schlecht-Grenze.');}
function worldKpiCellState(cell){
  let best=null;for(const e of cell.entities){const row=worldKpiRow(e);if(!row)continue;const value=classMetricValue(row,worldUi.kpi);if(value!==null&&(best===null||value>best.value))best={value,state:classMetricAssessment(row,worldUi.kpi).state};}
  return best?.state||null;
}
function renderWorldLegend(){
  const legend=document.getElementById('world-legend');hideMetricTooltip(legend);legend.textContent='';
  const rows=worldUi.lens==='problems'?[
    ['flagged',t('Cycle / finding','Zyklus / Befund'),t('Red outlines mark areas in a cycle or with recorded findings. Red arrows show cyclic dependencies at this level; opposite directions have separate curves. A package cycle does not mean every class participates, or that a file cycle exists.','Rote Umrisse markieren Bereiche mit Zyklus oder erfassten Befunden. Rote Pfeile zeigen zyklische Abhängigkeiten dieser Ebene; Gegenrichtungen verlaufen getrennt. Ein Paketzyklus bedeutet nicht, dass jede Klasse beteiligt ist oder ein Dateizyklus vorliegt.')],
    ['neutral',t('Classes / files','Klassen / Dateien'),t('Blue surfaces represent classes or files, including inside cyclic packages. Their color is neutral: it does not rate their quality or claim they are free of findings.','Blaue Flächen stehen für Klassen oder Dateien, auch innerhalb zyklischer Pakete. Die Farbe ist neutral: Sie bewertet weder deren Qualität noch behauptet sie Befundfreiheit.')]
  ]:worldUi.lens==='complexity'?(worldKpiActive()?[
    ...[['low',t('Low','Niedrig')],['medium',t('Elevated','Erhöht')],['high',t('High','Hoch')],...(worldUi.kpi==='noc'?[['info',t('Informational','Information')]]:[])].map(([key,label])=>[key,label,()=>classMetricStateLabel(key)+' · '+worldKpiHelpText()]),
    ['unknown',t('Unknown','Unbekannt'),t('No measurement for {kpi} in the imported class metrics for this surface. Gray does not mean a low value; missing class data stays unknown.','Für diese Fläche fehlt in den importierten Klassenkennzahlen ein Messwert für {kpi}. Grau bedeutet nicht niedriger Wert; fehlende Klassendaten bleiben unbekannt.',{kpi:worldKpiLabel()})]
  ]:[
    ['complexity',t('Low → high','Gering → hoch'),t('Observed maximum cyclomatic method complexity, logarithmically colored from teal to orange. Grouped surfaces use the largest recorded value. The scale is continuous, not a good/bad threshold.','Erfasste maximale zyklomatische Methodenkomplexität, logarithmisch von Türkis nach Orange eingefärbt. Gebündelte Flächen verwenden den größten erfassten Wert. Die Skala ist kontinuierlich, keine Gut-/Schlecht-Grenze.')],
    ['unknown',t('Unknown','Unbekannt'),t('No method complexity has been recorded for this surface. Gray does not mean low complexity.','Für diese Fläche ist keine Methodenkomplexität erfasst. Grau bedeutet nicht geringe Komplexität.')]
  ]):[
    ['flagged',t('Test gaps','Testlücken'),t('The imported file coverage contains uncovered executable lines. Grouped surfaces are red if all members have usable coverage and at least one has a gap. This does not identify missing test cases.','Die importierte Dateiabdeckung enthält nicht abgedeckte ausführbare Zeilen. Gebündelte Flächen sind rot, wenn alle Mitglieder nutzbare Abdeckung haben und mindestens eines eine Lücke hat. Daraus lassen sich keine konkreten fehlenden Testfälle ablesen.')],
    ['covered',t('Covered','Abgedeckt'),t('All executable lines recorded in the imported file coverage are covered. This does not prove current, complete or passing tests; the report revision at import is unverified.','Alle in der importierten Dateiabdeckung erfassten ausführbaren Zeilen sind abgedeckt. Das beweist keine aktuellen, vollständigen oder bestandenen Tests; die Berichtsrevision beim Import ist ungeprüft.')],
    ['unknown',t('Unknown','Unbekannt'),t('Coverage is missing, stale or cannot be mapped unambiguously. A grouped surface stays gray when any member lacks usable file coverage.','Die Abdeckung ist fehlend, veraltet oder nicht eindeutig zuordenbar. Eine gebündelte Fläche bleibt grau, sobald einem Mitglied eine nutzbare Dateiabdeckung fehlt.')]
  ];
  for(const [key,label,help] of rows){
    const item=button('',()=>{showMetricTooltip(item);metricTooltipPinned=true;},'world-legend-item');item.setAttribute('data-world-legend',key);
    const outline=worldUi.lens==='problems'&&key==='flagged',swatch=element('span',undefined,'world-swatch');swatch.setAttribute('data-world-swatch',outline?'outline':key);swatch.setAttribute('aria-hidden','true');
    swatch.style.background=outline?'transparent':key==='complexity'?'linear-gradient(90deg,'+[0,1,3,7,15,31,63,127].map(worldComplexityColor).join(',')+')':`var(${WORLD_TONES[key]})`;
    if(outline)swatch.style.border=`2px solid var(${WORLD_TONES[key]})`;
    item.appendChild(swatch);item.appendChild(element('span',label));legend.appendChild(explainAction(item,()=>help));
  }
}
function worldScope(){
  const atlas=architectureAtlasModel(data,worldUi.within),owners=new Map(),index=new Map(data.nodes.map(n=>[n.id,n])),cut=new Map(atlas.areas.map(a=>[a.src,a.name]));
  for(const n of data.nodes){let id=n.id;while(id!=null){if(cut.has(id)){owners.set(n.name,cut.get(id));break;}id=index.get(id)?.parent;}}
  const grouped=new Map(atlas.areas.map(a=>[a.name,[]]));
  for(const entity of data.insights.world.entities){const owner=owners.get(entity.node);if(owner)grouped.get(owner).push(entity);}
  return {...atlas,areas:atlas.areas.map(a=>({...a,entities:grouped.get(a.name),findings:(data.findings||[]).filter(f=>(f.nodes||[]).some(n=>n===a.name||n.startsWith(a.name+'/')))}))};
}
function worldOpenArea(name){
  if(isRepositoryOverview()){repositoryOpenWorld(name);return;}
  const node=data.nodes.find(n=>n.name===name);if(!node)return;
  if(node.kind==='file'){worldUi.selection={area:name};setWorldDetailsOpen(true);worldDraw();return;}
  worldNavigate(name);
}
function worldNavigate(within){
  if(within===worldUi.within||within!==null&&!data.nodes.some(n=>n.name===within&&n.kind==='module'))return;
  worldUi.history.push({within:worldUi.within,camera:{...worldUi.camera},selection:worldUi.selection,limit:worldUi.limit});
  worldUi.within=within;worldUi.selection=null;worldUi.limit=20;worldUi.camera={...worldUi.camera,x:0,y:0,zoom:1};finishWorldNavigation();
}
function finishWorldNavigation(){
  hideMetricTooltip();worldUi.hover=null;worldUi.hits=[];document.getElementById('world-tip').hidden=true;worldFlowStart();
  renderWorld();scrollStartWorld();document.getElementById('world-scope').focus();
}
function renderWorldPath(){
  const path=document.getElementById('world-scope');hideMetricTooltip(path);path.textContent='';
  const index=new Map(data.nodes.map(n=>[n.id,n])),ancestors=[];
  let node=data.nodes.find(n=>n.name===worldUi.model.scope);
  while(node&&node.name!==worldUi.homeScope){ancestors.unshift(node);node=index.get(node.parent);}
  if(repositoryModel()?.mode==='project'&&repositoryLive()){const overview=element('a',repositoryModel().root.replace(/\\/g,'/').split('/').filter(Boolean).pop(),'world-crumb');repositoryLink(overview,null);path.appendChild(overview);path.appendChild(element('span','/','muted'));}
  const parts=[{name:rootName,scope:null},...ancestors.map(n=>({name:n.name.split('/').pop(),scope:n.name}))];
  path.setAttribute('aria-label',t('Package path: {scope}','Paketpfad: {scope}',{scope:worldUi.model.scope||rootName}));
  for(const [i,part] of parts.entries()){
    if(i){const divider=element('span','/','muted');divider.setAttribute('aria-hidden','true');path.appendChild(divider);}
    const crumb=button(part.name,()=>worldNavigate(part.scope),'world-crumb');crumb.setAttribute('data-world-scope',part.scope??'');
    if(i===parts.length-1){crumb.setAttribute('aria-current','page');crumb.disabled=true;path.appendChild(crumb);}
    else path.appendChild(explainAction(crumb,()=>part.scope||t('Project overview','Projektübersicht')));
  }
}
function worldMatches(cell){return !worldUi.query||cell.entities.some(e=>((e.class_id||e.node)||'').toLowerCase().includes(worldUi.query));}
function worldCopyValue(selection,area){const entity=selection?.entity;return entity?.line!=null&&entity.file?entity.file+':'+entity.line:entity?.file||entity?.node||area.name;}
function worldCycleOf(name){return cyclesForView().find(c=>(c.nodes||[]).some(n=>n===name||n.startsWith(name+'/')));}
// A World package cycle can vanish at the graph's finer start depth. Collapse
// the graph to the World areas so its SCCs are the same package cycles.
function worldGraphAtPackageLevel(){
  trail=[];navigationAtRoot=false;treeLocation=null;isolated=null;ui.cycle=null;treeBaseCollapses.clear();
  for(const area of worldUi.model.areas){const node=area.kind==='module'&&nodeByName(work,area.name);if(node)treeBaseCollapses.add(node.id);}
  recompute();
}
function worldJumpCycle(name){
  const area=worldUi.model.areas.find(a=>a.name===name||name.startsWith(a.name+'/'));
  if(area?.cyclic)worldGraphAtPackageLevel();
  const cycle=worldCycleOf(area?.cyclic?area.name:name);if(cycle)focusCycle(cycle);else showTab('cycles');
}
// Package pins are DOM buttons layered over the canvas, so a plain canvas
// capture would export the scene without any names. Repaint every visible pin
// onto the offscreen context at the position worldDraw gave it.
function worldPaintPins(ctx,canvas){
  const box=canvas.getBoundingClientRect(),dpr=box.width?canvas.width/box.width:(window.devicePixelRatio||1);
  ctx.setTransform(dpr,0,0,dpr,0,0);ctx.textBaseline='top';
  const clip=(text,max)=>{if(ctx.measureText(text).width<=max)return text;let s=text;while(s.length>1&&ctx.measureText(s+'…').width>max)s=s.slice(0,-1);return s+'…';};
  for(const b of document.querySelectorAll('#world-labels button[data-world-area]')){
    if(b.hidden)continue;
    const x=parseFloat(b.style.left),y=parseFloat(b.style.top),bw=b.offsetWidth,bh=b.offsetHeight;
    if(!Number.isFinite(x)||!Number.isFinite(y)||!bw||!bh)continue;
    const left=x-bw/2,top=y-bh/2,cs=getComputedStyle(b);
    ctx.beginPath();if(ctx.roundRect)ctx.roundRect(left,top,bw,bh,9);else ctx.rect(left,top,bw,bh);
    ctx.fillStyle=cs.backgroundColor;ctx.fill();ctx.strokeStyle=cs.borderColor;ctx.lineWidth=1;ctx.stroke();
    // Padding mirrors .world-pin (8px/12px); module pins reserve the arrow gutter.
    const reserve=b.getAttribute('data-world-kind')==='module'?16:0;
    let ty=top+8;
    for(const sel of [['strong',reserve],['small',0]]){const node=b.querySelector(sel[0]);if(!node)continue;
      const s=getComputedStyle(node);ctx.font=s.font;ctx.fillStyle=s.color;
      ctx.fillText(clip(node.textContent,bw-24-sel[1]),left+12,ty);
      ty+=(parseFloat(s.lineHeight)||parseFloat(s.fontSize)*1.4)+5;}
    const arrow=b.querySelector('.world-pin-next');if(arrow){const s=getComputedStyle(arrow);ctx.font=s.font;ctx.fillStyle=s.color;ctx.textAlign='right';ctx.fillText(arrow.textContent,left+bw-10,top+7);ctx.textAlign='left';}
  }
  ctx.setTransform(1,0,0,1,0,0);
}
function exportWorldImage(){
  const canvas=document.getElementById('world-canvas'),css=getComputedStyle(document.documentElement);let url=null;
  // The canvas itself is transparent; the page paints its backdrop in CSS, so
  // the composite adds that background before reading pixels back out.
  try{const off=document.createElement('canvas');off.width=canvas.width;off.height=canvas.height;const ctx=off.getContext&&off.getContext('2d');if(ctx){ctx.fillStyle=css.getPropertyValue('--panel').trim()||'#ffffff';ctx.fillRect(0,0,off.width,off.height);ctx.drawImage(canvas,0,0);worldPaintPins(ctx,canvas);url=off.toDataURL&&off.toDataURL('image/png');}if(!url)url=canvas.toDataURL&&canvas.toDataURL('image/png');}catch{}
  if(!url){feedback(t('Image export is unavailable in this browser.','Der Bildexport ist in diesem Browser nicht verfügbar.'));return;}
  const anchor=document.createElement('a');anchor.href=url;anchor.download='archguard-world-'+((worldUi.within||'overview').replace(/[^\w.-]+/g,'-'))+'.png';anchor.click();
  feedback(t('Exported {filename} with the current view.','{filename} mit der aktuellen Ansicht exportiert.',{filename:anchor.download}));
}
function worldHelp(){if(isRepositoryOverview())return repositoryWorldHelp();return t('Click package labels or double-click a surface to open a package. Use the path or Back to navigate. Wheel: zoom at cursor; at the maximum further wheeling over a package opens it, at the minimum it returns to the previous view. Left-drag or one finger: pan. Right-drag, Shift-drag or Q/E: rotate. Two fingers: pinch to zoom, twist to rotate and move together to pan. The orientation control restores the original direction. + / −: zoom, 0: fit, arrows: pan. Each surface represents a class, or a file without class data. Large scopes group surfaces; select one to see every member. Height at each surface center: log-scaled observed maximum cyclomatic method complexity; corners interpolate adjacent heights. Colors are observations, not a quality verdict. Coverage belongs to files; report revision at import is unverified. Red package cycles may exist without file cycles. Search dims surfaces whose class or file name lacks the term; Enter selects the first match. The metric selector in the Explore lens colours and lifts surfaces by the chosen class metric (CBO, RFC, DIT, NOC, WMC, LCOM4) using the same language-relative colours as the class heatmap; heights normalise to the highest value in this view and unmeasured classes stay gray. Image export saves the current view as PNG. Select a class to open its details. For packages use Details or Enter; Esc closes the overlay without clearing selection. The detail panel jumps to the source graph, cycles list, dependency matrix, calls and class metrics without leaving the report. Buttons glide to the new pose only while motion is allowed; the ground lattice and light pool are decoration without measurement. This is the original analysis, independent of graph simulations.','Paketnamen anklicken oder eine Fläche doppelklicken, um ein Paket zu öffnen. Über Pfad und Zurück navigieren. Mausrad: Zoom am Zeiger; am Maximum öffnet weiteres Zoomen über einem Paket dieses, am Minimum kehrt es zur vorherigen Ansicht zurück. Linksziehen oder ein Finger: verschieben. Rechtsziehen, Umschalt + Ziehen oder Q/E: drehen. Zwei Finger: auseinanderziehen zum Zoomen, drehen zum Rotieren und gemeinsam bewegen zum Verschieben. Die Richtungsanzeige stellt die Ausgangsrichtung wieder her. + / −: Zoom, 0: einpassen, Pfeiltasten: verschieben. Jede Fläche steht für eine Klasse oder eine Datei ohne Klassendaten. Große Ausschnitte bündeln Flächen; Auswählen zeigt alle Mitglieder. Höhe im Flächenmittelpunkt: logarithmisch skalierte, erfasste maximale Methodenkomplexität; Ecken interpolieren die benachbarten Höhen. Farben zeigen Beobachtungen, kein Qualitätsurteil. Abdeckung bezieht sich auf Dateien; die Berichtsrevision beim Import ist ungeprüft. Paketzyklen können ohne Dateizyklen bestehen. Die Suche blendet Flächen aus, deren Klassen- oder Dateiname den Begriff nicht enthält; Enter wählt den ersten Treffer. Die Kennzahlen-Auswahl im Erkunden-Modus färbt und hebt Flächen nach der gewählten Klassenkennzahl (CBO, RFC, DIT, NOC, WMC, LCOM4) in denselben sprachrelativen Farben wie die Klassen-Heatmap; Höhen sind auf den höchsten Wert dieser Ansicht normiert, nicht gemessene Klassen bleiben grau. Der Bildexport speichert die aktuelle Ansicht als PNG. Eine Klasse auswählen öffnet ihre Details. Bei Paketen Details oder Enter verwenden; Esc schließt das Overlay, ohne die Auswahl zu löschen. Das Detailfeld springt zu Quellgraph, Zyklusliste, Abhängigkeitsmatrix, Aufrufen und Klassenkennzahlen, ohne den Bericht zu verlassen. Buttons gleiten nur in die neue Pose, wenn Bewegung erlaubt ist; Bodengitter und Lichtpool sind Dekoration ohne Messwert. Gezeigt wird die ursprüngliche Analyse, unabhängig von Graphsimulationen.');}
function renderWorld(){
  if(isRepositoryOverview())return renderRepositoryWorld();
  const root=document.getElementById('start-world');root.hidden=!data.insights?.world;if(root.hidden)return false;
  if(!worldUi.bound)initWorld();
  document.getElementById('world-canvas').setAttribute('aria-label',t('Architecture landscape. Wheel or pinch to zoom. Drag to pan. Right-drag, Shift-drag, Q/E or twist two fingers to rotate. Use the area selector for accessible navigation.','Architekturlandschaft. Mausrad oder Zwei-Finger-Geste zum Zoomen. Ziehen zum Verschieben. Rechtsziehen, Umschalt + Ziehen, Q/E oder zwei Finger drehen zum Drehen. Bereichsauswahl für barrierefreie Navigation.'));
  worldUi.model=worldScope();
  // Class metrics may still be on the live server; colour them once they arrive.
  const kpiPending=worldKpiActive()&&deferredSection('code_metrics');
  if(kpiPending)loadDeferredSection('code_metrics').then(()=>{if(ui.workspace==='start')renderWorld();},error=>{document.getElementById('world-status').textContent=t('Class metrics could not be loaded: {error}','Klassenkennzahlen konnten nicht geladen werden: {error}',{error:error.message});});
  // Heights in the metric lenses are normalised to the highest value visible in
  // this scope, so a package of small-valued classes still reads as a relief.
  const kpiViewMax=worldKpiActive()?Math.max(0,...worldUi.model.areas.flatMap(a=>a.entities).map(worldEntityValue).filter(Number.isFinite)):0;
  worldUi.layout=worldLayout(worldUi.model.areas,e=>worldEntityValue(e),worldKpiActive()?(value=>value===null?.08:.08+Math.log2(1+value)/Math.log2(1+Math.max(1,kpiViewMax))*2):undefined);
  if(worldUi.within===null)worldUi.homeScope=worldUi.model.scope;
  renderWorldPath();
  for(const lens of ['problems','complexity','coverage'])document.getElementById('world-lens-'+lens).setAttribute('aria-pressed',String(worldUi.lens===lens));
  const kpiSelect=document.getElementById('world-kpi'),kpiHelp=document.getElementById('world-kpi-help');kpiSelect.value=worldUi.kpi;kpiSelect.disabled=kpiHelp.disabled=worldUi.lens!=='complexity';
  renderWorldLegend();
  const back=document.getElementById('world-back');back.disabled=worldUi.history.length===0&&!(repositoryModel()?.mode==='project'&&repositoryLive());
  const count=worldUi.layout.cells.reduce((n,c)=>n+c.entities.length,0);
  const cells=worldUi.layout.cells,areaMax=new Map();
  for(const c of cells)if(c.value!==null)areaMax.set(c.area,Math.max(areaMax.get(c.area)??-Infinity,c.value));
  const matching=cells.filter(worldMatches).length;
  document.getElementById('world-count').textContent=t('{count} classes / files · {tiles} surfaces','{count} Klassen / Dateien · {tiles} Flächen',{count,tiles:cells.length})+(worldUi.query?t(' · {k} surfaces match',' · {k} Flächen entsprechen',{k:matching}):'');
  const labels=document.getElementById('world-labels');labels.textContent='';
  // Every area with content gets a name pin in every lens. Ranking cycles and
  // findings first only orders the DOM; dropping the tail left acyclic
  // packages nameless exactly when nothing else highlighted them.
  const pinned=worldUi.model.areas.filter(a=>a.entities.length);
  // The complexity lens ranks surfaces, so its pins lead with the worst value;
  // other lenses keep cycles and findings first.
  if(worldUi.lens==='complexity')pinned.sort((a,b)=>(areaMax.get(b.name)??-1)-(areaMax.get(a.name)??-1)||compareNames(a.name,b.name));
  else pinned.sort((a,b)=>Number(b.cyclic||b.findings.length>0)-Number(a.cyclic||a.findings.length>0)||b.entities.length-a.entities.length);
  for(const area of pinned){
    const b=button('',()=>worldOpenArea(area.name),'world-pin');b.setAttribute('data-world-area',area.name);b.setAttribute('data-world-kind',area.kind);
    // Pins only ever show siblings of one scope, whose names are unique.
    b.appendChild(element('strong',area.name.split('/').pop()));
    if(area.kind==='module'){const arrow=element('span','›','world-pin-next');arrow.setAttribute('aria-hidden','true');b.appendChild(arrow);}
    const marks=[area.cyclic?t('Package cycle','Paketzyklus'):null,area.findings.length?area.findings.length+' '+t('findings','Befunde'):null,worldUi.lens==='complexity'&&areaMax.has(area.name)?(worldKpiActive()?t('{kpi} max. {value}','{kpi} max. {value}',{kpi:worldKpiLabel(),value:areaMax.get(area.name)}):t('max {value}','max. {value}',{value:areaMax.get(area.name)})):null].filter(Boolean);
    b.appendChild(element('small',marks.length?marks.join(' · '):plural(area.entities.length,'{count} entry','{count} entries','{count} Eintrag','{count} Einträge')));
    const purpose=area.kind==='module'?t('Open package','Paket öffnen'):t('Show details','Details anzeigen'),description=[area.name,...marks,purpose].join(' · ');b.setAttribute('aria-label',description);explainAction(b,()=>description);labels.appendChild(b);
  }
  const scope=document.getElementById('world-area');scope.textContent='';let option=element('option',t('Select area…','Bereich wählen…'));option.value='';scope.appendChild(option);
  for(const area of worldUi.model.areas){option=element('option',area.name);option.value=area.name;scope.appendChild(option);}scope.value=worldUi.selection?.area||'';
  worldDetails();worldDraw();
  if(kpiPending)document.getElementById('world-status').textContent=t('Loading class metrics from the live viewer…','Klassenkennzahlen werden vom Live-Viewer geladen…');
  return true;
}
// The status line depends on the selection, so every detail refresh updates it;
// selecting through the canvas, the list or the picker must not leave it stale.
function worldStatus(){
  const cells=worldUi.layout.cells,knownRows=cells.flatMap(c=>c.entities).filter(e=>e.coverage?.lines);
  const covered=knownRows.reduce((n,e)=>n+e.coverage.lines.covered,0),totalLines=knownRows.reduce((n,e)=>n+e.coverage.lines.total,0),gaps=knownRows.filter(e=>e.coverage.lines.covered<e.coverage.lines.total).length;
  const topCell=cells.filter(c=>c.value!==null).sort((a,b)=>b.value-a.value||compareNames(a.area,b.area))[0];
  const status=document.getElementById('world-status');status.textContent=worldUi.lens==='problems'?t('{count} package cycle groups','{count} Paketzyklus-Gruppen',{count:worldUi.model.cycle_groups}):worldUi.lens==='coverage'?(knownRows.length?t('{gaps} surfaces with gaps · {covered}/{total} lines','{gaps} Flächen mit Lücken · {covered}/{total} Zeilen',{gaps,covered,total:totalLines}):t('Unknown · no usable coverage','Unbekannt · keine nutzbare Abdeckung')):topCell?t('Highest recorded {kpi}: {value} · {area}','Höchster erfasster {kpi}: {value} · {area}',{kpi:worldKpiLabel(),value:topCell.value,area:topCell.area}):t('Observed {kpi} measurements','Erfasste {kpi}-Messwerte',{kpi:worldKpiLabel()});
}
function setWorldDetailsOpen(open,focus=false){
  worldUi.detailsOpen=open;worldDetails();
  if(focus)(open?document.getElementById('world-detail'):document.getElementById('world-canvas')).focus({preventScroll:true});
}
function worldDetailPosition(){
  const panel=document.getElementById('world-detail'),canvas=document.getElementById('world-canvas'),host=document.getElementById('start-world');
  panel.style.top=Math.max(0,canvas.getBoundingClientRect().top-host.getBoundingClientRect().top)+'px';
}
function renderWorldSourceDetails(panel,selection,area){
  const source=nodeByName(data,selection.entity?.node||area.name);if(!source)return;
  const ids=subtree(data,source.id),incoming=data.edges.filter(e=>ids.has(e.to)&&!ids.has(e.from)),outgoing=data.edges.filter(e=>ids.has(e.from)&&!ids.has(e.to));
  panel.appendChild(element('p',selection.entity?.class_id?t('Source file dependencies: {name}. Classes in the same file share this graph evidence.','Abhängigkeiten der Quelldatei: {name}. Klassen derselben Datei teilen diese Graphbelege.',{name:source.name}):t('Original source graph · dependencies crossing this selection.','Ursprünglicher Quellgraph · Abhängigkeiten über die Grenze dieser Auswahl.'),'muted'));
  renderDependencyMetrics(panel,incoming,outgoing);
  const otherIds=new Set([...incoming.map(e=>e.from),...outgoing.map(e=>e.to)]),edges=[...incoming.map(e=>({...e,to:source.id})),...outgoing.map(e=>({...e,from:source.id}))];
  const openSource=name=>{showTab('graph');revealNode(name);openInspector();};
  if(edges.length)renderDetailGraph(panel,{nodes:[source,...data.nodes.filter(n=>otherIds.has(n.id))],edges,focusId:source.id,label:t('Incoming and outgoing dependencies','Eingehende und ausgehende Abhängigkeiten'),onNode:n=>openSource(n.name)});
  for(const [direction,rows]of [['in',incoming],['out',outgoing]]){
    const section=element('details',undefined,'inspector-section');section.setAttribute('data-inspector-neighbors',direction);section.appendChild(element('summary',(direction==='in'?t('Used by','Verwendet von'):t('Depends on','Abhängig von'))+' · '+rows.length));panel.appendChild(section);
    for(const edge of rows.slice(0,worldUi.limit)){
      const node=nodeById(data,direction==='in'?edge.from:edge.to),item=element('details');item.appendChild(element('summary',node.name+' · '+weightLabel(edge)+' '+graphEvidenceUnit()));
      item.appendChild(button(t('Open in source graph','Im Quellgraph öffnen'),()=>openSource(node.name),'btn btn-secondary'));
      for(const ref of (edge.refs||[]).slice(0,worldUi.limit))item.appendChild(element('pre',ref.file+':'+ref.line+' '+ref.raw));
      if((edge.refs||[]).length>worldUi.limit)item.appendChild(button(t('More evidence','Weitere Belege'),()=>{worldUi.limit+=20;worldDetails();},'btn btn-secondary'));section.appendChild(item);
    }
    if(!rows.length)section.appendChild(element('p',t('No recorded dependencies in this direction.','Keine erfassten Abhängigkeiten in dieser Richtung.'),'muted'));
    if(rows.length>worldUi.limit)section.appendChild(button(t('More neighbors','Weitere Nachbarn'),()=>{worldUi.limit+=20;worldDetails();},'btn btn-secondary'));
  }
  renderTestCoverageDetails(panel,source.id);
  renderQualityInspector(panel,source.id,data,worldDetails);renderPerformanceDetails(panel,source.id,data);
  if(!ops.length)renderReviewDetails(panel,source.id);
  else panel.appendChild(element('p',t('Simulation is active. Additional graph review details are available in the source graph; this view retains original dependencies.','Simulation aktiv. Zusätzliche Graph-Reviewdetails stehen im Quellgraph; diese Ansicht behält die ursprünglichen Abhängigkeiten.'),'muted'));
}
// Index compiler measurements once, without expanding the pooled call payload.
let worldMethodCache=null;
function worldClassMethods(entity){
  const calls=data.calls||{},code=engineeringData('code_metrics');
  if(!worldMethodCache||worldMethodCache.calls!==calls||worldMethodCache.code!==code){
    const compact=calls.compact,get=(row,key)=>compact?.encoding==='string-pool-v1'&&compact.fields?.methods?.includes(key)&&typeof row[key]==='number'?compact.strings[row[key]]:row[key];
    const extended=new Map((code?.methods||[]).map(m=>[m.method_id,m])),byClass=new Map(),byFile=new Map(),seen=new Set();
    const add=row=>{for(const [map,key]of [[byClass,row.class_id],[byFile,row.file]]){if(!map.has(key))map.set(key,[]);map.get(key).push(row);}};
    for(const row of calls.methods||[]){const id=get(row,'id'),classId=get(row,'class_id'),extra=extended.get(id);seen.add(id);add({...((extra?.class_id===classId&&extra)||{}),id,class_id:classId,name:get(row,'signature')||get(row,'name')||id,file:get(row,'file'),line:row.line,end_line:row.end_line,complexity:row.complexity});}
    for(const row of code?.methods||[])if(!seen.has(row.method_id))add({...row,id:row.method_id,name:row.method_id,complexity:null});
    worldMethodCache={calls,code,byClass,byFile};
  }
  return (entity.class_id?worldMethodCache.byClass.get(entity.class_id):worldMethodCache.byFile.get(entity.file||entity.node))||[];
}
function renderWorldContext(panel,selection,area){
  const entity=selection.entity,members=entity?[entity]:(selection.members||area.entities),section=element('section');section.setAttribute('data-world-context',worldUi.lens);panel.appendChild(section);
  const heading=worldUi.lens==='complexity'?worldKpiLabel():worldUi.lens==='coverage'?t('Test gaps','Testlücken'):t('Problems and cycles','Probleme und Zyklen');section.appendChild(element('h3',heading));
  if(worldUi.lens==='complexity'){
    // Class metrics and method records may still be on the live server.
    if(entity&&awaitDeferred(section,['calls','code_metrics'],()=>worldDetails())){/* loading note shown */}
    else if(entity){
      section.appendChild(element('p',entity.class_id?t('Class: {name}','Klasse: {name}',{name:entity.class_id}):t('File · no class assignment','Datei · keine Klassenzuordnung'),'path'));
      if(worldKpiActive()){
        const row=worldKpiRow(entity),value=row?classMetricValue(row,worldUi.kpi):null;
        section.appendChild(explainMetric(element('h2',worldKpiLabel()+': '+engineeringNumber(value)),worldUi.kpi));
        section.appendChild(element('p',row?classMetricInterpretation(row,worldUi.kpi):t('No imported measurement for this class.','Kein importierter Messwert für diese Klasse.'),'muted'));
        const measurement=row?.[worldUi.kpi];if(value==null&&measurement?.status!=='not_applicable'&&measurement?.observed!=null)section.appendChild(element('p',t('Incomplete measurement · observed lower bound: {count}','Unvollständige Messung · beobachtete Untergrenze: {count}',{count:measurement.observed}),'muted'));
        for(const evidence of (measurement?.evidence||[]).slice(0,worldUi.limit))section.appendChild(element('p',evidence,'path'));
        if((measurement?.evidence||[]).length>worldUi.limit)section.appendChild(button(t('More evidence','Weitere Belege'),()=>{worldUi.limit+=20;worldDetails();},'btn btn-secondary'));
      }else{
        section.appendChild(element('h2',t('Maximum cyclomatic complexity: {value}','Maximale zyklomatische Komplexität: {value}',{value:entity.max_complexity??'—'})));
        section.appendChild(element('p',t('{count}/{total} methods measured. Height and color show the highest recorded method value, not a class average.','{count}/{total} Methoden gemessen. Höhe und Farbe zeigen den höchsten erfassten Methodenwert, keinen Klassenmittelwert.',{count:entity.measured_methods??'—',total:entity.declared_methods??'—'}),'muted'));
      }
      const methods=[...worldClassMethods(entity)].sort((a,b)=>(b.complexity?.cyclomatic??-1)-(a.complexity?.cyclomatic??-1)||compareNames(a.id,b.id));
      const methodHost=worldKpiActive()?element('details'):element('section');if(worldKpiActive())methodHost.appendChild(element('summary',t('Method complexity','Methodenkomplexität')));else methodHost.appendChild(element('h4',t('Methods · highest complexity first','Methoden · höchste Komplexität zuerst')));section.appendChild(methodHost);
      methodHost.appendChild(element('p',t('{shown}/{total} method records · missing measurements stay unknown.','{shown}/{total} Methodeneinträge · fehlende Messwerte bleiben unbekannt.',{shown:Math.min(worldUi.limit,methods.length),total:methods.length}),'muted'));
      for(const method of methods.slice(0,worldUi.limit)){
        const item=element('details');item.setAttribute('data-world-method',method.id);item.open=!worldKpiActive()&&method===methods[0];item.appendChild(element('summary',method.name+' · CC '+(method.complexity?.cyclomatic??'—')));
        item.appendChild(element('p',t('Cyclomatic: {cc} · maximum nesting: {nesting}','Zyklomatisch: {cc} · maximale Verschachtelung: {nesting}',{cc:method.complexity?.cyclomatic??'—',nesting:method.complexity?.max_nesting??'—'})));
        item.appendChild(element('p',method.file+(method.line?':'+method.line:'')+(method.end_line?'–'+method.end_line:''),'path'));
        if(method.cognitive!=null||method.maintainability_index!=null)renderMethodMetricsCard(item,{...method,method_id:method.id});
        if(!method.complexity)item.appendChild(element('p',t('No cyclomatic measurement for this method.','Keine zyklomatische Messung für diese Methode.'),'muted'));methodHost.appendChild(item);
      }
      if(methods.length>worldUi.limit)methodHost.appendChild(button(t('More methods','Weitere Methoden'),()=>{worldUi.limit+=20;worldDetails();},'btn btn-secondary'));
      const row=worldKpiRow(entity);if(row){const metrics=element('details');metrics.appendChild(element('summary',t('Other class metrics','Weitere Klassenkennzahlen')));for(const key of WORLD_CLASS_KPIS)metrics.appendChild(explainMetric(element('p',key.toUpperCase()+': '+engineeringNumber(classMetricValue(row,key))),key));section.appendChild(metrics);}
    }else{
      const measured=members.map(worldEntityValue).filter(Number.isFinite);section.appendChild(element('p',t('{measured}/{total} entries measured · maximum {metric}: {value}','{measured}/{total} Einträge gemessen · Maximum {metric}: {value}',{measured:measured.length,total:members.length,metric:worldKpiLabel(),value:measured.length?measured.reduce((max,value)=>Math.max(max,value),0):'—'})));
      section.appendChild(element('p',t('Select a class below for its methods and measurements.','Unten eine Klasse auswählen, um ihre Methoden und Messwerte zu sehen.'),'muted'));
    }
  }else if(worldUi.lens==='coverage'){
    const files=new Map(members.map(e=>[e.node,e])),measured=[...files.values()].filter(e=>e.coverage?.lines),covered=measured.reduce((sum,e)=>sum+e.coverage.lines.covered,0),total=measured.reduce((sum,e)=>sum+e.coverage.lines.total,0);
    section.appendChild(element('p',measured.length?t('File coverage: {covered}/{total} lines · {gaps} uncovered','Dateiabdeckung: {covered}/{total} Zeilen · {gaps} nicht abgedeckt',{covered,total,gaps:Math.max(0,total-covered)}):t('File coverage: unknown','Dateiabdeckung: unbekannt')));
    section.appendChild(element('p',t('{measured}/{total} source files measured. Coverage is file-level, not class- or method-level; import revision is unverified.','{measured}/{total} Quelldateien gemessen. Abdeckung gilt für Dateien, nicht einzelne Klassen oder Methoden; Importrevision ungeprüft.',{measured:measured.length,total:files.size}),'muted'));
    if(entity){const source=nodeByName(data,entity.node);if(source)renderTestCoverageDetails(section,source.id);}
  }else{
    section.appendChild(element('p',area.cyclic?t('The containing package belongs to a package cycle. This does not establish a class-level cycle.','Das zugehörige Paket liegt in einem Paketzyklus. Das belegt keinen Zyklus dieser Klasse.'):t('No package cycle recorded for this area.','Kein Paketzyklus für diesen Bereich erfasst.')));
    section.appendChild(element('p',t('{count} findings in the containing area','{count} Befunde im zugehörigen Bereich',{count:area.findings.length})));
    for(const finding of area.findings.slice(0,worldUi.limit))section.appendChild(element('p',finding.message||finding.kind));
  }
  if(!entity){const sorted=[...members].sort((a,b)=>worldUi.lens==='complexity'?(worldEntityValue(b)??-1)-(worldEntityValue(a)??-1)||compareNames(a.class_id||a.node,b.class_id||b.node):compareNames(a.class_id||a.node,b.class_id||b.node)),list=element('div',undefined,'world-members');section.appendChild(list);
    for(const row of sorted.slice(0,worldUi.limit)){const label=(row.class_id||row.node)+(worldUi.lens==='complexity'?' · '+worldKpiLabel()+': '+(worldEntityValue(row)??'—'):''),b=button(label,()=>{worldUi.selection={area:area.name,entity:row};worldUi.limit=20;worldDetails();worldDraw();},'btn btn-secondary');b.setAttribute('data-world-entity',row.id);list.appendChild(b);}if(sorted.length>worldUi.limit)list.appendChild(button(t('More ({count})','Weitere ({count})',{count:sorted.length-worldUi.limit}),()=>{worldUi.limit+=20;worldDetails();},'btn btn-secondary'));
  }
}

function worldDetails(){
  const panel=document.getElementById('world-detail'),previousKey=worldUi.detailsKey,previousContext=worldUi.detailsContext,scroll=panel.scrollTop,openSections=new Set([...panel.querySelectorAll('details')].filter(n=>n.open).map(n=>n.querySelector('summary')?.textContent));hideMetricTooltip(panel);panel.textContent='';const selection=worldUi.selection,area=worldUi.model.areas.find(a=>a.name===selection?.area);
  if(isRepositoryOverview()){repositoryWorldDetails(panel);return;}
  worldStatus();
  const key=area?JSON.stringify([area.name,selection.entity?.id||selection.entity?.node||null]):null;
  if(key!==worldUi.detailsKey&&(selection?.entity||area?.kind==='file'))worldUi.detailsOpen=true;
  worldUi.detailsKey=key;
  if(worldUi.detailButton){worldUi.detailButton.disabled=!area;worldUi.detailButton.textContent=t('Details','Details');worldUi.detailButton.setAttribute('aria-expanded',String(!!area&&!!worldUi.detailsOpen));}
  if(!area){panel.hidden=true;worldUi.detailsOpen=false;return;}panel.hidden=!worldUi.detailsOpen;
  panel.setAttribute('role','dialog');panel.setAttribute('aria-modal','false');panel.setAttribute('aria-label',t('Selection details','Details zur Auswahl'));panel.setAttribute('tabindex','-1');worldDetailPosition();
  const title=element('div',undefined,'world-detail-title');title.appendChild(element('strong',selection.entity?.class_id||selection.entity?.node||area.name));title.appendChild(button('×',()=>setWorldDetailsOpen(false,true),'btn btn-secondary'));title.lastChild?.setAttribute('aria-label',t('Close details (Esc)','Details schließen (Esc)'));panel.appendChild(title);
  const context=worldUi.lens+':'+worldUi.kpi;worldUi.detailsContext=context;
  renderWorldContext(panel,selection,area);
  const entity=selection.entity;
  const actions=element('div',undefined,'world-actions');panel.appendChild(actions);
  if(!entity&&area.kind==='module')actions.appendChild(button(t('Explore','Erkunden'),()=>worldOpenArea(area.name),'btn btn-primary'));
  actions.appendChild(button(t('Source graph','Quellgraph'),()=>{showTab('graph');revealNode(entity?.node||area.name);},'btn btn-secondary'));
  actions.appendChild(explainAction(button(t('Cycles','Zyklen'),()=>worldJumpCycle(entity?.node||area.name),'btn btn-secondary'),()=>t('Opens the cycles list, focusing a recorded cycle that contains this area when one exists.','Öffnet die Zyklusliste und fokussiert, wenn vorhanden, einen erfassten Zyklus, der diesen Bereich enthält.')));
  actions.appendChild(explainAction(button(t('Matrix','Matrix'),()=>showTab('dsm'),'btn btn-secondary'),()=>t('Opens the dependency matrix of the current graph view.','Öffnet die Abhängigkeitsmatrix der aktuellen Graphansicht.')));
  if(entity?.class_id){
    actions.appendChild(button(t('Calls','Aufrufe'),()=>openClassMetricCalls(entity),'btn btn-secondary'));
    actions.appendChild(explainAction(button(t('Class metrics','Klassenkennzahlen'),()=>{engineeringUi.section='heatmap';engineeringUi.target='';engineeringUi.query=entity.class_id;showTab('quality');},'btn btn-secondary'),()=>t('Opens Software quality with this class as the search term.','Öffnet Softwarequalität mit dieser Klasse als Suchbegriff.')));
  }else if(area.kind==='module')actions.appendChild(explainAction(button(t('Quality scope','Qualitätsumfang'),()=>{engineeringUi.section='cohesion';engineeringUi.target=area.name;showTab('quality');},'btn btn-secondary'),()=>t('Opens Software quality scoped to this package.','Öffnet Softwarequalität eingeschränkt auf dieses Paket.')));
  actions.appendChild(button(t('Copy path','Pfad kopieren'),()=>copyText(worldCopyValue(selection,area)),'btn btn-secondary'));
  actions.appendChild(explainAction(button(t('AI context','KI-Kontext'),()=>copyText(JSON.stringify({project:data.root,selection:entity?.class_id||area.name,tools:[{name:'architecture_world',arguments:{path:data.root,within:entity?.node||area.name,expected_revision:data.insights.world.revision,...(entity?.class_id?{query:entity.class_id}:{})}},{name:'architecture_atlas',arguments:{path:data.root,within:area.kind==='module'?area.name:worldUi.model.scope}},{name:'architecture_findings',arguments:{path:data.root,query:entity?.node||area.name}}]},null,2)),'btn btn-secondary'),()=>t('Copy read-only MCP requests for the exact source selection.','Lesende MCP-Abfragen für diese Quellauswahl kopieren.')));
  const evidence=element('details');evidence.setAttribute('data-world-evidence','true');evidence.appendChild(element('summary',t('Evidence','Belege')));panel.appendChild(evidence);
  const edges=worldUi.model.connections.filter(e=>e.from===area.name||e.to===area.name);
  for(const edge of edges.slice(0,worldUi.limit))evidence.appendChild(element('p',edge.from+' → '+edge.to+(edge.cyclic?' · '+t('package cycle','Paketzyklus'):'')));
  const findings=area.findings;
  for(const finding of findings.slice(0,worldUi.limit)){evidence.appendChild(element('p',finding.message||finding.kind));for(const ref of (finding.evidence||[]).slice(0,5))evidence.appendChild(element('p',ref.file+':'+ref.line+' '+(ref.raw||''),'muted'));}
  if(!edges.length&&!findings.length)evidence.appendChild(element('p',t('No connections or findings recorded at this level.','Keine Verbindungen oder Befunde auf dieser Ebene erfasst.'),'muted'));
  if(edges.length>worldUi.limit||findings.length>worldUi.limit)evidence.appendChild(button(t('More evidence','Weitere Belege'),()=>{worldUi.limit+=20;worldDetails();document.getElementById('world-detail').querySelector('details[data-world-evidence]').open=true;},'btn btn-secondary'));
  const background=element('details');background.setAttribute('data-world-background','true');background.appendChild(element('summary',t('Source dependencies and further evidence','Quellabhängigkeiten und weitere Belege')));panel.appendChild(background);renderWorldSourceDetails(background,selection,area);
  if(previousKey===key&&previousContext===context){for(const section of panel.querySelectorAll('details'))if(openSections.has(section.querySelector('summary')?.textContent))section.open=true;panel.scrollTop=scroll;}else panel.scrollTop=0;
}
// Motion polish: button camera moves ease over frames and change arrows flow.
// Without requestAnimationFrame (the deterministic test harness) or with a
// reduced-motion preference, every move lands immediately and flow stays still.
const worldNow=()=>typeof performance!=='undefined'&&performance.now?performance.now():Date.now();
const worldEase=t=>t<.5?4*t*t*t:1-Math.pow(-2*t+2,3)/2;
function worldMotion(){return typeof requestAnimationFrame!=='undefined'&&!reducedMotion;}
const worldCameraAnim={token:0};
// Pointer and wheel events can outpace the display; they update the camera at
// once and paint at most once per frame. Without frames the paint is immediate.
let worldDrawFrame=null;
function worldRequestDraw(){
  if(typeof requestAnimationFrame==='undefined'){worldDraw();return;}
  if(worldDrawFrame==null)worldDrawFrame=requestAnimationFrame(()=>{worldDrawFrame=null;worldDraw();});
}
function worldCameraTo(target,duration=320){
  const from={...worldUi.camera},to={...worldUi.camera,...target},start=worldNow(),token=++worldCameraAnim.token;
  if(!worldMotion()||duration<=0){worldUi.camera=to;worldDraw();return;}
  const fromRotation=from.rotation||0,delta=((to.rotation-fromRotation+Math.PI)%(Math.PI*2)+Math.PI*2)%(Math.PI*2)-Math.PI;
  const step=()=>{if(token!==worldCameraAnim.token)return;
    const t=Math.min(1,(worldNow()-start)/duration),e=worldEase(t);
    worldUi.camera={x:from.x+(to.x-from.x)*e,y:from.y+(to.y-from.y)*e,zoom:from.zoom+(to.zoom-from.zoom)*e,rotation:fromRotation+delta*e};
    worldDraw();
    if(t<1)requestAnimationFrame(step);};
  requestAnimationFrame(step);
}
let worldFlowFrame=null;
function worldFlowActive(){
  if(ui.workspace!=='start')return false;
  return isRepositoryOverview()&&repositoryWorldState.dataFlow;
}
function worldFlowStart(){
  if(worldFlowFrame!=null&&typeof cancelAnimationFrame!=='undefined')cancelAnimationFrame(worldFlowFrame);
  worldFlowFrame=null;
  if(!worldMotion())return;
  const step=()=>{worldFlowFrame=null;
    if(worldFlowActive()){
      worldUi.flow+=.03;worldDraw();worldFlowFrame=requestAnimationFrame(step);}};
  worldFlowFrame=requestAnimationFrame(step);
}
function worldDraw(){
  if(worldUi.bound)worldDetailPosition();
  const canvas=document.getElementById('world-canvas'),ctx=canvas.getContext?.('2d');if(!ctx||!worldUi.layout||ui.workspace!=='start')return;
  const box=canvas.getBoundingClientRect(),w=box.width,h=box.height;if(!w||!h)return;
  const dpr=Math.min(window.devicePixelRatio||1,2);if(canvas.width!==Math.round(w*dpr)||canvas.height!==Math.round(h*dpr)){canvas.width=Math.round(w*dpr);canvas.height=Math.round(h*dpr);}ctx.setTransform(dpr,0,0,dpr,0,0);ctx.clearRect(0,0,w,h);
  const css=getComputedStyle(document.documentElement),color=name=>css.getPropertyValue(name).trim(),bg=color('--panel'),line=color('--border'),muted=color(WORLD_TONES.unknown),accent=color(WORLD_TONES.neutral),red=color(WORLD_TONES.flagged),green=color(WORLD_TONES.covered);
  const {cells,areas}=worldUi.layout,points=areas.flatMap(a=>[{x:a.x,y:a.y},{x:a.x+a.w,y:a.y+a.h}]);
  if(!points.length){ctx.fillStyle=muted;ctx.font='14px system-ui';ctx.textAlign='center';ctx.fillText(t('No source areas','Keine Quellbereiche'),w/2,h/2);return;}
  let minx=Infinity,maxx=-Infinity,miny=Infinity,maxy=-Infinity;for(const p of points){minx=Math.min(minx,p.x);maxx=Math.max(maxx,p.x);miny=Math.min(miny,p.y);maxy=Math.max(maxy,p.y);}const maxZ=cells.reduce((n,c)=>Math.max(n,c.z),0);
  // A fixed enclosing radius keeps scale and the rotation center stable through a full turn.
  const radius=Math.max(1,Math.hypot(maxx-minx,maxy-miny)/2),base=Math.min((w-60)/(radius*Math.SQRT2*1.72),(h-150)/(radius*Math.SQRT2*.88+maxZ)),cx=(minx+maxx)/2,cy=(miny+maxy)/2,cam=worldUi.camera,rotation=cam.rotation||0;
  worldUi.groundLift=base*maxZ/2;
  const project=(x,y,z=0)=>{const p=worldProject(x-cx,y-cy,z,rotation);return {x:w/2+cam.x+p.x*base*cam.zoom,y:h/2+cam.y+(p.y+maxZ/2)*base*cam.zoom};};
  const poly=(ps,fill,stroke)=>{ctx.beginPath();ps.forEach((p,i)=>i?ctx.lineTo(p.x,p.y):ctx.moveTo(p.x,p.y));ctx.closePath();if(fill){ctx.fillStyle=fill;ctx.fill();}if(stroke){ctx.strokeStyle=stroke;ctx.lineWidth=.65;ctx.stroke();}};
  const byName=new Map(worldUi.model.areas.map(a=>[a.name,a]));
  // Ground plane: a faint lattice on the shared floor and a soft edge vignette
  // anchor the floating plates. Purely spatial decoration, no measurement.
  try{
    const vignette=ctx.createRadialGradient(w/2,h/2,Math.min(w,h)*.25,w/2,h/2,Math.max(w,h)*.72);
    if(vignette&&vignette.addColorStop){vignette.addColorStop(0,'rgba(0,0,0,0)');vignette.addColorStop(1,'rgba(0,0,0,.10)');ctx.fillStyle=vignette;ctx.fillRect(0,0,w,h);}
    ctx.globalAlpha=.45;ctx.strokeStyle=line;ctx.lineWidth=.5;
    for(let gx=Math.floor(minx);gx<=Math.ceil(maxx);gx+=2){const p=project(gx,miny-1,-.12),q=project(gx,maxy+1,-.12);ctx.beginPath();ctx.moveTo(p.x,p.y);ctx.lineTo(q.x,q.y);ctx.stroke();}
    for(let gy=Math.floor(miny);gy<=Math.ceil(maxy);gy+=2){const p=project(minx-1,gy,-.12),q=project(maxx+1,gy,-.12);ctx.beginPath();ctx.moveTo(p.x,p.y);ctx.lineTo(q.x,q.y);ctx.stroke();}
    ctx.globalAlpha=1;
  }catch{}
  for(const a of areas){const ps=[[a.x-.25,a.y-.25],[a.x+a.w+.25,a.y-.25],[a.x+a.w+.25,a.y+a.h+.25],[a.x-.25,a.y+a.h+.25]].map(([x,y])=>project(x,y,-.12));ctx.shadowColor='rgba(0,0,0,.22)';ctx.shadowBlur=16;ctx.shadowOffsetY=7;poly(ps,bg,(byName.get(a.name).cyclic||byName.get(a.name).findings.length)&&worldUi.lens==='problems'?red:line);ctx.shadowColor='transparent';ctx.shadowBlur=0;ctx.shadowOffsetY=0;}
  const heights=new Map(cells.map(c=>[c.x+','+c.y,c]));
  // Every cell uses the same interpolated boundary, including unknown values.
  // Measurement availability controls colour and peak height, not shared vertices.
  const corner=(x,y,area)=>{let sum=0;for(const dx of [-1,0])for(const dy of [-1,0]){const c=heights.get((x+dx)+','+(y+dy));sum+=c?.area===area?c.z:0;}return sum/4;};
  worldUi.hits=[];
  // Painter depth doubles as a distance cue: surfaces far up the plane recede
  // into the backdrop a little, which keeps large scopes readable when rotated.
  const depths=cells.map(c=>worldProject(c.x+.5,c.y+.5,0,rotation).y),minD=Math.min(...depths),maxD=Math.max(...depths);
  for(const cell of cells.slice().sort((a,b)=>worldProject(a.x+.5,a.y+.5,0,rotation).y-worldProject(b.x+.5,b.y+.5,0,rotation).y)){
    const {x,y,z}=cell,ps=[[x,y],[x+1,y],[x+1,y+1],[x,y+1]].map(([px,py])=>project(px,py,corner(px,py,cell.area)));
    let tone=muted;if(worldUi.lens==='problems')tone=accent;
    // Metric lenses reuse the class-heatmap cohort states; unknown measurements
    // stay the neutral surface colour instead of pretending to be zero.
    if(worldUi.lens==='complexity'&&cell.value!==null)tone=worldKpiActive()?color(WORLD_TONES[worldKpiCellState(cell)||'unknown']):worldComplexityColor(cell.value);
    if(worldUi.lens==='coverage'){const known=cell.entities.filter(e=>e.coverage?.lines);if(known.length===cell.entities.length&&known.length){const gap=Math.max(...known.map(e=>1-e.coverage.lines.covered/e.coverage.lines.total));tone=gap?red:green;}}
    const selected=worldUi.selection?.entity?cell.entities.some(e=>e.id===worldUi.selection.entity.id):cell.area===worldUi.selection?.area;
    // A search keeps matches readable by taking the others almost entirely out
    // of the picture instead of recolouring them into another legend entry.
    const fog=.72+.28*((worldProject(x+.5,y+.5,0,rotation).y-minD)/((maxD-minD)||1));
    ctx.globalAlpha=(worldUi.query&&!worldMatches(cell)?.12:cell.value===null&&worldUi.lens==='complexity'?.35:.85)*fog;
    for(const [i,j,dx,dy]of [[0,1,0,-1],[1,2,1,0],[2,3,0,1],[3,0,-1,0]])if(worldProject(dx,dy,0,rotation).y>1e-8&&heights.get((x+dx)+','+(y+dy))?.area!==cell.area){const corners=[[x,y],[x+1,y],[x+1,y+1],[x,y+1]],side=[ps[i],ps[j],project(...corners[j],-.12),project(...corners[i],-.12)];poly(side,tone,line);poly(side,'rgba(0,0,0,.16)',null);}
    ctx.globalAlpha=1;poly(ps,bg,null);ctx.globalAlpha=selected?.95:.64;poly(ps,tone,null);ctx.globalAlpha=1;
    const peak=project(x+.5,y+.5,z);
    // A soft light pool at the summit reads as a lit roof under the same
    // rotation as the facets; it carries no measurement of its own. Filling the
    // outline directly matches a clip without a viewport-sized pass per surface.
    try{const glow=ctx.createRadialGradient(peak.x,peak.y,0,peak.x,peak.y,Math.max(6,base*cam.zoom));
      if(glow&&glow.addColorStop){glow.addColorStop(0,'rgba(255,255,255,.18)');glow.addColorStop(1,'rgba(255,255,255,0)');
        ctx.globalAlpha=(selected?.95:.64)*fog;poly(ps,glow,null);ctx.globalAlpha=1;}}catch{}
    for(let i=0;i<4;i++){const triangle=[ps[i],ps[(i+1)%4],peak],light=Math.cos(rotation+(i-.5)*Math.PI/2);poly(triangle,tone,null);poly(triangle,light>0?`rgba(255,255,255,${.05+.14*light})`:`rgba(0,0,0,${.05-.18*light})`,line);}
    if(selected||cell===worldUi.hover){ctx.lineWidth=2;ctx.strokeStyle=color('--text');ctx.beginPath();ps.forEach((p,i)=>i?ctx.lineTo(p.x,p.y):ctx.moveTo(p.x,p.y));ctx.closePath();ctx.stroke();}
    worldUi.hits.push({cell,ps,triangles:ps.map((p,i)=>[p,ps[(i+1)%4],peak])});
  }
  if(worldUi.lens==='problems'){
    const visible=new Map(areas.map(a=>[a.name,a]));const edges=isRepositoryOverview()?repositoryVisibleConnections():worldUi.model.connections.filter(e=>e.cyclic);
    for(const [index,e] of edges.slice(0,60).entries()){
      const a=visible.get(e.from),b=visible.get(e.to);if(!a||!b)continue;
      const p=project(a.x+a.w/2,a.y+a.h/2,.2),q=project(b.x+b.w/2,b.y+b.h/2,.2),dx=q.x-p.x,dy=q.y-p.y,length=Math.max(1,Math.hypot(dx,dy)),bend=Math.min(55,length*.2);
      const control={x:(p.x+q.x)/2-dy/length*bend,y:(p.y+q.y)/2+dx/length*bend},tone=isRepositoryOverview()?(e.cyclic?red:e.to===worldUi.selection?.area?accent:green):red;
      ctx.strokeStyle=tone;ctx.globalAlpha=.85;ctx.lineWidth=1.7;
      // Event connections of the repository overview are dashed.
      if(isRepositoryOverview()&&e.kind==='event'&&ctx.setLineDash){ctx.setLineDash([7,5]);ctx.lineDashOffset=0;}
      ctx.beginPath();ctx.moveTo(p.x,p.y);ctx.quadraticCurveTo(control.x,control.y,q.x,q.y);ctx.stroke();if(ctx.setLineDash)ctx.setLineDash([]);
      const t=.72,s=1-t,tip={x:s*s*p.x+2*s*t*control.x+t*t*q.x,y:s*s*p.y+2*s*t*control.y+t*t*q.y};
      const angle=Math.atan2(s*(control.y-p.y)+t*(q.y-control.y),s*(control.x-p.x)+t*(q.x-control.x));
      poly([tip,{x:tip.x-8*Math.cos(angle-.45),y:tip.y-8*Math.sin(angle-.45)},{x:tip.x-8*Math.cos(angle+.45),y:tip.y-8*Math.sin(angle+.45)}],tone,null);
      if(isRepositoryOverview()&&repositoryWorldState.dataFlow&&worldMotion()&&worldFlowCarrier(e.kind))for(const packet of worldFlowPackets(p,control,q,worldUi.flow,index*.37)){
        ctx.globalAlpha=packet.alpha;ctx.fillStyle=tone;ctx.strokeStyle=bg;ctx.lineWidth=1;ctx.beginPath();ctx.arc(packet.x,packet.y,3.2,0,Math.PI*2);ctx.fill();ctx.stroke();
      }
    }ctx.globalAlpha=1;
  }
  const occupied=[];
  for(const id of ['world-orientation','world-fit']){const r=document.getElementById(id).parentElement?.getBoundingClientRect();if(r)occupied.push({x:r.left-box.left+r.width/2,y:r.top-box.top+r.height/2,w:r.width,h:r.height});}
  for(const b of document.getElementById('world-labels').querySelectorAll('button[data-world-area]')){
    const a=areas.find(a=>a.name===b.getAttribute('data-world-area'));if(!a)continue;
    const p=project(a.x+a.w/2,a.y+a.h/2,0);b.hidden=false;
    const bw=Math.min(180,b.offsetWidth||120),bh=b.offsetHeight||42;let position=null;
    if(p.x>=0&&p.x<=w&&p.y>=0&&p.y<=h)for(const dy of [0,-1,1,-2,2,-3,3,-4,4]){
      for(const dx of [0,-1,1]){
        const x=Math.max(bw/2+8,Math.min(w-bw/2-8,p.x+dx*(bw+8))),y=Math.max(bh/2+8,Math.min(h-bh/2-8,p.y-65+dy*(bh+8)));
        if(!occupied.some(o=>Math.abs(o.x-x)<(o.w+bw)/2+6&&Math.abs(o.y-y)<(o.h+bh)/2+6)){position={x,y,w:bw,h:bh};break;}
      }if(position)break;
    }
    b.hidden=!position;if(!position)continue;
    b.style.left=position.x+'px';b.style.top=position.y+'px';occupied.push(position);ctx.beginPath();ctx.moveTo(position.x,position.y+bh/2);ctx.lineTo(p.x,p.y);ctx.strokeStyle=muted;ctx.lineWidth=.7;ctx.stroke();
  }
  document.getElementById('world-zoom-value').textContent=Math.round(cam.zoom*100)+'%';
  document.getElementById('world-bearing').style.transform=`rotate(${rotation}rad)`;document.getElementById('world-angle').textContent=(Math.round(rotation*180/Math.PI)%360)+'°';
}
function initWorld(){
  const host=document.getElementById('start-world'),action=button(t('Details','Details'),()=>setWorldDetailsOpen(!worldUi.detailsOpen,true),'btn btn-secondary');
  action.setAttribute('data-world-details','true');action.setAttribute('aria-controls','world-detail');action.setAttribute('aria-keyshortcuts','Enter');worldUi.detailButton=action;
  (document.getElementById('world-export').parentElement||host).appendChild(action);
  host.appendChild(element('style','#start-world{position:relative}#start-world #world-detail{position:absolute;right:0;bottom:0;width:min(420px,calc(100% - 16px));box-sizing:border-box;z-index:8;overflow:auto;overscroll-behavior:contain;background:var(--panel);border:1px solid var(--border);box-shadow:-8px 0 28px #0003;padding:0 16px 16px}#world-detail .world-detail-title{position:sticky;top:0;z-index:2;background:var(--panel);padding:10px 0;border-bottom:1px solid var(--border);margin-bottom:14px}#world-detail .world-detail-title strong{overflow-wrap:anywhere}#world-detail .world-detail-title button{flex-shrink:0}#world-detail pre{white-space:pre-wrap;overflow-wrap:anywhere}'));
  worldUi.bound=true;const canvas=document.getElementById('world-canvas'),tip=document.getElementById('world-tip');
  let linkedScope='';try{linkedScope=decodeURIComponent(window.location?.hash?.match(/^#world=(.*)$/)?.[1]||'');}catch{}
  if(linkedScope&&data.nodes.some(n=>n.name===linkedScope&&n.kind==='module')){
    worldUi.homeScope=architectureAtlasModel(data).scope;worldUi.history.push({within:null,camera:{...worldUi.camera},selection:null,limit:20});worldUi.within=linkedScope;
  }
  const back=()=>{const previous=worldUi.history.pop();if(!previous){if(repositoryModel()?.mode==='project'&&repositoryLive())repositoryOpenWorld(null);return;}Object.assign(worldUi,previous);finishWorldNavigation();};
  const labels=document.getElementById('world-labels');let zoomIdle=null;
  const zoom=(factor,point={x:0,y:0})=>{
    ++worldCameraAnim.token;
    // Pins and tooltip would jump with every wheel tick; take them out of the
    // way for the gesture and let the last tick place them again.
    labels.hidden=true;if(zoomIdle)clearTimeout(zoomIdle);
    zoomIdle=setTimeout(()=>{zoomIdle=null;labels.hidden=false;worldDraw();},180);
    const zoomed=worldZoom(worldUi.camera,factor,point);
    // Reading a package at the 800 % maximum continues the exploration:
    // wheeling on opens it like a double-click, and navigating refits the view.
    const drill=factor>1&&zoomed.zoom>=8&&!drag&&worldUi.hover&&(isRepositoryOverview()?{name:worldUi.hover.area}:data.nodes.find(n=>n.name===worldUi.hover.area&&n.kind==='module'));
    // Zooming out at the minimum mirrors that: the opened package hands the
    // view back like Back does. Opening always stores the parent fitted
    // (zoom 1), so the next wheel-out above the minimum cannot cascade.
    const pop=factor<1&&zoomed.zoom<=.4&&!drag&&(worldUi.within||repositoryModel()?.mode==='project');
    worldUi.camera=zoomed;tip.hidden=true;worldRequestDraw();
    if(drill)worldOpenArea(drill.name);
    else if(pop)back();
  };
  // Buttons ease into their pose; the wheel keeps frame-exact control because
  // the gesture itself is the animation.
  document.getElementById('world-zoom-in').addEventListener('click',()=>worldCameraTo({zoom:Math.min(8,worldUi.camera.zoom*1.25)}));
  document.getElementById('world-zoom-out').addEventListener('click',()=>worldCameraTo({zoom:Math.max(.4,worldUi.camera.zoom*.8)}));
  const rotate=delta=>{worldUi.hover=null;tip.hidden=true;worldCameraTo({rotation:worldRotate(worldUi.camera,delta).rotation});};
  document.getElementById('world-rotate-left').addEventListener('click',()=>rotate(-Math.PI/12));document.getElementById('world-rotate-right').addEventListener('click',()=>rotate(Math.PI/12));
  document.getElementById('world-orientation').addEventListener('click',()=>rotate(-(worldUi.camera.rotation||0)));
  for(const [id,help] of [['world-rotate-left',()=>t('Rotate left · Q. Right-drag, Shift-drag or twist two fingers to rotate freely.','Links drehen · Q. Mit Rechtsziehen, Umschalt + Ziehen oder zwei Fingern frei drehen.')],['world-rotate-right',()=>t('Rotate right · E. Right-drag, Shift-drag or twist two fingers to rotate freely.','Rechts drehen · E. Mit Rechtsziehen, Umschalt + Ziehen oder zwei Fingern frei drehen.')],['world-orientation',()=>t('Restore the original orientation. Zoom and selection stay.','Ausgangsrichtung wiederherstellen. Zoom und Auswahl bleiben.')]])explainAction(document.getElementById(id),help);
  document.getElementById('world-fit').addEventListener('click',()=>worldCameraTo({x:0,y:0,zoom:1}));
  document.getElementById('world-reset').addEventListener('click',()=>{worldUi.within=null;worldUi.history=[];worldUi.selection=null;worldUi.limit=20;worldUi.camera={x:0,y:0,zoom:1,rotation:0};worldUi.lens='problems';worldUi.kpi='complexity';worldUi.query='';document.getElementById('world-search').value='';finishWorldNavigation();});
  const search=document.getElementById('world-search');
  search.addEventListener('input',e=>{worldUi.query=(e.target?.value??search.value).trim().toLowerCase();renderWorld();});
  search.addEventListener('keydown',e=>{if(e.key!=='Enter')return;const cell=worldUi.layout.cells.filter(worldMatches).sort((a,b)=>compareNames(a.area,b.area))[0];if(!cell)return;if(isRepositoryOverview()){repositorySelectWorld(cell.area);return;}worldUi.selection={area:cell.area,...(cell.entities.length===1?{entity:cell.entities[0]}:{members:cell.entities})};worldUi.limit=20;worldDetails();worldDraw();worldFlowStart();});
  document.getElementById('world-export').addEventListener('click',exportWorldImage);
  document.getElementById('world-fullscreen').addEventListener('click',event=>workspaceFullscreenToggle(event.currentTarget||event.target));
  document.getElementById('world-back').addEventListener('click',back);
  document.getElementById('world-area').addEventListener('change',e=>{if(isRepositoryOverview()){repositoryOpenWorld(e.target.value);return;}worldUi.selection=e.target.value?{area:e.target.value}:null;worldDetails();worldDraw();worldFlowStart();});
  for(const lens of ['problems','complexity','coverage'])document.getElementById('world-lens-'+lens).addEventListener('click',()=>{worldUi.lens=lens;renderWorld();worldFlowStart();});
  // Choosing a KPI is also choosing the lens it belongs to; the selector then
  // stays disabled in the other lenses instead of looking inert-but-live.
  document.getElementById('world-kpi').addEventListener('change',e=>{worldUi.kpi=['complexity',...WORLD_CLASS_KPIS].includes(e.target.value)?e.target.value:'complexity';worldUi.lens='complexity';renderWorld();worldFlowStart();});
  // The provider is re-read on every tooltip show, so hover, focus and click
  // always explain the KPI that is currently selected.
  bindActionHelp(document.getElementById('world-kpi'),document.getElementById('world-kpi-help'),worldKpiHelpText);
  explainAction(document.getElementById('world-help'),worldHelp);document.getElementById('world-help').addEventListener('click',()=>{showMetricTooltip(document.getElementById('world-help'));metricTooltipPinned=true;});
  canvas.addEventListener('wheel',e=>{e.preventDefault();if(pointers.size)return;const b=canvas.getBoundingClientRect(),delta=e.deltaY*(e.deltaMode===1?16:e.deltaMode===2?b.height:1);zoom(Math.exp(-Math.max(-180,Math.min(180,delta))*.005),{x:e.clientX-b.left-b.width/2,y:e.clientY-b.top-b.height/2});},{passive:false});
  const hit=e=>{const b=canvas.getBoundingClientRect(),p={x:e.clientX-b.left,y:e.clientY-b.top};return worldUi.hits.slice().reverse().find(h=>h.triangles.some(triangle=>worldContains(p,triangle)))?.cell;};
  const pointers=new Map();let drag=null,pair=null,suppressDoubleClick=false;
  const contact=e=>({id:e.pointerId,x:e.clientX,y:e.clientY,type:e.pointerType||'mouse'});
  const touchPair=()=>{
    const [a,b]=[...pointers.values()].filter(p=>p.type==='touch');if(!b)return null;
    const box=canvas.getBoundingClientRect(),dx=b.x-a.x,dy=b.y-a.y;
    return {center:{x:(a.x+b.x)/2-box.left-box.width/2,y:(a.y+b.y)/2-box.top-box.height/2},distance:Math.hypot(dx,dy),angle:Math.atan2(dy,dx)};
  };
  const setGestureLabelsHidden=hidden=>{if(zoomIdle)clearTimeout(zoomIdle);zoomIdle=null;labels.hidden=hidden;tip.hidden=true;worldUi.hover=null;};
  const release=id=>{if(!canvas.hasPointerCapture||canvas.hasPointerCapture(id))canvas.releasePointerCapture?.(id);};
  const endPointer=(e,cancelled=false)=>{
    if(!pointers.has(e.pointerId))return;
    const tap=!cancelled&&pointers.size===1&&drag?.id===e.pointerId&&!drag.moved&&!drag.rotate;
    const area=drag?.area;
    pointers.delete(e.pointerId);release(e.pointerId);pair=touchPair();
    const remaining=pointers.values().next().value;
    // Continuing with one finger starts from the current pose, never from the
    // position before the pinch; its eventual release cannot become a tap.
    drag=remaining?{...remaining,camera:{...worldUi.camera},rotate:false,moved:true}:null;
    if(!remaining){labels.hidden=false;worldDraw();}
    if(tap){
      if(area){if(isRepositoryOverview())repositoryTapWorld(area);else worldOpenArea(area);return;}
      const cell=hit(e);if(cell){if(isRepositoryOverview()){repositoryTapWorld(cell.area);return;}worldUi.selection={area:cell.area,...(cell.entities.length===1?{entity:cell.entities[0]}:{members:cell.entities})};worldUi.limit=20;worldDetails();worldDraw();worldFlowStart();}
    }
  };
  const down=e=>{
    if(e.button!==0&&e.button!==2)return;
    if(pointers.size&&[...pointers.values()].some(p=>p.type!=='touch'||e.pointerType!=='touch'))return;
    const area=e.target?.closest?.('[data-world-area]')?.getAttribute('data-world-area');
    if(e.target!==canvas&&!area)return;
    // Keep native mouse compatibility events on the terrain: cancelling its
    // primary pointerdown also prevents the browser's double-click event.
    if(e.pointerType==='touch'||e.button===2||area)e.preventDefault();
    ++worldCameraAnim.token;canvas.focus({preventScroll:true});
    const point=contact(e);pointers.set(e.pointerId,point);
    if(pointers.size===1){suppressDoubleClick=false;drag={...point,camera:{...worldUi.camera},rotate:e.button===2||e.shiftKey||e.ctrlKey,moved:false,area};}
    pair=touchPair();if(pair){drag.moved=true;suppressDoubleClick=true;setGestureLabelsHidden(true);}
    canvas.setPointerCapture?.(e.pointerId);tip.hidden=true;
  };
  const move=e=>{
    if(pointers.has(e.pointerId)){
      pointers.set(e.pointerId,contact(e));
      const next=touchPair();
      if(next&&pair){worldUi.camera=worldGesture(worldUi.camera,pair,next,worldUi.groundLift||0);pair=next;setGestureLabelsHidden(true);worldRequestDraw();return;}
      if(drag&&e.pointerId===drag.id){
        const x=e.clientX-drag.x,y=e.clientY-drag.y;if(Math.hypot(x,y)>4)drag.moved=true;
        // Rotation keeps package names available for orientation; worldDraw
        // reprojects their pins together with the terrain on every move.
        if(drag.moved){suppressDoubleClick=true;setGestureLabelsHidden(!drag.rotate);worldUi.camera=drag.rotate?worldRotate(drag.camera,x*.008):{...drag.camera,x:drag.camera.x+x,y:drag.camera.y+y};worldRequestDraw();}
      }
      return;
    }
    if(pointers.size||e.pointerType==='touch')return;
    const cell=hit(e);if(cell!==worldUi.hover){worldUi.hover=cell;worldRequestDraw();}tip.hidden=!cell;if(cell){const b=canvas.getBoundingClientRect(),row=cell.entities[0];tip.textContent=isRepositoryOverview()?cell.area+' · '+serviceProjectKind(row.project.kind):(cell.entities.length>1?cell.entities.length+' '+t('grouped entries','gebündelte Einträge'):row.class_id||row.node)+' · '+(worldUi.lens==='complexity'?worldKpiLabel():t('Complexity','Komplexität'))+': '+(cell.value??t('unknown','unbekannt'));tip.style.left=Math.max(8,Math.min(b.width-240,e.clientX-b.left+12))+'px';tip.style.top=Math.max(8,e.clientY-b.top-55)+'px';}
  };
  canvas.addEventListener('dblclick',e=>{if(e.button!==0||suppressDoubleClick)return;const cell=hit(e);if(cell){e.preventDefault();worldOpenArea(cell.area);}});
  canvas.addEventListener('contextmenu',e=>e.preventDefault());
  canvas.addEventListener('pointerdown',down);
  canvas.addEventListener('pointermove',move);
  canvas.addEventListener('pointerup',e=>endPointer(e));
  for(const event of ['pointercancel','lostpointercapture'])canvas.addEventListener(event,e=>endPointer(e,true));
  // Package pins are part of the touch surface too. Pointer capture transfers
  // their gestures to the canvas; a stationary tap still opens the package.
  labels.style.touchAction='none';labels.addEventListener('pointerdown',down);
  labels.addEventListener('dblclick',e=>{const path=e.target.closest?.('[data-world-area]')?.getAttribute('data-world-area');if(isRepositoryOverview()&&path&&!suppressDoubleClick)repositoryOpenWorld(path);});
  labels.addEventListener('contextmenu',e=>e.preventDefault());
  labels.addEventListener('click',e=>{if(e.detail>0){e.preventDefault();e.stopPropagation();}},true);
  window.addEventListener('blur',()=>{const ids=[...pointers.keys()];pointers.clear();drag=null;pair=null;for(const id of ids)release(id);labels.hidden=false;tip.hidden=true;});
  canvas.addEventListener('pointerleave',()=>{worldUi.hover=null;tip.hidden=true;});
  canvas.addEventListener('keydown',e=>{const key=e.key.toLowerCase();if(['+','=','-','0','q','e','arrowleft','arrowright','arrowup','arrowdown'].includes(key)){e.preventDefault();if(key==='+'||key==='=')zoom(1.25);else if(key==='-')zoom(.8);else if(key==='q'||key==='e')rotate((key==='q'?-1:1)*Math.PI/12);else if(key==='0')worldCameraTo({x:0,y:0,zoom:1});else{worldUi.camera.x+=key==='arrowleft'?40:key==='arrowright'?-40:0;worldUi.camera.y+=key==='arrowup'?40:key==='arrowdown'?-40:0;worldDraw();}}});
  if(typeof ResizeObserver!=='undefined')new ResizeObserver(()=>worldDraw()).observe(canvas);
}
