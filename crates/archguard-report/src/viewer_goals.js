'use strict';
// Quality goal assessment (ADR 0003), mirrors arc42_goals.rs `evidence_lines`.
// A status describes the available evidence, never whether a goal is met.
const GOAL_FINDING_IDS=30;
function goalStatusLabel(status) {
  return {unclassified:t('No characteristic given','Kein Merkmal angegeben'),not_assessable:t('Not assessable','Nicht bewertbar'),no_evidence:t('No evidence','Keine Belege'),findings:t('Findings','Befunde'),no_findings:t('No findings from the available sources','Keine Befunde aus den verfügbaren Quellen')}[status]||status;
}
function goalMeasured(goal) { return goal.status==='findings'||goal.status==='no_findings'; }
function renderQualityGoals(root) {
  const goals=data.insights?.architecture?.goal_assessment;
  if(!Array.isArray(goals))return;
  const section=element('section',undefined,'architecture-section');section.setAttribute('data-quality-goals','true');root.appendChild(section);
  const heading=element('h3',t('Quality goals','Qualitätsziele'));section.appendChild(heading);
  heading.appendChild(contextHelp(t('Quality goal assessment','Bewertung der Qualitätsziele'),t('architecture.goals with an ISO/IEC 25010 characteristic, linked to findings and evidence sources by a fixed mapping (ADR 0003). The status describes the available evidence, not whether a goal is met.','architecture.goals mit Merkmal nach ISO/IEC 25010, über eine feste Zuordnung mit Befunden und Belegquellen verknüpft (ADR 0003). Der Status beschreibt die verfügbaren Belege, nicht die Erfüllung eines Ziels.')));
  if(!goals.length){section.appendChild(element('p',t('No quality goals declared.','Keine Qualitätsziele deklariert.'),'muted'));return;}
  const measured=goals.filter(goalMeasured).length;
  section.appendChild(element('p',t('{measured} measured · {missing} not measured','{measured} gemessen · {missing} nicht gemessen',{measured,missing:goals.length-measured}),'muted'));
  const byId=new Map((data.findings||[]).map(f=>[f.id,f]));
  for(const goal of goals){
    const row=element('details',undefined,'architecture-section');row.setAttribute('data-goal-status',goal.status);row.setAttribute('data-goal-measured',String(goalMeasured(goal)));
    row.open=goal.status==='findings';
    row.appendChild(element('summary',goal.goal+' · '+(goalMeasured(goal)?'':t('Not measured','Nicht gemessen')+' · ')+goalStatusLabel(goal.status)));
    const notGiven=t('not given','nicht angegeben');
    row.appendChild(element('p',t('Characteristic: {attribute} · scope: {scope} · priority: {priority}','Merkmal: {attribute} · Umfang: {scope} · Priorität: {priority}',{attribute:goal.attribute||notGiven,scope:goal.scope?.length?goal.scope.join(', '):t('Whole system','Gesamtsystem'),priority:goal.priority?.trim()?goal.priority:notGiven}),'muted'));
    if(goal.findings?.length){
      const severities=Object.entries(goal.finding_count_by_severity||{}).map(([severity,n])=>severity+' '+n).join(', ');
      row.appendChild(element('h4',t('Findings ({count}; {severities})','Befunde ({count}; {severities})',{count:goal.findings.length,severities})));
      for(const id of goal.findings.slice(0,GOAL_FINDING_IDS)){const f=byId.get(id);row.appendChild(element('div',f?id+' · '+f.severity+' · '+(f.message||f.kind):id,'mono'));}
      if(goal.findings.length>GOAL_FINDING_IDS)row.appendChild(element('p',t('and {count} more','und {count} weitere',{count:goal.findings.length-GOAL_FINDING_IDS}),'muted'));
    }
    if(goal.kpis?.length)row.appendChild(element('p',t('Metrics (project-wide): {kpis}','Kennzahlen (projektweit): {kpis}',{kpis:goal.kpis.map(k=>k.name+' '+k.value).join(', ')})));
    if(goal.evidence_sources?.length)row.appendChild(element('p',t('Evidence sources: {sources}','Belegquellen: {sources}',{sources:goal.evidence_sources.map(s=>s.source+' '+(s.present?t('present','vorhanden'):t('missing','fehlt'))).join(', ')})));
    for(const gap of goal.gaps||[])row.appendChild(element('p',gap,'muted'));
    section.appendChild(row);
  }
}
