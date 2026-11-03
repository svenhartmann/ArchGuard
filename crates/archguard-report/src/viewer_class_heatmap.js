'use strict';
const CLASS_METRICS=['cbo','rfc','dit','noc','wmc','lcom4'];
const classHeatmapUi={filter:'all',sort:'priority',selected:null,source:null,cohorts:new Map()};
function classMetricValue(row,key){
  const m=row[key],value=key==='lcom4'?m:m?.value;
  return Number.isFinite(value)&&value>=0&&(key==='lcom4'||!m.status||m.status==='ready')?value:null;
}
function classMetricLanguage(row){
  const ext=(row.file||'').split('.').pop().toLowerCase();
  return ({go:'go',java:'java',cs:'csharp',ts:'typescript',tsx:'typescript',js:'javascript',jsx:'javascript',mjs:'javascript',cjs:'javascript'})[ext]||'unknown';
}
function classMetricCohorts(){
  const source=engineeringData('code_metrics');if(classHeatmapUi.source===source)return classHeatmapUi.cohorts;
  classHeatmapUi.source=source;const cohorts=new Map();
  for(const row of source?.classes||[])for(const key of CLASS_METRICS){const value=classMetricValue(row,key);if(value===null)continue;const id=classMetricLanguage(row)+':'+key;if(!cohorts.has(id))cohorts.set(id,[]);cohorts.get(id).push(value);}
  for(const values of cohorts.values())values.sort((a,b)=>a-b);
  classHeatmapUi.cohorts=cohorts;return cohorts;
}
function classMetricAssessment(row,key){
  const value=classMetricValue(row,key),language=classMetricLanguage(row),values=classMetricCohorts().get(language+':'+key)||[];
  const result={value,state:row[key]?.status==='not_applicable'?'not_applicable':'unknown',population:values.length,language};
  if(value===null)return result;
  if(key==='noc')return {...result,state:'info'};
  if(value===0||(key==='lcom4'&&value===1))return {...result,state:'low'};
  if(language==='unknown'||values.length<5)return {...result,state:'small'};
  if(values[0]===values[values.length-1])return {...result,state:'equal'};
  // Strictly smaller values give equal measurements the same rank. The full
  // language cohort stays fixed when users narrow the visible class list.
  let lo=0,hi=values.length;while(lo<hi){const mid=(lo+hi)>>>1;if(values[mid]<value)lo=mid+1;else hi=mid;}
  const rank=lo/(values.length-1);
  return {...result,state:rank>=.8?'high':rank>=.5?'medium':'low'};
}
function classMetricStateLabel(state){return ({not_applicable:t('Not applicable','Nicht anwendbar'),low:t('Low','Niedrig'),medium:t('Elevated','Erhöht'),high:t('High','Hoch'),unknown:t('Unknown','Unbekannt'),small:t('Few comparisons','Wenig Vergleichsdaten'),equal:t('Equal values','Gleiche Werte'),info:t('Informational','Information')})[state];}
function classMetricInterpretation(row,key){
  if(row[key]?.status==='not_applicable')return t('Not applicable · Go embedding is composition, not inheritance.','Nicht anwendbar · Go-Embedding ist Komposition, keine Vererbung.');
  const a=classMetricAssessment(row,key);
  let text=classMetricStateLabel(a.state)+' · '+engineeringNumber(a.value)+'. ';
  if(a.state==='unknown')text+=t('Incomplete measurement. Observed lower bounds are not rated.','Unvollständige Messung. Beobachtete Untergrenzen werden nicht bewertet.');
  else if(a.state==='info')text+=t('More subclasses can reflect intended reuse; no good/bad rating.','Mehr Unterklassen können gewollte Wiederverwendung bedeuten; keine Gut-/Schlecht-Bewertung.');
  else text+=t('Comparison: {count} measured classes of the same source language. Search and package filters do not change the colors.','Vergleich: {count} gemessene Klassen derselben Quellsprache. Suche und Paketfilter ändern die Farben nicht.',{count:a.population});
  return text;
}
function classHeatmapPriority(row){return CLASS_METRICS.reduce((sum,key)=>sum+({high:2,medium:1}[classMetricAssessment(row,key).state]||0),0);}
function classHeatmapRows(rows){
  return rows.filter(row=>{
    const states=CLASS_METRICS.filter(k=>k!=='noc').map(k=>classMetricAssessment(row,k).state);
    return classHeatmapUi.filter==='high'?states.includes('high'):classHeatmapUi.filter==='low'?states.every(s=>s==='low'):classHeatmapUi.filter==='unknown'?CLASS_METRICS.some(k=>classMetricAssessment(row,k).state==='unknown'):true;
  }).slice().sort((a,b)=>{
    const key=classHeatmapUi.sort;
    return (key==='name'?0:key==='priority'?classHeatmapPriority(b)-classHeatmapPriority(a):(classMetricValue(b,key)??-1)-(classMetricValue(a,key)??-1))||compareNames(a.class_id,b.class_id);
  });
}
function openClassMetricCalls(row){
  prepareCallModel();const cls=callClasses.get(row.class_id);
  if(!cls){feedback(t('No call declaration available for this class.','Keine Aufrufdeklaration für diese Klasse verfügbar.'));return;}
  callUi.language=callClassLanguage(cls)||'all';callUi.package=cls.package;callUi.query='';callUi.resolution='all';callUi.internal=true;callUi.relation=null;callUi.element={kind:'class',id:cls.id};callUi.limit=100;callUi.siteLimit=100;
  callRankingUi.mode='connections';document.getElementById('calls-filter').value='';showTab('calls');
}
function renderClassHeatmap(container,rows){
  const controls=element('div',undefined,'class-heatmap-controls');container.appendChild(controls);
  function choice(label,attribute,field,options){const wrap=element('label'),select=element('select');wrap.appendChild(element('span',label));select.setAttribute(attribute,'true');for(const [value,text]of options){const option=element('option',text);option.value=value;select.appendChild(option);}select.value=classHeatmapUi[field];select.addEventListener('change',()=>{classHeatmapUi[field]=select.value;engineeringUi.limit=20;renderEngineeringQuality();document.getElementById('engineering-list').querySelector('select['+attribute+']')?.focus();});wrap.appendChild(select);controls.appendChild(wrap);}
  choice(t('Show','Zeigen'),'data-heatmap-filter','filter',[['all',t('All classes','Alle Klassen')],['high',t('High values','Hohe Werte')],['low',t('Low values throughout','Durchgehend niedrige Werte')],['unknown',t('Missing measurements','Fehlende Messwerte')]]);
  choice(t('Sort','Sortierung'),'data-heatmap-sort','sort',[['priority',t('Review first','Zuerst prüfen')],['name',t('Class name','Klassenname')],...CLASS_METRICS.map(k=>[k,k.toUpperCase()])]);
  const legend=element('div',undefined,'class-heatmap-legend');for(const state of ['low','medium','high','unknown','info']){const item=element('span',classMetricStateLabel(state),'class-metric-tone');item.setAttribute('data-metric-state',state);legend.appendChild(item);}controls.appendChild(legend);
  legend.appendChild(contextHelp(t('Heatmap colors','Farben der Heatmap'),t('Relative comparison within the original project and source language: low below the 50% rank, elevated from 50%, high from 80%. Equal values share a rank; fewer than five measurements or a constant nonzero population are neutral. Zero and one connected LCOM4 group are low. NOC is informational. Colors guide review; they do not prove class quality.','Relativer Vergleich im ursprünglichen Projekt und derselben Quellsprache: niedrig unter Rang 50%, erhöht ab 50%, hoch ab 80%. Gleiche Werte erhalten denselben Rang; weniger als fünf Messwerte oder identische Nichtnullwerte bleiben neutral. Null und eine zusammenhängende LCOM4-Gruppe gelten als niedrig. NOC dient zur Information. Farben unterstützen die Prüfung; sie beweisen keine Klassenqualität.')));
  const selected=rows.find(row=>row.class_id===classHeatmapUi.selected);
  if(selected){const detail=element('div');detail.setAttribute('data-heatmap-detail','true');detail.setAttribute('tabindex','-1');container.appendChild(detail);renderClassMetricsCard(detail,selected);detail.appendChild(button(t('Back to heatmap','Zur Heatmap'),()=>{classHeatmapUi.selected=null;renderEngineeringQuality();[...document.getElementById('engineering-list').querySelectorAll('button[data-class-select]')].find(n=>n.getAttribute('data-class-select')===selected.class_id)?.focus();},'btn btn-secondary'));}
  else classHeatmapUi.selected=null;
  if(!rows.length)return;
  const scroll=element('div',undefined,'class-heatmap-scroll');scroll.setAttribute('tabindex','0');scroll.setAttribute('role','region');scroll.setAttribute('aria-label',t('Class metric heatmap','Klassenmetriken-Heatmap'));container.appendChild(scroll);
  const table=element('table',undefined,'class-heatmap');scroll.appendChild(table);table.appendChild(element('caption',t('Select a class for details and navigation.','Klasse für Details und Navigation auswählen.'),'visually-hidden'));
  const head=element('thead'),header=element('tr');head.appendChild(header);table.appendChild(head);const name=element('th',t('Class','Klasse'));name.setAttribute('scope','col');header.appendChild(name);
  for(const key of CLASS_METRICS){const th=element('th');th.setAttribute('scope','col');const label=element('span',key.toUpperCase());label.setAttribute('data-metric-key',key);th.appendChild(explainMetric(label,key));header.appendChild(th);}
  const body=element('tbody');table.appendChild(body);
  function choose(row){classHeatmapUi.selected=row.class_id;renderEngineeringQuality();const detail=document.getElementById('engineering-list').querySelector('div[data-heatmap-detail]');detail?.focus();detail?.scrollIntoView?.({block:'nearest'});}
  for(const row of rows.slice(0,engineeringUi.limit)){
    const tr=element('tr');tr.setAttribute('data-heatmap-class',row.class_id);if(selected===row)tr.setAttribute('data-selected','true');body.appendChild(tr);
    const th=element('th');th.setAttribute('scope','row');const pick=button(row.class_id.split('.').pop(),()=>choose(row),'class-heatmap-name');pick.setAttribute('data-class-select',row.class_id);pick.setAttribute('aria-label',row.class_id);explainAction(pick,()=>row.class_id+' · '+row.file);th.appendChild(pick);tr.appendChild(th);
    for(const key of CLASS_METRICS){const a=classMetricAssessment(row,key),cell=element('td'),pick=button(engineeringNumber(a.value),()=>choose(row),'class-metric-tone');pick.setAttribute('data-metric-state',a.state);pick.setAttribute('aria-label',row.class_id+' · '+key.toUpperCase()+' '+engineeringNumber(a.value)+' · '+classMetricStateLabel(a.state));cell.appendChild(explainAction(pick,()=>metricHelp(key)+' '+classMetricInterpretation(row,key)));tr.appendChild(cell);}
  }
}
