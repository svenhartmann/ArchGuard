// Cycle groups retain complete project call evidence through Calls filters.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launchHarness=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
function launch(data,options){
  const instance=launchHarness(data,options);
  // The shared double indexes static IDs without their HTML ancestry. Attach
  // the actual detail section's descendants so containment assertions inspect
  // its rendered witness/member/edge content, as they do in the browser DOM.
  const detailMarkup=html.match(/<section\b[^>]*\bid="calls-cycle-detail"[^>]*>([\s\S]*?)<\/section>/);
  if(detailMarkup)for(const [,id] of detailMarkup[1].matchAll(/\bid="([^"]+)"/g))instance.ids.get('calls-cycle-detail').appendChild(instance.ids.get(id));
  return instance;
}
const cls=(id,pkg,extra={})=>({id,name:id,package:pkg,file:id+'.java',graph_node:id+'.java',line:1,kind:'class',language:'java',...extra});
const method=(owner,name)=>({id:owner+'#'+name+'()',class_id:owner,name,signature:name+'()',file:owner+'.java',line:2,is_static:false});
const call=(from,to,line,extra={})=>({from_class:from.class_id,from_method:from.id,to_class:to?.class_id||null,to_method:to?.id||null,target_label:to?.id||'unknown()',dispatch:'instance',resolution:to?'project':'unresolved',file:from.file,line,column:1,code:'cycle-site-'+line+'();',...extra});
const classes=[cls('a.A','a'),cls('b.B','b'),cls('c.C','c'),cls('d.D','d'),cls('extra.OneWay','extra')];
const methods=[method('a.A','a'),method('b.B','b'),method('b.B','c'),method('a.A','d'),method('a.A','x'),method('b.B','y'),method('c.C','z'),method('d.D','r'),method('extra.OneWay','run')];
const calls=[call(methods[0],methods[1],10),call(methods[2],methods[3],20),call(methods[4],methods[5],30),call(methods[5],methods[6],40),call(methods[6],methods[4],50),call(methods[7],methods[7],60),call(methods[8],methods[4],70),call(methods[0],null,80),call(methods[0],null,90,{resolution:'external',to_class:'SDK',to_method:'SDK#run()'})];
const fixture={root:'/tmp/call-cycles',nodes:classes.map((c,id)=>({id,name:c.graph_node,kind:'file',parent:null,lang:c.language})),edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},calls:{status:'partial',files:classes.length,classes,methods,calls,diagnostics:['Missing SDK source']}};
const app=launch(fixture,{library:false});
assert.ok(app.ids.has('calls-show-cycles'),'Calls needs a discoverable cycle groups view');
const choose=(instance,id,value)=>{instance.ids.get(id).value=value;instance.ids.get(id).dispatch('change');};
const search=(instance,value)=>{instance.ids.get('calls-filter').value=value;instance.ids.get('calls-filter').dispatch('input');};
const rows=instance=>Array.from(instance.ids.get('calls-cycles-list').querySelectorAll('button[data-call-cycle]'));
const edges=instance=>Array.from(instance.ids.get('calls-cycle-detail').querySelectorAll('button[data-cycle-edge]'));
const members=button=>JSON.parse(button.getAttribute('data-call-cycle'))[1];
const snapshot=instance=>JSON.parse(instance.run('JSON.stringify(exportSnapshot())'));
const treeIds=instance=>Array.from(instance.ids.get('calls-tree').querySelectorAll('button[data-call-element]'),button=>JSON.parse(button.getAttribute('data-call-element')).id);
const openCycles=instance=>{instance.run("showTab('calls')");instance.click('calls-show-cycles');};
const withCalls=(types,members,sites,status='ready')=>launch({...fixture,nodes:types.map((c,id)=>({id,name:c.graph_node,kind:'file',parent:null,lang:c.language})),calls:{status,files:types.length,classes:types,methods:members,calls:sites,diagnostics:[]}},{library:false});
app.run('var cycleComputations=0,originalCycles=computeCallCycles;computeCallCycles=function(...args){cycleComputations++;return originalCycles(...args);};');
assert.equal(app.run('callModelPrepared'),false,'graph startup leaves optional call analysis lazy');
assert.equal(app.run('cycleComputations'),0);
app.run("showTab('calls')");
assert.equal(app.run('cycleComputations'),0,'ordinary Connections does not compute the hidden cycle model');
choose(app,'calls-resolution','unresolved');
const graphState=app.run('JSON.stringify([work,view,ops])');
app.click('calls-show-cycles');
assert.equal(app.run('callRankingUi.mode'),'cycles');
assert.equal(app.ids.get('calls-cycles').hidden,false);
assert.equal(app.ids.get('calls-relations').hidden,true);
assert.equal(app.ids.get('calls-relations').querySelectorAll('button[data-relation]').length,0,'the inactive Connections panel does not render duplicate cycle relations');
assert.equal(app.ids.get('calls-ranking').hidden,true);
assert.equal(app.ids.get('calls-show-cycles').getAttribute('aria-pressed'),'true');
assert.equal(app.ids.get('calls-show-connections').getAttribute('aria-pressed'),'false');
assert.equal(app.ids.get('calls-show-rankings').getAttribute('aria-pressed'),'false');
assert.equal(app.run('cycleComputations'),1);
assert.equal(app.ids.get('calls-resolution').disabled,true,'cycle detection uses resolved project calls independently of the connection target filter');
assert.equal(app.ids.get('calls-internal').disabled,true);
assert.equal(app.run('callUi.resolution'),'unresolved','opening cycles preserves the previous target filter');
assert.equal(app.run('callUi.internal'),false);
assert.equal(rows(app).length,1);
assert.deepEqual(members(rows(app)[0]),['a.A','b.B','c.C'],'the class component retains its return path outside the selected package');
assert.match(app.ids.get('calls-cycles-status').textContent,/partial|incomplete/i);
assert.match(app.ids.get('calls-cycle-detail').textContent,/example|witness/i,'a closed witness must not imply enumeration of every elementary cycle');
assert.equal(snapshot(app).filters.call_view,'cycles');
assert.equal(snapshot(app).selected_cycle.kind,'class');
assert.ok(!snapshot(app).selected_cycle.members.includes('d.D'),'same-class calls do not create a class dependency cycle');

choose(app,'calls-cycle-kind','method');
assert.equal(rows(app).length,1,'package a matches a complete cross-package method cycle');
const methodCycle=[methods[4].id,methods[5].id,methods[6].id];
assert.deepEqual(members(rows(app)[0]),methodCycle);
let exported=snapshot(app);
assert.equal(exported.selected_cycle.kind,'method');
assert.deepEqual(exported.selected_cycle.members,methodCycle);
assert.equal(exported.selected_cycle.witness[0],exported.selected_cycle.witness.at(-1),'the exported witness is closed');
assert.equal(exported.selected_cycle.witness.length,4);
for(let i=1;i<exported.selected_cycle.witness.length;i++)assert.ok(exported.selected_cycle.edges.some(edge=>edge.from===exported.selected_cycle.witness[i-1]&&edge.to===exported.selected_cycle.witness[i]),'each witness step has a real directed call edge');
assert.equal(exported.selected_cycle.edges.reduce((sum,edge)=>sum+edge.call_sites,0),3);
assert.deepEqual(exported.nodes.map(node=>node.id).sort(),methodCycle,'method-cycle exports use compiler method IDs as graph nodes');
assert.ok(exported.edges.every(edge=>methodCycle.includes(edge.from)&&methodCycle.includes(edge.to)),'method-cycle Mermaid endpoints must not be collapsed to class identities');
assert.equal(exported.edges.reduce((sum,edge)=>sum+edge.evidence.length,0),3,'exports retain all selected-cycle evidence');
assert.deepEqual(exported.edges.flatMap(edge=>edge.evidence.map(site=>site.line)).sort((a,b)=>a-b),[30,40,50]);
const methodMarkdown=app.run('exportMarkdown(exportSnapshot())');
assert.match(methodMarkdown,/strongly connected|SCC/i,'cycle documentation explains the grouped analysis');
assert.match(methodMarkdown,/closed.{0,80}example|example.{0,80}closed|closed.{0,80}witness|witness.{0,80}closed/i);
assert.doesNotMatch(methodMarkdown,/undefined -->|-->[^\n]*undefined/,'every Mermaid edge resolves to an exported method node');
for(const id of methodCycle)assert.ok(methodMarkdown.includes(id.replaceAll('#','&#35;')),'Markdown preserves exact method identities');
for(const id of methodCycle)assert.ok(app.ids.get('calls-cycle-witness').textContent.includes(id.split('#')[0]),'the witness identifies owning classes, not ambiguous short method names');
const exactEdge=edges(app).find(button=>JSON.stringify(JSON.parse(button.getAttribute('data-cycle-edge')))===JSON.stringify([methods[4].id,methods[5].id]));
assert.ok(exactEdge);exactEdge.click();
assert.equal(app.ids.get('calls-evidence').querySelectorAll('article[data-call-site]').length,1);
assert.match(app.ids.get('calls-evidence').textContent,/cycle-site-30/);
assert.doesNotMatch(app.ids.get('calls-evidence').textContent,/cycle-site-(10|20|40|50|60|70)/,'a method edge does not expand into unrelated calls between the same classes');
assert.match(app.ids.get('sidebar').textContent,/1 call sites.*1 target methods/,'Inspector counts the exact method edge, not the two calls and target methods between these classes');
assert.match(app.ids.get('sidebar').textContent,/a\.A#x\(\).*b\.B#y\(\)/,'Inspector identifies the exact qualified method endpoints');
assert.match(app.ids.get('sidebar').textContent,/a\.A\.java:30:1/,'Inspector gives the first selected-edge source location');
assert.match(app.ids.get('sidebar').textContent,/cycle-site-30/,'Inspector shows the first selected-edge source evidence');
assert.match(app.ids.get('sidebar').textContent,/first.{0,60}(source|evidence|site)/i,'the single Inspector evidence preview is explicitly labeled');
assert.doesNotMatch(app.ids.get('sidebar').textContent,/cycle-site-(10|20|40|50|60|70)/);
app.click('calls-tree-expand');
assert.deepEqual(treeIds(app).filter(id=>id.includes('#')).sort(),methodCycle,'the tree shows the whole cycle and excludes unrelated methods');
const markedTreeIds=Array.from(app.ids.get('calls-tree').querySelectorAll('div[data-call-cycle-member]')).flatMap(row=>Array.from(row.querySelectorAll('button[data-call-element]'),button=>JSON.parse(button.getAttribute('data-call-element')).id));
for(const id of methodCycle)assert.ok(markedTreeIds.includes(id),'selected cyclic members are visibly marked in the Calls tree');
app.click('calls-tree-collapse');app.click('calls-tree-expand');
assert.deepEqual(treeIds(app).filter(id=>id.includes('#')).sort(),methodCycle,'Expand all after Collapse all keeps the cycle scope');
app.ids.get('calls-tree').querySelectorAll('button[data-call-element]').find(button=>JSON.parse(button.getAttribute('data-call-element')).id===methods[5].id).click();
assert.deepEqual(snapshot(app).selected_cycle.members,methodCycle,'element focus does not truncate the selected cyclic group');
assert.equal(snapshot(app).edges.reduce((sum,edge)=>sum+edge.evidence.length,0),3,'a focused method does not truncate cycle exports');
assert.equal(snapshot(app).selected_method,undefined,'cycle exports do not carry a separate stale selected-method declaration');
choose(app,'calls-package','*');
search(app,'z');
assert.equal(rows(app).length,1);
assert.deepEqual(snapshot(app).selected_cycle.members,methodCycle,'matching one member retains its entire SCC');
assert.equal(app.run('cycleComputations'),1,'selection, scope and search reuse the project cycle model');
app.run("setViewerLanguage('de')");
assert.match(app.ids.get('calls-show-cycles').textContent,/Zykl/);
assert.match(app.ids.get('calls-cycles-status').textContent,/Zykl|zykl/);
assert.equal(app.run('cycleComputations'),1,'translation does not recompute the cycle model');
app.run("setViewerLanguage('en')");
app.click('calls-cycles-show-all');
assert.equal(app.run('callUi.package'),'*');assert.equal(app.run('callUi.language'),'all');assert.equal(app.run('callUi.query'),'');
assert.equal(rows(app).length,2,'All cycle groups also reveals direct recursion outside the previously selected package');
rows(app).find(button=>members(button).includes(methods[7].id)).click();
exported=snapshot(app);
assert.deepEqual(exported.selected_cycle.members,[methods[7].id]);
assert.deepEqual(exported.selected_cycle.witness,[methods[7].id,methods[7].id]);
assert.equal(exported.selected_cycle.edges.length,1);assert.equal(exported.selected_cycle.edges[0].call_sites,1);
assert.match(app.ids.get('calls-evidence').textContent,/cycle-site-60/,'direct recursion is visible despite Connections excluding internal calls');
app.click('calls-show-connections');
assert.equal(app.ids.get('calls-cycles').hidden,true);assert.equal(app.ids.get('calls-relations').hidden,false);
assert.equal(app.ids.get('calls-resolution').disabled,false);assert.equal(app.ids.get('calls-internal').disabled,false);
assert.equal(app.ids.get('calls-resolution').value,'unresolved');assert.equal(app.run('callUi.internal'),false);
assert.equal(app.ids.get('calls-relations').querySelectorAll('button[data-relation]').length,1,'returning to Connections restores its filters');
app.click('calls-internal');assert.equal(app.run('callUi.internal'),true);
app.click('calls-show-cycles');assert.equal(app.run('callUi.internal'),true);
app.click('calls-internal');assert.equal(app.run('callUi.internal'),true,'disabled internal control does not alter the preserved connection preference');
app.click('calls-show-connections');assert.equal(app.run('callUi.internal'),true);
app.click('calls-show-rankings');
assert.equal(app.ids.get('calls-ranking').hidden,false);assert.equal(app.ids.get('calls-cycles').hidden,true);
app.click('calls-show-cycles');assert.equal(app.run('cycleComputations'),1);
assert.equal(app.run('JSON.stringify([work,view,ops])'),graphState,'cycle browsing never mutates the dependency graph or simulations');

// A class SCC is not proof of a continuous cycle between its methods.
const classOnly=withCalls(classes.slice(0,2),methods.slice(0,4),calls.slice(0,2));
openCycles(classOnly);assert.equal(rows(classOnly).length,1);
choose(classOnly,'calls-cycle-kind','method');assert.equal(rows(classOnly).length,0);
assert.match(classOnly.ids.get('calls-cycles-status').textContent,/no|0/i);
assert.equal(snapshot(classOnly).selected_cycle,null);
assert.equal(classOnly.ids.get('calls-evidence').querySelectorAll('article[data-call-site]').length,0,'empty method cycles do not leak the previous class-cycle evidence');

// Same-class mutual calls are real method cycles even with internal calls off.
const mutualMethods=[method('d.D','one'),method('d.D','two')];
const mutual=withCalls([classes[3]],mutualMethods,[call(mutualMethods[0],mutualMethods[1],1),call(mutualMethods[1],mutualMethods[0],2)]);
openCycles(mutual);assert.equal(rows(mutual).length,0);
choose(mutual,'calls-cycle-kind','method');assert.equal(rows(mutual).length,1);
assert.deepEqual(snapshot(mutual).selected_cycle.members,mutualMethods.map(m=>m.id));

// Every group is reachable; large result lists and group details are bounded.
const manyClasses=Array.from({length:50},(_,i)=>cls('bulk.C'+String(i).padStart(2,'0'),'bulk'));
const manyMethods=manyClasses.map(c=>method(c.id,'run'));
const manyCalls=manyMethods.map((m,i)=>call(m,manyMethods[i%2===0?i+1:i-1],i+1));
const many=withCalls(manyClasses,manyMethods,manyCalls);openCycles(many);
assert.equal(rows(many).length,20);assert.equal(snapshot(many).cycle_groups.length,25,'group exports include groups beyond the visible list page');
assert.match(many.run('exportMarkdown(exportSnapshot())'),/bulk\.C49/,'Markdown includes filtered cycle groups beyond the visible first page');
many.click('calls-cycles-more');assert.equal(rows(many).length,25);
assert.equal(new Set(rows(many).map(button=>button.getAttribute('data-call-cycle'))).size,25);
assert.equal(many.ids.get('calls-cycles-more').hidden,true);
const retainedGroup=rows(many)[24].getAttribute('data-call-cycle');rows(many)[24].click();
search(many,'bulk');
assert.equal(many.run('callCycleUi.selected'),retainedGroup,'a matching selection survives scope changes');
assert.ok(rows(many).some(button=>button.getAttribute('data-call-cycle')===retainedGroup&&button.getAttribute('aria-pressed')==='true'),'the selected group remains visible when scope changes reset the first page');
assert.ok(rows(many).length<=21,'pinning a late selection keeps rendering bounded to the first page plus that group');
const ringCalls=manyMethods.map((m,i)=>call(m,manyMethods[(i+1)%manyMethods.length],i+1));
const lastClass=cls('bulk.C50','bulk'),lastMethod=method(lastClass.id,'run');
ringCalls[49]=call(manyMethods[49],lastMethod,50);ringCalls.push(call(lastMethod,manyMethods[0],51));
const ring=withCalls([...manyClasses,lastClass],[...manyMethods,lastMethod],ringCalls);openCycles(ring);
assert.equal(edges(ring).length,50);assert.equal(ring.ids.get('calls-cycle-edges-more').hidden,false);
assert.equal(ring.ids.get('calls-cycle-members-more').hidden,false);
assert.equal(snapshot(ring).selected_cycle.members.length,51);
assert.equal(snapshot(ring).selected_cycle.edges.length,51);
assert.equal(snapshot(ring).edges.reduce((sum,edge)=>sum+edge.evidence.length,0),51,'the first edge page does not truncate exported source calls');
ring.click('calls-cycle-edges-more');assert.equal(edges(ring).length,51);assert.equal(ring.ids.get('calls-cycle-edges-more').hidden,true);
ring.click('calls-cycle-members-more');assert.equal(ring.ids.get('calls-cycle-members-more').hidden,true);

// Language scopes choose complete groups without merging same display names.
const csharpClasses=[cls('cs:A','cs',{name:'a.A',language:'csharp'}),cls('cs:B','cs',{name:'b.B',language:'csharp'})];
const csharpMethods=csharpClasses.map(c=>method(c.id,'run'));
const mixed=withCalls([...classes,...csharpClasses],[...methods,...csharpMethods],[...calls,call(csharpMethods[0],csharpMethods[1],100),call(csharpMethods[1],csharpMethods[0],101)]);
openCycles(mixed);mixed.click('calls-cycles-show-all');choose(mixed,'calls-language','csharp');
assert.equal(rows(mixed).length,1);assert.deepEqual(members(rows(mixed)[0]),['cs:A','cs:B']);
search(mixed,'missing');assert.equal(rows(mixed).length,0);
mixed.click('calls-cycles-show-all');assert.equal(rows(mixed).length,2);
assert.equal(mixed.run('callUi.language'),'all');assert.equal(mixed.run('callUi.query'),'');

// Names and diagnostics are source text, never HTML.
const unsafeClasses=[cls('safe.A','safe',{name:'safe.<img src=x onerror=alert(1)>'}),cls('safe.B','safe')];
const unsafeMethods=unsafeClasses.map(c=>method(c.id,'<svg onload=alert(1)>'));
const unsafe=withCalls(unsafeClasses,unsafeMethods,[call(unsafeMethods[0],unsafeMethods[1],1),call(unsafeMethods[1],unsafeMethods[0],2)]);
openCycles(unsafe);assert.match(unsafe.ids.get('calls-cycles-list').textContent,/<img/);
choose(unsafe,'calls-cycle-kind','method');assert.match(unsafe.ids.get('calls-cycle-detail').textContent,/<svg/);
for(const id of ['calls-cycles-list','calls-cycle-detail','calls-evidence']){
  assert.equal(unsafe.ids.get(id).querySelectorAll('img').length,0);
  assert.equal(unsafe.ids.get(id).querySelectorAll('svg').length,0);
}
for(const status of ['partial','unavailable']){
  const missing=withCalls([],[],[],status);openCycles(missing);
  assert.equal(rows(missing).length,0);
  assert.match(missing.ids.get('calls-cycles-status').textContent,status==='partial'?/partial|incomplete/i:/unavailable/i,'no detected cycle must not hide incomplete analysis');
  assert.doesNotMatch(missing.ids.get('calls-cycles-status').textContent,/cycle.free|no cycles exist/i);
}
console.log('Calls cycles: complete class/method SCCs, recursive calls, scope preservation, exact evidence, bounded lists, exports, lazy caching and safe bilingual UI passed');
