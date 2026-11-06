// One top bar carries navigation; each workspace gets one context row below it.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
const between=(start,end)=>{const from=html.indexOf(start),to=html.indexOf(end,from);assert.ok(from>=0&&to>from,start+' precedes '+end);return html.slice(from,to);};
const top=between('<div id="topbar"','<div id="main"');
for(const id of ['project-menu','repository-navigation','tabs','filter','export-menu','workspace-mcp','share-actions','toggle-explorer','toggle-inspector','settings-menu','settings-content'])
  assert.ok(top.includes('id="'+id+'"'),id+' belongs to the top bar');
assert.ok(!top.includes('id="refactor-menu"'),'graph-only actions leave the top bar so it does not change between tabs');
assert.ok(between('<div id="graph-options"','<div id="viewport"').includes('id="refactor-menu"'),'refactoring sits with the other graph options');
const bar=between('<div class="workspace-bar"','<div id="graph-options"');
for(const id of ['breadcrumbs','summary'])assert.ok(bar.includes('id="'+id+'"'),id+' shares the compact context row');
const tabs=between('<div id="tabs"','</div>');
assert.deepEqual([...tabs.matchAll(/data-tab="([^"]+)"/g)].map(m=>m[1]),['start','architecture','graph','dsm','calls','services','quality','security','assessment'],'tabs are ordered as overview | structure | findings');
assert.equal((tabs.match(/class="tab-divider"/g)||[]).length,2,'two dividers separate the three tab groups');
assert.match(html,/\.report-workspace #toggle-explorer[^{]*\{\s*visibility:\s*hidden;/,'hidden panel toggles keep their place so the top bar does not jump');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=launch({root:'/tmp/topbar',nodes:[{id:0,name:'A',kind:'module',parent:null}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
assert.ok(app.ids.get('settings-content').children.some(child=>child.getAttribute('id')==='kpi-help-open'),'help opens from the settings menu instead of a loose top-bar link');
const settings=app.ids.get('settings-menu');settings.open=true;settings.dispatch('toggle');
app.ids.get('settings-content').children.find(child=>child.getAttribute('id')==='kpi-help-open').click();
assert.equal(settings.open,false,'opening help closes the menu it came from');
app.ids.get('export-menu').open=true;app.click('workspace-mcp');assert.equal(app.ids.get('export-menu').open,false,'share actions close the export menu');
assert.equal(app.run("WORKSPACE_MENUS['project-menu']"),'project-summary','the project menu follows the shared one-open/Escape rules');
assert.equal(app.ids.get('project-path').textContent,'/tmp/topbar','the project menu names the analyzed root');
console.log('Top bar: grouped tabs, project menu, share actions and stable layout passed');
