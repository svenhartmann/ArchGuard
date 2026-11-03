// Starterseite (Welt): every package gets a name pin in every lens, deep zoom
// over a package opens it fitted, and the stage offers browser fullscreen.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});

// Twelve sibling packages, no cycles and no findings: exactly the state the
// user reported, where packages without cycle marks lost their name pins.
const many=[];const entities=[];
for(let i=0;i<12;i++){
  const p=String(i).padStart(2,'0');
  many.push({id:2*i,name:'pkg'+p,kind:'module',parent:null});
  many.push({id:2*i+1,name:'pkg'+p+'/Main'+p+'.kt',kind:'file',parent:2*i});
  entities.push({id:'class:Main'+p,node:'pkg'+p+'/Main'+p+'.kt',class_id:'Main'+p,max_complexity:i,coverage:null});
}
const base={root:'/project/todo',developmentPaths:JSON.parse(fs.readFileSync('crates/archguard-core/src/development_paths.json','utf8')),edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'b'.repeat(64),status:'partial',entities}}};
const manyApp=launch({...base,nodes:many},{library:false});
manyApp.run("showTab('start');renderWorld()");
for(const lens of ['problems','complexity','coverage','change']){
  manyApp.run("worldUi.lens='"+lens+"';renderWorld()");
  const pins=manyApp.ids.get('world-labels').querySelectorAll('button[data-world-area]');
  assert.equal(pins.length,12,lens+' lens labels every package, not only the top-ranked ones');
  assert.ok(pins.some(b=>b.getAttribute('data-world-area')==='pkg11'),'even the smallest acyclic package gets a name');
}
// Deep zoom continues exploration: wheeling on at the 800 % maximum over a
// package opens it like a double-click, fitted to the new scope.
const embyNodes=[{id:0,name:'Emby',kind:'module',parent:null},{id:1,name:'Emby/Naming',kind:'module',parent:0},{id:2,name:'Emby/Naming/Video',kind:'module',parent:1},{id:3,name:'Emby/Naming/Video/Resolver.cs',kind:'file',parent:2},{id:4,name:'MediaBrowser',kind:'module',parent:null},{id:5,name:'MediaBrowser/Model.cs',kind:'file',parent:4}];
const embyEntities=[{id:'class:Resolver',node:embyNodes[3].name,class_id:'Resolver',max_complexity:8,coverage:null},{id:'class:Model',node:embyNodes[5].name,class_id:'Model',max_complexity:2,coverage:null}];
const emby={...base,nodes:embyNodes,edges:[],findings:[],insights:{world:{...base.insights.world,entities:embyEntities}}};
const zoomApp=launch(emby,{library:false});
zoomApp.run("showTab('start')");
const canvas=zoomApp.ids.get('world-canvas');
canvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
function hoverOver(area){
  zoomApp.run("worldUi.hits=[{cell:{area:'"+area+"',entities:[{id:'c1',node:'x',class_id:'X',max_complexity:1}]},triangles:[[{x:0,y:0},{x:400,y:0},{x:0,y:400}]]}]");
  canvas.dispatch('pointermove',{clientX:20,clientY:20,pointerId:1});
}
hoverOver('Emby');
assert.equal(zoomApp.run('worldUi.hover.area'),'Emby','pointermove sets the hover cell');
zoomApp.run('worldUi.camera={x:0,y:0,zoom:8,rotation:0}');
canvas.dispatch('wheel',{deltaY:-120,clientX:20,clientY:20});
assert.equal(zoomApp.run('worldUi.within'),'Emby','wheeling on at the zoom maximum opens the package under the pointer');
assert.equal(zoomApp.run('worldUi.camera.zoom'),1,'the opened package is framed by the fit');
// Below the threshold the same wheel only zooms.
const gentle=launch(emby,{library:false});gentle.run("showTab('start')");
const gentleCanvas=gentle.ids.get('world-canvas');gentleCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
gentle.run("worldUi.hits=[{cell:{area:'Emby',entities:[{id:'c1',node:'x',class_id:'X',max_complexity:1}]},triangles:[[{x:0,y:0},{x:400,y:0},{x:0,y:400}]]}]");
gentleCanvas.dispatch('pointermove',{clientX:20,clientY:20,pointerId:1});
gentle.run('worldUi.camera={x:0,y:0,zoom:2,rotation:0}');
gentleCanvas.dispatch('wheel',{deltaY:-120,clientX:20,clientY:20});
assert.equal(gentle.run('worldUi.within'),null,'zoom below the threshold never navigates');
// No hover: zooming at the maximum keeps the camera.
const bare=launch(emby,{library:false});bare.run("showTab('start')");
const bareCanvas=bare.ids.get('world-canvas');bareCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
bare.run('worldUi.camera={x:0,y:0,zoom:8,rotation:0};worldUi.hover=null;worldUi.hits=[]');
bareCanvas.dispatch('wheel',{deltaY:-120,clientX:400,clientY:400});
assert.equal(bare.run('worldUi.within'),null,'a further wheel without a package under the pointer never navigates');
// Files at the threshold select their entry instead of drilling the tree.
const fileHover=launch(emby,{library:false});fileHover.run("showTab('start');worldUi.camera={x:0,y:0,zoom:8,rotation:0}");
const fileCanvas=fileHover.ids.get('world-canvas');fileCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
fileHover.run("worldUi.hits=[{cell:{area:'MediaBrowser/Model.cs',entities:[{id:'class:Model',node:'MediaBrowser/Model.cs',class_id:'Model'}]},triangles:[[{x:0,y:0},{x:400,y:0},{x:0,y:400}]]}]");
fileCanvas.dispatch('pointermove',{clientX:20,clientY:20,pointerId:1});
fileCanvas.dispatch('wheel',{deltaY:-120,clientX:20,clientY:20});
assert.equal(fileHover.run('worldUi.within'),null,'a file surface never changes the scope');

// Zooming out at the minimum mirrors the drill-in: an opened package hands
// the view back to the scope before it, framed as it was when it opened.
const popApp=launch(emby,{library:false});popApp.run("showTab('start');worldOpenArea('Emby')");
assert.equal(popApp.run('worldUi.within'),'Emby','the fixture starts inside the opened package');
assert.equal(popApp.run('worldUi.history.length'),1,'opening recorded the previous view');
const popCanvas=popApp.ids.get('world-canvas');popCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
popApp.run('worldUi.camera={x:0,y:0,zoom:.4,rotation:0}');
popCanvas.dispatch('wheel',{deltaY:120,clientX:20,clientY:20});
assert.equal(popApp.run('worldUi.within'),null,'wheeling out at the zoom minimum returns to the previous world');
assert.equal(popApp.run('worldUi.history.length'),0,'the pop consumes the recorded view like Back does');
assert.equal(popApp.run('worldUi.camera.zoom'),1,'the parent view is framed as it was when the package opened');
// Above the minimum the same wheel only shrinks the view.
const shallow=launch(emby,{library:false});shallow.run("showTab('start');worldOpenArea('Emby')");
const shallowCanvas=shallow.ids.get('world-canvas');shallowCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
shallowCanvas.dispatch('wheel',{deltaY:120,clientX:20,clientY:20});
assert.equal(shallow.run('worldUi.within'),'Emby','zooming out above the minimum never navigates');
// At the root the extra wheel-out stays a camera move.
const rootOut=launch(emby,{library:false});rootOut.run("showTab('start');worldUi.camera={x:0,y:0,zoom:.4,rotation:0}");
const rootCanvas=rootOut.ids.get('world-canvas');rootCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
rootCanvas.dispatch('wheel',{deltaY:120,clientX:20,clientY:20});
assert.equal(rootOut.run('worldUi.within'),null,'a wheel-out at the root never invents a navigation');
// While zooming, the pin overlay is taken out of the way and restored after
// the last wheel tick placed it; the flicker between the two irritates less
// than pins that jump with every event.
const overApp=launch(emby,{library:false});
const overTimers=[];let overTimerId=0;
overApp.context.setTimeout=(fn)=>{overTimers[++overTimerId]=fn;return overTimerId;};
overApp.context.clearTimeout=(id)=>{delete overTimers[id];};
overApp.run("showTab('start')");
const overCanvas=overApp.ids.get('world-canvas');overCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
const pinBox=overApp.ids.get('world-labels'),hiddenSets=[];
let hiddenValue=pinBox.hidden;
Object.defineProperty(pinBox,'hidden',{get:()=>hiddenValue,set:v=>{hiddenSets.push(v);hiddenValue=v;}});
overCanvas.dispatch('wheel',{deltaY:-120,clientX:20,clientY:20});
assert.deepEqual(hiddenSets,[true],'the pins are taken out of the way during the gesture');
assert.equal(pinBox.hidden,true,'they stay hidden while the wheel keeps firing');
overCanvas.dispatch('wheel',{deltaY:-120,clientX:20,clientY:20});
Object.values(overTimers).forEach(fn=>fn());
assert.deepEqual(hiddenSets,[true,true,false],'each tick re-arms the restore, the last one reveals');
assert.equal(pinBox.hidden,false,'the overlay is visible again once the gesture ends');

// The stage toolbar toggles browser fullscreen.
const fsApp=launch(emby,{library:false});fsApp.run("showTab('start')");
const fsButton=fsApp.ids.get('world-fullscreen');
assert.ok(fsButton,'the stage controls offer a fullscreen toggle');
let entered=0,exited=0;
const root=fsApp.document.documentElement;
root.requestFullscreen=()=>{entered++;fsApp.document.fullscreenElement=root;return Promise.resolve();};
fsApp.document.exitFullscreen=()=>{exited++;fsApp.document.fullscreenElement=null;return Promise.resolve();};
fsButton.click();assert.equal(entered,1);assert.equal(fsButton.getAttribute('aria-pressed'),'true');
fsButton.click();assert.equal(exited,1);assert.equal(fsButton.getAttribute('aria-pressed'),'false');
const noFs=launch(emby,{library:false});noFs.run("showTab('start')");
noFs.document.documentElement.requestFullscreen=undefined;noFs.document.exitFullscreen=undefined;
noFs.ids.get('world-fullscreen').click();
assert.match(noFs.ids.get('feedback').textContent,/fullscreen|vollbild/i,'a missing fullscreen API is reported, not thrown');
console.log('world labels, zoom drill-in and fullscreen passed');
