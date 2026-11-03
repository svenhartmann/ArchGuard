// Pointer interaction owns the terrain; image export requires its toolbar action.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},{id:2,name:'p/A.cs',kind:'file',parent:0},{id:3,name:'q/B.cs',kind:'file',parent:1}];
const app=launch({root:'/project',nodes,edges:[{from:2,to:3,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:nodes.filter(n=>n.kind==='file').map(n=>({id:n.name,node:n.name,class_id:n.name,max_complexity:3,coverage:null}))}}},{library:false});
const canvas=app.ids.get('world-canvas'),labels=app.ids.get('world-labels');
canvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
canvas.getContext=()=>new Proxy({},{get:(target,key)=>target[key]??(()=>{})});
let imageExports=0;canvas.toDataURL=()=>{imageExports++;return 'data:image/png;base64,AAA';};
app.run("showTab('start');worldDraw()");
const evidence=app.run('JSON.stringify([data,work,ops])');
for(const target of [canvas,labels])for(const event of [{button:2},{button:0,ctrlKey:true}]){
 let prevented=false;target.dispatch('contextmenu',{...event,preventDefault(){prevented=true;}});
 assert.equal(prevented,true,'the World surface suppresses native Save image menus, including macOS Ctrl-click');
}
const point=()=>JSON.parse(app.run('JSON.stringify((()=>{const h=worldUi.hits.at(-1),t=h.triangles[0];return {x:t.reduce((s,p)=>s+p.x,0)/3,y:t.reduce((s,p)=>s+p.y,0)/3,area:h.cell.area};})())'));
const send=(type,p,event={})=>canvas.dispatch(type,{pointerId:1,pointerType:'mouse',button:0,clientX:p.x,clientY:p.y,...event});
for(const event of [{button:2},{button:0,ctrlKey:true},{button:0,shiftKey:true}]){
 app.click('world-reset');const p=point();send('pointerdown',p,event);send('pointerup',p,event);
 assert.equal(app.run('worldUi.selection'),null,'secondary clicks must not select terrain');
 assert.equal(app.run('worldUi.within'),null,'secondary clicks must not navigate');
 send('pointerdown',p,event);send('pointermove',{x:p.x+50,y:p.y+15},event);
 assert.equal(labels.hidden,false,'package labels stay visible while mouse rotation is still in progress');
 send('pointerup',{x:p.x+50,y:p.y+15},event);
 assert.equal(labels.hidden,false,'package labels remain visible after rotation');
 assert.ok(Math.abs(app.run('worldUi.camera.rotation')-.4)<1e-8,'secondary dragging rotates');
 assert.equal(app.run('worldUi.camera.x'),0);assert.equal(app.run('worldUi.camera.y'),0);
}
app.click('world-reset');const p=point();send('pointerdown',p);send('pointerup',p);
assert.equal(app.run('worldUi.selection.area'),p.area,'normal primary click still selects');
app.click('world-reset');send('pointerdown',p);send('pointermove',{x:p.x+40,y:p.y+20});send('pointerup',{x:p.x+40,y:p.y+20});
assert.equal(app.run('worldUi.camera.x'),40);assert.equal(app.run('worldUi.camera.y'),20);assert.equal(app.run('worldUi.camera.rotation'),0);
assert.equal(app.run('worldUi.selection'),null,'primary dragging only pans');
assert.equal(imageExports,0,'pointer interactions must never export an image');
app.click('world-export');assert.equal(imageExports,1,'the explicit PNG export remains available');
assert.equal(app.run('JSON.stringify([data,work,ops])'),evidence);
console.log('World pointer menu: native menu suppressed, Ctrl/right drag rotates, primary click selects, primary drag pans, explicit export retained');
