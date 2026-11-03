const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const prefix='company/product';
const n=(id,name,parent=null,kind='file')=>({id,name:prefix+'/'+name,parent,kind,lang:kind==='file'?'java':null});
const e=(from,to,file,count=1)=>({from,to,count,refs:[{file,line:3,raw:'import company.product.Target;'}]});
const fixture={root:'/repo',nodes:[n(0,'',null,'module'),n(1,'client',0,'module'),n(2,'search',0,'module'),n(3,'search/Use.java',2),n(4,'client/Run.java',1),n(5,'search/Input.java',2),n(6,'client/RunTest.java',1),n(7,'search/UseTest.java',2)],edges:[e(3,4,'src/main/java/search/Use.java'),e(4,5,'main/java/client/Run.java'),e(6,5,'src/test/java/client/RunTest.java'),e(7,4,'test/java/search/UseTest.java')],findings:[],cyclicEdges:[],view:{depth:1,visible:null}};
function open(data=fixture){const app=launch(data,{library:false});app.run("selectEdge(view.edges.find(e=>nodeById(view,e.from).name.endsWith('/client')))");return app;}
const app=open(),sidebar=app.ids.get('sidebar'),explanation=sidebar.querySelector('section[data-cycle-explanation]');
assert.ok(explanation,'a package cycle needs an explanation before graphs and raw evidence');
assert.match(explanation.querySelector('p').textContent,/search uses client.*types in search/);
const model=JSON.parse(app.run("JSON.stringify(packageCycleExplanation(ui.selection.from,ui.selection.to))"));
assert.deepEqual(model.witness.nodes,[3,4,5],'the example is a connected production path across the boundary and back');
assert.equal(model.witness.closed,false);assert.equal(model.witness.connected,true);
assert.deepEqual(model.directions.map(d=>d.counts),[{production:1,test:1,mixed:0,unknown:0},{production:1,test:1,mixed:0,unknown:0}]);
assert.equal(explanation.querySelectorAll('path[data-detail-edge]').length,2);
assert.match(explanation.textContent,/different file|another file/,'a package return is not a closed file cycle');
assert.ok(explanation.querySelector('table[data-cycle-counts]'));
assert.ok(explanation.children.indexOf(explanation.querySelector('table[data-cycle-counts]'))<explanation.children.indexOf(explanation.querySelector('details[data-witness-evidence]')),'show direction counts before source evidence');
const immutable=app.run('JSON.stringify([data,work,ops])');
sidebar.querySelectorAll('button[data-package-direction]').find(b=>b.getAttribute('data-package-direction')==='reverse').click();
assert.equal(app.run('ui.selection.from'),prefix+'/search');assert.equal(app.run('JSON.stringify([data,work,ops])'),immutable);
app.run("setViewerLanguage('de')");assert.match(sidebar.textContent,/Produktionscode.*Tests/s);assert.match(sidebar.textContent,/Paketzyklus/);
assert.equal(sidebar.querySelector('h2').textContent,'search → client');
// Counts describe distinct file pairs, not reference totals or sampled snippets.
const varied=open({...fixture,edges:[e(3,4,'src/main/java/search/Use.java',9),e(4,5,'src/main/java/client/Run.java'),{from:6,to:5,count:2,refs:[{file:'src/main/java/X.java',line:1,raw:'import X;'},{file:'src/test/java/X.java',line:1,raw:'import X;'}]},e(7,4,'unconventional/Use.java')]});
const directions=JSON.parse(varied.run('JSON.stringify(packageCycleExplanation(ui.selection.from,ui.selection.to).directions)'));
assert.deepEqual(directions.map(d=>d.counts),[{production:1,test:0,mixed:1,unknown:0},{production:0,test:0,mixed:0,unknown:2}]);
assert.match(varied.ids.get('sidebar').textContent,/Unknown|unknown/);assert.match(varied.ids.get('sidebar').textContent,/Mixed|mixed/);
// A package named "test" inside src/main remains production; names never decide roles.
assert.equal(app.run("cycleSourceScope({count:1,refs:[{file:'src/main/java/org/test/NotATest.java'}]})"),'production');
assert.equal(app.run("cycleSourceScope({count:null,refs:[{file:'src/main/java/A.java'}]})"),'unknown');
assert.equal(app.run("cycleSourceScope({count:0,refs:[]})"),'unknown');
// Opposite edges can have unrelated endpoints. Never splice them into a path.
const disconnected=open({...fixture,edges:[e(3,4,'src/main/java/search/Use.java'),e(6,5,'src/test/java/client/RunTest.java')]});
const separate=JSON.parse(disconnected.run('JSON.stringify(packageCycleExplanation(ui.selection.from,ui.selection.to).witness)'));
assert.equal(separate.connected,false);assert.equal(separate.edges.length,2);assert.match(disconnected.ids.get('sidebar').textContent,/separate|Separate/);
const closed=open({...fixture,edges:[e(3,4,'src/main/java/search/Use.java'),e(4,3,'src/main/java/client/Run.java')]});
assert.equal(closed.run('packageCycleExplanation(ui.selection.from,ui.selection.to).witness.closed'),true);
assert.match(closed.ids.get('sidebar').textContent,/same file/);
app.run("setSimulation(true);recordOp(encodeCut('"+prefix+"/client/Run.java','"+prefix+"/search/Input.java'));selectEdge(view.edges.find(e=>nodeById(view,e.from).name.endsWith('/client')))");
assert.equal(app.run('packageCycleExplanation(ui.selection.from,ui.selection.to).directions[0].counts.production'),0,'a simulated cut updates the explanation');
assert.match(sidebar.textContent,/Simuliert|simulated|Simulated/);
const hostile=open({...fixture,nodes:fixture.nodes.map(node=>node.id===5?{...node,name:prefix+'/search/<img>.java'}:node)});assert.equal(hostile.ids.get('sidebar').querySelectorAll('img').length,0);
console.log('Cycle explanation: actual witness paths, independent reverse pairs, path-based source splits, incomplete samples, simulation, literal names and DE/EN passed');
