'use strict';
// Review Dashboard health panel (R39). Ampel, score, trend delta and the
// KPI parts from review_kpis.health (archguard-core's review_kpi.rs),
// plus a hand-rolled SVG sparkline through the embedded health series
// data.reviewHistory — one {timestamp, tag, score} row per snapshot,
// oldest first (review_cmd.rs reads it from read_snapshots).
// Honesty rule: a null score is NOT MEASURED — the panel renders that
// text, never an invented zero. Pre-R28 history files carry no KPIs at
// all, so their rows plot as absent points, not as zeros.

const DASH_HEALTH_COLOR={green:'var(--c-resolved)',yellow:'var(--c-priority)',red:'var(--c-cycle)'};

function dashHealthDate(row){
  return Number.isFinite(row.timestamp)?new Date(row.timestamp*1000).toISOString():'?';
}

function renderDashHealth(container,kpis){
  dashClearPanel(container);
  const health=kpis?.health;
  if(!health){
    // No health index in this analysis — every number below would be a guess.
    container.appendChild(element('p',t('Not measured — this analysis carries no health index. Re-run archguard analyze with the KPI pipeline; absent values are never shown as zero.','Nicht gemessen — diese Analyse enthält keinen Gesundheits-Index. archguard analyze mit der KPI-Pipeline erneut ausführen; fehlende Werte werden nie als null angezeigt.'),'empty'));
  }else{
    const head=element('div',undefined,'dash-health-head');
    const dot=element('span',undefined,'dash-health-dot');
    if(health.level){dot.setAttribute('data-level',health.level);dot.setAttribute('title',dashHealthLevel(health.level));dot.style.background=DASH_HEALTH_COLOR[health.level]||'var(--muted)';}
    head.appendChild(dot);
    head.appendChild(element('strong',health.score==null?dashNotMeasured():dashFixed(health.score)));
    if(health.level)head.appendChild(element('span',dashHealthLevel(health.level),'dash-health-level'));
    if(health.trend!=null){
      // Higher score is better: up is resolved-green, down is cycle-red.
      const delta=element('span',(health.trend>=0?'▲ +':'▼ ')+dashFixed(health.trend,1),'dash-health-trend');
      delta.style.color=health.trend>=0?'var(--c-resolved)':'var(--c-cycle)';
      delta.title=t('Delta against the previous snapshot; a higher score is better.','Delta zum vorherigen Snapshot; ein höherer Score ist besser.');
      head.appendChild(delta);
    }
    container.appendChild(head);
    const parts=Object.entries(health.parts||{}).sort(([a],[b])=>a<b?-1:a>b?1:0);
    if(parts.length){
      const list=element('ul',undefined,'dash-health-parts');
      for(const [name,value] of parts)list.appendChild(element('li',name+' '+dashFixed(value)));
      container.appendChild(list);
    }else container.appendChild(element('p',t('The index carries no measurable parts — not measured, never zero.','Der Index enthält keine messbaren Teile — nicht gemessen, nie null.'),'muted'));
  }
  renderDashHealthSparkline(container);
}

function renderDashHealthSparkline(container){
  // Only measured rows are plotted; null scores (pre-R28 baselines) drop out.
  const rows=(data.reviewHistory||[]).filter(row=>Number.isFinite(row?.score));
  if(rows.length<2){
    container.appendChild(element('p',t('Trend needs at least two measured snapshots.','Verlauf braucht mindestens zwei gemessene Schnitte.'),'empty'));
    return;
  }
  const left=34,top=8,plotW=250,plotH=48,bottom=20;
  const scores=rows.map(row=>row.score);
  const min=Math.min(...scores),max=Math.max(...scores);
  // Oldest first on x; y spans min..max. An equal series has no range to
  // scale — a centred flat line, never a division by zero.
  const x=i=>left+i*plotW/(rows.length-1);
  const y=s=>min===max?top+plotH/2:top+(1-(s-min)/(max-min))*plotH;
  const svg=dashSvg('svg',{viewBox:`0 0 ${left+plotW+10} ${top+plotH+bottom}`,role:'img','aria-label':t('Health score over measured snapshots','Gesundheits-Score über die gemessenen Schnitte'),'data-dash-sparkline':'true'});
  const svgTitle=dashSvg('title',{});svgTitle.appendChild(document.createTextNode(t('Health score per snapshot, oldest first','Gesundheits-Score pro Snapshot, ältester zuerst')));svg.appendChild(svgTitle);
  svg.appendChild(dashSvg('line',{x1:left,y1:top+plotH,x2:left+plotW,y2:top+plotH,style:'stroke:var(--border)'}));
  svg.appendChild(dashSvg('polyline',{points:rows.map((row,i)=>`${x(i)},${y(row.score)}`).join(' '),style:'fill:none;stroke:var(--c-node);stroke-width:2'}));
  for(const i of [0,rows.length-1]){
    const row=rows[i];
    const circle=dashSvg('circle',{cx:x(i),cy:y(row.score),r:3,style:'fill:var(--c-node)'});
    const tip=dashSvg('title',{});
    tip.appendChild(document.createTextNode(dashHealthDate(row)+' · '+dashFixed(row.score)+(row.tag?' · '+row.tag:'')));
    circle.appendChild(tip);svg.appendChild(circle);
  }
  // Axis labels from the first and last measured timestamps.
  for(const i of [0,rows.length-1]){
    const label=dashSvg('text',{x:x(i),y:top+plotH+14,'text-anchor':i===0?'start':'end',style:'fill:var(--muted)','font-size':'10'});
    label.appendChild(document.createTextNode(dashHealthDate(rows[i]).slice(0,10)));
    svg.appendChild(label);
  }
  container.appendChild(svg);
}
