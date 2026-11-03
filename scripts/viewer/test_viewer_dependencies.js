// Optional dependency details: directional focus and exact evidence beside the SVG tree.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const node=(id,name,parent=null,kind='file')=>({id,name,parent,kind,lang:'ts'});
const edge=(from,to,count=1)=>({from,to,count,refs:[{file:'source.ts',line:1,raw:'import "<img src=x onerror=alert(1)>";'}]});
const fixture={root:'/tmp/readable',nodes:['A','B','C','Quiet'].map((name,id)=>node(id,name)),edges:[edge(0,1,25),edge(1,0,2),edge(2,0,5)],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const buttons=app=>app.ids.get('dependency-browser').querySelectorAll('button');
const mode=(app,value)=>app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===value).click();
const section=(app,name)=>app.ids.get('dependency-browser').querySelectorAll('section').find(el=>el.getAttribute('data-dependency-section')===name);
const pairs=element=>Array.from(element?.querySelectorAll('button[data-dependency-pair]')||[],el=>el.getAttribute('data-dependency-pair')).sort();
const focus=(app,name)=>{app.click('diagram-dependencies');app.run('selectNode('+JSON.stringify(name)+')');mode(app,'both');};
const openScope=(app,id)=>{app.click('diagram-dependencies');app.run('navigateExplorerNode('+id+')');mode(app,'both');};
const app=launch(fixture);
assert.equal(app.run('ui.diagram'),'overview','the SVG tree remains the main presentation');
assert.equal(app.ids.get('dependency-browser').hidden,true,'dependency lists are opt-in');
app.click('diagram-dependencies');
assert.equal(app.ids.get('dependency-browser').hidden,false);
assert.equal(app.ids.get('graph').hidden,true);
assert.equal(app.ids.get('edge-legend').hidden,true);
assert.equal(app.ids.get('graph-controls').hidden,true);
assert.deepEqual(pairs(section(app,'list')),['0 1','1 0','2 0'],'Dependencies opens the complete directed list when no reference is selected');
const analysis=app.run('JSON.stringify([work,view,sccs,currentAcd,ops])');
buttons(app).find(b=>b.getAttribute('data-dependency-node')==='A').click();
assert.equal(app.run('ui.diagram'),'dependencies');
assert.equal(app.run('ui.selection.name'),'A');
assert.deepEqual(pairs(section(app,'incoming')),['1 0','2 0']);
assert.deepEqual(pairs(section(app,'outgoing')),['0 1']);
assert.match(section(app,'incoming').textContent,/2/);
assert.match(section(app,'outgoing').textContent,/25/);
assert.equal(app.run('JSON.stringify([work,view,sccs,currentAcd,ops])'),analysis,'presentation preserves analysis');
assert.equal(app.ids.get('dependency-browser').querySelectorAll('img').length,0);
mode(app,'incoming');assert.deepEqual(pairs(app.ids.get('dependency-browser')),['1 0','2 0']);
mode(app,'outgoing');assert.deepEqual(pairs(app.ids.get('dependency-browser')),['0 1']);
mode(app,'both');app.click('toggle-cycles');assert.deepEqual(pairs(app.ids.get('dependency-browser')),['0 1','1 0']);
app.click('toggle-cycles');
buttons(app).find(b=>b.getAttribute('data-dependency-pair')==='0 1').click();
assert.equal(app.run('ui.selection.kind'),'edge');
assert.match(app.ids.get('sidebar').textContent,/25 import references/);
assert.match(app.ids.get('sidebar').textContent,/<img src=x onerror=alert\(1\)>/);
mode(app,'all');assert.equal(app.run('ui.diagram'),'dependencies','All is a readable complete list unless Network was explicitly chosen');
assert.deepEqual(pairs(section(app,'list')),['0 1','1 0','2 0']);
app.click('diagram-network');assert.equal(app.ids.get('graph').hidden,false);assert.equal(app.ids.get('dependency-browser').hidden,true);
assert.equal(app.graph.data.links.filter(app.graph.linkVisibility).length,3,'explicit Network retains full edges and hit testing');
app.click('diagram-overview');assert.equal(app.ids.get('graph').hidden,true);
assert.deepEqual(JSON.parse(app.run('JSON.stringify(ui.selection)')),{kind:'edge',from:'A',to:'B',reference:'A'},'switching presentation preserves the selected dependency and its direction reference');
assert.equal(app.run('JSON.stringify([work,view,sccs,currentAcd,ops])'),analysis);
focus(app,'A');app.click('simulation-mode');app.run("recordOp(encodeCut('B','A'))");
assert.deepEqual(pairs(section(app,'incoming')),['2 0']);
app.click('undo-ops');assert.deepEqual(pairs(section(app,'incoming')),['1 0','2 0']);
const noLibrary=launch(fixture,{library:false});
noLibrary.click('diagram-dependencies');
assert.equal(noLibrary.ids.get('dependency-browser').hidden,false,'dependency details work without the canvas library');
assert.deepEqual(pairs(section(noLibrary,'list')),['0 1','1 0','2 0']);
focus(noLibrary,'A');assert.deepEqual(pairs(section(noLibrary,'outgoing')),['0 1']);
noLibrary.click('diagram-network');assert.match(noLibrary.ids.get('graph-state').textContent,/Loading network/);
noLibrary.document.head.children[0].onerror();assert.match(noLibrary.ids.get('graph-state').textContent,/could not load/);
noLibrary.click('diagram-overview');assert.equal(noLibrary.ids.get('graph-state').hidden,true);
const packages={root:'/tmp/packages',nodes:[node(0,'A',null,'module'),node(1,'A/Start.ts',0),node(2,'A/End.ts',0),node(3,'B',null,'module'),node(4,'B/Other.ts',3),node(5,'C',null,'module'),node(6,'C/Other.ts',5)],edges:[edge(1,4),edge(4,6),edge(6,2)],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const opened=launch(packages);focus(opened,'A');
buttons(opened).find(b=>b.getAttribute('data-open-package')==='A').click();
assert.equal(opened.run('ui.selection.name'),'A','Open package selects its contents as the dependency reference');
assert.ok(section(opened,'incoming'));
assert.ok(section(opened,'outgoing'));
assert.equal(buttons(opened).some(b=>b.getAttribute('data-open-package')==='A'),false,'an already opened parent has no ineffective Open action');
const context=launch(packages);openScope(context,0);
assert.equal(context.run('ui.diagram'),'dependencies');
assert.equal(pairs(section(context,'incoming')).length,1);
assert.equal(pairs(section(context,'outgoing')).length,1);
assert.equal(pairs(section(context,'return')).length,1,'outside-to-outside cycle return paths remain explicit');
assert.match(section(context,'return').textContent,/cycle/i);
const allPairs=JSON.parse(context.run("JSON.stringify(view.edges.filter(e=>dependencyMatchesSelection(e.from,e.to)).map(e=>e.from+' '+e.to).sort())"));
assert.deepEqual(pairs(context.ids.get('dependency-browser')),allPairs);
// The optional dependency list also respects cycle filtering.
const cyclesInDetails=launch(fixture);cyclesInDetails.click('diagram-dependencies');cyclesInDetails.click('toggle-cycles');
assert.equal(cyclesInDetails.run('ui.diagram'),'dependencies');
assert.equal(cyclesInDetails.run('ui.edgeMode'),'all');
assert.deepEqual(pairs(section(cyclesInDetails,'list')),['0 1','1 0']);

// Large inbound/outbound lists page independently, with explicit totals.
const manyNodes=[node(0,'Center')],manyEdges=[];
for(let i=1;i<=101;i++){
  manyNodes.push(node(i,'Incoming-'+String(i).padStart(3,'0')));
  manyNodes.push(node(101+i,'Outgoing-'+String(i).padStart(3,'0')));
  manyEdges.push(edge(i,0,i),edge(0,101+i,i+100));
}
const paged=launch({...fixture,nodes:manyNodes,edges:manyEdges});
focus(paged,'Center');
assert.equal(pairs(section(paged,'incoming')).length,100);
assert.equal(pairs(section(paged,'outgoing')).length,100);
assert.match(section(paged,'incoming').textContent,/Showing 100 of 101 dependencies/);
buttons(paged).find(b=>b.getAttribute('data-dependency-more')==='incoming').click();
assert.equal(pairs(section(paged,'incoming')).length,101);
assert.equal(pairs(section(paged,'outgoing')).length,100,'expanding incoming does not expand outgoing');
buttons(paged).find(b=>b.getAttribute('data-dependency-more')==='outgoing').click();
assert.equal(pairs(section(paged,'outgoing')).length,101);
mode(paged,'all');assert.equal(pairs(section(paged,'list')).length,100);
assert.match(section(paged,'list').textContent,/Showing 100 of 202 dependencies/);
buttons(paged).find(b=>b.getAttribute('data-dependency-node')==='Center').click();
assert.equal(paged.run('ui.edgeMode'),'both','selecting a node from All restores directional focus');
assert.equal(pairs(section(paged,'incoming')).length,100,'a new focus resets independent page budgets');

// Directional totals remain lower bounds when only sampled evidence exists.
const unknown=launch({...fixture,edges:[{...edge(0,1),count:null,refs:[{file:'a.ts',line:1,raw:'import B'}, {file:'a.ts',line:2,raw:'import B'}]}]});
focus(unknown,'A');
const sampled=buttons(unknown).find(b=>b.getAttribute('data-dependency-pair')==='0 1');
assert.match(sampled.textContent,/2\+ import references/);
assert.match(sampled.getAttribute('title'),/lower bound/);

// Names are text; repeated basenames remain distinguishable without hover.
const hostileName='<img src=x onerror=alert(1)>';
const named=launch({...fixture,nodes:[node(0,hostileName),node(1,'left/common/Item.ts'),node(2,'right/common/Item.ts')],edges:[edge(0,1),edge(2,0)]});
named.click('diagram-dependencies');
assert.equal(named.ids.get('dependency-browser').querySelectorAll('img').length,0);
const namedActions=buttons(named).filter(b=>b.getAttribute('data-dependency-node')!==null);
assert.ok(namedActions.some(b=>b.getAttribute('title')===hostileName));
const left=namedActions.find(b=>b.getAttribute('data-dependency-node')==='left/common/Item.ts'),right=namedActions.find(b=>b.getAttribute('data-dependency-node')==='right/common/Item.ts');
assert.notEqual(left.textContent,right.textContent,'same basename and immediate parent must not produce identical dependency labels');
assert.equal(left.getAttribute('title'),'left/common/Item.ts');
focus(named,hostileName);
assert.equal(named.ids.get('dependency-browser').querySelectorAll('img').length,0);
assert.deepEqual(pairs(named.ids.get('dependency-browser')),['0 1','2 0']);

// Internal package calls and external return hops are separate exhaustive lists.
const withInternal=launch({...packages,edges:[...packages.edges,edge(1,2,7)]});
openScope(withInternal,0);
assert.equal(pairs(section(withInternal,'internal')).length,1);
assert.match(section(withInternal,'internal').textContent,/7 import references/);
assert.equal(pairs(section(withInternal,'return')).length,1);
const expectedInternal=JSON.parse(withInternal.run("JSON.stringify(view.edges.filter(e=>dependencyMatchesSelection(e.from,e.to)).map(e=>e.from+' '+e.to).sort())"));
assert.deepEqual(pairs(withInternal.ids.get('dependency-browser')),expectedInternal,'each visible matching dependency appears once, including internal and return paths');

console.log('Dependency details: explicit view, directional focus, reciprocal counts, exact evidence, filters, cycles, external returns, simulation/undo and no-library fallback passed');
