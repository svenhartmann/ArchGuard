const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const model=vm.createContext({});vm.runInContext(fs.readFileSync('crates/archguard-report/src/viewer_world_model.js','utf8'),model);
const near=(a,b)=>assert.ok(Math.abs(a-b)<1e-7,`${a} != ${b}`);
const plain=value=>JSON.parse(JSON.stringify(value));
// The same ground point stays under the moving gesture center through an
// isometric turn, pinch and pan, even when the requested zoom is clamped.
for(const zoom of [.4,1,7.8,8])for(const factor of [.1,.8,1,1.7,40])for(const turn of [-Math.PI,-.5,0,.5,Math.PI]){
  const camera={x:31,y:-17,zoom,rotation:.7},lift=23,ground={x:47,y:-25};
  const projected=model.worldProject(ground.x,ground.y,0,camera.rotation);
  const previous={center:{x:camera.x+projected.x*zoom,y:camera.y+(projected.y+lift)*zoom},distance:120,angle:3};
  const current={center:{x:previous.center.x+42,y:previous.center.y-13},distance:120*factor,angle:3+turn};
  const baseline=JSON.stringify([camera,previous,current]);
  const next=model.worldGesture(camera,previous,current,lift),position=model.worldProject(ground.x,ground.y,0,next.rotation);
  near(next.x+position.x*next.zoom,current.center.x);near(next.y+(position.y+lift)*next.zoom,current.center.y);
  near(next.zoom,Math.max(.4,Math.min(8,zoom*factor)));
  assert.equal(JSON.stringify([camera,previous,current]),baseline);
}
const tiny={center:{x:0,y:0},distance:0,angle:0},normal={...tiny,distance:100},camera={x:3,y:4,zoom:2,rotation:.7};
assert.deepEqual(plain(model.worldGesture(camera,tiny,normal)),camera);
assert.deepEqual(plain(model.worldGesture(camera,normal,tiny)),camera);
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},
  {id:2,name:'p/A.cs',kind:'file',parent:0},{id:3,name:'q/B.cs',kind:'file',parent:1}];
const app=launch({root:'/project',nodes,edges:[{from:2,to:3,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},
  insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:nodes.filter(n=>n.kind==='file').map(n=>({id:n.name,node:n.name,class_id:n.name,max_complexity:3,coverage:null}))}}},{library:false});
app.run("showTab('start')");
const canvas=app.ids.get('world-canvas'),labels=app.ids.get('world-labels'),captured=new Set();
canvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
canvas.getContext=()=>new Proxy({},{get:(target,key)=>target[key]??(()=>{})});
canvas.setPointerCapture=id=>captured.add(id);canvas.hasPointerCapture=id=>captured.has(id);canvas.releasePointerCapture=id=>{captured.delete(id);canvas.dispatch('lostpointercapture',{pointerId:id});};
const send=(type,id,x,y,extras={})=>canvas.dispatch(type,{button:0,pointerType:'touch',pointerId:id,clientX:x,clientY:y,...extras});
const pose=()=>plain(app.run('worldUi.camera'));
app.run('worldDraw()');const original=app.run('JSON.stringify([data,work,ops,worldUi.layout])');
send('pointerdown',11,350,300);send('pointerdown',22,450,300);
assert.equal(labels.hidden,true);assert.equal(captured.size,2);
send('pointermove',11,300,300);send('pointermove',22,500,300);
near(pose().zoom,2);near(pose().rotation,0);near(pose().x,0);near(pose().y,0);
send('pointermove',11,400,200);send('pointermove',22,400,400);
near(pose().zoom,2);near(pose().rotation,Math.PI/2);
const rotated=pose();send('pointermove',11,430,220);send('pointermove',22,430,420);
near(pose().x,rotated.x+30);near(pose().y,rotated.y+20);near(pose().zoom,2);near(pose().rotation,Math.PI/2);
const beforeLift=pose();send('pointerup',22,430,420);
assert.deepEqual(pose(),beforeLift,'lifting a finger does not jump');assert.equal(labels.hidden,true);
send('pointermove',11,445,229);near(pose().x,beforeLift.x+15);near(pose().y,beforeLift.y+9);
send('pointerup',11,445,229);assert.equal(labels.hidden,false);assert.equal(captured.size,0);
assert.equal(app.run('worldUi.selection'),null,'a pinch and its remaining finger never select a surface');
assert.equal(app.run('worldUi.within'),null,'pinching does not navigate');
// A third contact does not replace the active pair. Releasing one of the first
// two rebases to the remaining contacts without a camera jump.
send('pointerdown',11,300,300);send('pointerdown',22,500,300);send('pointerdown',33,600,400);
const beforeThird=pose();send('pointermove',33,700,450);assert.deepEqual(pose(),beforeThird);
send('pointerup',11,300,300);assert.deepEqual(pose(),beforeThird);
send('pointermove',33,710,460);assert.notDeepEqual(pose(),beforeThird);
send('pointercancel',22,500,300);send('pointercancel',33,710,460);
assert.equal(captured.size,0);assert.equal(labels.hidden,false);assert.equal(app.run('worldUi.selection'),null);
// Lost capture cancels its pointer; a new independent gesture remains usable.
send('pointerdown',11,200,200);send('pointermove',11,230,210);send('lostpointercapture',11,230,210);
const cancelled=pose();send('pointermove',11,300,300);assert.deepEqual(pose(),cancelled);
send('pointerdown',22,250,250);send('pointermove',22,260,265);send('pointerup',22,260,265);
near(pose().x,cancelled.x+10);near(pose().y,cancelled.y+15);
// Right-drag rotates independently of pan; a stationary right click selects
// nothing. Shift-drag remains supported by the existing rotation suite.
app.ids.get('world-reset').click();const token=app.run('worldCameraAnim.token');
send('pointerdown',44,300,300,{pointerType:'mouse',button:2});assert.ok(app.run('worldCameraAnim.token')>token);
send('pointermove',44,400,350,{pointerType:'mouse',button:2});send('pointerup',44,400,350,{pointerType:'mouse',button:2});
near(pose().rotation,.8);near(pose().x,0);near(pose().y,0);near(pose().zoom,1);
send('pointerdown',44,300,300,{pointerType:'mouse',button:2});send('pointerup',44,300,300,{pointerType:'mouse',button:2});
assert.equal(app.run('worldUi.selection'),null);
let prevented=false;canvas.dispatch('contextmenu',{preventDefault(){prevented=true;}});assert.equal(prevented,true);
// Pinching to either zoom limit is spatial navigation only, not package drill.
for(const [startZoom,distance] of [[7.9,500],[.41,20]]){
  app.run(`worldUi.camera={x:0,y:0,zoom:${startZoom},rotation:0};worldUi.hover={area:'p'}`);
  send('pointerdown',11,350,300);send('pointerdown',22,450,300);
  send('pointermove',11,400-distance/2,300);send('pointermove',22,400+distance/2,300);
  send('pointerup',11,400-distance/2,300);send('pointerup',22,400+distance/2,300);
  assert.equal(app.run('worldUi.within'),null);assert.ok(pose().zoom>=.4&&pose().zoom<=8);
}
// A tap still selects using the current rotated hit geometry.
app.ids.get('world-reset').click();
const tap=plain(app.run('(()=>{const t=worldUi.hits.at(-1).triangles[0];return {x:t.reduce((s,p)=>s+p.x,0)/3,y:t.reduce((s,p)=>s+p.y,0)/3};})()'));
send('pointerdown',55,tap.x,tap.y);send('pointerup',55,tap.x,tap.y);assert.ok(app.run('worldUi.selection'));
app.run("setViewerLanguage('de')");assert.match(canvas.getAttribute('aria-label'),/Zwei-Finger|zwei Finger/);assert.match(app.run('worldHelp()'),/Rechtsziehen/);
assert.equal(app.run('JSON.stringify([data,work,ops,worldUi.layout])'),original,'gestures never alter source evidence or terrain layout');
console.log('World gestures: anchored isometric pinch/rotation/pan, limits, contact transitions, cancellation, capture, right-drag, animation interruption, taps, DE help and immutable evidence passed');
