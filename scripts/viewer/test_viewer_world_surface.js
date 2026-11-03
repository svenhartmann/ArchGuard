const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},...['p/A','p/B','p/C','p/D','q/Q'].map((name,i)=>({id:i+2,name:name+'.java',kind:'file',parent:name.startsWith('p/')?0:1}))];
const app=launch({root:'/surface',nodes,edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:nodes.slice(2).map((n,i)=>({id:n.name,node:n.name,class_id:n.name,max_complexity:[30,null,null,2,4][i],coverage:null}))}}},{library:false});
const canvas=app.ids.get('world-canvas');
canvas.getBoundingClientRect=()=>({left:0,top:0,width:900,height:650});
canvas.getContext=()=>new Proxy({},{get(target,key){return target[key]??(()=>{});}});
app.run("showTab('start')");
const evidence=app.run('JSON.stringify([data,work,ops])');
for(const lens of ['problems','complexity','coverage']){
 app.run("worldUi.lens='"+lens+"';renderWorld()");
 const layout=app.run('JSON.stringify(worldUi.layout)');
 assert.equal(app.run('worldUi.layout.cells.filter(c=>c.value===null).length'),2,'unknown measurements remain unknown');
 for(const rotation of [0,.4,Math.PI/2,Math.PI,Math.PI*1.5]){
  app.run('worldUi.camera.rotation='+rotation+';worldDraw()');
  const hits=JSON.parse(app.run('JSON.stringify(worldUi.hits)')),corners=new Map();let shared=0;
  for(const hit of hits){
   const {x,y,area}=hit.cell;
   for(const [index,dx,dy] of [[0,0,0],[1,1,0],[2,1,1],[3,0,1]]){
    const key=JSON.stringify([area,x+dx,y+dy]),point=hit.ps[index];
    if(corners.has(key)){assert.deepEqual(point,corners.get(key),'adjacent known/unknown surfaces share exactly the same projected corner: '+lens+' / '+rotation+' / '+key);shared++;}
    else corners.set(key,point);
    assert.deepEqual(hit.triangles[index][0],point,'hit testing uses the drawn corner');
   }
  }
  assert.ok(shared>=7,'fixture exercises horizontal and vertical shared edges');
 }
 assert.equal(app.run('JSON.stringify(worldUi.layout)'),layout,'geometry repair does not invent measurement heights');
}
assert.equal(app.run('JSON.stringify([data,work,ops])'),evidence);
console.log('World surface: shared known/unknown corners across lenses and rotations, matching hit geometry and unchanged evidence passed');
