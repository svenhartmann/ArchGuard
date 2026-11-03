// Repository overview must not imply that unrelated project sources were analyzed together.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const project=(path,kind='application')=>({id:'project:'+path+'/pom.xml',name:'search',path,manifest:path+'/pom.xml',ecosystem:'maven',kind,runtime:kind==='application'?'azure-functions':null,evidence:[]});
const projects=[project('functions/search'),project('functions/import data'),project('tests/browser','tooling'),project('shared/core','library'),project('parent','workspace'),project('.', 'library')];
const services=projects.filter(p=>p.kind==='application').map(p=>({id:p.id,project:p.id,name:p.name,compose_file:'',build_context:p.path,networks:[],profiles:[]}));
const repository={mode:'overview',root:"/repo/company's search",active_project:null,projects,excluded_projects:[]};
const fixture={root:repository.root,nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},initialTab:'calls',insights:{repository},services:{schema_version:1,root:repository.root,monorepo:true,monorepo_reason:'Multiple projects',complete:true,projects,services,compose_files:[],dependencies:[],cycles:[],diagnostics:[]}};
const app=launch(fixture,{live:true,location:{protocol:'http:',href:'http://localhost:8123/'}});
assert.equal(app.run('ui.workspace'),'start','an unavailable initial or restored source workspace must fall back to the repository service overview');
assert.equal(app.graph.updates,0,'repository overview does not initialize an empty source graph');
assert.equal(app.run('fg'),null);
assert.equal(app.ids.get('summary').hidden,true);
assert.equal(app.ids.get('summary').textContent,'','zero import KPIs must not be presented as repository analysis');
for(const id of ['explorer','explorer-resizer','toggle-explorer','filter','refactor-menu'])assert.equal(app.ids.get(id).hidden,true,id+' is unavailable in repository overview');
assert.equal(app.ids.get('main').classList.contains('explorer-closed'),true);
for(const tab of app.ids.get('tabs').querySelectorAll('button')){
  const source=!['start','services','security'].includes(tab.dataset.tab);
  assert.equal(tab.hidden,source);assert.equal(tab.disabled,source);
}
for(const invalid of ['graph','calls','dsm','cycles','architecture']){
  app.run('showTab('+JSON.stringify(invalid)+')');assert.equal(app.run('ui.workspace'),'start');
}
app.run("setSimulation(true);askSimulation({cut_edge:{from:'a',to:'b'}},'Cut');recordOp({cut_edge:{from:'a',to:'b'}});scenarioOpen()");
assert.equal(app.run('ui.simulation'),false);assert.equal(app.run('ops.length'),0);
assert.equal(app.run('ui.pending'),null);assert.equal(app.ids.get('action-dialog').open,false);assert.equal(app.ids.get('scenario-dialog').open,false);
app.run("ui.workspace='graph';ui.simulation=true;ops=[{cut_edge:{from:'a',to:'b'}}];renderAll()");
assert.equal(app.run('ui.workspace'),'start');assert.equal(app.run('ops.length'),0);assert.equal(app.run('ui.simulation'),false);
assert.equal(app.ids.get('ops-strip').hidden,true);assert.equal(app.ids.get('simulation-summary').textContent,'');
for(const id of ['undo-ops','clear-ops','export-ops'])assert.equal(app.ids.get(id).disabled,true,'source operation controls stay disabled in repository overview');
assert.match(app.ids.get('repository-heading').textContent,/Monorepo/);
assert.match(app.ids.get('repository-scope').textContent,/separately/);
const selector=app.ids.get('repository-project');
assert.equal(selector.querySelectorAll('option').length,5,'applications, libraries and tools are selectable; workspace declarations are inventory only');
selector.value=projects[1].path;selector.dispatch('change');
assert.equal(app.ids.get('repository-open').getAttribute('href'),'?project=functions%2Fimport%20data#tab=start','navigation uses a same-origin query with the exact project path');
app.run("setViewerLanguage('de')");
assert.equal(selector.value,projects[1].path);assert.match(app.ids.get('repository-open').textContent,/Projekt/);
assert.match(app.ids.get('repository-scope').textContent,/getrennt/);
app.run("setViewerLanguage('en')");
assert.equal(selector.value,projects[1].path);
app.run('selectServiceProject('+JSON.stringify(projects[0].id)+')');
const inspectorLink=app.ids.get('sidebar').querySelectorAll('a[data-repository-project]')[0];
assert.equal(inspectorLink.getAttribute('href'),'?project=functions%2Fsearch#tab=start');
selector.value='../outside';selector.dispatch('change');
assert.notEqual(app.ids.get('repository-open').getAttribute('href'),'?project=..%2Foutside','arbitrary or unrecognized paths cannot become a project link');
app.run("ui.workspace='calls'");
const exported=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(exported.kind,'services','direct export guards a stale source-workspace value');
assert.equal(exported.repository.mode,'overview');assert.equal(exported.metrics.acd,undefined);assert.equal(exported.simulation_ops,undefined);
assert.equal(exported.projects.length,projects.length);
app.run("ui.workspace='graph'");assert.equal(app.run('exportSvgSource()'),null,'direct SVG export cannot reuse a hidden source diagram in overview');
app.run("showTab('security')");
assert.equal(app.run('ui.workspace'),'security');
assert.match(app.ids.get('security-snapshot-note').textContent,/Repository-wide/);
assert.doesNotMatch(app.ids.get('security-snapshot-note').textContent,/Top workspace KPIs/);
assert.equal(app.run('exportSnapshot().repository.mode'),'overview');
assert.match(app.run('exportMarkdown(exportSnapshot())'),/Repository scope.*company.*search/,'security documentation identifies the repository scan scope');

const active=projects[0].path;
const source={...fixture,root:repository.root+'/'+active,nodes:[{id:0,name:'A.java',kind:'file',parent:null}],initialTab:'graph',insights:{repository:{...repository,mode:'project',active_project:active,excluded_projects:['nested/app']}}};
const single=launch(source,{live:true,location:{protocol:'https:',href:'https://localhost/?project=functions%2Fsearch'}});
assert.equal(single.run('ui.workspace'),'graph');assert.equal(single.ids.get('summary').hidden,false);
assert.equal(single.ids.get('summary').children.filter(n=>n.className==='metric').length,4);
assert.equal(single.ids.get('repository-overview').getAttribute('href'),'/#tab=start');
assert.equal(single.ids.get('repository-project').value,active);
assert.match(single.ids.get('repository-scope').textContent,/nested\/app/);
assert.equal(single.ids.get('refactor-menu').hidden,false);
single.run("showTab('calls')");assert.equal(single.run('ui.workspace'),'calls');
single.run("showTab('security')");assert.match(single.ids.get('security-snapshot-note').textContent,/functions\/search/);
assert.match(single.run('exportMarkdown(exportSnapshot())'),/Project scope.*functions\/search/,'security documentation identifies the independently selected project');
let prevented=false;single.run("ops=[{cut_edge:{from:'a',to:'b'}}]");
single.ids.get('repository-overview').dispatch('click',{preventDefault(){prevented=true;}});
assert.equal(prevented,true,'project navigation must not discard an unsaved simulation');
assert.match(single.ids.get('feedback').textContent,/Export.*simulated/i);

const offline=launch({...fixture,initialTab:'services'},{library:false,live:true,location:{protocol:'file:',href:'file:///tmp/report.html'}});
assert.equal(offline.ids.get('repository-open').hidden,true);assert.equal(offline.ids.get('repository-open').getAttribute('href'),null);
assert.equal(offline.ids.get('repository-standalone').hidden,false);
assert.equal(offline.ids.get('repository-command').textContent,"archguard serve -- '/repo/company'\"'\"'s search'");
assert.match(offline.ids.get('repository-standalone-note').textContent,/snapshot.*server/i);
offline.run('selectServiceProject('+JSON.stringify(projects[0].id)+')');
assert.equal(offline.ids.get('sidebar').querySelectorAll('a[data-repository-project]').length,0,'standalone HTML must not offer nonfunctional project-query navigation');
assert.match(offline.ids.get('sidebar').textContent,/archguard serve/);
const staticHttp=launch(fixture,{library:false,location:{protocol:'https:',href:'https://reports.example/architecture.html'}});
assert.equal(staticHttp.ids.get('repository-open').hidden,true,'HTTP-hosted snapshots cannot navigate to a nonexistent project-analysis backend');
assert.equal(staticHttp.ids.get('repository-open').getAttribute('href'),null);
assert.equal(staticHttp.ids.get('repository-standalone').hidden,false);
staticHttp.run('window.archguardLiveViewer=true;renderRepositoryNavigation();renderSidebar()');
assert.equal(staticHttp.ids.get('repository-open').hidden,false,'the live server can enable navigation after the embedded viewer initializes');

const legacy=launch({...source,insights:{}},{library:false});
assert.equal(legacy.ids.get('repository-navigation').hidden,true);assert.equal(legacy.run('ui.workspace'),'graph');
assert.equal(legacy.ids.get('summary').hidden,false);
assert.equal(legacy.run('exportSnapshot().repository'),undefined);

const rootProject=launch({...source,root:repository.root,insights:{repository:{...repository,mode:'project',active_project:'.',excluded_projects:['functions/search','functions/import data']}}},{library:false,live:true,location:{protocol:'http:'}});
assert.equal(rootProject.ids.get('repository-project').value,'.','a root project remains selectable when the backend excludes its nested projects');
assert.equal(rootProject.ids.get('repository-overview').getAttribute('href'),'/#tab=start');
assert.equal(rootProject.ids.get('repository-open').getAttribute('href'),'?project=.#tab=start');
const many=Array.from({length:2005},(_,index)=>project('apps/'+index));
const bounded=launch({...fixture,services:{...fixture.services,projects:many,services:[]},insights:{repository:{...repository,projects:many}}},{library:false,live:true,location:{protocol:'http:'}});
assert.equal(bounded.ids.get('repository-project').querySelectorAll('option').length,2000,'the project selector stays bounded for large workspaces');
bounded.run("showTab('services')");
assert.equal(bounded.ids.get('services-project-list').querySelectorAll('button[data-service-project]').length,100);
assert.match(bounded.ids.get('repository-scope').textContent,/First 2000/);
bounded.run('selectServiceProject('+JSON.stringify(many[2004].id)+')');
assert.equal(bounded.ids.get('sidebar').querySelectorAll('a[data-repository-project]')[0].getAttribute('href'),'?project=apps%2F2004#tab=start','projects beyond the dropdown bound remain reachable through inventory inspection');
const workspace={...project('functions/search','workspace'),id:'project:functions/search/package.json',manifest:'functions/search/package.json'};
const overlapping=launch({...fixture,services:{...fixture.services,projects:[...projects,workspace]},insights:{repository:{...repository,projects:[...projects,workspace]}}},{library:false,live:true,location:{protocol:'http:'}});
overlapping.run('selectServiceProject('+JSON.stringify(workspace.id)+')');
assert.equal(overlapping.ids.get('sidebar').querySelectorAll('a[data-repository-project]').length,0,'workspace declarations remain non-selectable even when an application manifest shares their directory');

(async()=>{
  assert.equal(await app.run('scenarioRequestLoad({})'),false,'direct scenario imports are ignored in repository overview');
  assert.equal(app.run('ops.length'),0);assert.equal(app.ids.get('action-dialog').open,false);
  console.log('Repository UI: source-free overview, independent project navigation, honest snapshots, state guards, exports, DE/EN and legacy behavior passed');
})().catch(error=>{console.error(error);process.exitCode=1;});

// The repository landscape uses manifest projects, never invented source metrics.
app.run("showTab('start')");
assert.equal(app.ids.get('start-world').hidden,false);
assert.equal(app.run('worldUi.model.areas.length'),5);
assert.equal(app.run('new Set(worldUi.layout.cells.map(c=>c.area)).size'),5);
assert.equal(app.run('worldUi.layout.cells.every(c=>c.value===null)'),true);
assert.equal(app.run('data.nodes.length'),0);
assert.equal(app.ids.get('world-lens-complexity').hidden,true);
assert.equal(app.ids.get('world-labels').querySelectorAll('button[data-world-area]').length,5);
assert.match(app.ids.get('world-count').textContent,/projects|Projekte/);
assert.ok(app.ids.get('world-labels').textContent.includes('functions/search'));
app.run("window.location.assign=path=>{window.openedProject=path};worldOpenArea('functions/import data')");
assert.equal(app.run('window.openedProject'),'?project=functions%2Fimport%20data#tab=start');
app.run("worldOpenArea('../outside')");
assert.equal(app.run('window.openedProject'),'?project=functions%2Fimport%20data#tab=start');
offline.run("showTab('start');worldOpenArea('functions/search')");
assert.match(offline.ids.get('feedback').textContent,/server|Server/);
console.log('Repository World: complete project landscape, honest unknown metrics, safe project deep links and offline fallback passed');

single.run("window.location.assign=path=>{window.openedProject=path};repositoryOpenWorld(null)");
assert.equal(single.run('window.openedProject'),undefined,'World return navigation retains the unsaved simulation guard');
single.run("ops=[];repositoryOpenWorld(null)");
assert.equal(single.run('window.openedProject'),'/#tab=start');

app.run("data.insights.repository.mode='project';data.insights.repository.active_project='functions/search';data.insights.repository.projects.find(p=>p.path==='functions/search').ecosystem='go';renderRepositoryNavigation()");
assert.equal(app.ids.get('repository-scope').hidden,false);
assert.match(app.ids.get('repository-scope').textContent,/Go.*(SDK|SDK)/);
