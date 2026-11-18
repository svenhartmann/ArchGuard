const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
// Shapes follow archguard_core::GoalAssessment (ADR 0003), serialized into insights.architecture.goal_assessment.
const finding={kind:'layer_violation',id:'layer-1',severity:'error',message:'ui imports db <img src=x onerror=alert(1)>',nodes:['ui/A.ts'],edge_count:1,cycles_sampled:0,suggested_cuts:[]};
const goal=(name,status,extra={})=>({goal:name,priority:'high',attribute:'maintainability',scope:[],status,findings:[],finding_count_by_severity:{},kpis:[],evidence_sources:[],gaps:[],...extra});
const goals=[
  goal('Changeable',"findings",{findings:['layer-1','missing-2'],finding_count_by_severity:{error:1,warning:1},kpis:[{name:'cycle_count',value:0}],evidence_sources:[{source:'static_analysis',present:true},{source:'review_kpis',present:true}],scope:['ui/**']}),
  goal('Clean core',"no_findings",{evidence_sources:[{source:'static_analysis',present:true}],gaps:['No findings from the available sources.']}),
  goal('Fast checkout',"no_evidence",{attribute:'performance_efficiency',evidence_sources:[{source:'measurement_runs',present:false}],gaps:['No saved measurement runs.']}),
  goal('Easy to use',"not_assessable",{attribute:'usability',gaps:['ArchGuard has no evidence source for usability.']}),
  goal('Vague',"unclassified",{attribute:null,gaps:['Which ISO 25010 characteristic does "Vague" address?']}),
];
const fixture={root:'/repo',nodes:[{id:0,name:'ui/A.ts',kind:'file',parent:null,lang:'ts'}],edges:[],findings:[finding],cyclicEdges:[],view:{depth:0,visible:null},insights:{architecture:{goal_assessment:goals,observed:{}}}};
const app=launch(fixture);app.run("showTab('architecture')");
const content=app.ids.get('architecture-content');
const section=content.querySelectorAll('section').find(s=>s.getAttribute('data-quality-goals')==='true');
assert.ok(section,'the architecture tab shows the quality goal assessment');
const rows=Array.from(section.querySelectorAll('details').filter(d=>d.getAttribute('data-goal-status')));
assert.deepEqual(rows.map(r=>r.getAttribute('data-goal-status')),['findings','no_findings','no_evidence','not_assessable','unclassified'],'one entry per goal in file order');
assert.deepEqual(rows.map(r=>r.getAttribute('data-goal-measured')),['true','true','false','false','false'],'not measured is kept apart from findings');
assert.match(rows[0].textContent,/Changeable.*Findings/);
assert.match(rows[0].textContent,/maintainability/);assert.match(rows[0].textContent,/ui\/\*\*/);
assert.match(rows[0].textContent,/layer-1.*error.*ui imports db <img/,'linked findings show their report message as text');
assert.equal(section.querySelectorAll('img').length,0,'finding text is never parsed as HTML');
assert.match(rows[0].textContent,/missing-2/,'a finding outside the payload keeps its id');
assert.match(rows[0].textContent,/error 1.*warning 1/);
assert.match(rows[0].textContent,/cycle_count 0/,'a measured zero stays a value');
assert.match(rows[0].textContent,/static_analysis.*present/);
assert.match(rows[1].textContent,/No findings from the available sources/);
assert.match(rows[2].textContent,/Not measured.*No evidence/);assert.match(rows[2].textContent,/measurement_runs.*missing/);assert.match(rows[2].textContent,/No saved measurement runs/);
assert.doesNotMatch(rows[2].textContent,/Findings \(/,'no findings line for a goal without evidence');
assert.match(rows[3].textContent,/Not measured.*Not assessable/);
assert.match(rows[4].textContent,/Not measured.*No characteristic given/);assert.match(rows[4].textContent,/not given/);
assert.match(section.textContent,/2 measured · 3 not measured/);
app.click('language-de');
const de=app.ids.get('architecture-content').querySelectorAll('section').find(s=>s.getAttribute('data-quality-goals')==='true');
assert.match(de.textContent,/Qualitätsziele/);assert.match(de.textContent,/Nicht gemessen/);assert.match(de.textContent,/Keine Belege/);assert.match(de.textContent,/Befunde/);assert.match(de.textContent,/fehlt/);
// No declared goals: the section says so instead of disappearing silently.
const none=launch({...fixture,insights:{architecture:{goal_assessment:[],observed:{}}}});none.run("showTab('architecture')");
assert.match(none.ids.get('architecture-content').textContent,/No quality goals declared/);
// Older reports without the field: nothing is invented.
const legacy=launch({...fixture,insights:{}});legacy.run("showTab('architecture')");
assert.ok(!legacy.ids.get('architecture-content').querySelectorAll('section').some(s=>s.getAttribute('data-quality-goals')),'reports without goal_assessment show no goal section');
// `archguard assessment --format html` nests the report in the review session.
const session=launch({...fixture,insights:{review_session:{report:{architecture:{goal_assessment:goals,observed:{}}}}}});session.run("showTab('architecture')");
assert.match(session.ids.get('architecture-content').textContent,/2 measured · 3 not measured/,'goals of an assessment report show too');
// Many findings stay bounded.
const many=launch({...fixture,insights:{architecture:{goal_assessment:[goal('Big',"findings",{findings:Array.from({length:45},(_,i)=>'f-'+i)})],observed:{}}}});many.run("showTab('architecture')");
assert.match(many.ids.get('architecture-content').textContent,/and 15 more/);
console.log('Quality goals: status per goal, linked findings, evidence sources, not measured apart from findings, safe text and DE/EN passed');
