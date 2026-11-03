// Compact presentation must retain scope, coverage, evidence and navigation.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const metric=value=>({value,observed:value??9,status:value==null?'partial':'ready',evidence:[]});
const row={class_id:'org.example.Search',file:'src/Search.java',node:'src/Search.java',cbo:metric(null),rfc:metric(4),dit:metric(1),noc:metric(null),wmc:metric(3),lcom4:1};
const fixture={root:'/tmp/clean-ui',nodes:[{id:0,name:'src/Search.java',kind:'file',lang:'java',parent:null}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},semantic:{status:'partial',analyzers:[],diagnostics:['Missing compiler dependency'],projected_calls:0,projected_supertypes:0},insights:{code_metrics:{status:'partial',classes:[row],methods:[]}},calls:{status:'partial',files:1,classes:[{id:row.class_id,name:row.class_id,file:row.file,graph_node:row.node,package:'org.example',kind:'class',language:'java'}],methods:[],calls:[],diagnostics:['Missing compiler dependency']}};
const app=launch(fixture,{library:false});
assert.equal(app.ids.get('edge-reference').textContent,'Select a package or file for incoming/outgoing dependencies.','empty graph reference is a single-sentence status that names the next step');
const coverage=app.ids.get('summary').querySelector('span[data-graph-coverage]');
assert.ok(coverage,'graph coverage uses a compact status instead of a paragraph');
assert.match(coverage.textContent,/partial/i);assert.ok(coverage.textContent.length<45);
assert.match(coverage.getAttribute('data-metric-description'),/snapshot|source|type/i,'coverage details stay available in contextual help');
coverage.dispatch('focus');assert.ok(app.ids.get('metric-tooltip').textContent.length>100);
for(const tab of ['calls','services']){app.run(`showTab('${tab}')`);assert.ok(app.document.documentElement.classList.contains('context-workspace'));}
app.run("showTab('security')");assert.ok(app.document.documentElement.classList.contains('report-workspace'),'security has its own metrics, without unrelated graph panels');
app.run("showTab('calls')");assert.match(app.ids.get('calls-status').textContent,/partial/i);assert.ok(app.ids.get('calls-status').textContent.length<80);assert.match(app.ids.get('calls-diagnostics').textContent,/original source snapshot/i);assert.match(app.ids.get('calls-diagnostics').textContent,/Missing compiler dependency/);
app.run("showTab('quality')");
const pick=app.ids.get('engineering-list').querySelector('button[data-class-select]');assert.equal(pick.textContent,'Search');assert.match(pick.getAttribute('data-metric-description'),/org\.example\.Search.*src\/Search\.java/);pick.click();
const unknown=app.ids.get('engineering-list').querySelectorAll('strong[data-metric-key]').find(n=>n.getAttribute('data-metric-key')==='cbo');assert.match(unknown.getAttribute('data-metric-description'),/observed 9/i,'lower bounds remain in metric help');
const scope=html.match(/<details id="engineering-scope-options"([^>]*)>([\s\S]*?)<\/details>/);assert.ok(scope);assert.doesNotMatch(scope[1],/\bopen\b/);assert.match(scope[2],/id="engineering-target"/);assert.doesNotMatch(scope[2],/id="engineering-query"/,'everyday search stays directly reachable');
app.ids.get('engineering-target').value=row.node;app.ids.get('engineering-target').dispatch('change');assert.match(app.ids.get('engineering-scope-summary').textContent,/Search.java/,'active scope remains visible with filters closed');
app.run("showTab('graph');setViewerLanguage('de')");assert.equal(app.document.documentElement.classList.contains('context-workspace'),false);assert.equal(app.document.documentElement.classList.contains('report-workspace'),false);assert.match(app.ids.get('summary').querySelector('span[data-graph-coverage]').textContent,/teilweise/);
assert.doesNotMatch(html,/ARCHITECTURE WORKSPACE/,'remove redundant brand prose');
assert.equal(app.run('JSON.stringify(data.nodes)'),JSON.stringify(fixture.nodes));
console.log('Clean UI: short statuses, retained evidence/tooltips, contextual panels, scope visibility and class navigation passed');
