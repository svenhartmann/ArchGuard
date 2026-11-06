'use strict';
// Decisions are editable drafts. Only an explicit CLI import records them.
const assessmentUi={query:'',filter:'all',limit:50,selected:null,drafts:new Map(),forms:new Map(),dirty:false,message:'',impactLimit:100};
function assessmentModel(){return data.insights?.review_session||{status:'not_recorded',report:null,diagnostics:[]};}
function assessmentCanReload(){return !assessmentUi.dirty;}
function assessmentStatus(value){
  const labels={
    current:['Current at analysis','Aktuell bei der Analyse'],stale:['Outdated review','Veraltetes Review'],not_recorded:['No recorded review','Kein gespeichertes Review'],
    needs_review:['Review needed','Prüfung nötig'],no_open_findings:['No open findings','Keine offenen Befunde'],
    open:['Open','Offen'],planned:['Planned','Geplant'],deferred:['Deferred','Zurückgestellt'],accepted_risk:['Accepted risk','Akzeptiertes Risiko'],fix:['Fix','Beheben'],defer:['Defer','Zurückstellen'],'accept-risk':['Accept risk','Risiko akzeptieren'],expired:['Expired','Abgelaufen'],changed:['Evidence changed','Belege geändert'],
    not_observed:['Not observed now','Aktuell nicht beobachtet'],not_observed_in_current:['Not observed now','Aktuell nicht beobachtet'],
    new:['New','Neu'],existing:['Existing','Bestehend'],not_compared:['Not compared','Nicht verglichen'],
    not_imported:['Not imported','Nicht importiert'],unavailable:['Unavailable','Nicht verfügbar'],partial:['Partial','Unvollständig'],unverified:['Unverified','Nicht verifiziert']
  };
  return labels[value]?t(...labels[value]):String(value||'—');
}
function assessmentCommand(importDecisions=false){
  const report=assessmentModel().report,quote=value=>"'"+String(value).replaceAll("'","'\\''")+"'";
  const root=report?.project_root||data.root||'.';
  return 'archguard review '+quote(root)+(report?.config_path?' --config '+quote(report.config_path):'')+(report?.base_ref?' --base '+quote(report.base_ref):'')+
    (importDecisions?' --decisions '+quote(root.replace(/\/$/,'')+'/.archguard/review-decisions-draft.json'):'')+' --save --format html';
}
function assessmentDecisionDocument(){
  const report=assessmentModel().report;
  // Register entries bound to this review; the import replaces exactly these.
  const decisions=new Map((report?.decisions||[]).map(d=>[d.finding_ref,d]));
  for(const [id,decision] of assessmentUi.drafts)decisions.set(id,decision);
  return {schema_version:2,review_revision:report?.revision||'',decision_revision:report?.decision_fingerprint||null,decisions:[...decisions.values()].sort((a,b)=>a.finding_ref.localeCompare(b.finding_ref))};
}
function assessmentExportDecisions(){
  if(assessmentModel().status!=='current')return;
  const content=JSON.stringify(assessmentDecisionDocument(),null,2)+'\n',anchor=document.createElement('a');
  anchor.href=URL.createObjectURL(new Blob([content],{type:'application/json;charset=utf-8'}));anchor.download='review-decisions-draft.json';anchor.click();
  setTimeout(()=>URL.revokeObjectURL(anchor.href),1000);
  assessmentUi.message=t('Downloaded decisions. Save the file under .archguard and run the import command below.','Entscheidungen heruntergeladen. Datei unter .archguard ablegen und den unten stehenden Importbefehl ausführen.');
  renderAssessment();
}
function assessmentSection(root,title){const box=element('section',undefined,'assessment-section');box.appendChild(element('h3',title));root.appendChild(box);return box;}
function assessmentField(root,name,label,value,multiline=false){
  const wrapper=element('label',undefined,'assessment-field');wrapper.appendChild(element('span',label));
  const input=element(multiline?'textarea':'input');input.value=value||'';input.setAttribute('data-review-field',name);input.setAttribute('aria-label',label);
  input.addEventListener('input',()=>{assessmentUi.forms.get(assessmentUi.selected)[name]=input.value;assessmentUi.dirty=true;});
  wrapper.appendChild(input);root.appendChild(wrapper);return input;
}
function assessmentDetail(root,finding){
  const details=assessmentSection(root,t('Finding and decision','Befund und Entscheidung'));
  details.appendChild(element('h4',finding.title));
  details.appendChild(element('p',finding.category+' · '+finding.severity+' · '+assessmentStatus(finding.change)+' · '+assessmentStatus(finding.disposition)));
  details.appendChild(element('p',t('Source status: ','Quellstatus: ')+assessmentStatus(finding.source_status),'muted'));
  for(const name of finding.nodes||[]){
    const source=nodeByName(data,name),current=source&&nodeById(work,source.id);
    if(current){details.appendChild(button(name+(current.name!==name?' → '+current.name:''),()=>{const target=nodeById(work,source.id);if(target){showTab('graph');revealNode(target.name);}},'btn btn-secondary'));}
    else details.appendChild(element('code',name));
  }
  const evidence=element('details');evidence.appendChild(element('summary',t('Source evidence','Quellbelege')));
  evidence.appendChild(element('pre',JSON.stringify(finding.evidence,null,2)));details.appendChild(evidence);
  if(finding.decision)details.appendChild(element('p',t('Recorded decision: ','Gespeicherte Entscheidung: ')+assessmentStatus(finding.decision.decision)+' · '+finding.decision.author+' · '+finding.decision.note+(finding.decision.expires_on?' · '+finding.decision.expires_on+' UTC':'')));
  if(assessmentModel().status!=='current'||finding.disposition==='not_observed'){
    details.appendChild(element('p',t('Generate a current review before recording a new decision for this finding.','Vor einer neuen Entscheidung für diesen Befund ein aktuelles Review erzeugen.')));return;
  }
  if(!assessmentUi.forms.has(finding.id)){
    const d=assessmentUi.drafts.get(finding.id)||finding.decision;
    assessmentUi.forms.set(finding.id,{status:d?.decision||'fix',owner:d?.author||(finding.suggested_owners||[]).join(', '),reason:d?.note||'',expires_on:d?.expires_on||'',links:(d?.links||[]).join('\n')});
  }
  const form=assessmentUi.forms.get(finding.id),controls={};
  const label=element('label',undefined,'assessment-field');label.appendChild(element('span',t('Decision','Entscheidung')));
  const select=element('select');select.setAttribute('data-review-field','status');select.setAttribute('aria-label',t('Decision','Entscheidung'));
  for(const status of ['fix','defer','accept-risk']){const option=element('option',assessmentStatus(status));option.value=status;select.appendChild(option);}
  select.value=form.status;select.addEventListener('change',()=>{form.status=select.value;assessmentUi.dirty=true;});label.appendChild(select);details.appendChild(label);controls.status=select;
  controls.owner=assessmentField(details,'owner',t('Responsible person or team','Verantwortliche Person oder Team'),form.owner);
  controls.reason=assessmentField(details,'reason',t('Reason and next action','Begründung und nächste Maßnahme'),form.reason,true);
  controls.expires_on=assessmentField(details,'expires_on',t('Valid through (UTC, YYYY-MM-DD; required for accepted risk)','Gültig bis (UTC, JJJJ-MM-TT; Pflicht für akzeptiertes Risiko)'),form.expires_on);
  controls.expires_on.type='date';
  controls.links=assessmentField(details,'links',t('ADR or issue references (one per line)','ADR- oder Ticketreferenzen (eine je Zeile)'),form.links,true);
  const stage=button(t('Add to decision export','Für Entscheidungsexport übernehmen'),()=>{
    for(const [name,input] of Object.entries(controls))form[name]=input.value;
    if(!form.owner.trim()||!form.reason.trim()||(form.status==='accept-risk'&&!form.expires_on)){
      assessmentUi.message=t('Enter an owner, a reason and an expiry date for accepted risks.','Verantwortliche Person, Begründung und für akzeptierte Risiken ein Ablaufdatum angeben.');renderAssessment();return;
    }
    const decision={id:finding.id,finding_ref:finding.id,finding_fingerprint:finding.fingerprint,decision:form.status,author:form.owner.trim(),note:form.reason.trim(),ts:'',expires_on:form.expires_on||null,links:form.links.split('\n').map(s=>s.trim()).filter(Boolean)};
    assessmentUi.drafts.set(finding.id,decision);assessmentUi.dirty=true;
    assessmentUi.message=t('Decision added to the export draft. The recorded review is unchanged.','Entscheidung im Exportentwurf übernommen. Das gespeicherte Review bleibt unverändert.');renderAssessment();
  },'btn btn-primary');stage.setAttribute('data-review-action','stage');details.appendChild(stage);
}
function renderAssessment(){
  const root=document.getElementById('assessment-content'),saved=assessmentModel(),report=saved.report;hideMetricTooltip(root);root.textContent='';
  root.appendChild(element('h2',t('Architecture review','Architekturreview')));
  root.appendChild(element('p',assessmentStatus(saved.status),'assessment-status'));
  for(const message of saved.diagnostics||[])root.appendChild(element('p',message,'assessment-warning'));
  if(!report){
    const setup=element('details',undefined,'context-note');setup.setAttribute('data-review-setup','true');setup.appendChild(element('summary',t('Create review','Review erstellen')));root.appendChild(setup);
    setup.appendChild(element('p',t('Create a review to collect findings, change impact, decisions and missing evidence. Add --base REF to review a change.','Ein Review bündelt Befunde, Änderungsreichweite, Entscheidungen und fehlende Belege. Für einen Änderungsvergleich --base REF ergänzen.')));
    setup.appendChild(element('code',assessmentCommand(),'repository-command'));return;
  }

  const cards=element('div',undefined,'assessment-summary');
  for(const [key,title] of [['open',t('Open','Offen')],['accepted_risk',t('Accepted risks','Akzeptierte Risiken')],['evidence_gaps',t('Evidence gaps','Beleglücken')]]){
    const card=element('div');card.appendChild(element('strong',String(report.summary?.[key]??0)));card.appendChild(element('span',title));cards.appendChild(card);
  }root.appendChild(cards);
  root.appendChild(element('p',(report.base_ref?t('Compared with: ','Verglichen mit: ')+report.base_ref+' · ':'')+t('Review: ','Review: ')+report.revision.slice(0,12),'mono'));
  const controls=element('div',undefined,'assessment-controls');
  const query=element('input');query.type='search';query.value=assessmentUi.query;query.setAttribute('aria-label',t('Search review findings','Review-Befunde suchen'));query.placeholder=t('Search finding, owner or source','Befund, Verantwortliche oder Quelle suchen');
  query.addEventListener('input',()=>{assessmentUi.query=query.value;assessmentUi.limit=50;renderAssessment();document.getElementById('assessment-content').querySelector('input').focus();});controls.appendChild(query);
  const filter=element('select');filter.setAttribute('aria-label',t('Finding status','Befundstatus'));
  for(const [key,label] of [['all',t('All findings','Alle Befunde')],['open',t('Needs action','Maßnahme nötig')],['new',t('New architecture findings','Neue Architekturbefunde')],['accepted_risk',t('Accepted risks','Akzeptierte Risiken')]]){
    const option=element('option',label);option.value=key;filter.appendChild(option);
  }filter.value=assessmentUi.filter;filter.addEventListener('change',()=>{assessmentUi.filter=filter.value;assessmentUi.limit=50;renderAssessment();});controls.appendChild(filter);
  const download=button(t('Export decisions','Entscheidungen exportieren'),assessmentExportDecisions,'btn btn-primary');download.setAttribute('data-review-action','export-decisions');download.disabled=saved.status!=='current';controls.appendChild(actionWithHelp(download,()=>saved.status!=='current'?t('Generate a current review before exporting decisions.','Vor dem Entscheidungsexport ein aktuelles Review erzeugen.'):t('Download decision drafts as JSON. Record them using the import command below; this download does not update the saved review.','Entscheidungsentwürfe als JSON herunterladen. Mit dem unten stehenden Importbefehl speichern; der Download aktualisiert das gespeicherte Review noch nicht.')));
  if(assessmentUi.dirty){
    const clear=button(t('Clear exported drafts','Exportierte Entwürfe leeren'),()=>{assessmentUi.drafts.clear();assessmentUi.forms.clear();assessmentUi.dirty=false;assessmentUi.message='';renderAssessment();});
    clear.setAttribute('data-review-action','clear');controls.appendChild(clear);
  }root.appendChild(controls);
  if(assessmentUi.message){const status=element('p',assessmentUi.message,'assessment-warning');status.setAttribute('role','status');root.appendChild(status);}
  const save=element('details',undefined,'context-note');save.appendChild(element('summary',t('Record or update this review','Review speichern oder aktualisieren')));
  save.appendChild(element('p',t('Place the downloaded decision file at .archguard/review-decisions-draft.json in the analyzed project, then run:','Heruntergeladene Entscheidungsdatei im analysierten Projekt unter .archguard/review-decisions-draft.json ablegen, dann ausführen:')));
  save.appendChild(element('code',assessmentCommand(true),'repository-command'));save.appendChild(element('p',t('Refresh without importing decisions:','Ohne Entscheidungsimport aktualisieren:')));save.appendChild(element('code',assessmentCommand(),'repository-command'));root.appendChild(save);
  const layout=element('div',undefined,'assessment-layout'),list=assessmentSection(layout,t('Prioritized findings','Priorisierte Befunde'));root.appendChild(layout);
  const q=assessmentUi.query.toLowerCase(),rows=report.findings.filter(f=>(!q||[f.title,...f.nodes,f.decision?.author||'',...f.suggested_owners].join(' ').toLowerCase().includes(q))&&
    (assessmentUi.filter==='all'||assessmentUi.filter==='new'&&f.change==='new'||assessmentUi.filter==='accepted_risk'&&f.disposition==='accepted_risk'||assessmentUi.filter==='open'&&!['accepted_risk','not_observed'].includes(f.disposition)));
  list.appendChild(explainContext(element('p',Math.min(rows.length,assessmentUi.limit)+' / '+rows.length,'muted'),t('Filtering does not change totals or exports.','Filter ändern weder Gesamtzahlen noch Exporte.')));
  for(const finding of rows.slice(0,assessmentUi.limit)){
    const row=button(finding.severity+' · '+assessmentStatus(finding.change)+' · '+assessmentStatus(finding.disposition)+'\n'+finding.title,()=>{assessmentUi.selected=finding.id;renderAssessment();},'assessment-finding');
    row.setAttribute('data-review-finding',finding.id);row.setAttribute('aria-pressed',String(assessmentUi.selected===finding.id));list.appendChild(row);
  }
  if(rows.length>assessmentUi.limit){const more=button(t('Show more findings','Weitere Befunde anzeigen'),()=>{assessmentUi.limit+=50;renderAssessment();});more.setAttribute('data-review-action','more');list.appendChild(more);}
  const selected=report.findings.find(f=>f.id===assessmentUi.selected);
  if(selected)assessmentDetail(layout,selected);
  const impact=assessmentSection(root,t('Affected components, files and tests','Betroffene Komponenten, Dateien und Tests'));
  impact.appendChild(element('p',(report.impact.affected_components||[]).join(', ')||t('No mapped logical component in this selection.','Keine zugeordnete logische Komponente in dieser Auswahl.')));
  const files=report.impact.affected_files||[];
  for(const file of files.slice(0,assessmentUi.impactLimit))impact.appendChild(element('code',file,'assessment-file'));
  for(const test of (report.impact.tests?.selected||[]).slice(0,assessmentUi.impactLimit))impact.appendChild(element('p',test.suite+' / '+test.name+' · '+test.outcome));
  if(files.length>assessmentUi.impactLimit||(report.impact.tests?.selected||[]).length>assessmentUi.impactLimit)impact.appendChild(button(t('Show more impact evidence','Weitere Reichweitenbelege anzeigen'),()=>{assessmentUi.impactLimit+=100;renderAssessment();}));
  impact.appendChild(explainContext(element('p',t('Test evidence: ','Testbelege: ')+assessmentStatus(report.impact.tests?.status),'muted'),t('Recommendations use static dependencies and imported mappings. Other tests cannot be considered safe to skip.','. Empfehlungen verwenden statische Abhängigkeiten und importierte Zuordnungen. Andere Tests gelten dadurch nicht als sicher überflüssig.')));
  const gaps=assessmentSection(root,t('Evidence gaps','Beleglücken'));
  for(const gap of report.gaps||[]){const item=element('details',undefined,'context-note');item.appendChild(element('summary',gap.source+' · '+assessmentStatus(gap.status)));item.appendChild(element('p',gap.reason));gaps.appendChild(item);}
  const provenance=element('details');provenance.appendChild(element('summary',t('Evidence status and origin','Belegstatus und Herkunft')));
  for(const source of report.evidence||[])provenance.appendChild(element('p',source.source+' · '+assessmentStatus(source.status)+' · '+assessmentStatus(source.producer_revision)));
  gaps.appendChild(provenance);
  if(report.architecture?.patterns){const patterns=assessmentSection(root,t('Architecture pattern candidates','Architekturmuster-Kandidaten'));const items=report.architecture.patterns.assessments;patterns.appendChild(element('p',t('{candidates} candidates · {total} assessments','{candidates} Kandidaten · {total} Prüfungen',{candidates:items.filter(p=>['candidate','mixed'].includes(p.status)).length,total:items.length})));patterns.appendChild(button(t('Inspect patterns and evidence','Muster und Belege prüfen'),()=>showTab('architecture'),'btn btn-secondary'));}
  const questions=assessmentSection(root,t('Architecture questions and decisions','Architekturfragen und Entscheidungen'));
  for(const question of report.architecture?.questions||[])questions.appendChild(element('p',question));
  for(const d of report.architecture?.intent?.decisions||[])questions.appendChild(element('p',d.id+' · '+d.title+' · '+d.status+' · '+d.chosen_option));
  if(report.unmatched_decisions?.length)questions.appendChild(element('p',t('{count} previous decisions have no matching current finding; they remain in the exported record.','{count} frühere Entscheidungen haben keinen passenden aktuellen Befund; sie bleiben im exportierten Datensatz.',{count:report.unmatched_decisions.length})));
  const limits=element('details',undefined,'context-note');limits.appendChild(element('summary',t('Review scope and limits','Review-Umfang und Grenzen')));
  limits.appendChild(element('p',t('Recorded observations. Graph simulations and draft decisions do not change measurements or CI gates.','Festgehaltene Beobachtungen. Graphsimulationen und Entscheidungsentwürfe verändern weder Messungen noch CI-Gates.'),'muted'));
  for(const note of report.limitations||[])limits.appendChild(element('p',note));root.appendChild(limits);
}
function assessmentExport(base){
  return {...base,kind:'assessment',review:assessmentModel(),draft_decisions:assessmentDecisionDocument(),drafts_recorded:false};
}
function assessmentMarkdown(snapshot){
  const cell=exportMarkdownCell,r=snapshot.review.report,lines=['# Architecture review','',cell(snapshot.review.status),''];
  for(const message of snapshot.review.diagnostics||[])lines.push(cell(message),'');
  if(!r)return lines.join('\n');
  lines.push('Revision: '+cell(r.revision),'','## Findings','','| Category | Severity | Change | Disposition | Finding | Owner | Reason | Expiry (UTC) |','| --- | --- | --- | --- | --- | --- | --- | --- |');
  for(const f of r.findings)lines.push('| '+[f.category,f.severity,f.change,f.disposition,f.title,f.decision?.author||'',f.decision?.note||'',f.decision?.expires_on||''].map(cell).join(' | ')+' |');
  lines.push('','## Evidence gaps','');
  for(const g of r.gaps)lines.push('- '+[g.source,g.status,g.reason].map(cell).join(' · '));
  lines.push('','## Impact','',...(r.impact.affected_components||[]).map(cell),...(r.impact.affected_files||[]).map(cell));
  for(const test of r.impact.tests?.selected||[])lines.push(cell(test.suite+' / '+test.name+' · '+test.outcome));
  lines.push('','## Scope','',...(r.limitations||[]).map(cell),'','Draft decisions are unrecorded. JSON export contains the complete evidence and decision document.','');
  if(typeof appendPatternsMarkdown==='function')appendPatternsMarkdown(lines,r.architecture?.patterns);
  return lines.join('\n');
}
