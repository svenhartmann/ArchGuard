const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const metric=value=>({value,observed:1,status:value==null?'partial':'ready',evidence:['pkg/B']});
const fixture={root:'/tmp/kpis',nodes:[{id:0,name:'pkg',kind:'module',parent:null},{id:1,name:'pkg/A.java',kind:'file',parent:0}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{
 code_metrics:{status:'partial',definition:'source-metrics/v1',classes:[{class_id:'A',file:'pkg/A.java',cbo:metric(3),rfc:metric(5),dit:metric(1),noc:metric(null),wmc:metric(8),lcom4:2}],methods:[{method_id:'A#run()',class_id:'A',file:'pkg/A.java',line:2,cognitive:7,maintainability_index:81.4,source_lines:6,halstead:{volume:53,difficulty:3,effort:159}}],diagnostics:[]},
 knowledge_metrics:{status:'ready',files:[{node:'pkg/A.java',touches:4,contributors:2,top_share:0.75,bus_factor_50:1,status:'ready',authors:[{author:'one@example.test',touches:3},{author:'two@example.test',touches:1}]}],packages:[],diagnostics:[]},
 measurement_runs:{status:'ready',flaky_test_percent:100,groups:[{kind:'test',name:'<img src=x> test',revision:'a'.repeat(40),environment:'ci',producer:'runner/1',samples:2,passed:1,failed:1,skipped:0,duration_samples:1,inconsistent:true,failure_percent:50,median_seconds:0.2,p95_seconds:0.2,run_ids:['one','two']}],diagnostics:[]}
}};
const app=launch(fixture,{library:false});app.run("showTab('quality')");
function select(value){app.ids.get('engineering-section').value=value;app.ids.get('engineering-section').dispatch('change');return app.ids.get('engineering-list').textContent;}
assert.match(select('classes'),/CBO.*3/);assert.match(app.ids.get('engineering-list').textContent,/NOC.*—/);
assert.doesNotMatch(app.ids.get('engineering-status').textContent,/imported|importiert/,'compiler metrics are analyzed, not imported');
const measuredApp=launch({...fixture,insights:{...fixture.insights,code_metrics:{...fixture.insights.code_metrics,status:'ready'}}},{library:false});measuredApp.run("showTab('quality');engineeringUi.section='classes';renderEngineeringQuality()");assert.doesNotMatch(measuredApp.ids.get('engineering-status').textContent,/imported|importiert/);
assert.match(select('methods'),/Cognitive.*7/);assert.match(app.ids.get('engineering-list').textContent,/81.4/);
assert.match(select('knowledge'),/75/);assert.match(app.ids.get('engineering-list').textContent,/Bus factor.*1/);
assert.match(select('measurements'),/mixed|inconsistent/i);assert.equal(app.ids.get('engineering-list').querySelectorAll('img').length,0);
assert.match(app.ids.get('engineering-list').textContent,/0.2/);
const exported=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));assert.equal(exported.code_metrics.classes.length,1);assert.equal(exported.measurement_runs.groups.length,1);
assert.match(app.run('exportMarkdown(exportSnapshot())'),/CBO/);assert.match(app.run('exportMarkdown(exportSnapshot())'),/Halstead/);
app.run("setViewerLanguage('de')");assert.match(select('classes'),/CBO/);assert.match(select('knowledge'),/Wissen|Beitrag|Verteilung/);
const old=launch({...fixture,insights:{}},{library:false});old.run("showTab('quality');engineeringUi.section='methods';renderEngineeringQuality()");assert.match(old.ids.get('engineering-list').textContent,/No matching|Keine passenden/);
const history=launch({...fixture,insights:{quality_history:{status:'ready',series:[{timestamp:1,tag:'before',modules:[{node:'pkg',metrics:{'cognitive.max':7}}]},{timestamp:2,tag:'after',modules:[{node:'pkg',metrics:{'cognitive.max':3}}]}],latest:{modules:[{node:'pkg',extended:{'cognitive.max':{value:3,measured:1,population:1}}}]}}}},{library:false});history.run("showTab('quality');engineeringUi.section='history';renderEngineeringQuality()");
assert.equal(history.ids.get('engineering-list').querySelectorAll('svg[data-quality-history]').length,1,'history needs a compact measurement chart');
assert.equal(history.ids.get('engineering-list').querySelectorAll('circle').length,2,'unknown samples are not plotted as zero');
console.log('Advanced metric UI: measurements, unknowns, scope, export, safe text and DE/EN passed');
