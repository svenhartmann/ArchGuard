'use strict';
// Review Dashboard A/I zone scatter (R38). Hand-rolled SVG: axes A
// (abstractness) and I (instability) from 0..1, the main-sequence diagonal
// A+I=1, shaded zone-of-pain / zone-of-uselessness regions and one point
// per AbstractionMetric with scope 'node' and a measured abstractness.
// Points come from the embedded payload data.performance.abstractions
// (archguard-core's performance.rs), the zone counts from review_kpis.zones.
// Honesty rule: unmeasured boundaries are never plotted and an absent
// measurement renders the empty state — never an empty chart with axes.

// Mirrored from archguard-analysis/src/zones.rs::zone_stats (R06):
// const MAIN_SEQUENCE_MAX_DISTANCE: f64 = 1.0 / 3.0; D' = |A + I − 1| is
// recomputed from A and I (the metric's `distance` field is not trusted);
// D' ≤ 1/3 → main sequence, otherwise A+I < 1 → zone of pain, A+I > 1 →
// zone of uselessness. zones.rs also requires coverage === 'complete' for
// a measurement; performance.rs only ever yields a Some abstractness for
// complete compiler data, so `abstractness != null` is the plot gate here.
const DASH_MAIN_SEQUENCE_MAX_DISTANCE=1/3;
// Same rounding tolerance as zones.rs: A=0, I=2/3 sits on the corridor edge.
const DASH_DISTANCE_TOLERANCE=1e-9;
function dashZoneOf(abstractness,instability){
  const sum=abstractness+instability;
  if(Math.abs(sum-1)<=DASH_MAIN_SEQUENCE_MAX_DISTANCE+DASH_DISTANCE_TOLERANCE)return 'main';
  return sum<1?'pain':'useless';
}
const DASH_ZONE_STYLE={
  pain:{fill:'var(--c-cycle)',label:()=>t('Zone of Pain','Pain-Zone')},
  useless:{fill:'var(--c-outgoing)',label:()=>t('Zone of Uselessness','Uselessness-Zone')},
  main:{fill:'var(--c-resolved)',label:()=>t('Main sequence','Hauptsequenz')},
};

function dashSvg(tag,attrs){const node=document.createElementNS('http://www.w3.org/2000/svg',tag);for(const [key,value]of Object.entries(attrs||{}))node.setAttribute(key,String(value));return node;}

function renderDashZones(container,kpis){
  dashClearPanel(container);
  const abstractions=data.performance?.abstractions||[];
  const points=abstractions.filter(metric=>metric.scope==='node'&&metric.abstractness!=null&&Number.isFinite(metric.abstractness)&&Number.isFinite(metric.instability));
  if(!kpis&&!points.length){
    // Neither KPI block nor compiler-backed A/I evidence — nothing to show.
    container.appendChild(element('p',t('Not measured — neither review KPIs nor compiler-backed A/I measurements are part of this analysis. Unmeasured boundaries are never plotted as points.','Nicht gemessen — weder Review-Kennzahlen noch compilerbasierte A/I-Messungen sind Teil dieser Analyse. Nicht gemessene Grenzen werden nie als Punkte eingezeichnet.'),'empty'));
    return;
  }
  // Zone distribution from the KPI payload; the unmeasured count stays visible.
  const zones=kpis?.zones;
  if(zones){
    container.appendChild(element('p',t('{pain} pain · {useless} uselessness · {main} main sequence · {unmeasured} unmeasured','{pain} Pain · {useless} Uselessness · {main} Hauptsequenz · {unmeasured} nicht gemessen',{pain:zones.zone_of_pain,useless:zones.zone_of_uselessness,main:zones.main_sequence,unmeasured:zones.unmeasured}),'dash-zone-counts'));
  }else container.appendChild(element('p',t('Zone distribution not measured — the KPI block carries no zone statistics.','Zonenverteilung nicht gemessen — die KPI-Payload enthält keine Zonen-Statistik.'),'muted'));
  if(!points.length){
    // The counts above are honest numbers; an axis frame without points
    // would claim a chart the evidence does not support.
    container.appendChild(element('p',t('No measurable A/I points in this report — unmeasured boundaries remain unmeasured, never plotted.','Keine messbaren A/I-Punkte in diesem Bericht — nicht gemessene Grenzen bleiben ungemessen und werden nie eingezeichnet.'),'empty'));
    return;
  }
  const left=44,top=12,plot=230,right=14,bottom=38;
  const x=i=>left+i*plot,y=a=>top+(1-a)*plot;
  const svg=dashSvg('svg',{viewBox:`0 0 ${left+plot+right} ${top+plot+bottom}`,role:'img','aria-label':t('Abstractness over instability with the Martin main sequence','Abstractness über Instabilität mit der Martin-Hauptsequenz'),'data-dash-zones':'true'});
  const svgTitle=dashSvg('title',{});svgTitle.appendChild(document.createTextNode(t('A/I zone scatter','A/I-Zonen-Streudiagramm')));svg.appendChild(svgTitle);
  // Shaded zones: the half-planes beyond the ±1/3 corridor (mirrored bounds).
  // Theme colours go through style="…" — var() never resolves in SVG
  // presentation attributes, so a fill="var(…)" attribute would drop them.
  const third=1/3,limit=1-DASH_MAIN_SEQUENCE_MAX_DISTANCE;
  svg.appendChild(dashSvg('polygon',{points:`${x(0)},${y(0)} ${x(limit)},${y(0)} ${x(0)},${y(limit)}`,style:'fill:var(--c-cycle);fill-opacity:0.12;stroke:none'}));
  svg.appendChild(dashSvg('polygon',{points:`${x(1)},${y(1)} ${x(third)},${y(1)} ${x(1)},${y(third)}`,style:'fill:var(--c-outgoing);fill-opacity:0.12;stroke:none'}));
  // Plot frame, main-sequence diagonal and the 0 / 0.5 / 1 ticks.
  svg.appendChild(dashSvg('rect',{x:left,y:top,width:plot,height:plot,style:'fill:none;stroke:var(--border)'}));
  svg.appendChild(dashSvg('line',{x1:x(0),y1:y(1),x2:x(1),y2:y(0),style:'stroke:var(--muted)','stroke-dasharray':'4 3'}));
  for(const tick of [0,0.5,1]){
    svg.appendChild(dashSvg('line',{x1:x(tick),y1:top+plot,x2:x(tick),y2:top+plot+4,style:'stroke:var(--muted)'}));
    svg.appendChild(dashSvg('line',{x1:left-4,y1:y(tick),x2:left,y2:y(tick),style:'stroke:var(--muted)'}));
    const xl=dashSvg('text',{x:x(tick),y:top+plot+16,'text-anchor':'middle',style:'fill:var(--muted)','font-size':'10'});xl.appendChild(document.createTextNode(String(tick)));svg.appendChild(xl);
    const yl=dashSvg('text',{x:left-7,y:y(tick)+3,'text-anchor':'end',style:'fill:var(--muted)','font-size':'10'});yl.appendChild(document.createTextNode(String(tick)));svg.appendChild(yl);
  }
  const xa=dashSvg('text',{x:left+plot/2,y:top+plot+32,'text-anchor':'middle',style:'fill:var(--muted)','font-size':'11'});xa.appendChild(document.createTextNode(t('Instability I','Instabilität I')));svg.appendChild(xa);
  const ya=dashSvg('text',{x:12,y:top+plot/2,'text-anchor':'middle',style:'fill:var(--muted)','font-size':'11',transform:`rotate(-90 12 ${top+plot/2})`});ya.appendChild(document.createTextNode(t('Abstractness A','Abstractness A')));svg.appendChild(ya);
  // One point per measured node boundary; click/Enter selects it in the graph.
  for(const metric of points){
    const zone=dashZoneOf(metric.abstractness,metric.instability);
    const circle=dashSvg('circle',{cx:x(metric.instability),cy:y(metric.abstractness),r:4.5,style:`fill:${DASH_ZONE_STYLE[zone].fill}`,'class':'dash-zone-point','data-dash-zone':zone,'data-node':metric.name,tabindex:'0','data-dash-point':metric.name});
    const title=dashSvg('title',{});
    title.appendChild(document.createTextNode(metric.name+'\nA='+metric.abstractness.toFixed(2)+' · I='+metric.instability.toFixed(2)+" · D'="+Math.abs(metric.abstractness+metric.instability-1).toFixed(2)));
    circle.appendChild(title);
    const select=()=>{showTab('graph');revealNode(metric.name);};
    circle.addEventListener('click',select);
    circle.addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();select();}});
    svg.appendChild(circle);
  }
  container.appendChild(svg);
  const legend=element('div',undefined,'dash-zone-legend');
  for(const key of ['pain','useless','main']){
    const entry=element('span');const swatch=element('i');swatch.style.background=DASH_ZONE_STYLE[key].fill;entry.appendChild(swatch);
    entry.appendChild(document.createTextNode(DASH_ZONE_STYLE[key].label()+' ('+points.filter(metric=>dashZoneOf(metric.abstractness,metric.instability)===key).length+')'));
    legend.appendChild(entry);
  }
  container.appendChild(legend);
}
