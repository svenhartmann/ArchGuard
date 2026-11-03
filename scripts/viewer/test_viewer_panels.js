// Resize the real embedded viewer through its separator events, without a browser.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launchRaw=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
// Keep the resize contract covered for the optional pinned layout.
const launch=(fixture,options)=>{const app=launchRaw(fixture,options);app.run("document.getElementById('main').classList.add('inspector-pinned');if(window.innerWidth>1100)openInspector();setWidth()");return app;};
const fixture={root:'/tmp/panels',nodes:[{id:0,name:'A',kind:'file',parent:null},{id:1,name:'B',kind:'file',parent:null}],edges:[{from:0,to:1,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const key='archguard.panel-widths.v1';
function memory(value){const values=new Map(value===undefined?[]:[[key,value]]),writes=[];return {values,writes,getItem:k=>values.get(k)??null,setItem(k,v){values.set(k,v);if(k===key)writes.push(JSON.parse(v));}};}
const handle=(app,panel)=>app.ids.get(panel+'-resizer');
const pixels=(app,panel)=>Number.parseFloat(app.ids.get('main').style['--'+panel+'-width']);
const aria=(app,panel,field)=>Number(handle(app,panel).getAttribute('aria-value'+field));
function documentEvent(app,type,event={}){for(const callback of app.document.events[type]||[])callback({preventDefault(){},stopPropagation(){},...event});}
function begin(app,panel,x=300,id=1){
  const h=handle(app,panel);h.captures=[];h.releases=[];
  h.setPointerCapture=value=>h.captures.push(value);h.releasePointerCapture=value=>h.releases.push(value);
  h.dispatch('pointerdown',{pointerId:id,button:0,clientX:x});return h;
}
const storage=memory(),app=launch(fixture,{storage});
assert.ok(handle(app,'explorer')&&handle(app,'inspector'),'both panels have an actual resize separator');
assert.equal(pixels(app,'explorer'),238);assert.equal(pixels(app,'inspector'),420);
assert.equal(aria(app,'explorer','now'),238);assert.equal(aria(app,'explorer','min'),180);
assert.equal(aria(app,'inspector','min'),220);
app.run("selectNode('A');treeZoom=1.5");
const analysis=app.run('JSON.stringify([work,view,ui.selection,trail,ops,treeZoom])'),updates=app.graph.updates;
const explorer=begin(app,'explorer');
for(let x=301;x<=400;x++)documentEvent(app,'pointermove',{pointerId:1,clientX:x});
assert.equal(pixels(app,'explorer'),338);
assert.equal(storage.writes.length,0,'pointer motion does not continuously write preferences');
assert.equal(app.ids.get('main').classList.contains('panel-resizing'),true);
documentEvent(app,'pointermove',{pointerId:2,clientX:9000});assert.equal(pixels(app,'explorer'),338,'a second pointer cannot hijack the drag');
documentEvent(app,'pointerup',{pointerId:1});
assert.deepEqual(explorer.captures,[1]);assert.deepEqual(explorer.releases,[1]);
assert.equal(storage.writes.at(-1).explorer,338);
assert.equal(app.ids.get('main').classList.contains('panel-resizing'),false);
assert.equal(app.run('JSON.stringify([work,view,ui.selection,trail,ops,treeZoom])'),analysis,'resizing preserves analysis, selection, navigation, simulation and tree zoom');
assert.equal(app.graph.updates,updates,'resizing does not rebuild graph data');
begin(app,'explorer');const maximum=aria(app,'explorer','max');
documentEvent(app,'pointermove',{pointerId:1,clientX:100000});assert.equal(pixels(app,'explorer'),maximum);
documentEvent(app,'pointerup',{pointerId:1});
assert.ok(1440-pixels(app,'explorer')-pixels(app,'inspector')-12>=360,'desktop keeps the center usable');
const beforeCancel=pixels(app,'explorer'),writeCount=storage.writes.length;
begin(app,'explorer');documentEvent(app,'pointermove',{pointerId:1,clientX:-1000});assert.equal(pixels(app,'explorer'),180);
documentEvent(app,'pointercancel',{pointerId:1});assert.equal(pixels(app,'explorer'),beforeCancel);assert.equal(storage.writes.length,writeCount);
assert.equal(app.ids.get('main').classList.contains('panel-resizing'),false);
begin(app,'explorer');documentEvent(app,'pointermove',{pointerId:1,clientX:330});handle(app,'explorer').dispatch('lostpointercapture',{pointerId:1});assert.equal(pixels(app,'explorer'),beforeCancel);
begin(app,'explorer');documentEvent(app,'pointermove',{pointerId:1,clientX:-1000});
let escapeStopped=false;handle(app,'explorer').dispatch('keydown',{key:'Escape',stopPropagation(){escapeStopped=true;}});
assert.equal(escapeStopped,true,'Escape cancellation must not bubble into the viewer selection-clearing shortcut');
assert.equal(pixels(app,'explorer'),beforeCancel);assert.equal(app.run('ui.selection.name'),'A');
begin(app,'explorer');documentEvent(app,'pointermove',{pointerId:1,clientX:-1000});app.click('toggle-explorer');
assert.equal(app.ids.get('main').classList.contains('panel-resizing'),false,'closing the actively resized panel releases its drag');
app.click('toggle-explorer');assert.equal(pixels(app,'explorer'),beforeCancel,'reopening restores the width before the canceled drag');
handle(app,'explorer').dispatch('pointerdown',{pointerId:1,button:2,clientX:300});documentEvent(app,'pointermove',{pointerId:1,clientX:500});assert.equal(pixels(app,'explorer'),beforeCancel,'right-click never starts resizing');

const keys=launch(fixture,{storage:memory()});
handle(keys,'explorer').dispatch('keydown',{key:'ArrowRight'});assert.equal(pixels(keys,'explorer'),254);
handle(keys,'explorer').dispatch('keydown',{key:'ArrowLeft',shiftKey:true});assert.equal(pixels(keys,'explorer'),206);
handle(keys,'inspector').dispatch('keydown',{key:'ArrowLeft'});assert.equal(pixels(keys,'inspector'),436,'leftward inspector handle enlarges the right panel');
handle(keys,'inspector').dispatch('keydown',{key:'ArrowRight',shiftKey:true});assert.equal(pixels(keys,'inspector'),388);
handle(keys,'explorer').dispatch('keydown',{key:'Home'});assert.equal(pixels(keys,'explorer'),180);
const keyMax=aria(keys,'explorer','max');handle(keys,'explorer').dispatch('keydown',{key:'End'});assert.equal(pixels(keys,'explorer'),keyMax);
handle(keys,'explorer').dispatch('dblclick');assert.equal(pixels(keys,'explorer'),238);
handle(keys,'inspector').dispatch('dblclick');assert.equal(pixels(keys,'inspector'),420);
const saved=pixels(keys,'explorer');keys.click('toggle-explorer');assert.equal(handle(keys,'explorer').hidden,true);assert.equal(handle(keys,'explorer').getAttribute('tabindex'),'-1');
keys.click('toggle-explorer');assert.equal(handle(keys,'explorer').hidden,false);assert.equal(pixels(keys,'explorer'),saved);
const listenerCount=handle(keys,'explorer').events.pointerdown.length;keys.run('initPanelResize()');assert.equal(handle(keys,'explorer').events.pointerdown.length,listenerCount,'initialization is idempotent');

const largeStorage=memory(JSON.stringify({explorer:600,inspector:700})),responsive=launch(fixture,{width:2000,storage:largeStorage});
assert.equal(pixels(responsive,'explorer'),600);assert.equal(pixels(responsive,'inspector'),700);
responsive.context.window.innerWidth=1200;responsive.run('setWidth()');
assert.ok(pixels(responsive,'explorer')>=180&&pixels(responsive,'inspector')>=220);
assert.ok(1200-pixels(responsive,'explorer')-pixels(responsive,'inspector')-12>=360);
assert.ok(pixels(responsive,'explorer')<600&&pixels(responsive,'inspector')<700,'both sides share responsive reduction');
assert.equal(largeStorage.writes.length,0,'responsive clamping is never persisted as user intent');
responsive.context.window.innerWidth=2000;responsive.run('setWidth()');assert.equal(pixels(responsive,'explorer'),600);assert.equal(pixels(responsive,'inspector'),700);
responsive.context.window.innerWidth=1200;responsive.run('setWidth()');
const blockedPreference=largeStorage.writes.length;
begin(responsive,'explorer');documentEvent(responsive,'pointermove',{pointerId:1,clientX:10000});documentEvent(responsive,'pointerup',{pointerId:1});
assert.equal(largeStorage.writes.length,blockedPreference,'dragging past a responsive maximum without changing its width must not replace the wider saved preference');
const opposite=pixels(responsive,'inspector');handle(responsive,'explorer').dispatch('keydown',{key:'ArrowLeft'});
assert.equal(pixels(responsive,'inspector'),opposite,'resizing one constrained panel does not make the other jump');
assert.equal(largeStorage.writes.at(-1).inspector,700,'resizing one panel does not overwrite the other preferred width with a responsive clamp');

const mobileStorage=memory(JSON.stringify({explorer:500,inspector:600})),mobile=launch(fixture,{width:400,storage:mobileStorage});
assert.equal(handle(mobile,'explorer').hidden,true);assert.equal(handle(mobile,'inspector').hidden,true);
mobile.click('toggle-explorer');assert.equal(handle(mobile,'explorer').hidden,false);assert.equal(pixels(mobile,'explorer'),352);assert.equal(aria(mobile,'explorer','max'),352);
mobile.click('toggle-inspector');assert.equal(handle(mobile,'explorer').hidden,true);assert.equal(handle(mobile,'inspector').hidden,false);assert.equal(pixels(mobile,'inspector'),352);
mobile.context.window.innerWidth=2000;mobile.run('setWidth()');mobile.click('toggle-explorer');assert.equal(pixels(mobile,'explorer'),500);assert.equal(pixels(mobile,'inspector'),600);assert.equal(mobileStorage.writes.length,0);
begin(mobile,'explorer');documentEvent(mobile,'pointermove',{pointerId:1,clientX:350});
mobile.context.window.innerWidth=700;mobile.run('setWidth()');
assert.equal(mobile.ids.get('main').classList.contains('panel-resizing'),false,'breakpoint change cancels active capture');
assert.equal(handle(mobile,'explorer').hidden,false);assert.equal(handle(mobile,'inspector').hidden,true,'without selection the explorer stays open on compact transition');
assert.equal(mobileStorage.writes.length,0);assert.equal(pixels(mobile,'explorer'),500,'canceled compact transition restores the preferred width');
const selected=launch(fixture);selected.run("selectNode('A')");selected.context.window.innerWidth=700;selected.run('setWidth()');
assert.equal(handle(selected,'explorer').hidden,true);assert.equal(handle(selected,'inspector').hidden,false,'with selection the inspector stays open on compact transition');
assert.equal(selected.ids.get('toggle-explorer').getAttribute('aria-expanded'),'false');

for(const value of ['not-json','null','[]',JSON.stringify({explorer:'400',inspector:null})]){
  const invalid=launch(fixture,{storage:memory(value)});assert.equal(pixels(invalid,'explorer'),238);assert.equal(pixels(invalid,'inspector'),420);
}
const restored=launch(fixture,{storage:memory(JSON.stringify({explorer:320,inspector:380}))});assert.equal(pixels(restored,'explorer'),320);assert.equal(pixels(restored,'inspector'),380);
const blocked=launch(fixture);handle(blocked,'explorer').dispatch('keydown',{key:'ArrowRight'});assert.equal(pixels(blocked,'explorer'),254,'blocked localStorage cannot prevent resizing');
blocked.ids.get('main').getBoundingClientRect=()=>({width:0,height:600});blocked.run('syncPanelSizes()');assert.equal(pixels(blocked,'explorer'),254,'zero measurement uses the window width');
console.log('Panel resizing: pointer, keyboard, capture cancellation, responsive bounds, overlays, persistence, blocked storage and unchanged analysis/zoom passed');
