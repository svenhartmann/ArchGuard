'use strict';
// Dependency findings are an immutable scanner snapshot, not graph simulation.
const securityUi={query:'',severity:'all',limit:100};
const SECURITY_SEVERITIES=['critical','high','medium','low','unknown'];
const EMPTY_SECURITY={schema_version:1,status:'not_scanned',scanner:'trivy',scanner_version:null,scanned_at:null,database_updated_at:null,fingerprint:'',packages_scanned:0,targets:[],findings:[],diagnostics:[]};
function securitySnapshot(){return data.insights?.security||EMPTY_SECURITY;}
function securitySeverity(value){return SECURITY_SEVERITIES.includes(value)?value:'unknown';}
function securitySeverityLabel(value){return {critical:t('Critical','Kritisch'),high:t('High','Hoch'),medium:t('Medium','Mittel'),low:t('Low','Niedrig'),unknown:t('Unknown','Unbekannt')}[securitySeverity(value)];}
function filteredSecurityFindings(){
  const query=securityUi.query.trim().toLowerCase();
  return (securitySnapshot().findings||[]).filter(row=>(securityUi.severity==='all'||securitySeverity(row.severity)===securityUi.severity)&&(row.package.toLowerCase().includes(query)||row.id.toLowerCase().includes(query)))
    .sort((a,b)=>SECURITY_SEVERITIES.indexOf(securitySeverity(a.severity))-SECURITY_SEVERITIES.indexOf(securitySeverity(b.severity))||compareNames(a.package,b.package)||compareNames(a.id,b.id)||compareNames(a.installed_version,b.installed_version));
}
function securityCounts(snapshot=securitySnapshot()){
  const findings=snapshot.findings||[],counts=Object.fromEntries(SECURITY_SEVERITIES.map(severity=>[severity,0]));
  for(const row of findings)counts[securitySeverity(row.severity)]++;
  return {packages_scanned:snapshot.packages_scanned||0,affected_packages:new Set(findings.map(row=>JSON.stringify([row.ecosystem,row.package,row.installed_version]))).size,findings:findings.length,targets:(snapshot.targets||[]).length,...counts};
}
function securityStatusText(snapshot=securitySnapshot()){
  return {
    not_scanned:t('Not scanned — run the CLI command to create a dependency scan.','Nicht gescannt — den CLI-Befehl für einen Abhängigkeitsscan ausführen.'),
    ready:t('Scan completed','Scan abgeschlossen'),
    partial:t('Partial scan — findings do not cover all dependencies.','Unvollständiger Scan — die Befunde decken nicht alle Abhängigkeiten ab.'),
    stale:t('Outdated snapshot — dependency inputs have changed; run a fresh scan.','Veralteter Snapshot — die Abhängigkeitsdateien wurden geändert; erneut scannen.'),
    error:t('Scan failed — dependency coverage is unknown.','Scan fehlgeschlagen — die Abdeckung der Abhängigkeiten ist unbekannt.'),
  }[snapshot.status]||t('Unknown scan status','Unbekannter Scanstatus');
}
function safeSecurityUrl(value){
  if(typeof value!=='string')return null;
  try{const url=new URL(value);return url.protocol==='https:'||url.protocol==='http:'?url.href:null;}catch(error){return null;}
}
function securityScanCommand(){return "archguard security -- '"+String(data.root||'.').replace(/'/g,"'\\''")+"'";}
function securityListDisclosure(parent,label,entries,format=value=>value){
  const details=element('details',undefined,'security-evidence');details.appendChild(element('summary',label+' ('+entries.length+')'));
  const list=element('div');details.appendChild(list);let shown=0;
  const more=button(t('Show more evidence','Weitere Belege anzeigen'),()=>append(true),'btn btn-secondary');details.appendChild(more);more.hidden=true;
  function append(focusNew=false){
    const end=Math.min(shown+100,entries.length);let first;
    while(shown<end){const row=element('p',format(entries[shown++]),'mono');row.setAttribute('tabindex','-1');first??=row;list.appendChild(row);}
    more.hidden=shown>=entries.length;if(focusNew)first?.focus();
  }
  details.addEventListener('toggle',()=>{if(details.open&&!shown)append();});parent.appendChild(details);return details;
}
function renderSecurityInspector(){
  const sidebar=document.getElementById('sidebar');sidebar.textContent='';
  sidebar.appendChild(element('h2',t('Dependency security','Sicherheit der Abhängigkeiten')));
  sidebar.appendChild(element('p',securityStatusText()));
  sidebar.appendChild(element('p',t('Original dependency scan. Package versions, advisory IDs and lockfile paths are scanner evidence. Graph navigation and simulation do not change this snapshot.','Ursprünglicher Abhängigkeitsscan. Paketversionen, Advisory-IDs und Lockfile-Pfade sind Scannerbelege. Graphnavigation und Simulation verändern diesen Snapshot nicht.'),'muted'));
  sidebar.appendChild(element('p',t('The scanner checks known dependency vulnerabilities. It does not audit application code or measure runtime exploitability.','Der Scanner prüft bekannte Schwachstellen in Abhängigkeiten. Er prüft keinen Anwendungscode und misst keine Ausnutzbarkeit zur Laufzeit.'),'muted'));
  sidebar.appendChild(element('p',t('Fixed versions are advisory information; no dependency files are changed automatically.','Behobene Versionen sind Angaben aus dem Advisory; Abhängigkeitsdateien werden nicht automatisch geändert.'),'muted'));
}
function renderSecurity(){
  const snapshot=securitySnapshot(),counts=securityCounts(snapshot),available=!['not_scanned','error'].includes(snapshot.status)&&counts.packages_scanned>0;
  const status=document.getElementById('security-status');status.textContent=securityStatusText(snapshot);status.setAttribute('data-status',snapshot.status);
  document.getElementById('security-snapshot-note').textContent=isRepositoryOverview()?
    t('Repository-wide dependency scan. Source projects are analyzed separately.','Repository-weiter Abhängigkeitsscan. Quellprojekte werden getrennt analysiert.'):
    repositoryModel()?t('Dependency scan for selected project: {path}. Graph simulations do not update these findings.','Abhängigkeitsscan für das gewählte Projekt: {path}. Graphsimulationen verändern diese Befunde nicht.',{path:repositoryModel().active_project||'.'}):
    t('Original scan snapshot; graph simulations do not update these findings. Top workspace KPIs describe source dependencies.','Ursprünglicher Scan-Snapshot; Graphsimulationen verändern diese Befunde nicht. Die oberen Arbeitsbereich-Kennzahlen beschreiben Quellabhängigkeiten.');
  const metrics=document.getElementById('security-metrics');metrics.textContent='';
  for(const [key,label] of [['packages_scanned',t('Packages scanned','Gescannte Pakete')],['affected_packages',t('Affected packages','Betroffene Pakete')],['findings',t('Findings','Befunde')],['targets',t('Manifest / lockfile targets','Manifest- / Lockfile-Ziele')]]){
    const card=element('div',undefined,'security-metric');card.setAttribute('data-security-metric',key);
    card.appendChild(element('strong',String(available||key==='packages_scanned'||key==='targets'?counts[key]:'—')));card.appendChild(element('span',label));
    card.title=key==='affected_packages'||key==='packages_scanned'?t('Distinct ecosystem, package name and installed version in the original scan. A package can have multiple findings.','Eindeutige Kombination aus Ökosystem, Paketname und installierter Version im ursprünglichen Scan. Ein Paket kann mehrere Befunde haben.'):t('Original snapshot total, before display filters.','Summe des ursprünglichen Snapshots, vor Anzeigefiltern.');metrics.appendChild(card);
  }
  const severities=document.getElementById('security-severities');severities.textContent='';
  for(const severity of SECURITY_SEVERITIES){const card=element('div',undefined,'security-severity');card.setAttribute('data-security-severity',severity);card.appendChild(element('strong',String(available?counts[severity]:'—')));card.appendChild(element('span',securitySeverityLabel(severity)));severities.appendChild(card);}
  const coverage=document.getElementById('security-coverage-content');coverage.textContent='';
  for(const [label,value] of [[t('Scanner','Scanner'),snapshot.scanner+(snapshot.scanner_version?' '+snapshot.scanner_version:'')],[t('Scanned at','Scanzeitpunkt'),snapshot.scanned_at],[t('Database updated at','Datenbank aktualisiert'),snapshot.database_updated_at]])coverage.appendChild(element('p',label+': '+(value||t('Not reported','Nicht gemeldet'))));
  coverage.appendChild(element('p',t('Run a fresh scan in your terminal; this viewer only reads the saved result.','Einen neuen Scan im Terminal ausführen; dieser Viewer liest nur das gespeicherte Ergebnis.')));
  coverage.appendChild(element('code',securityScanCommand(),'security-command'));
  const copy=button(t('Copy scan command','Scanbefehl kopieren'),()=>copyText(securityScanCommand()),'btn btn-secondary');copy.setAttribute('data-security-copy','true');coverage.appendChild(copy);
  securityListDisclosure(coverage,t('Manifest / lockfile coverage','Manifest- / Lockfile-Abdeckung'),snapshot.targets||[],target=>target.path+' · '+target.ecosystem+' · '+t('{count} packages','{count} Pakete',{count:target.packages}));
  if(snapshot.diagnostics?.length)securityListDisclosure(coverage,t('Scanner diagnostics (original text)','Scannerdiagnosen (Originaltext)'),snapshot.diagnostics);
  document.getElementById('security-query').value=securityUi.query;document.getElementById('security-severity').value=securityUi.severity;
  const findings=filteredSecurityFindings(),list=document.getElementById('security-findings');list.textContent='';
  document.getElementById('security-count').textContent=t('{shown} / {matching} matching findings shown · {total} in snapshot','{shown} / {matching} passende Befunde angezeigt · {total} im Snapshot',{shown:Math.min(securityUi.limit,findings.length),matching:findings.length,total:counts.findings});
  for(const row of findings.slice(0,securityUi.limit)){
    const card=element('article',undefined,'security-finding');card.setAttribute('data-security-finding',row.id);card.setAttribute('tabindex','-1');
    const header=element('div',undefined,'security-finding-heading'),severity=element('span',securitySeverityLabel(row.severity),'security-severity');severity.setAttribute('data-security-severity',securitySeverity(row.severity));header.appendChild(severity);header.appendChild(element('h3',row.package));header.appendChild(element('span',row.id,'mono'));card.appendChild(header);
    card.appendChild(element('p',row.title));
    const versions=element('div',undefined,'security-versions');versions.appendChild(element('span',row.ecosystem+' · '+t('Installed: {version}','Installiert: {version}',{version:row.installed_version})));versions.appendChild(element('span',row.fixed_version?t('Fixed version(s): {version}','Behobene Version(en): {version}',{version:row.fixed_version}):t('No fixed version reported','Keine behobene Version gemeldet')));card.appendChild(versions);
    const url=safeSecurityUrl(row.url);if(url){const link=element('a',t('Open advisory','Advisory öffnen'));link.setAttribute('href',url);link.setAttribute('target','_blank');link.setAttribute('rel','noopener noreferrer');card.appendChild(link);}
    securityListDisclosure(card,t('Manifest / lockfile evidence','Manifest- / Lockfile-Belege'),row.targets||[]);list.appendChild(card);
  }
  if(!findings.length){
    const text=counts.findings?t('No findings match these filters.','Keine Befunde entsprechen diesen Filtern.'):
      snapshot.status==='not_scanned'?t('Not scanned. No security conclusion is available.','Nicht gescannt. Es liegt keine Sicherheitsbewertung vor.'):
      snapshot.status==='error'?t('Scan failed. The absence of findings does not establish dependency safety.','Scan fehlgeschlagen. Fehlende Befunde belegen keine sicheren Abhängigkeiten.'):
      !counts.packages_scanned?t('No packages were scanned. The absence of findings does not establish dependency safety.','Es wurden keine Pakete gescannt. Fehlende Befunde belegen keine sicheren Abhängigkeiten.'):
      snapshot.status==='ready'?t('No vulnerabilities reported for the scanned package versions in this snapshot.','In diesem Snapshot wurden keine Schwachstellen für die gescannten Paketversionen gemeldet.'):
      t('No findings were recorded; this incomplete or outdated snapshot does not establish dependency safety.','Es wurden keine Befunde erfasst; dieser unvollständige oder veraltete Snapshot belegt keine sicheren Abhängigkeiten.');
    list.appendChild(element('p',text,'empty'));
  }
  document.getElementById('security-more').hidden=findings.length<=securityUi.limit;
}
document.getElementById('security-query').addEventListener('input',event=>{securityUi.query=event.target.value;securityUi.limit=100;renderSecurity();});
document.getElementById('security-severity').addEventListener('change',event=>{securityUi.severity=event.target.value;securityUi.limit=100;renderSecurity();});
document.getElementById('security-more').addEventListener('click',()=>{
  const firstAdded=securityUi.limit;securityUi.limit+=100;renderSecurity();
  document.getElementById('security-findings').querySelectorAll('article[data-security-finding]')[firstAdded]?.focus();
});
