const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
// A package cycle does not imply that its classes form a file cycle.
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},
 ...['p/A.cs','p/D.cs','q/B.cs','q/C.cs','p/Unrelated.cs'].map((name,i)=>({id:i+2,name,kind:'file',parent:name.startsWith('p/')?0:1}))];
const fixture={root:'/project',nodes,edges:[{from:2,to:4,count:1,refs:[]},{from:5,to:3,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},
 insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:nodes.filter(n=>n.kind==='file').map(n=>({id:n.name,node:n.name,class_id:n.name,max_complexity:3,coverage:null}))}}};
const app=launch(fixture,{library:false});app.run("showTab('start');setViewerLanguage('en')");
assert.equal(app.run('worldUi.model.cycle_groups'),1);
assert.equal(app.run('findSccs(data).length'),0,'fixture has no file cycle');
const original=app.run('JSON.stringify([data,work,ops])'),paint=[];let path=[];
const ctx={setTransform(){},clearRect(){},beginPath(){path=[];},closePath(){},moveTo(...p){path.push(['move',...p]);},lineTo(...p){path.push(['line',...p]);},quadraticCurveTo(...p){path.push(['curve',...p]);},arc(...p){path.push(['arc',...p]);},fill(){paint.push({kind:'fill',color:this.fillStyle,path:[...path]});},stroke(){paint.push({kind:'stroke',color:this.strokeStyle,path:[...path]});}};
app.context.getComputedStyle=()=>({getPropertyValue:name=>name});app.ids.get('world-canvas').getContext=()=>ctx;
const draw=()=>{paint.length=0;app.run('worldDraw()');};draw();
const redFills=()=>paint.filter(p=>p.kind==='fill'&&p.color==='--c-cycle');
const redBorders=()=>paint.filter(p=>p.kind==='stroke'&&p.color==='--c-cycle'&&p.path.length===4&&p.path.every(v=>v[0]==='move'||v[0]==='line'));
const cycleEdges=()=>paint.filter(p=>p.kind==='stroke'&&p.color==='--c-cycle'&&p.path.some(v=>v[0]==='curve'));
assert.equal(redFills().length,2,'only the two direction arrows may be red; package membership must not fill class surfaces red');
assert.equal(redBorders().length,2,'both cyclic package boundaries remain marked');
assert.equal(cycleEdges().length,2,'both observed cycle directions remain visible');
const controls=cycleEdges().map(p=>p.path.find(v=>v[0]==='curve').slice(1,3));assert.notDeepEqual(controls[0],controls[1],'opposite directions use distinct curves');
assert.ok(paint.filter(p=>p.kind==='fill'&&p.color==='--accent').length>=5,'all classes, including unrelated ones, retain a neutral surface');
assert.equal(app.run('JSON.stringify([data,work,ops])'),original);
app.run("worldOpenArea('p')");draw();assert.equal(cycleEdges().length,0,'opening the package does not invent an internal cycle');assert.equal(redFills().length,0);
app.run("worldNavigate(null);data.findings=[{kind:'dependency_rule',message:'Forbidden dependency',nodes:['p/A.cs'],evidence:[]}];renderWorld()");
const pin=app.ids.get('world-labels').querySelectorAll('button[data-world-area]').find(b=>b.getAttribute('data-world-area')==='p');
assert.match(pin.textContent,/Package cycle/);assert.match(pin.textContent,/1 findings/,'findings remain visible alongside a cycle');
app.run('data.edges=[];renderWorld()');draw();assert.equal(cycleEdges().length,0);assert.equal(redFills().length,0,'a package finding also must not color every class red');assert.equal(redBorders().length,1,'the area with a finding keeps its outline');
console.log('World cycles: neutral classes, package outlines, distinct directed cycle connections, drilldown, findings and immutable evidence passed');
