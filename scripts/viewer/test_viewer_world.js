const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const context=vm.createContext({});
vm.runInContext(fs.readFileSync('crates/archguard-report/src/viewer_world_model.js','utf8'),context);
const run=code=>vm.runInContext(code,context);
run('var camera={x:24,y:-10,zoom:1};var point={x:180,y:90};var next=worldZoom(camera,2,point);');
assert.equal(run('(point.x-next.x)/next.zoom'),run('(point.x-camera.x)/camera.zoom'));
assert.equal(run('(point.y-next.y)/next.zoom'),run('(point.y-camera.y)/camera.zoom'));
assert.equal(run('worldZoom(camera,100,point).zoom'),8);
assert.equal(run('worldZoom(camera,.001,point).zoom'),.4);
assert.equal(run('JSON.stringify(camera)'),'{"x":24,"y":-10,"zoom":1}');
run('var regions=[{name:"a",entities:[{id:"1",max_complexity:4},{id:"2",max_complexity:null}]},{name:"b",entities:[{id:"3",max_complexity:12}]}];var layout=worldLayout(regions);');
assert.equal(run('layout.cells.length'),3);
assert.equal(run('layout.cells.find(c=>c.entities[0].id==="2").value'),null);
assert.ok(run('layout.cells.find(c=>c.entities[0].id==="3").z>layout.cells.find(c=>c.entities[0].id==="1").z'));
assert.equal(run('JSON.stringify(worldLayout(regions))'),run('JSON.stringify(worldLayout(regions.slice().reverse()))'),'input order never changes positions');
assert.equal(run('worldLayout([{name:"large",entities:Array.from({length:10000},(_,i)=>({id:String(i),max_complexity:i}))}]).cells.reduce((n,c)=>n+c.entities.length,0)'),10000,'aggregation preserves every entity');
assert.ok(run('worldLayout([{name:"large",entities:Array.from({length:10000},(_,i)=>({id:String(i),max_complexity:i}))}]).cells.length<=900'));
// A metric lens selects its own value accessor and normalises heights to the
// highest value; the legacy complexity plate stays untouched without them.
run('var kpiLayout=worldLayout([{name:"a",entities:[{id:"1",cbo:1},{id:"2",cbo:99},{id:"3",cbo:null}]}],e=>e.cbo,v=>v===null?.08:.08+Math.log2(1+v)/Math.log2(100)*2);');
assert.equal(run('kpiLayout.cells.find(c=>c.entities[0].id==="3").value'),null,'unmeasured members never blend into zero');
assert.ok(run('Math.abs(kpiLayout.cells.find(c=>c.entities[0].id==="2").z-2.08)<1e-9'),'normalised top value reaches the full plate height');
assert.ok(run('kpiLayout.cells.find(c=>c.entities[0].id==="2").z>kpiLayout.cells.find(c=>c.entities[0].id==="1").z'));
assert.equal(run('worldLayout([{name:"a",entities:[{id:"1",max_complexity:4,cbo:99}]}]).cells[0].value'),4,'the default accessor still measures method complexity only');
console.log('World: cursor-anchored zoom, limits, immutable inputs, deterministic terrain, unknowns, lossless aggregation and metric accessors passed');
// Packets only ride connections that have a call or event direction.
for(const kind of ['http_call','grpc_call','event','configured_http'])assert.equal(run(`worldFlowCarrier(${JSON.stringify(kind)})`),true,kind);
for(const kind of ['configured_reference','startup'])assert.equal(run(`worldFlowCarrier(${JSON.stringify(kind)})`),false,kind);
run('var from={x:0,y:0},bend={x:200,y:0},to={x:400,y:0};');
assert.equal(run('worldFlowPackets(from,bend,to,0).length'),4,'long connections cap the packet count');
assert.equal(run('worldFlowPackets(from,{x:10,y:0},{x:20,y:0},0).length'),1,'short connections keep one packet');
assert.equal(run('worldFlowPackets(from,from,from,0).length'),1,'a zero-length connection cannot divide by zero');
assert.ok(run('worldFlowPackets(from,from,from,1).every(p=>Number.isFinite(p.x)&&Number.isFinite(p.y))'));
assert.ok(run('worldFlowPackets(from,bend,to,.1)[0].x>worldFlowPackets(from,bend,to,0)[0].x'),'packets travel from source to target');
assert.ok(run('worldFlowPackets(from,bend,to,3.7).every(p=>p.x>=0&&p.x<=400&&p.alpha>=0&&p.alpha<=1)'),'packets stay on the connection');
assert.ok(run('worldFlowPackets(from,bend,to,0)[0].alpha<.01'),'packets fade in at the source');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={developmentPaths:JSON.parse(fs.readFileSync('crates/archguard-core/src/development_paths.json','utf8')),root:'/project',nodes:[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'p/A.java',kind:'file',parent:0},{id:2,name:'q',kind:'module',parent:null},{id:3,name:'q/B.java',kind:'file',parent:2}],edges:[{from:1,to:3,count:1,refs:[]},{from:3,to:1,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:[{id:'class:p.A',node:'p/A.java',class_id:'p.A',max_complexity:7,coverage:null},{id:'file:q/B.java',node:'q/B.java',class_id:null,max_complexity:null,coverage:null}]}}};
const app=launch(fixture,{library:false});app.run("showTab('start')");
assert.equal(app.ids.get('start-world').hidden,false);
assert.equal(app.ids.get('world-lens-change'),undefined,'Change is not offered in World View');
assert.doesNotMatch(app.run('worldHelp()'),/change lens|Änderungslinse/i);
assert.equal(app.ids.get('start-atlas').hidden,true);
assert.equal(app.run('worldUi.model.areas.length'),2);
assert.equal(app.run('worldUi.model.areas.filter(a=>a.cyclic).length'),2);
assert.equal(app.run('callModelPrepared'),false);
const original=app.run('JSON.stringify([data,work,ops])');
app.ids.get('world-zoom-in').click();assert.ok(app.run('worldUi.camera.zoom>1'));
app.ids.get('world-fit').click();assert.equal(app.run('worldUi.camera.zoom'),1);
app.ids.get('world-lens-coverage').click();assert.equal(app.run('worldUi.lens'),'coverage');
assert.ok(app.ids.get('world-status').textContent.includes('Unknown')||app.ids.get('world-status').textContent.includes('Unbekannt'));
app.run("worldOpenArea('p')");assert.equal(app.run('worldUi.within'),'p');
app.ids.get('world-reset').click();assert.equal(app.run('worldUi.within'),null);assert.equal(app.run('worldUi.camera.zoom'),1);
assert.equal(app.run('JSON.stringify([data,work,ops])'),original);
console.log('World entry: real package cycles, unknown coverage, zoom/fit/reset, drilldown, immutable evidence and lazy calls passed');
app.run("data.findings=[{kind:'dependency_rule',message:'Forbidden dependency',nodes:['p/A.java'],evidence:[]}];renderWorld()");
assert.equal(app.run("worldUi.model.areas.find(a=>a.name==='p').findings.length"),1,'non-cycle findings also mark their actual area');
// The legend follows the active lens and explains observations without claiming
// unmeasured code is healthy or imported coverage proves passing tests.
const legend=app.ids.get('world-legend');assert.ok(legend,'the landscape needs an always visible color legend');
app.run("setViewerLanguage('de')");
const legendLabels=()=>Array.from(legend.querySelectorAll('button[data-world-legend]'),b=>b.textContent);
for(const [lens,labels] of [
 ['problems',['Zyklus / Befund','Klassen / Dateien']],
 ['complexity',['Gering → hoch','Unbekannt']],
 ['coverage',['Testlücken','Abgedeckt','Unbekannt']]
]){
 app.ids.get('world-lens-'+lens).click();assert.deepEqual(legendLabels(),labels,lens);
 const entry=legend.querySelectorAll('button[data-world-legend]')[0];const before=app.run('JSON.stringify([worldUi.camera,worldUi.selection,ops])');entry.click();
 assert.equal(app.ids.get('metric-tooltip').hidden,false,'legend help works on click/touch');assert.ok(app.ids.get('metric-tooltip').textContent.length>30);assert.equal(app.run('JSON.stringify([worldUi.camera,worldUi.selection,ops])'),before);
}
app.ids.get('world-lens-coverage').click();assert.equal(app.ids.get('metric-tooltip').hidden,true,'lens changes close stale legend help');
const unknown=legend.querySelectorAll('button[data-world-legend]')[2];unknown.dispatch('focus');assert.match(app.ids.get('metric-tooltip').textContent,/veraltet|fehlend/i);
app.run("setViewerLanguage('en')");assert.deepEqual(legendLabels(),['Test gaps','Covered','Unknown']);
app.ids.get('world-lens-complexity').click();
const gradient=legend.querySelectorAll('span[data-world-swatch]')[0];
assert.ok(gradient.style.background.includes(app.run('worldComplexityColor(7)')),'legend gradient uses the renderer color scale');
console.log('World legend: lens-specific colors, shared scale, unknown evidence, DE/EN and accessible help passed');
// KPI selection: a single class metric from the quality heatmap (CBO, RFC,
// DIT, NOC, WMC, LCOM4) colours and lifts the Explore surfaces. The join runs
// on the compiler class id; missing measurements stay unknown, never zero.
// Every option names its metric (title + DE title) so the dropdown explains
// itself before anything is selected; the ? button carries the full detail.
const kpiTemplate=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8').match(/<select id="world-kpi"[\s\S]*?<\/select>/)[0];
for(const kpi of ['complexity','cbo','rfc','dit','noc','wmc','lcom4']){
  assert.match(kpiTemplate,new RegExp('value="'+kpi+'"[^>]*title="[^"]{15,}"'),'option '+kpi+' has an English title');
  assert.match(kpiTemplate,new RegExp('value="'+kpi+'"[^>]*data-i18n-title-de="[^"]{15,}"'),'option '+kpi+' has a German title');
}
const kpiSelect=app.ids.get('world-kpi'),kpiHelp=app.ids.get('world-kpi-help');
assert.equal(app.run('worldUi.lens'),'complexity');
assert.equal(kpiSelect.disabled,false,'the selector is live in the Explore lens');assert.equal(kpiHelp.disabled,false);
kpiSelect.value='cbo';kpiSelect.dispatch('change');
assert.equal(app.run('worldUi.kpi'),'cbo');assert.equal(app.run('worldUi.lens'),'complexity');
assert.equal(app.run('worldUi.layout.cells.find(c=>c.area==="p").value'),null,'without imported class metrics the metric stays unknown');
app.run("data.insights.code_metrics={classes:[{class_id:'p.A',file:'p/A.java',cbo:{value:99,status:'ready'},rfc:{value:99,status:'ready'},dit:{value:2,status:'ready'},noc:{value:0,status:'ready'},wmc:{value:99,status:'ready'},lcom4:9}].concat(Array.from({length:5},(_,i)=>({class_id:'x.C'+i,file:'x/C'+i+'.java',cbo:{value:i+1,status:'ready'},rfc:{value:i+1,status:'ready'},dit:{value:1,status:'ready'},noc:{value:1,status:'ready'},wmc:{value:i+1,status:'ready'},lcom4:1})))};renderWorld()");
assert.equal(app.run('worldUi.layout.cells.find(c=>c.area==="p").value'),99,'Explore height follows the chosen class metric');
assert.equal(app.run('worldUi.layout.cells.find(c=>c.area==="q").value'),null,'files without a class row stay unknown');
assert.equal(app.run('worldKpiCellState(worldUi.layout.cells.find(c=>c.area==="p"))'),'high','cohort rating matches the class heatmap');
assert.match(app.ids.get('world-status').textContent,/CBO/,'the status line names the selected KPI');
const cboPins=app.ids.get('world-labels').querySelectorAll('button[data-world-area]').flatMap(b=>b.querySelectorAll('small'));
assert.ok(cboPins.some(s=>/CBO max\. 99/.test(s.textContent)),'pins announce the selected KPI and its peak');
app.run("worldUi.selection={area:'p',entity:worldUi.model.areas.find(a=>a.name==='p').entities[0]};worldDetails()");
assert.match(app.ids.get('world-detail').textContent,/CBO: 99/,'the detail panel rates the selected KPI');
app.run("setViewerLanguage('de')");app.run("renderWorld()");
assert.deepEqual(legendLabels(),['Niedrig','Erhöht','Hoch','Unbekannt'],'KPI legend uses the heatmap state words');
app.run("worldUi.kpi='noc';renderWorld()");
assert.deepEqual(legendLabels(),['Niedrig','Erhöht','Hoch','Information','Unbekannt'],'NOC stays informational, never a defect colour');
app.run("worldUi.kpi='complexity';renderWorld();setViewerLanguage('en')");
assert.deepEqual(legendLabels(),['Low → high','Unknown'],'the classic complexity legend is untouched');
app.ids.get('world-lens-problems').click();
assert.equal(kpiSelect.disabled,true,'the selector is inert outside the Explore lens');
kpiSelect.value='wmc';kpiSelect.dispatch('change');
assert.equal(app.run('worldUi.lens'),'complexity','picking a KPI enters the Explore lens');assert.equal(app.run('worldUi.kpi'),'wmc');
// The ? tooltip describes the currently selected KPI and follows the selection;
// hovering and clicking both open the shared, pinnable metric tooltip.
kpiHelp.dispatch('focus');assert.equal(app.ids.get('metric-tooltip').hidden,false,'the metric needs an explainer');
assert.match(app.ids.get('metric-tooltip').textContent,/weighted/i,'the help explains the selected KPI');
kpiSelect.value='cbo';kpiSelect.dispatch('change');kpiHelp.dispatch('focus');
assert.match(app.ids.get('metric-tooltip').textContent,/coupling between objects/i,'the tooltip follows the selection');
app.ids.get('metric-tooltip').hidden=true;
app.ids.get('world-lens-problems').click();
assert.equal(kpiHelp.disabled,true,'the metric explainer is inert outside the Explore lens');
app.run('worldUi.kpi="complexity";worldUi.lens="complexity";renderWorld()');kpiHelp.dispatch('focus');
assert.match(app.ids.get('metric-tooltip').textContent,/cyclomatic/i,'the classic complexity selection keeps its explanation');app.ids.get('metric-tooltip').hidden=true;
app.ids.get('world-reset').click();
assert.equal(app.run('worldUi.kpi'),'complexity','reset returns to method complexity');assert.equal(kpiSelect.value,'complexity');
console.log('World KPI lens: heatmap metrics selectable by class id, cohort states, normalised heights, pins, status, detail, DE/EN legend and reset passed');
