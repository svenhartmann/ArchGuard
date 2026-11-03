// Dense real-edge fixtures exercise the ordering objective, hierarchy and UI.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const context=vm.createContext({});
vm.runInContext(fs.readFileSync('crates/archguard-report/src/viewer_tree_layout.js','utf8'),context);
const layout=context.packageTreeLayout,plain=value=>JSON.parse(JSON.stringify(value));
const packages=['Api','Domain','Services','Storage','Messaging','Shared'];
const names=[['OrdersController','UsersController','RequestMapper','ApiValidation','ApiRoutes'],['Order','Customer','PricePolicy','OrderRules','DomainEvents'],['OrderService','UserService','CheckoutService','EventDispatcher','ServiceRegistry'],['OrderRepository','UserRepository','UnitOfWork','DatabaseSession','QueryBuilder'],['MessageBus','EventPublisher','MessageHandler','Subscription','RetryPolicy'],['Result','Clock','Identifiers','Serialization','Diagnostics']];
const files=names.flatMap((group,p)=>group.map((name,j)=>({src:p*5+j,name:packages[p]+'/'+name+'.cs',kind:'file',p})));
const input={nodes:[{src:null,name:'Project',kind:'project',context:true,project:true},...packages.map((name,i)=>({src:30+i,name,kind:'module'})),...files],edges:[...packages.map((_,i)=>({from:null,to:30+i})),...files.map(f=>({from:30+f.p,to:f.src}))]};
const keys=new Set(),add=(a,b)=>{if(a!==b)keys.add(a+' '+b);};
for(let i=0;i<15;i++){[1,2,4,7,11].forEach(j=>add(i,(i+j)%15));for(let j=0;j<4;j++)add(i,15+(i+j*3)%10);for(let j=0;j<2;j++)add(i,25+(i+j*2)%5);}
for(let i=0;i<10;i++){[1,3,6].forEach(j=>add(15+i,15+(i+j)%10));for(let j=0;j<2;j++)add(15+i,25+(i+j*2)%5);}
for(let i=25;i<30;i++)for(let j=i+1;j<30;j++)add(i,j);
for(const cycle of [[0,10,5,1,0],[15,20,17,15]])cycle.slice(0,-1).forEach((a,i)=>add(a,cycle[i+1]));
const links=[...keys].map(key=>{const [from,to]=key.split(' ').map(Number);return {key,from,to};});
assert.equal(links.length,231);
const before=JSON.stringify([input,links]),options={order:'connections',tracks:true,width:1400};
const alpha=layout(input,links,{...options,order:'alphabetical'}),start=performance.now(),related=layout(input,links,options),coldMs=performance.now()-start;
function crossings(nodes){const position=new Map(nodes.map((n,i)=>[n.src,i]));let count=0;for(let i=0;i<links.length;i++)for(let j=i+1;j<links.length;j++){const a=links[i],b=links[j];if(a.from===b.from||a.from===b.to||a.to===b.from||a.to===b.to)continue;const [lo,hi]=[position.get(a.from),position.get(a.to)].sort((x,y)=>x-y),[left,right]=[position.get(b.from),position.get(b.to)].sort((x,y)=>x-y);if(lo<left&&left<hi&&hi<right||left<lo&&lo<right&&right<hi)count++;}return count;}
assert.ok(crossings(related.nodes)<crossings(alpha.nodes),'dependency ordering must reduce interleaving on the dense accepted fixture');
for(const result of [alpha,related]){
  assert.deepEqual(plain(result.routes.map(e=>e.key).sort()),links.map(e=>e.key).sort(),'all original directed identities survive sorting');
  for(let p=0;p<6;p++){const at=result.nodes.findIndex(n=>n.src===30+p),descendants=result.nodes.slice(at+1,at+6);assert.equal(descendants.length,5);assert.ok(descendants.every(n=>n.p===p&&n.depth===2),'the five files remain contiguous under their own package');}
}
const rows=d=>plain(d.nodes.map(({src,x,y,w,h,depth})=>({src,x,y,w,h,depth})));
assert.deepEqual(plain(related),plain(layout(input,[...links].reverse(),options)),'edge enumeration cannot change ordering or tracks');
context.packageTreeConnectionsOrder.cache.clear();
const shuffled=layout({...input,nodes:[...input.nodes].reverse(),edges:[...input.edges].reverse()},links,options);
assert.deepEqual(rows(related),rows(shuffled),'node enumeration cannot change ordering, even with an empty cache');
assert.deepEqual(plain(related.routes),plain(shuffled.routes),'node enumeration cannot change edge geometry');
function verifyTracks(result){
  for(const route of result.routes){
    assert.match(route.path,/ C /);assert.equal(route.points.length,6);
    assert.equal(route.points[0].y,route.points[1].y);assert.equal(route.points[4].y,route.points[5].y,'arrows approach the target horizontally');
    for(const point of route.points){assert.ok(Number.isFinite(point.x)&&Number.isFinite(point.y));assert.ok(point.x>=0&&point.x<result.width&&point.y>=0&&point.y<result.height);assert.ok(route.side==='left'?point.x<result.nodes[0].x-result.nodes[0].w/2:point.x>result.nodes[0].x+result.nodes[0].w/2,'relations stay physically on their assigned side of the tree');}
  }
}
for(const width of [320,1400,2600]){
  const normal=layout(input,links,{...options,width});verifyTracks(normal);assert.ok(normal.routes.every(r=>r.side==='right'));
  for(const side of ['left','right']){const directional=layout(input,links.map(l=>({...l,side})),{...options,width,directional:true});verifyTracks(directional);assert.deepEqual(rows(directional),rows(normal),'switching In/Out does not move tree rows');assert.equal(directional.width,normal.width);}
}
const parallel=layout({nodes:[{src:0,name:'A',kind:'module'},{src:1,name:'B',kind:'module'}],edges:[]},Array.from({length:80},(_,i)=>({from:0,to:1,key:String(i)})),options);
verifyTracks(parallel);assert.equal(new Set(parallel.routes.map(r=>r.path)).size,80,'parallel links keep separate tracks');
const self=layout({nodes:[{src:0,name:'A',kind:'module'}],edges:[]},[{from:0,to:0,key:'self'}],options);verifyTracks(self);assert.notEqual(self.routes[0].points[0].y,self.routes[0].points.at(-1).y);
assert.equal(JSON.stringify([input,links]),before,'layout never mutates analysis inputs');
const largeInput={nodes:Array.from({length:500},(_,i)=>({src:i,name:'Package-'+i,kind:'module'})),edges:[]};
const largeLinks=Array.from({length:800},(_,i)=>({from:i%500,to:(i*7+13)%500,key:'edge-'+i}));
const large=layout(largeInput,largeLinks,options);assert.equal(large.nodes.length,500);assert.equal(large.routes.length,800,'bounded ordering work must not cap rendered relationships');verifyTracks(large);

const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/tmp/tree-order',nodes:input.nodes.filter(n=>n.src!==null).map(n=>({id:n.src,name:n.name,kind:n.kind,parent:n.kind==='file'?30+n.p:null,lang:'cs'})),edges:links.map(e=>({from:e.from,to:e.to,count:1,refs:[{file:files[e.from].name,line:3,raw:'using '+files[e.to].name}]})),view:{depth:0,visible:null},cyclicEdges:[],findings:[]};
const app=launch(fixture,{library:false}),tree=app.ids.get('package-tree');app.run('setAllTreeBranches(true)');
const button=(attribute,value)=>tree.querySelectorAll('button['+attribute+']').find(b=>b.getAttribute(attribute)===value);
const positions=()=>JSON.stringify(tree.querySelectorAll('g[data-tree-source]').map(n=>[n.getAttribute('data-tree-source'),n.getAttribute('transform')]));
const edgeKeys=()=>tree.querySelectorAll('g[data-tree-edge]').map(e=>e.getAttribute('data-tree-edge')).sort();
const mode=value=>app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===value).click();
assert.equal(edgeKeys().length,231);assert.equal(app.run('treeSort'),'connections');
const analysis=app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),edgeBefore=edgeKeys();
app.run('setPackageTreeZoom(1.25)');tree.scrollTop=80;
for(const sort of ['alphabetical','connections']){
  button('data-tree-sort',sort).click();assert.equal(app.run('treeSort'),sort);assert.equal(button('data-tree-sort',sort).getAttribute('aria-pressed'),'true');assert.equal(app.document.activeElement,button('data-tree-sort',sort));assert.equal(tree.scrollTop,80);assert.equal(app.run('treeZoom'),1.25);assert.deepEqual(edgeKeys(),edgeBefore);assert.equal(app.run('JSON.stringify([data,work,view,sccs,currentAcd,ops])'),analysis);
  const baseline=positions();app.run("selectNode('Api/OrdersController.cs')");
  mode('incoming');const incoming=edgeKeys();mode('outgoing');const outgoing=edgeKeys();mode('both');const both=edgeKeys(),incident=new Set([...incoming,...outgoing]);assert.ok([...incident].every(key=>both.includes(key)));assert.ok(both.filter(key=>!incident.has(key)).every(key=>app.run('packageContext.edges.has('+JSON.stringify(key)+')')),'Both also retains the existing complete package-cycle context');assert.ok(incoming.length&&outgoing.length);
  mode('all');app.click('toggle-cycles');assert.equal(edgeKeys().length,111,'cycles-only retains every intra-SCC edge');app.click('toggle-cycles');assert.equal(positions(),baseline,'selection, In/Out and cycle filters preserve this ordering');
  button('data-tree-routing','direction').click();assert.equal(positions(),baseline,'In/Out arrangement preserves the same order');
  const out=tree.querySelectorAll('g[data-tree-edge]').find(e=>e.getAttribute('data-tree-side')==='right');assert.ok(out);out.click();assert.equal(app.run('ui.selection.kind'),'edge');assert.match(app.ids.get('sidebar').textContent,/using /,'edge evidence remains reachable');
  button('data-tree-routing','direction').click();app.run('ui.selection=null;reconcileSelection();renderAll()');mode('all');
}
app.click('language-de');assert.equal(button('data-tree-sort','connections').textContent,'Nach Beziehungen');assert.match(tree.textContent,/Zyklusgruppe/);assert.doesNotMatch(tree.textContent,/Umlauf/);assert.equal(button('data-tree-routing','direction').textContent,'Eingehend / Ausgehend');
app.run('focusCycle(cyclesForView()[0])');assert.equal(app.run('ui.selection.kind'),'cycle');assert.ok(edgeKeys().length>0);assert.equal(app.run('treeSort'),'connections','focusing a complete cycle group keeps the chosen sorting');
app.run('clearCycle();resetPackageTree()');assert.equal(app.run('treeSort'),'connections');assert.equal(app.run('treeRouting'),'direction');
console.log('Tree ordering: '+crossings(alpha.nodes)+' → '+crossings(related.nodes)+' interleaving pairs; cold layout '+Math.round(coldMs)+' ms. Hierarchy, all 231 edges, rounded geometry, filters, evidence, cycle groups and DE terminology passed.');
