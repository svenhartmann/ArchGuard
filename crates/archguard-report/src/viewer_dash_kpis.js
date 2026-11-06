'use strict';
// Start KPI cards (R37). Renders the headline KPIs of the
// ReviewKpis document (archguard-core's review_kpi.rs) as a card grid into
// the Start overview (viewer_development.js). Field names below are the embedded JSON keys of that
// struct: propagation_cost, sdp_violations, tangledness, hub_gini, debt,
// leaks, risk_gaps, health. Honesty rule: a null KPI means NOT MEASURED —
// the card shows that text, never an invented zero. Cards with a real
// graph identity select it through the same entry point the review rows
// use (showTab('graph') + revealNode, viewer.js); cards without one stay
// plain. Read-only: this panel never edits code or decisions.

// Shared with viewer_dash_zones.js: empties a KPI panel down to its
// static <h3> heading so re-renders (tab switch, language change) replace
// the dynamic content without duplicating it.
function dashClearPanel(container){for(const child of [...container.children])if(child.tagName!=='H3')child.remove();}
function dashNotMeasured(){return t('not measured','nicht gemessen');}
function dashPercent(value){return value==null?dashNotMeasured():(Math.round(value*1000)/10)+'%';}
function dashFixed(value,digits=2){return value==null?dashNotMeasured():(Math.round(value*10**digits)/10**digits).toFixed(digits);}
function dashHealthLevel(value){return ({green:t('Green','Grün'),yellow:t('Yellow','Gelb'),red:t('Red','Rot')})[value]||value;}

// One card: big value, label, optional sub-line and tooltip. With nodeName
// the card becomes keyboard-focusable and selects that node on click or
// Enter (same navigation as the review rows in viewer.js).
function dashCard(label,value,tip,sub,nodeName,level){
  const card=element('div',undefined,'dash-card');
  if(level)card.setAttribute('data-level',level);
  card.appendChild(element('strong',String(value)));
  card.appendChild(element('span',label));
  if(sub)card.appendChild(element('small',sub,'dash-card-sub'));
  if(tip)card.title=tip;
  if(nodeName){
    card.setAttribute('role','button');card.setAttribute('tabindex','0');card.setAttribute('data-node',nodeName);
    const select=()=>{showTab('graph');revealNode(nodeName);};
    card.addEventListener('click',select);
    card.addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();select();}});
  }
  return card;
}

function renderDashKpis(container,kpis){
  dashClearPanel(container);
  if(!kpis){
    // No KPI pass in this analysis — every card would be a guess.
    container.appendChild(element('p',t('Not measured — this analysis carries no review KPIs. Re-run archguard analyze with the KPI pipeline; absent values are never shown as zero.','Nicht gemessen — diese Analyse enthält keine Review-Kennzahlen. archguard analyze mit der KPI-Pipeline erneut ausführen; fehlende Werte werden nie als null angezeigt.'),'empty'));
    return;
  }
  const grid=element('div',undefined,'dash-cards');container.appendChild(grid);
  // Propagation cost: the highest-impact SCC (deterministic: pc, then id).
  const top=(kpis.propagation_cost||[]).slice().sort((a,b)=>(b.pc??0)-(a.pc??0)||compareNames(a.scc_id,b.scc_id))[0];
  if(top){
    const cycle=(data.findings||[]).find(f=>f.kind==='cycle'&&f.id===top.scc_id);
    grid.appendChild(dashCard(t('Propagation cost (top cycle)','Propagationskosten (Top-Zyklus)'),dashPercent(top.pc),
      t('Share of nodes reachable from cycle {id}: {affected} of {total} nodes ({size} in the cycle).','Anteil der von Zyklus {id} erreichbaren Knoten: {affected} von {total} ({size} im Zyklus).',{id:top.scc_id,affected:top.affected,total:top.total,size:top.size}),
      top.scc_id+' · '+top.affected+'/'+top.total,cycle?cycle.nodes[0]:undefined));
  }else grid.appendChild(dashCard(t('Propagation cost (top cycle)','Propagationskosten (Top-Zyklus)'),t('None recorded','Nicht erfasst'),t('The payload carries no propagation-cost entries — that is the recorded state, not a measurement of zero.','Die Payload enthält keine Propagationskosten-Einträge — das ist der erfasste Stand, keine Messung von null.')));
  // SDP violations: count with the first offending edge as navigation.
  const sdp=(kpis.sdp_violations||[])[0];
  grid.appendChild(dashCard(t('SDP violations','SDP-Verstöße'),String((kpis.sdp_violations||[]).length),
    sdp?t('A stable module imports a less stable one: {from} → {to}.','Ein stabiles Modul importiert ein instabileres: {from} → {to}.',{from:sdp.from,to:sdp.to}):t('No Stable Dependencies Principle violations recorded.','Keine Verstöße gegen das Stable Dependencies Principle erfasst.'),
    sdp?sdp.from+' → '+sdp.to:undefined,sdp?sdp.from:undefined));
  // Tangledness: share of nodes inside cyclic SCCs (null = not computable).
  const tangled=kpis.tangledness;
  grid.appendChild(dashCard(t('Tangledness','Verflechtung'),tangled?dashPercent(tangled.quote):dashNotMeasured(),
    tangled?t('{tangled} of {total} nodes live in a cyclic group.','{tangled} von {total} Knoten liegen in einer zyklischen Gruppe.',{tangled:tangled.tangled_nodes,total:tangled.total}):t('The quote could not be computed — not measured, never zero.','Die Quote ließ sich nicht berechnen — nicht gemessen, nie null.'),
    tangled?tangled.tangled_nodes+' / '+tangled.total:undefined));
  // Hub concentration: Gini of the fan-in distribution.
  const gini=kpis.hub_gini;
  grid.appendChild(dashCard(t('Hub Gini','Hub-Gini'),gini?dashFixed(gini.gini):dashNotMeasured(),
    gini?t('Inequality of the fan-in distribution (0 equal … 1 single hub); top-5 hubs take {share} of all edges.','Ungleichheit der Fan-in-Verteilung (0 gleich … 1 einzelner Hub); die Top-5-Hubs halten {share} aller Kanten.',{share:dashPercent(gini.top_share)}):t('Not computable from this analysis — not measured, never zero.','Aus dieser Analyse nicht berechenbar — nicht gemessen, nie null.'),
    gini?t('top share {share}','Top-Anteil {share}',{share:dashPercent(gini.top_share)}):undefined));
  // Health: score, Ampel level and the trend delta against the last snapshot.
  const health=kpis.health;
  const score=health?.score,level=health?.level,trend=health?.trend;
  const parts=health&&health.parts?Object.entries(health.parts).map(([key,value])=>key+' '+dashFixed(value,2)).join(' · '):undefined;
  grid.appendChild(dashCard(t('Health','Gesundheit'),score==null?dashNotMeasured():(Math.round(score*10)/10)+(level?' · '+dashHealthLevel(level):''),
    health?t('Composite index 0–100 from the measurable KPI parts; absent parts are excluded, never zeroed.','Zusammengesetzter Index 0–100 aus den messbaren KPI-Teilen; fehlende Teile entfallen, werden nie null gesetzt.'):t('No measurable KPI part — the index is not measured, never zero.','Kein messbarer KPI-Teil — der Index ist nicht gemessen, nie null.'),
    (trend==null?'':(t('trend','Trend')+' '+(trend>=0?'+':'')+dashFixed(trend,1)+' ')+t('vs. previous snapshot','ggü. vorherigem Snapshot'))+(parts?' · '+parts:''),undefined,score==null?undefined:level));
  // Debt markers: density per kloc (null without a scanned loc).
  const debt=kpis.debt;
  grid.appendChild(dashCard(t('Debt markers','Schuld-Marker'),debt?String(debt.markers):dashNotMeasured(),
    debt?t('{markers} TODO/FIXME/HACK/XXX markers in {loc} source lines.','{markers} TODO/FIXME/HACK/XXX-Marker in {loc} Quellzeilen.',{markers:debt.markers,loc:debt.loc}):t('No debt scan ran — not measured, never zero.','Kein Schulden-Scan lief — nicht gemessen, nie null.'),
    debt?(debt.per_kloc==null?t('per kloc not measured','pro kloc nicht gemessen'):dashFixed(debt.per_kloc,1)+' '+t('per kloc','pro kloc')):undefined));
  // Leaky encapsulation: count with the first leaking file as navigation.
  const leak=(kpis.leaks||[])[0];
  grid.appendChild(dashCard(t('Leaks','Lecks'),String((kpis.leaks||[]).length),
    leak?t('{from} reaches {symbol}, which {in_file} does not export.','{from} nutzt {symbol}, das {in_file} nicht exportiert.',{from:leak.from_file,symbol:leak.symbol,in_file:leak.in_file}):t('No leaky-encapsulation findings recorded.','Keine Leck-Funde erfasst.'),
    leak?leak.from_file+' → '+leak.symbol:undefined,leak?leak.from_file:undefined));
  // Risk-weighted coverage gaps: count with the first gap path as navigation.
  const gap=(kpis.risk_gaps||[])[0];
  grid.appendChild(dashCard(t('Risk coverage gaps','Risiko-Abdeckungslücken'),String((kpis.risk_gaps||[]).length),
    gap?t('Hotspot {hotspot} with {coverage} uncovered — the gap stays null when coverage is unknown.','Hotspot {hotspot} mit {coverage} ungedeckten Zeilen — bei unbekannter Abdeckung bleibt die Lücke null.',{hotspot:dashFixed(gap.hotspot,1),coverage:gap.coverage==null?dashNotMeasured():dashPercent(1-gap.coverage)}):t('No risk-weighted coverage gaps recorded.','Keine risiko-gewichteten Abdeckungslücken erfasst.'),
    gap?gap.path:undefined,gap?gap.path:undefined));
}
