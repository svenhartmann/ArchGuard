const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'org',kind:'module',parent:null},{id:1,name:'org/app',kind:'module',parent:0},{id:2,name:'org/app/search',kind:'module',parent:1},{id:3,name:'org/app/client',kind:'module',parent:1},{id:4,name:'org/app/search/A.ts',kind:'file',parent:2},{id:5,name:'org/app/client/B.ts',kind:'file',parent:3},{id:6,name:'org/app/search/T.ts',kind:'file',parent:2}];
const fixture={developmentPaths:JSON.parse(fs.readFileSync('crates/archguard-core/src/development_paths.json','utf8')),root:'/tmp/start',nodes,edges:[{from:4,to:5,count:1,refs:[]},{from:5,to:6,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},semantic:{status:'partial',analyzers:[]}};
const app=launch(fixture,{library:false});
assert.ok(app.ids.has('tab-start'),'a dedicated task-oriented entry must exist');
app.run("showTab('start')");assert.equal(app.run('ui.workspace'),'start');
for(const id of ['start-tools','start-missions','start-paths'])assert.equal(app.ids.has(id),false,'duplicate tools navigation must be removed: '+id);
const atlas=JSON.parse(app.run('JSON.stringify(architectureAtlasModel(work))'));
assert.equal(atlas.scope,'org/app');assert.equal(atlas.cycle_groups,1);assert.equal(atlas.areas.length,2);assert.equal(atlas.areas.reduce((sum,n)=>sum+n.source_files,0),3);
assert.equal(app.ids.get('start-atlas').querySelectorAll('button[data-atlas-area]').length,2);
app.ids.get('start-atlas').querySelectorAll('button[data-atlas-area]')[0].click();assert.ok(app.run('startUi.within'));
app.run("startUi.within=null;renderStart()");
assert.equal(app.run('callModelPrepared'),false,'entry never decodes the Calls payload eagerly');
const before=app.run('JSON.stringify([data,work,ops])');
for(const view of ['graph','dsm','calls','services','architecture','assessment','quality','security']){
 const tab=app.ids.get('tabs').querySelectorAll('button').find(b=>b.dataset.tab===view);assert.ok(tab,view+' stays in the primary navigation');tab.click();assert.equal(app.run('ui.workspace'),view);
}
app.run("showTab('start');setViewerLanguage('de');globalThis.startCopied=null;copyText=text=>{globalThis.startCopied=text;}");
app.ids.get('workspace-mcp').click();assert.deepEqual(JSON.parse(app.run('startCopied')),{name:'development_guide',arguments:{path:'/tmp/start'}},'MCP start keeps all routes available without an invisible mission filter');
assert.equal(app.run('JSON.stringify([data,work,ops])'),before);
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
assert.doesNotMatch(html,/>Arrows and colors<|>Counting and coverage<|>View info</,'pure explanations belong to contextual help');
console.log('Development entry: actual package atlas, honest cycle scope, drill-down, navigation, lazy calls and immutable evidence passed');
