const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
// The graph starts at file depth, where p ↔ q is no cycle; the World flags it
// at package level, so its cycle action must show that same level in the graph.
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},{id:2,name:'r',kind:'module',parent:null},
 ...['p/A.cs','p/D.cs','q/B.cs','q/C.cs','r/E.cs'].map((name,i)=>({id:i+3,name,kind:'file',parent:{p:0,q:1,r:2}[name[0]]}))];
const fixture={root:'/project',nodes,edges:[{from:3,to:5,count:1,refs:[]},{from:6,to:4,count:1,refs:[]},{from:7,to:5,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:1,visible:null},
 insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:nodes.filter(n=>n.kind==='file').map(n=>({id:n.name,node:n.name,class_id:n.name,max_complexity:3,coverage:null}))}}};
const app=launch(fixture,{library:false});app.run("showTab('start');setViewerLanguage('en')");
assert.equal(app.run("JSON.stringify(worldUi.model.areas.filter(a=>a.cyclic).map(a=>a.name))"),JSON.stringify(['p','q']));
assert.equal(app.run('cyclesForView().length'),0,'the file-level graph has no cycle');
const original=app.run('JSON.stringify([data,ops])');
app.run("worldJumpCycle('p/A.cs')");
assert.equal(app.run('ui.workspace'),'graph','the cycle action opens the graph');
assert.equal(app.run("JSON.stringify(cyclesForView().map(c=>c.nodes.slice().sort()))"),JSON.stringify([['p','q']]),'the graph lists the World package cycle');
assert.ok(app.run('ui.cycle'),'the package cycle is focused');
assert.equal(app.run("JSON.stringify([...isolated].map(id=>nodeById(view,id).name).sort())"),JSON.stringify(['p','q']));
assert.equal(app.run('cyclicKeys.size'),2,'both directions are cyclic graph edges');
assert.equal(app.run('JSON.stringify([data,ops])'),original,'evidence stays unchanged');
const plain=launch(fixture,{library:false});plain.run("showTab('start');setViewerLanguage('en');worldJumpCycle('r/E.cs')");
assert.equal(plain.run('ui.cycle'),null,'an acyclic area keeps the cycles list fallback');
assert.equal(plain.run('treeBaseCollapses.size'),0,'an acyclic area leaves the graph level unchanged');
console.log('World cycle parity: World package cycles open as the same graph cycle');
