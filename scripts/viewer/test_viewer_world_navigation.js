const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'Emby',kind:'module',parent:null},{id:1,name:'Emby/Naming',kind:'module',parent:0},{id:2,name:'Emby/Naming/Video',kind:'module',parent:1},{id:3,name:'Emby/Naming/Video/Resolver.cs',kind:'file',parent:2},{id:4,name:'MediaBrowser',kind:'module',parent:null},{id:5,name:'MediaBrowser/Model.cs',kind:'file',parent:4}];
const entities=[{id:'class:Resolver',node:nodes[3].name,class_id:'Resolver',max_complexity:8,coverage:null},{id:'class:Model',node:nodes[5].name,class_id:'Model',max_complexity:2,coverage:null}];
const fixture={root:'/project/jellyfin',developmentPaths:JSON.parse(fs.readFileSync('crates/archguard-core/src/development_paths.json','utf8')),nodes,edges:[{from:3,to:5,count:1,refs:[]},{from:5,to:3,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities}}};
const app=launch(fixture,{library:false});app.run("showTab('start');worldUi.lens='complexity';worldUi.camera={x:50,y:-20,zoom:2};renderWorld()");
const original=app.run('JSON.stringify([data,work,ops])');
const pin=name=>app.ids.get('world-labels').querySelectorAll('button[data-world-area]').find(b=>b.getAttribute('data-world-area')===name);
pin('Emby').click();assert.equal(app.run('worldUi.within'),'Emby','package label opens its children directly');
assert.equal(app.run('worldUi.camera.zoom'),1,'fit the opened package');assert.equal(app.run('worldUi.lens'),'complexity','navigation preserves the perspective');
assert.equal(app.ids.get('world-detail').hidden,true);assert.equal(app.ids.get('metric-tooltip').hidden,true,'navigation closes detached tooltips');
assert.equal(app.document.activeElement,app.ids.get('world-scope'),'keyboard focus follows navigation');
pin('Emby/Naming').click();pin('Emby/Naming/Video').click();
const crumbs=()=>app.ids.get('world-scope').querySelectorAll('button[data-world-scope]');
assert.deepEqual(Array.from(crumbs(),b=>b.textContent),['jellyfin','Emby','Naming','Video']);
assert.equal(crumbs().at(-1).getAttribute('aria-current'),'page');
crumbs().find(b=>b.getAttribute('data-world-scope')==='Emby').click();assert.equal(app.run('worldUi.within'),'Emby','breadcrumb jumps to ancestor');
app.ids.get('world-back').click();assert.equal(app.run('worldUi.within'),'Emby/Naming/Video','back restores the previously visited scope');
app.ids.get('world-back').click();assert.equal(app.run('worldUi.within'),'Emby/Naming');
app.ids.get('world-back').click();assert.equal(app.run('worldUi.within'),'Emby');
app.ids.get('world-back').click();assert.equal(app.run('worldUi.within'),null);assert.equal(app.run('JSON.stringify(worldUi.camera)'),'{"x":50,"y":-20,"zoom":2}');assert.equal(app.ids.get('world-back').disabled,true);
pin('MediaBrowser').click();pin('MediaBrowser/Model.cs').click();assert.equal(app.run('worldUi.within'),'MediaBrowser','file label keeps the package scope');assert.equal(app.ids.get('world-detail').hidden,false,'file label opens details');
const selectedView=app.run('JSON.stringify([worldUi.within,worldUi.camera,worldUi.selection])');
app.run('setWorldDetailsOpen(false)');assert.equal(app.ids.get('world-detail').hidden,true);
pin('MediaBrowser/Model.cs').click();assert.equal(app.ids.get('world-detail').hidden,false,'Show details must reopen the same selected file after closing its overlay');
assert.equal(app.run('JSON.stringify([worldUi.within,worldUi.camera,worldUi.selection])'),selectedView,'reopening details preserves the landscape and selection');
crumbs()[0].click();assert.equal(app.run('worldUi.within'),null);
const stable=app.run('JSON.stringify([worldUi.within,worldUi.history])');app.run("worldOpenArea('missing')");assert.equal(app.run('JSON.stringify([worldUi.within,worldUi.history])'),stable);
// Double-clicking the terrain enters its package; single click still selects evidence.
app.run("worldUi.hits=[{cell:{area:'Emby'},triangles:[[{x:0,y:0},{x:100,y:0},{x:0,y:100}]]}]");
app.ids.get('world-canvas').getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
app.ids.get('world-canvas').dispatch('dblclick',{button:0,clientX:20,clientY:20});assert.equal(app.run('worldUi.within'),'Emby');
app.ids.get('world-reset').click();assert.equal(app.run('worldUi.history.length'),0);assert.equal(app.run('worldUi.within'),null);assert.equal(app.ids.get('world-back').disabled,true);
assert.equal(app.run('JSON.stringify([data,work,ops])'),original);assert.equal(app.run('callModelPrepared'),false);
// The automatic initial prefix is the overview, not a loop when navigating back.
const single=launch({...fixture,nodes:nodes.slice(0,4),edges:[],insights:{world:{...fixture.insights.world,entities:entities.slice(0,1)}}},{library:false});single.run("showTab('start')");assert.equal(single.run('worldUi.model.scope'),'Emby/Naming/Video');
assert.equal(single.ids.get('world-scope').querySelectorAll('button[data-world-scope]').length,1);
const linked=launch(fixture,{library:false,location:{hash:'#world=Emby%2FNaming'}});linked.run("showTab('start')");
assert.equal(linked.run('worldUi.within'),'Emby/Naming','a package link opens the requested source scope');
linked.ids.get('world-back').click();assert.equal(linked.run('worldUi.within'),null,'linked scope retains a route to the full project');
for(const hash of ['#world=missing','#world=%broken','#world=Emby%2FNaming%2FVideo%2FResolver.cs']){
 const invalid=launch(fixture,{library:false,location:{hash}});invalid.run("showTab('start')");assert.equal(invalid.run('worldUi.within'),null,'invalid and file-only links keep the overview');
}
console.log('World package navigation: direct labels, nested scopes, breadcrumbs, back/zoom restoration, terrain double-click, file details, reset and immutable evidence passed');
