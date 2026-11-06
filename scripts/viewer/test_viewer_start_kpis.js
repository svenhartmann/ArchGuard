const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
let harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
harness=harness.replace("'viewer_development.js'","'viewer_dash_kpis.js','viewer_dash_zones.js','viewer_dash_health.js','viewer_development.js'");
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/repo',nodes:[{id:0,name:'p',kind:'module',parent:null}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},reviewKpis:{health:{score:53.4,level:'yellow',trend:null},tangledness:{quote:.35,tangled_nodes:7,total:20},debt:{markers:91,loc:1000,per_kloc:91}}};
const app=launch(fixture,{library:false});app.run("showTab('start')");
const home=app.run('startUi.kpis');assert.ok(home,'Start contains the KPI overview');
const cards=home.querySelectorAll('strong');assert.ok(cards.some(n=>n.textContent==='53.4'));assert.ok(cards.some(n=>n.textContent==='35%'));assert.ok(cards.some(n=>n.textContent==='91'));
assert.equal(app.run('startUi.kpis.querySelectorAll("div").filter(n=>n.classList.contains("dash-card")).length'),8);
const before=app.run('JSON.stringify(data.reviewKpis)');app.run('renderStart();renderStart()');assert.equal(app.run('JSON.stringify(data.reviewKpis)'),before);assert.equal(app.run('startUi.kpis'),home);
assert.ok(app.run('Array.from(document.querySelectorAll("#tabs button")).filter(b=>b.dataset.tab==="assessment").every(b=>b.hidden)'));
app.run('data.reviewKpis.health.score=null;renderStart()');assert.match(home.textContent,/not measured/);assert.doesNotMatch(home.textContent,/53.4/);
app.run('data.reviewKpis=null;renderStart()');assert.match(home.textContent,/Not measured/);assert.equal(home.querySelectorAll('strong').length,0);
console.log('Start KPIs: eight measurements, unknowns, immutable payload, repeat rendering and hidden legacy tabs passed');
// Zones: the KPI counts reach the panel, and a point exactly on the corridor
// edge (A=0, I=2/3, D'=1/3 up to float rounding) is drawn as main sequence.
const zoneFixture={...fixture,reviewKpis:{zones:{zone_of_pain:2,zone_of_uselessness:0,main_sequence:1,unmeasured:0}},performance:{abstractions:[
  {scope:'node',name:'edge',abstractness:0,instability:2/3,coverage:'complete'},
  {scope:'node',name:'pain',abstractness:0,instability:1/3,coverage:'complete'},
  {scope:'scc',name:'scc-1',abstractness:0,instability:0,coverage:'complete'}]}};
const zoneApp=launch(zoneFixture,{library:false});
zoneApp.run("var zonePanel=element('div');renderDashZones(zonePanel,data.reviewKpis)");
assert.match(zoneApp.run('zonePanel.textContent'),/2 pain · 0 uselessness · 1 main sequence · 0 unmeasured/);
const zonesOf=zoneApp.run("(function walk(n,out){if(n.getAttribute&&n.getAttribute('data-dash-point'))out[n.getAttribute('data-dash-point')]=n.getAttribute('data-dash-zone');for(const c of n.children||n.childNodes||[])walk(c,out);return out;})(zonePanel,{})");
assert.deepEqual({...zonesOf},{edge:'main',pain:'pain'},'SCC metrics are not plotted; the corridor edge is main');
console.log('Start zones: KPI counts, corridor edge and node-only points passed');
