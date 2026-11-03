// Compact exploration geometry: label tree + separate relation axis with directed arcs.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const context=vm.createContext({});
vm.runInContext(fs.readFileSync('crates/archguard-report/src/viewer_tree_layout.js','utf8'),context);
const layout=context.packageTreeLayout,plain=value=>JSON.parse(JSON.stringify(value));
const node=(src,name,kind='module',extra={})=>({src,name,kind,context:false,project:false,...extra});
const root=node(null,'Project','project',{context:true,project:true});
const input={nodes:[root,node(1,'a'),node(2,'b'),node(3,'a/first.ts','file'),node(4,'a/nested'),node(5,'a/nested/last.ts','file')],edges:[{from:null,to:1},{from:null,to:2},{from:1,to:3},{from:1,to:4},{from:4,to:5}]};
const links=[{from:3,to:2,key:'3 2'},{from:2,to:3,key:'2 3'},{from:5,to:2,key:'5 2'},{from:2,to:5,key:'2 5'},{from:1,to:1,key:'1 1'}];
const before=JSON.stringify([input,links]),result=layout(input,links);
assert.ok(result.nodes.every(n=>n.h<=30),'compact exploration rows replace oversized package cards');
assert.ok(Number.isFinite(result.relationX),'relationships have a separate common icon axis');
assert.equal(JSON.stringify([input,links]),before,'layout cannot mutate analysis inputs');
assert.deepEqual(plain(result),plain(layout(input,[...links].reverse())),'edge input order does not change geometry');
assert.deepEqual(plain(result.routes.map(r=>r.key).sort()),links.map(l=>l.key).sort(),'every dependency retains its identity');
assert.deepEqual(plain(result.nodes.map(n=>n.src)),[null,1,4,5,3,2],'preorder retains nested branches and deterministic sibling order inside SCCs');
function inside(p,n){return p.x>n.x-n.w/2&&p.x<n.x+n.w/2&&p.y>n.y-n.h/2&&p.y<n.y+n.h/2;}
function sampleRoute(route,t){
  const [,a,b,c,d]=route.points,u=1-t;
  return {x:u*u*u*a.x+3*u*u*t*b.x+3*u*t*t*c.x+t*t*t*d.x,y:u*u*u*a.y+3*u*u*t*b.y+3*u*t*t*c.y+t*t*t*d.y};
}
function verify(diagram,gutter=194){
  const nodes=new Map(diagram.nodes.map(n=>[n.src,n]));
  for(let i=0;i<diagram.nodes.length;i++){
    const n=diagram.nodes[i];
    assert.equal(n.h,28);assert.equal(n.indent,n.depth*18);assert.equal(n.relationX,diagram.relationX);
    assert.ok(n.x-n.w/2>=0&&n.y-n.h/2>=0&&n.x+n.w/2<diagram.relationX-190,'tree labels stay entirely left of relation gutter');
    if(i)assert.equal(n.y-diagram.nodes[i-1].y,30,'one compact row pitch, independent of dependency count');
    assert.ok(n.y+n.h/2<diagram.height);
  }
  for(const edge of diagram.hierarchy){
    const from=nodes.get(edge.from),to=nodes.get(edge.to);
    assert.equal(to.depth,from.depth+1);assert.ok(from.y<to.y);
    assert.ok(edge.points.every(p=>Number.isFinite(p.x)&&Number.isFinite(p.y)));
    assert.equal(edge.points.at(-1).y,to.y);
    assert.ok(edge.points.at(-1).x<=to.x-to.w/2+to.indent+12,'hierarchy ends at disclosure prefix, before label text');
  }
  for(const route of diagram.routes){
    const from=nodes.get(route.from),to=nodes.get(route.to),sign=route.side==='left'?-1:1;
    assert.equal(route.points.length,6,'source lead, cubic control points and target lead are explicit');
    const [start,lead,c1,c2,tail,end]=route.points;
    assert.equal(start.x,diagram.relationX+sign*8);assert.equal(end.x,start.x,'both ends attach to the same side of mirror icons');
    assert.equal(start.y,from.y+3);assert.equal(end.y,to.y-3);
    assert.equal(start.y,lead.y);assert.equal(end.y,tail.y);
    assert.ok((lead.x-start.x)*sign>=10&&(tail.x-end.x)*sign>=10,'arrows retain at least10px horizontal endpoint room');
    assert.match(route.path,/^M .* L .* C .* L /,'full smooth arc, not orthogonal buses');
    assert.equal(route.path,`M ${start.x} ${start.y} L ${lead.x} ${lead.y} C ${c1.x} ${c1.y} ${c2.x} ${c2.y} ${tail.x} ${tail.y} L ${end.x} ${end.y}`);
    for(const point of route.points){
      assert.ok([point.x,point.y].every(Number.isFinite));
      assert.ok(point.x>=0&&point.x<diagram.width&&point.y>=0&&point.y<diagram.height);
      assert.ok((point.x-diagram.relationX)*sign>=8,'complete arc stays on its chosen side of the axis');
      assert.ok(Math.abs(point.x-diagram.relationX)<=gutter,'curves stay inside the available relation gutter, even for dense graphs');
    }
    for(let step=0;step<=80;step++){
      const p=sampleRoute(route,step/80);
      for(const n of diagram.nodes)assert.equal(inside(p,n),false,'relation curve cannot cover labels');
    }
  }
}
verify(result);
// Wide workspaces extend the label and relation regions, not the text or rows.
for(const width of [320,1200,2000,4000]){
  const resized=layout(input,links,{width});
  assert.equal(resized.width,Math.max(result.width,width),'the diagram uses the available width without shrinking readable rows on small screens');
  assert.equal(resized.height,result.height,'width changes must not consume more vertical space');
  assert.deepEqual(plain(resized.nodes.map(({src,y,h,depth,level,siblingGroup})=>({src,y,h,depth,level,siblingGroup}))),plain(result.nodes.map(({src,y,h,depth,level,siblingGroup})=>({src,y,h,depth,level,siblingGroup}))),'responsive sizing preserves order, hierarchy and row geometry');
  assert.deepEqual(plain(resized.routes.map(({key,from,to,side})=>({key,from,to,side}))),plain(result.routes.map(({key,from,to,side})=>({key,from,to,side}))),'responsive sizing preserves all relationships and directions');
  assert.deepEqual(plain(resized.hierarchy),plain(result.hierarchy),'hierarchy connectors remain aligned with disclosure icons');
  assert.ok(resized.nodes.every(n=>n.w<=700),'very wide workspaces leave useful room for relations rather than an oversized label column');
  if(width>result.width){
    assert.ok(resized.nodes[0].w>result.nodes[0].w,'long labels gain readable space');
    assert.ok(resized.relationX>result.relationX,'the relation axis uses the newly available area');
    assert.ok(resized.routes.every((r,i)=>Math.abs(r.points[2].x-resized.relationX)>Math.abs(result.routes[i].points[2].x-result.relationX)),'relation curves spread with the available gutter');
  }else assert.deepEqual(plain(resized),plain(result),'narrow workspaces retain the natural horizontal scrolling extent');
  verify(resized,resized.width-resized.relationX-24);
  assert.deepEqual(plain(resized),plain(layout(input,[...links].reverse(),{width})),'resize stays independent of dependency input order');
  for(const side of ['left','right']){
    const directed=layout(input,links.map(link=>({...link,side})),{width});
    assert.deepEqual(plain(directed.nodes),plain(resized.nodes),'direction toggles do not alter responsive row positions');
    assert.equal(directed.width,resized.width);verify(directed,directed.width-directed.relationX-24);
  }
}
for(const width of [undefined,NaN,Infinity,-Infinity,-1,0,'1600',null]){
  assert.deepEqual(plain(layout(input,links,{width})),plain(result),'invalid or absent viewport widths preserve intrinsic geometry');
}
assert.deepEqual(plain(layout(input,links,null)),plain(result),'an absent sizing object uses intrinsic geometry');
const deep={nodes:Array.from({length:30},(_,i)=>node(i,'nested-'+i)),edges:Array.from({length:29},(_,i)=>({from:i,to:i+1}))};
const deepMinimum=layout(deep,[]),deepWide=layout(deep,[],{width:2000});
assert.ok(deepMinimum.nodes[0].w>700,'deep containment needs more intrinsic label space');
assert.equal(deepWide.nodes[0].w,deepMinimum.nodes[0].w,'the label cap never shrinks the space required by deep nesting');
verify(deepWide,deepWide.width-deepWide.relationX-24);
assert.equal(JSON.stringify([input,links]),before,'responsive sizing never mutates graph inputs');
const byId=new Map(result.nodes.map(n=>[n.src,n]));
for(const edge of result.routes)assert.equal(edge.side,byId.get(edge.from).y<byId.get(edge.to).y?'left':'right','Ordered arcs run counter-clockwise: left downward, right upward');
const allLeft=layout(input,links.map(l=>({...l,side:'left'}))),allRight=layout(input,links.map(l=>({...l,side:'right'})));
verify(allLeft);verify(allRight);
assert.deepEqual(plain(allLeft.nodes),plain(result.nodes),'selected In/Out routing cannot move labels');
assert.deepEqual(plain(allRight.nodes),plain(result.nodes));
assert.deepEqual(plain(allLeft.hierarchy),plain(allRight.hierarchy));
assert.equal(allLeft.width,allRight.width,'layout bounds do not jump when selecting a direction reference');
assert.ok(allLeft.routes.every(r=>r.side==='left'));assert.ok(allRight.routes.every(r=>r.side==='right'));
const reciprocal=allLeft.routes.filter(r=>['3 2','2 3'].includes(r.key));
assert.notEqual(reciprocal[0].path,reciprocal[1].path,'reciprocal edges have separate endpoint tracks');
const construction={nodes:[root,node(0,'api'),node(1,'common'),node(2,'domain'),node(3,'storage')],edges:[0,1,2,3].map(to=>({from:null,to}))};
const constructed=layout(construction,[{from:0,to:2,key:'api domain'},{from:2,to:3,key:'domain storage'},{from:3,to:1,key:'storage common'}]);
assert.deepEqual(plain(constructed.nodes.map(n=>n.src)),[null,0,2,3,1],'dependers precede dependencies within the same parent');
assert.deepEqual(plain(constructed.nodes.filter(n=>n.src!==null).map(n=>n.level)),[3,2,1,0],'levels describe construction depth of the sibling condensation');
assert.ok(constructed.nodes.every(n=>!n.siblingCycle));verify(constructed);
const disconnected={nodes:[root,node(0,'AQuiet'),node(1,'ZApi'),node(2,'Domain'),node(3,'Storage')],edges:[0,1,2,3].map(to=>({from:null,to}))};
const levelOrder=layout(disconnected,[{from:1,to:2,key:'api domain'},{from:2,to:3,key:'domain storage'}]);
assert.deepEqual(plain(levelOrder.nodes.filter(n=>n.src!==null).map(n=>n.level)),[2,1,0,0],'disconnected names cannot split or invert construction-level groups');
assert.deepEqual(plain(levelOrder.nodes.map(n=>n.src)),[null,1,2,0,3],'stable name ties apply within a level, after construction depth');

const siblingCycle=layout(construction,[{from:0,to:2,key:'0 2'},{from:2,to:0,key:'2 0'},{from:2,to:3,key:'2 3'}]);
assert.equal(siblingCycle.nodes.find(n=>n.src===0).siblingCycle,true);assert.equal(siblingCycle.nodes.find(n=>n.src===2).siblingCycle,true);assert.equal(siblingCycle.nodes.find(n=>n.src===3).siblingCycle,false);
// Cross-branch file links order their containing packages, without flattening the tree.
const nested={nodes:[root,node(1,'a'),node(2,'z'),node(3,'a/impl.ts','file'),node(4,'z/main.ts','file')],edges:[{from:null,to:1},{from:null,to:2},{from:1,to:3},{from:2,to:4}]};
assert.deepEqual(plain(layout(nested,[{from:4,to:3,key:'4 3'}]).nodes.map(n=>n.src)),[null,2,4,1,3]);
const commons={nodes:[root,...Array.from({length:18},(_,i)=>node(i,'package-'+i))],edges:Array.from({length:18},(_,i)=>({from:null,to:i}))};
const commonsLinks=Array.from({length:32},(_,i)=>({from:i%18,to:(i*5+3)%18,key:'edge-'+i}));
const large=layout(commons,commonsLinks);verify(large);assert.ok(large.height<650&&large.width<900,'18-package overview remains compact at readable text size');
const stress={nodes:[root,...Array.from({length:500},(_,i)=>node(i,'Package-'+i))],edges:Array.from({length:500},(_,i)=>({from:null,to:i}))};
const stressLinks=Array.from({length:499},(_,i)=>({from:i,to:i+1,key:i+' '+(i+1)}));
const stressResult=layout(stress,stressLinks);verify(stressResult);assert.equal(stressResult.routes.length,499);assert.ok(stressResult.width<900);
const hubLinks=Array.from({length:160},(_,i)=>({from:i%2?0:i+1,to:i%2?i+1:0,key:'hub-'+i}));
const hub=layout(stress,hubLinks);verify(hub);assert.equal(hub.nodes.find(n=>n.src===0).h,28,'dense hubs do not become giant cards');assert.ok(hub.width<900);
assert.equal(new Set(hub.routes.map(r=>r.path)).size,hub.routes.length,'bounded gutter preserves distinct paths');
const parallel=layout({nodes:[node(0,'A'),node(1,'B')],edges:[]},Array.from({length:80},(_,i)=>({from:0,to:1,key:String(i)})));verify(parallel);assert.equal(new Set(parallel.routes.map(r=>r.path)).size,80,'parallel relations have distinct bounded curves');
const self=layout({nodes:[node(0,'A')],edges:[]},[{from:0,to:0,key:'self'}]);verify(self);assert.notEqual(self.routes[0].points[0].y,self.routes[0].points.at(-1).y,'self arc retains nonzero geometry');
const compressed=layout({nodes:[root,node(10,'org/example','module',{context:true,sourceIds:[8,9,10],contextNames:['org','org/example']}),node(11,'org/example/child')],edges:[{from:null,to:10},{from:10,to:11}]},[]);
assert.deepEqual(plain(compressed.nodes.find(n=>n.src===10).sourceIds),[8,9,10]);verify(compressed);
assert.deepEqual(plain(layout({nodes:[],edges:[]},[])),{nodes:[],hierarchy:[],routes:[],width:0,height:0,relationX:0});
console.log('Exploration geometry: responsive widths, readable compact rows, SCCs, mirrored relation axis, bounded curved arcs, stable In/Out override and card-free500-node stress passed');

assert.equal(typeof context.roundedTreePath,'function','tree edges need a rounded path builder');
const rounded=context.roundedTreePath;
const corners=[{x:0,y:0},{x:100,y:0},{x:100,y:100},{x:0,y:100}];
assert.equal(rounded(corners),'M 0 0 L 90 0 Q 100 0 100 10 L 100 90 Q 100 100 90 100 L 0 100');
assert.equal(rounded([{x:0,y:0},{x:4,y:0},{x:4,y:6}],20),'M 0 0 L 2 0 Q 4 0 4 2 L 4 6','short segments limit the corner radius to half of either neighbor');
assert.equal(rounded([{x:0,y:0},{x:4,y:0},{x:4,y:2},{x:0,y:2}],20),'M 0 0 L 3 0 Q 4 0 4 1 L 4 1 Q 4 2 3 2 L 0 2','adjacent rounded corners cannot overshoot a short shared segment');
assert.equal(rounded([]),'');
assert.equal(rounded([{x:7,y:8}]),'M 7 8');
assert.equal(rounded([{x:7,y:8},{x:7,y:8},{x:7,y:8}]),'M 7 8','identical points cannot produce NaN');
assert.equal(rounded([{x:7,y:8},{x:10,y:8}]),'M 7 8 L 10 8');
assert.equal(rounded([{x:0,y:0},{x:0,y:0},{x:100,y:0},{x:100,y:0},{x:100,y:100}]),'M 0 0 L 90 0 Q 100 0 100 10 L 100 100','duplicate points do not remove a real corner');
assert.equal(rounded([{x:0,y:0},{x:5,y:0},{x:10,y:0}]),'M 0 0 L 5 0 L 10 0','straight runs remain straight');
assert.equal(rounded([{x:0,y:0},{x:5,y:0},{x:0,y:0}]),'M 0 0 L 5 0 L 0 0','a reversal cannot become an invalid rounded corner');
assert.equal(rounded(corners,0),'M 0 0 L 100 0 L 100 100 L 0 100');
assert.equal(rounded(corners,-10),rounded(corners,0));
assert.equal(rounded(corners,NaN),rounded(corners,0));
assert.equal(JSON.stringify(corners),'[{"x":0,"y":0},{"x":100,"y":0},{"x":100,"y":100},{"x":0,"y":100}]','path generation never changes the route used for arrow direction and hit geometry');

console.log('Hierarchy rounding: bounded quadratic corners, degenerate points, stable endpoints passed');

const diagonal=[{x:0,y:0},{x:10,y:0},{x:20,y:10},{x:20,y:30}];
const soft=rounded(diagonal,4);assert.match(soft,/Q 10 0/);assert.match(soft,/Q 20 10/);assert.ok(soft.startsWith('M 0 0 L 6 0 '));assert.ok(soft.endsWith(' L 20 30'));assert.doesNotMatch(soft,/NaN|Infinity/);

for(const route of layout(input,links,{tracks:true,width:1400}).routes){assert.match(route.path,/ C /,'Graph tracks use the original continuous cubic arcs');assert.equal(route.points[1].y,route.points[2].y);assert.equal(route.points[3].y,route.points[4].y);}
