// Canvas operation-count regressions for the World view: the drawing must stay
// proportional to the visible surfaces, and pointer bursts within one frame
// must paint once. Counts, not timings, so the test is machine independent.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const files=Array.from({length:40},(_,i)=>(i<20?'p/':'q/')+'C'+i+'.java');
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},...files.map((name,i)=>({id:i+2,name,kind:'file',parent:name.startsWith('p/')?0:1}))];
const app=launch({root:'/draw-cost',nodes,edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:files.map((name,i)=>({id:name,node:name,class_id:name,max_complexity:i%7}))}}},{library:false});
const canvas=app.ids.get('world-canvas'),W=900,H=650;
canvas.getBoundingClientRect=()=>({left:0,top:0,width:W,height:H});
const calls=[];
canvas.getContext=()=>new Proxy({},{get(target,key){if(key in target)return target[key];return (...args)=>{calls.push([key,...args]);return key==='createRadialGradient'||key==='createLinearGradient'?{addColorStop(){}}:undefined;};},set(target,key,value){target[key]=value;return true;}});
app.run("showTab('start')");
const cells=app.run('worldUi.layout.cells.length');
assert.ok(cells>=40,'fixture draws one surface per file');
calls.length=0;app.run('worldDraw()');
// The summit light used to clip every surface and then fill the whole canvas
// through that clip: one viewport-sized raster pass per surface.
const fullCanvas=calls.filter(([key,x,y,w,h])=>key==='fillRect'&&x===0&&y===0&&w===W&&h===H).length;
assert.ok(fullCanvas<=1,`only the backdrop may fill the whole canvas, got ${fullCanvas} for ${cells} surfaces`);
assert.equal(calls.filter(([key])=>key==='clip').length,0,'surfaces fill their own outline instead of clipping');
// Pointer moves arrive faster than the display refreshes. With frames
// available, a burst of moves paints once in the next frame.
app.run('var worldDrawCount=0,worldFrames=[];var originalWorldDraw=worldDraw;worldDraw=function(){worldDrawCount++;return originalWorldDraw();};globalThis.requestAnimationFrame=callback=>{worldFrames.push(callback);return worldFrames.length;};globalThis.cancelAnimationFrame=()=>{};');
const pointer=(type,x,y,extra={})=>canvas.dispatch(type,{pointerId:1,pointerType:'mouse',button:0,clientX:x,clientY:y,target:canvas,...extra});
pointer('pointerdown',300,300);
for(let i=1;i<=12;i++)pointer('pointermove',300+i*8,300+i*3);
assert.equal(app.run('worldDrawCount'),0,'drag moves defer painting to the next frame');
assert.ok(app.run('worldUi.camera.x')>80,'the camera follows every move immediately');
app.run('for(const frame of worldFrames.splice(0))frame(0)');
assert.equal(app.run('worldDrawCount'),1,'a burst of drag moves paints once per frame');
pointer('pointerup',396,336);
const afterRelease=app.run('worldDrawCount');
app.run('for(const frame of worldFrames.splice(0))frame(0)');
assert.ok(app.run('worldDrawCount')<=afterRelease+1);
app.run('worldDrawCount=0');
for(let i=0;i<6;i++)canvas.dispatch('wheel',{deltaY:-40,deltaMode:0,clientX:450,clientY:325,preventDefault(){}});
app.run('for(const frame of worldFrames.splice(0))frame(0)');
assert.equal(app.run('worldDrawCount'),1,'a burst of wheel ticks paints once per frame');
console.log(`World draw cost: ${cells} surfaces without full-canvas fills or clips; drag and wheel bursts paint once per frame`);
