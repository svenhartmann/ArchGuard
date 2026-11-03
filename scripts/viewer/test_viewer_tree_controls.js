// Execute the SVG renderer: direction counts, cycle colors, rounded routes and zoom.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/tmp/controls',nodes:['A','B','In','Out','Other','Else',...Array.from({length:20},(_,i)=>'Quiet-'+i)].map((name,id)=>({id,name,parent:null,kind:'module',lang:'ts'})),edges:[[0,1],[1,0],[2,0],[0,3],[4,5]].map(([from,to])=>({from,to,count:1,refs:[]})),view:{depth:0,visible:null},cyclicEdges:[],findings:[]};
const app=launch(fixture),tree=app.ids.get('package-tree');
const control=name=>tree.querySelectorAll('button').find(b=>b.getAttribute('data-tree-zoom')===name);
assert.ok(control('in'),'the package tree must expose zoom controls');
const svg=()=>tree.querySelector('svg');
const groups=()=>tree.querySelectorAll('g[data-package-node]');
const a=()=>groups().find(n=>n.getAttribute('data-package-node')==='A');
assert.match(a().textContent,/In 2/);assert.match(a().textContent,/Out 2/);
assert.equal(a().querySelector('text[data-tree-incoming]').getAttribute('fill'),app.run('C.incoming'));
assert.equal(a().querySelector('text[data-tree-outgoing]').getAttribute('fill'),app.run('C.outgoing'));
assert.match(a().querySelector('text[data-tree-incoming]').textContent,/^In /,'incoming count remains on the left');
assert.equal(a().querySelector('text[data-tree-outgoing]').getAttribute('text-anchor'),'end','outgoing count sits at the right edge');
assert.ok(Number(a().querySelector('text[data-tree-incoming]').getAttribute('x'))<Number(a().querySelector('text[data-tree-outgoing]').getAttribute('x')),'compact row counters retain In before Out');
// The default separates incoming/outgoing around the tree, with every real
// cycle on the right regardless of its direction relative to the selection.
assert.equal(app.run('treeRouting'),'direction');
for(const selected of [null,'A','B']){
 if(selected)app.run('selectNode('+JSON.stringify(selected)+')');
 app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges==='all').click();
 const cyclic=tree.querySelectorAll('g[data-tree-edge]').filter(g=>app.run('cyclicKeys.has('+JSON.stringify(g.getAttribute('data-tree-edge'))+')'));
 assert.equal(cyclic.length,2,'both real directions of the cycle remain visible');
 assert.deepEqual(Array.from(cyclic,g=>g.getAttribute('data-tree-side')).sort(),['right','right']);
 for(const group of cyclic){
  const [from,to]=group.getAttribute('data-tree-edge').split(' ').map(id=>app.run('view.nodes['+Number(id)+'].name'));
  const side=group.getAttribute('data-tree-side'),color=side==='right'?'cycle':to===selected?'incoming':from===selected?'outgoing':'edge';
  const line=group.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-line');
  assert.equal(line.getAttribute('stroke'),app.run('C.'+color),'both directed cycle edges stay red in the compact graph');
  assert.equal(group.querySelector('polygon').getAttribute('fill'),app.run('C.'+color));
  assert.equal(line.getAttribute('stroke-dasharray'),null,'a real cycle is not coarser package context');
  assert.match(group.textContent,/cyclic dependency/i,'left and right edges both explain actual cycle membership');
 }
}
a().click();
app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges==='all').click();
function edge(from,to){const key=app.run("view.nodes.find(n=>n.name==="+JSON.stringify(from)+").id+' '+view.nodes.find(n=>n.name==="+JSON.stringify(to)+").id");return tree.querySelectorAll('g[data-tree-edge]').find(e=>e.getAttribute('data-tree-edge')===key);}
for(const [from,to,color,side] of [['A','B','cycle','right'],['B','A','cycle','right'],['In','A','incoming','left'],['A','Out','outgoing','right'],['Other','Else','edge','right']]){
 const group=edge(from,to),line=group.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-line');
 assert.equal(line.getAttribute('stroke'),app.run('C.'+color),from+' → '+to+' line');
 assert.equal(group.querySelector('polygon').getAttribute('fill'),app.run('C.'+color),from+' → '+to+' arrow');
 assert.equal(group.getAttribute('data-tree-side'),side,from+' → '+to+' stays wholly on the expected side');
 assert.match(group.querySelector('polygon').getAttribute('transform'),side==='left'?/rotate\(0\)$/:/rotate\(180\)$/,'the arrow points into the endpoint on the chosen side');
 const row=a(),center=Number(row.getAttribute('transform').match(/translate\(([-\d.]+)/)[1]),half=Number(row.querySelector('rect[data-tree-row]').getAttribute('width'))/2;
 const xs=line.getAttribute('d').match(/-?\d+(?:\.\d+)?/g).map(Number).filter((_,i)=>i%2===0);
 assert.ok(xs.every(x=>side==='left'?x<center-half:x>center+half),from+' → '+to+' lies outside the tree on its assigned side');
 assert.match(line.getAttribute('d'),/^M .* L .* C .* L /,'relations use original smooth arcs with horizontal endpoint leads');
 assert.match(line.getAttribute('d'),/ C /,'dependencies use continuous cubic bows');
 const hit=group.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-hit');
 assert.equal(hit.getAttribute('d'),line.getAttribute('d'),'hit target follows the actual rounded path');
 assert.notEqual(hit.getAttribute('vector-effect'),'non-scaling-stroke','zooming out must not grow a hit target across neighboring lanes');
 assert.ok(Number(hit.getAttribute('stroke-width'))<=8,'hit strokes remain narrower than the separation between adjacent ports');
}
const paths=()=>JSON.stringify(tree.querySelectorAll('g[data-tree-edge]').map(g=>[g.getAttribute('data-tree-edge'),g.getAttribute('data-tree-side'),g.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-line').getAttribute('d')]));
const beforeEvidence=paths();edge('A','Out').click();
assert.equal(paths(),beforeEvidence,'reading edge evidence keeps both complete routes on their current sides');
assert.equal(app.run('ui.selection.reference'),'A');
const completeRoutes=new Map(tree.querySelectorAll('g[data-tree-edge]').map(g=>[g.getAttribute('data-tree-edge'),g.querySelector('path').getAttribute('d')]));
for(const mode of ['incoming','outgoing']){
 app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges===mode).click();
 const shown=tree.querySelectorAll('g[data-tree-edge]');assert.equal(shown.length,2);
 for(const g of shown)assert.equal(g.querySelector('path').getAttribute('d'),completeRoutes.get(g.getAttribute('data-tree-edge')),'direction filters keep complete routes on their assigned sides');
 if(mode==='incoming'){
  assert.equal(edge('B','A').getAttribute('data-tree-side'),'right','an incoming cycle remains visible on the right even in Incoming-only mode');
  assert.equal(edge('In','A').getAttribute('data-tree-side'),'left');
  assert.match(edge('B','A').textContent,/Incoming to A.*Right of the tree/s,'a cyclic incoming edge retains its direction in the explanation');
 }
}
app.ids.get('edge-modes').querySelectorAll('button').find(b=>b.dataset.edges==='all').click();

assert.equal(Number(a().querySelector('rect[data-tree-row]').getAttribute('height')),28,'packages use compact rows with a small folder icon');
assert.ok(a().querySelectorAll('path').some(p=>p.getAttribute('class')==='tree-row-icon'));
const positions=()=>JSON.stringify(groups().map(n=>[n.getAttribute('data-package-node'),n.getAttribute('transform')]));
const before=positions(),analysis=app.run('JSON.stringify([work,view,sccs,currentAcd,ops])'),viewBox=svg().getAttribute('viewBox');
const initialWidth=Number(svg().getAttribute('width')),initialHeight=Number(svg().getAttribute('height'));
tree.getBoundingClientRect=()=>({width:300,height:400,left:0,top:0});
tree.querySelector('div[data-tree-toolbar]').getBoundingClientRect=()=>({height:40,width:300});
assert.ok(initialWidth-300>=100&&initialHeight+40-400>=200,'the scroll position is reachable in an actual viewport');
tree.scrollLeft=100;tree.scrollTop=200;
control('in').click();
assert.equal(Number(svg().getAttribute('width')),initialWidth*1.25);
assert.equal(Number(svg().getAttribute('height')),initialHeight*1.25);
assert.equal(svg().getAttribute('viewBox'),viewBox,'zoom scales the SVG viewport rather than relaying out nodes');
assert.equal(positions(),before);
assert.equal(tree.scrollLeft,162.5,'button zoom anchors the viewport center horizontally');
assert.equal(tree.scrollTop,295,'button zoom anchors the visible diagram below its toolbar vertically');
assert.match(tree.querySelector('output[data-tree-scale]').textContent,/125%/);
control('out').click();assert.equal(Number(svg().getAttribute('width')),initialWidth);
control('in').click();app.click('toggle-cycles');assert.equal(Number(svg().getAttribute('width')),initialWidth*1.25,'filtering preserves zoom');
app.run("applyTheme('dark')");assert.equal(Number(svg().getAttribute('width')),initialWidth*1.25,'theme repaint preserves zoom');
tree.querySelector('div[data-tree-toolbar]').getBoundingClientRect=()=>({height:40,width:300});
control('reset').click();assert.equal(Number(svg().getAttribute('width')),initialWidth);
let prevented=false;
tree.dispatch('wheel',{deltaY:120,clientX:300,clientY:300,preventDefault(){prevented=true;}});
assert.equal(prevented,false,'ordinary wheel scrolling stays native');assert.equal(Number(svg().getAttribute('width')),initialWidth);
tree.dispatch('wheel',{ctrlKey:true,deltaY:-100,deltaMode:0,clientX:300,clientY:300,preventDefault(){prevented=true;}});
assert.equal(prevented,true);assert.ok(Number(svg().getAttribute('width'))>initialWidth,'Ctrl-wheel zooms the tree');
control('reset').click();tree.querySelector('div[data-tree-toolbar]').getBoundingClientRect=()=>({height:40,width:300});
control('fit').click();
assert.ok(Number(svg().getAttribute('width'))<=276);
assert.ok(Number(svg().getAttribute('height'))<=336);
assert.equal(tree.scrollLeft,0);assert.equal(tree.scrollTop,0);
assert.equal(app.run('JSON.stringify([work,view,sccs,currentAcd,ops])'),analysis,'zoom and color controls preserve analysis');
for(let i=0;i<60;i++)control('in').click();assert.ok(control('in').disabled,'upper zoom limit is reflected by the button');
for(let i=0;i<80;i++)control('out').click();assert.ok(control('out').disabled,'lower zoom limit is reflected by the button');
const offline=launch(fixture,{library:false});
offline.ids.get('package-tree').querySelectorAll('button').find(b=>b.getAttribute('data-tree-zoom')==='in').click();
assert.match(offline.ids.get('package-tree').querySelector('output[data-tree-scale]').textContent,/125%/,'zoom works without ForceGraph');
console.log('Tree controls: incoming/outgoing counts and colors, red cycles, rounded visible/hit paths, anchored zoom, fit, bounds and offline behavior passed');

for(const reverse of [false,true]){
 const points=[{x:10,y:0},{x:20,y:0},{x:100,y:0},{x:100,y:100},{x:20,y:100},{x:10,y:100}];if(reverse)points.reverse();
 const path=app.run(`treeClippedPath(${JSON.stringify(points)},25,75)`);assert.match(path,/ C /);assert.doesNotMatch(path,/Q|NaN|Infinity/);
 const numbers=path.match(/-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/g).map(Number);assert.equal(numbers.length,8);
 assert.ok(Math.abs(numbers[1]-(reverse?75:25))<1e-6);assert.ok(Math.abs(numbers[7]-(reverse?25:75))<1e-6);
 // The clipped cubic must pass through the original bow's midpoint.
 assert.ok(Math.abs((numbers[0]+3*numbers[2]+3*numbers[4]+numbers[6])/8-80)<1e-6);
 assert.ok(Math.abs((numbers[1]+3*numbers[3]+3*numbers[5]+numbers[7])/8-50)<1e-6);
 assert.equal(app.run(`treeClippedPath(${JSON.stringify(points)},150,200)`).trim(),'');
 const hit=JSON.parse(app.run(`JSON.stringify(treeRouteHitPoints({points:${JSON.stringify(points)}}))`));assert.ok(hit.some(p=>p.x===80&&p.y===50),'batched hit testing follows the actual cubic instead of its control polygon');
}
console.log('Clipped cubic paths and hit geometry preserve the original curve in both directions');
