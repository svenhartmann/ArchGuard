const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const model=vm.createContext({});vm.runInContext(fs.readFileSync('crates/archguard-report/src/viewer_world_model.js','utf8'),model);
const run=code=>vm.runInContext(code,model),near=(actual,expected)=>assert.ok(Math.abs(actual-expected)<1e-8,`${actual} != ${expected}`);
near(run('worldProject(1,0,0,Math.PI/2).x'),-.86);near(run('worldProject(1,0,0,Math.PI/2).y'),.44);
near(run('worldProject(2,3,4,Math.PI*2).x'),run('worldProject(2,3,4).x'));
near(run('worldProject(2,3,4,Math.PI*2).y'),run('worldProject(2,3,4).y'));
near(run('worldZoom({x:3,y:4,zoom:2,rotation:.7},2,{x:80,y:40}).rotation'),.7);
run('var camera={x:3,y:4,zoom:2,rotation:6};var rotated=worldRotate(camera,1)');
near(run('rotated.rotation'),7-Math.PI*2);assert.equal(run('JSON.stringify(camera)'),'{"x":3,"y":4,"zoom":2,"rotation":6}');
assert.equal(run('rotated.x'),3);assert.equal(run('rotated.zoom'),2);
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},
 ...['p/A.cs','p/B.cs','q/C.cs'].map((name,i)=>({id:i+2,name,kind:'file',parent:name.startsWith('p/')?0:1}))];
const app=launch({root:'/project',nodes,edges:[{from:2,to:4,count:1,refs:[]},{from:4,to:3,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},
 insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:nodes.filter(n=>n.kind==='file').map(n=>({id:n.name,node:n.name,class_id:n.name,max_complexity:3,coverage:null}))}}},{library:false});
app.run("showTab('start')");const canvas=app.ids.get('world-canvas');
canvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
canvas.getContext=()=>new Proxy({},{get(target,key){return target[key]??(()=>{});}});
app.run('worldUi.camera={x:30,y:-10,zoom:1.5,rotation:.7};worldDraw()');
const evidence=app.run('JSON.stringify([data,work,ops,worldUi.layout])'),geometry=app.run('JSON.stringify(worldUi.hits.map(h=>h.ps))');
app.ids.get('world-rotate-right').click();near(app.run('worldUi.camera.rotation'),.7+Math.PI/12);
app.ids.get('world-rotate-left').click();near(app.run('worldUi.camera.rotation'),.7);
app.ids.get('world-zoom-in').click();near(app.run('worldUi.camera.rotation'),.7);
app.ids.get('world-fit').click();near(app.run('worldUi.camera.rotation'),.7);assert.equal(app.run('worldUi.camera.zoom'),1);
canvas.dispatch('pointerdown',{button:0,pointerId:1,clientX:350,clientY:300,shiftKey:true});
canvas.dispatch('pointermove',{pointerId:1,clientX:450,clientY:320});canvas.dispatch('pointerup',{pointerId:1,clientX:450,clientY:320});
assert.ok(app.run('worldUi.camera.rotation>.7'),'Shift+drag rotates');assert.equal(app.run('worldUi.camera.x'),0,'rotation does not pan');assert.equal(app.run('worldUi.selection'),null,'rotation does not select');
assert.notEqual(app.run('JSON.stringify(worldUi.hits.map(h=>h.ps))'),geometry,'rotated geometry drives hit testing');
for(const rotation of [0,Math.PI/2,Math.PI,Math.PI*1.5]){
 app.run(`worldUi.camera.rotation=${rotation};worldDraw()`);
 const depth=Array.from(app.run('worldUi.hits.map(h=>worldProject(h.cell.x+.5,h.cell.y+.5,0,worldUi.camera.rotation).y)'));
 assert.deepEqual(depth,[...depth].sort((a,b)=>a-b),'painter order follows the current orientation');
}
const angle=app.run('worldUi.camera.rotation');
canvas.dispatch('pointerdown',{button:0,pointerId:2,clientX:300,clientY:300});canvas.dispatch('pointermove',{pointerId:2,clientX:330,clientY:320});canvas.dispatch('pointerup',{pointerId:2,clientX:330,clientY:320});
near(app.run('worldUi.camera.rotation'),angle);assert.equal(app.run('worldUi.camera.x'),30,'plain drag still pans');
app.ids.get('world-orientation').click();near(app.run('worldUi.camera.rotation'),0);assert.equal(app.run('worldUi.camera.x'),30,'orientation reset preserves pan');
canvas.dispatch('keydown',{key:'e'});near(app.run('worldUi.camera.rotation'),Math.PI/12);canvas.dispatch('keydown',{key:'q'});near(app.run('worldUi.camera.rotation'),0);
app.ids.get('world-rotate-right').click();const before=app.run('JSON.stringify(worldUi.camera)');app.run("worldOpenArea('p')");near(app.run('worldUi.camera.rotation'),Math.PI/12);app.ids.get('world-back').click();assert.equal(app.run('JSON.stringify(worldUi.camera)'),before);
app.ids.get('world-reset').click();near(app.run('worldUi.camera.rotation'),0);
assert.equal(app.run('JSON.stringify([data,work,ops,worldUi.layout])'),evidence,'rotation and navigation do not mutate analysis or layout');
console.log('World rotation: projection, cursor zoom, controls, Shift-drag, pan, painter order, hit geometry, history, reset and immutable evidence passed');
