'use strict';
// Review Dashboard decisions panel (R40). Lists open error/warning findings
// from data.findings that the register (R02) carries no decision for, and
// the decisions recorded so far. Labels use the severities the payload
// actually carries — no invented P1/P2 vocabulary; a review priority is
// shown only when data.insights.review_session really has one. Recording
// goes ONLY through the serve API (POST /api/decisions, R41): a static
// report shows the buttons disabled and says so. Server failures (the ADR
// gate 400, a corrupt-register 500) are shown verbatim, never swallowed.

const DASH_DECISION_LIMIT=25;
let dashDecisionFeedback=null;

// Latest matching entry wins, matching the register's order semantics; the
// serve upsert (R41) is idempotent by id, so "serve-<finding>-<kind>" is a
// stable identity: re-clicking the same button replaces the same entry.
function dashLatestDecision(register,findingId){
  let latest=null;
  for(const d of (register&&Array.isArray(register.decisions)?register.decisions:[]))if(d&&d.finding_ref===findingId)latest=d;
  return latest;
}
function dashDecisionKindLabel(kind){
  return({fix:t('Fix','Beheben'),defer:t('Defer','Zurückstellen'),'accept-risk':t('Accept risk','Risiko akzeptieren')})[kind]||String(kind);
}
// The report payload carries severities ("error", "warning", "info"); the
// saved review (insights.review_session) carries none today — a real
// priority string, if one ever appears, is labelled as the review priority.
function dashFindingLabel(finding){
  const session=data.insights&&data.insights.review_session;
  for(const row of (session&&(session.findings||session.report&&session.report.findings))||[])
    if(row&&row.id===finding.id&&typeof row.priority==='string'&&row.priority.trim())
      return t('review priority {p}','Review-Priorität {p}',{p:row.priority});
  return t('severity {s}','Schweregrad {s}',{s:String(finding.severity)});
}
// Decisions live in the embedded payload object so a re-render (which
// re-reads data.reviewRegister) keeps what this session just recorded.
function dashDecisionRegister(){
  if(!data.reviewRegister||typeof data.reviewRegister!=='object')data.reviewRegister={version:1,decisions:[]};
  if(!Array.isArray(data.reviewRegister.decisions))data.reviewRegister.decisions=[];
  return data.reviewRegister;
}
function dashDecisionPost(finding,kind,noteValue,adrValue,statusEl){
  const body={id:'serve-'+finding.id+'-'+kind,finding_ref:finding.id,decision:kind,adr_link:adrValue.trim()||null,note:noteValue,author:'viewer',ts:''};
  statusEl.textContent=t('Saving…','Speichere…');
  fetch('/api/decisions',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)})
    .then(response=>response.text().then(text=>({ok:response.ok,text})))
    .then(result=>{
      // 400 (empty note, ADR gate) and 500 (register not saved) arrive
      // here as the server's own words — shown verbatim, nothing recorded.
      if(!result.ok){statusEl.textContent=result.text;return;}
      let recorded=body;
      try{const served=JSON.parse(result.text);const found=served&&served.register&&served.register.decisions&&served.register.decisions.find(d=>d&&d.id===body.id);if(found)recorded=found;}catch(error){/* keep what was sent */}
      const register=dashDecisionRegister();
      const at=register.decisions.findIndex(d=>d&&d.id===recorded.id);
      if(at>=0)register.decisions[at]=recorded;else register.decisions.push(recorded);
      dashDecisionFeedback=t('Recorded {id}.','Entscheidung {id} erfasst.',{id:recorded.id});
      renderReviewDashboard();
    })
    .catch(error=>{statusEl.textContent=t('Could not reach the serve API: {error}','serve-API nicht erreichbar: {error}',{error:String(error)});});
}

function renderDashDecisions(container,kpis,register){
  dashClearPanel(container);
  const served=reviewServed();
  const decisions=register&&Array.isArray(register.decisions)?register.decisions:[];
  // (a) Open findings needing a decision: error/warning severities as the
  // report carries them, minus anything the register already decided about.
  const open=(data.findings||[]).filter(f=>f&&(f.severity==='error'||f.severity==='warning')&&!dashLatestDecision(register,f.id));
  container.appendChild(element('h4',t('Open findings needing a decision','Offene Befunde mit Entscheidungsbedarf')));
  if(!open.length)
    container.appendChild(element('p',
      (data.findings||[]).length
        ?t('No open findings — every reported error or warning already has a recorded decision.','Keine offenen Befunde — jeder gemeldete Fehler oder jede Warnung hat bereits eine erfasste Entscheidung.')
        :t('The report lists no error or warning findings — nothing here needs a decision.','Der Bericht listet keine Fehler- oder Warnbefunde — hier ist nichts zu entscheiden.'),
      'empty'));
  else{
    for(const finding of open.slice(0,DASH_DECISION_LIMIT)){
      const row=element('div',undefined,'dash-decision-row');
      row.appendChild(element('strong',finding.id));
      row.appendChild(element('span',dashFindingLabel(finding),'dash-decision-severity'));
      const list=document.createElement('span');
      list.appendChild(document.createTextNode((finding.kind||'?')+(finding.nodes&&finding.nodes.length?' · '+finding.nodes.slice(0,3).join(', ')+(finding.nodes.length>3?' …':''):'')));
      if(finding.message)list.appendChild(element('small',finding.message,'muted dash-decision-message'));
      row.appendChild(list);
      const controls=element('div',undefined,'dash-decision-controls');
      const note=element('input');note.type='text';note.placeholder=t('Note (required by the server)','Notiz (vom Server erforderlich)');
      const adr=element('input');adr.type='text';adr.placeholder=t('ADR link (for accept-risk)','ADR-Link (für Risiko akzeptieren)');adr.className='dash-decision-adr';
      const status=element('span',undefined,'muted dash-decision-status');
      controls.appendChild(note);controls.appendChild(adr);
      for(const kind of ['fix','defer','accept-risk']){
        const action=element('button',dashDecisionKindLabel(kind),'btn btn-quiet');
        action.type='button';
        if(!served)action.disabled=true;
        else action.addEventListener('click',()=>dashDecisionPost(finding,kind,note.value,adr.value,status));
        controls.appendChild(action);
      }
      controls.appendChild(status);
      row.appendChild(controls);
      container.appendChild(row);
    }
    if(open.length>DASH_DECISION_LIMIT)
      container.appendChild(element('p',t('Showing the first {shown} of {total} open findings — record decisions or re-run the analysis to narrow the list.','Die ersten {shown} von {total} offenen Befunden — Entscheidungen erfassen oder die Analyse erneut ausführen, um die Liste einzugrenzen.',{shown:DASH_DECISION_LIMIT,total:open.length}),'muted'));
  }
  // (b) What the register already carries.
  container.appendChild(element('h4',t('Recorded decisions','Erfasste Entscheidungen')));
  if(!decisions.length)
    container.appendChild(element('p',t('The register carries no decisions yet — nothing has been recorded through the serve API.','Das Register enthält noch keine Entscheidungen — über die serve-API wurde nichts erfasst.'),'empty'));
  else for(const d of decisions){
    const row=element('div',undefined,'dash-decision-row dash-decision-recorded');
    row.appendChild(element('strong',String(d.finding_ref)));
    row.appendChild(element('span',dashDecisionKindLabel(d.decision),'dash-decision-kind'));
    row.appendChild(element('span',d.author+' · '+(d.ts||t('no timestamp','kein Zeitstempel')),'muted'));
    const adrLink=(d.adr_link||'').trim();
    if(adrLink){
      if(/^https?:\/\//i.test(adrLink)){const link=element('a',adrLink);link.href=adrLink;link.rel='noopener noreferrer';row.appendChild(link);}
      else row.appendChild(element('span',adrLink));
    }
    // The ADR gate (R31/R34/R41) lives server-side; the client cannot know
    // the config — an ADR-less accept-risk is flagged as such, nothing more.
    if(d.decision==='accept-risk'&&!adrLink)
      row.appendChild(element('span',t('accepted risk without ADR','akzeptiertes Risiko ohne ADR'),'dash-decision-badge'));
    container.appendChild(row);
  }
  if(!served)container.appendChild(element('p',t('Static reports are read-only: decisions cannot be recorded from a file. Run "archguard serve <path>" and open the report there to record Fix, Defer or Accept risk.','Statische Berichte sind schreibgeschützt: Entscheidungen können aus einer Datei nicht erfasst werden. "archguard serve <Pfad>" ausführen und den Bericht dort öffnen, um Beheben, Zurückstellen oder Risiko akzeptieren zu erfassen.'),'muted'));
  if(dashDecisionFeedback){container.appendChild(element('p',dashDecisionFeedback,'dash-decision-feedback'));dashDecisionFeedback=null;}
}
