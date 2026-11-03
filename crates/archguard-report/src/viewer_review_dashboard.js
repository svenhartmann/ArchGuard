'use strict';
// Review Dashboard skeleton (R36). Mirrors the KPI payload the pipeline
// attaches to the analysis (archguard-engine's kpi_payload / R23); the
// panels in viewer_dash_*.js (R37–R42) render into the containers below.
// Like every other view this never edits code: decisions persist only
// through the serve API (R41), and a static report shows them read-only.
const EMPTY_REVIEW_KPIS=null;
const dashboardUi={payloadSource:'embedded',loadError:null};

// The embedded payload carries review_kpis from the analyzed graph
// (html.rs embeds g.review_kpis). null means NOT MEASURED — the panels
// must show that honestly, never zeros.
function reviewKpisSnapshot(){
  if(dashboardUi.payload==='served')return dashboardUi.servedKpis??EMPTY_REVIEW_KPIS;
  return data.reviewKpis??EMPTY_REVIEW_KPIS;
}
function reviewRegisterSnapshot(){return data.reviewRegister??null;}
function reviewServed(){return location.protocol==='http:'||location.protocol==='https:';}

// In a `serve` session the dashboard can refresh from the API (R41);
// the static single-file report only ever reads the embedded payload.
function loadDashboardPayload(){
  if(!reviewServed()){dashboardUi.payloadSource='embedded';renderReviewDashboard();return;}
  fetch('/api/review-dashboard',{headers:{accept:'application/json'}})
    .then(response=>response.ok?response.json():Promise.reject(new Error('HTTP '+response.status)))
    .then(value=>{dashboardUi.payloadSource='served';dashboardUi.servedKpis=value?.review_kpis??EMPTY_REVIEW_KPIS;dashboardUi.servedRegister=value?.register??null;dashboardUi.loadError=null;renderReviewDashboard();})
    .catch(error=>{dashboardUi.payloadSource='embedded';dashboardUi.loadError=String(error);renderReviewDashboard();});
}

function dashboardStatusText(){
  if(dashboardUi.payloadSource==='served')return t('Live payload from the serve API.','Live-Payload aus der serve-API.');
  if(dashboardUi.loadError)return t('Serve API unreachable — showing the embedded payload.','serve-API nicht erreichbar — eingebettete Payload wird angezeigt.');
  return t('Embedded payload from the last analysis.','Eingebettete Payload der letzten Analyse.');
}

function renderReviewDashboardSidebar(){
  const sidebar=document.getElementById('sidebar');sidebar.textContent='';
  sidebar.appendChild(element('h2',t('Review Dashboard','Review-Dashboard')));
  sidebar.appendChild(element('p',dashboardStatusText()));
  sidebar.appendChild(element('p',t('KPIs computed by the analysis pipeline (propagation cost, SDP, zones, tangledness, hubs, drift, debt, API surface, leaks, risk gaps, dependencies, health). Read-only in the viewer; decisions persist only through the serve API.','Kennzahlen aus der Analyse-Pipeline (Propagationskosten, SDP, Zonen, Verflechtung, Hubs, Drift, Schulden, API-Oberfläche, Lecks, Risikolücken, Abhängigkeiten, Gesundheit). Im Viewer schreibgeschützt; Entscheidungen werden nur über die serve-API gespeichert.'),'muted'));
  if(!reviewKpisSnapshot())sidebar.appendChild(element('p',t('The analysis carries no review KPIs — not measured, never zero. Re-run archguard analyze with the KPI pipeline.','Die Analyse enthält keine Review-Kennzahlen — nicht gemessen, nie null. archguard analyze mit der KPI-Pipeline erneut ausführen.'),'muted'));
}

function renderReviewDashboard(){
  renderReviewDashboardSidebar();
  const root=document.getElementById('tab-review-dashboard');if(!root)return;
  const status=document.getElementById('dashboard-status');
  if(status){status.textContent=dashboardStatusText();status.setAttribute('data-source',dashboardUi.payloadSource);}
  const kpis=reviewKpisSnapshot();
  root.setAttribute('data-measured',kpis?'true':'false');
  // Each dash panel module (R37–R42) fills its container when loaded;
  // the skeleton stays functional without them and states the absence.
  for(const [panel,render] of [['dashboard-kpis','renderDashKpis'],['dashboard-zones','renderDashZones'],['dashboard-health','renderDashHealth'],['dashboard-decisions','renderDashDecisions'],['dashboard-pr','renderDashPr']]){
    const container=root.querySelector('#'+panel);if(!container)continue;
    if(typeof window[render]==='function'){window[render](container,kpis,reviewRegisterSnapshot?reviewRegisterSnapshot():null);}
    else if(!container.childElementCount)container.appendChild(element('p',t('Panel not loaded in this build.','Panel ist in diesem Build nicht geladen.'),'empty'));
  }
}

const dashboardRefresh=document.getElementById('dashboard-refresh');
if(dashboardRefresh)dashboardRefresh.addEventListener('click',loadDashboardPayload);
