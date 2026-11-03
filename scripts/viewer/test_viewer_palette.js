// Use the actual light/dark CSS tokens: the shared harness's empty styles
// otherwise cannot detect stale inline SVG colors after a theme change.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
const palettes=Object.fromEntries([['light',':root {'],['dark',"html[data-theme='dark'] {"]].map(([theme,start])=>[theme,Object.fromEntries([...html.split(start)[1].split('}')[0].matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(match=>[match[1],match[2].trim()]))]));
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const names=['A','B','Incoming','Outgoing','Other','Else'],pairs=[[0,1],[1,0],[2,0],[0,3],[4,5]];
const services=names.map(name=>({id:name,name,compose_file:'compose.yaml',build_context:name,networks:['default'],profiles:[]}));
const fixture={root:'/tmp/palette',nodes:[...names,...Array.from({length:20},(_,i)=>'Quiet-'+i)].map((name,id)=>({id,name,kind:'module',parent:null})),edges:pairs.map(([from,to])=>({from,to,count:1,refs:[]})),findings:[],cyclicEdges:[],view:{depth:0,visible:null},services:{schema_version:1,root:'/tmp/palette',complete:true,monorepo:true,monorepo_reason:'Local contexts',compose_files:['compose.yaml'],services,dependencies:pairs.map(([a,b])=>({from:names[a],to:names[b],kind:'configured_http',evidence:[]})),cycles:[{kind:'configured_http',nodes:['A','B']}],diagnostics:[]}};
const app=launch(fixture,{library:false});
app.context.getComputedStyle=()=>({getPropertyValue:key=>palettes[app.document.documentElement.getAttribute('data-theme')||'light'][key]||''});
app.run("applyTheme('light');selectNode('A');ui.edgeMode='all';renderGraphControls();setPackageTreeZoom(1.25)");
const tree=app.ids.get('package-tree');tree.scrollTop=120;
const geometry=()=>Array.from(tree.querySelectorAll('g[data-tree-edge]'),edge=>[edge.getAttribute('data-tree-edge'),edge.getAttribute('data-tree-side'),edge.querySelector('path').getAttribute('d')]);
const before=JSON.stringify(geometry()),analysis=app.run('JSON.stringify([work,view,ops,ui.selection,currentAcd])');
function packageColors(theme){
  const ids=JSON.parse(app.run('JSON.stringify(Object.fromEntries(view.nodes.map(node=>[node.name,node.id])))'));
  for(const [from,to,direction,cyclic] of [[0,1,'outgoing',true],[1,0,'incoming',true],[2,0,'incoming'],[0,3,'outgoing'],[4,5,'edge']]){
    const edge=tree.querySelectorAll('g[data-tree-edge]').find(e=>e.getAttribute('data-tree-edge')===ids[names[from]]+' '+ids[names[to]]);
    const kind=cyclic&&edge.getAttribute('data-tree-side')==='right'?'cycle':direction;
    assert.equal(edge.querySelectorAll('path').find(p=>p.getAttribute('class')==='tree-edge-line').getAttribute('stroke'),palettes[theme]['--c-'+kind],theme+': '+names[from]+' → '+names[to]+' line uses the active '+kind+' color');
    assert.equal(edge.querySelector('polygon').getAttribute('fill'),palettes[theme]['--c-'+kind],theme+': arrow and line share their direction or cycle color');
  }
  const node=tree.querySelectorAll('g[data-package-node]').find(n=>n.getAttribute('data-package-node')==='A');
  assert.equal(node.querySelector('text[data-tree-incoming]').getAttribute('fill'),palettes[theme]['--c-incoming']);
  assert.equal(node.querySelector('text[data-tree-outgoing]').getAttribute('fill'),palettes[theme]['--c-outgoing']);
}
packageColors('light');app.run("applyTheme('dark')");packageColors('dark');
assert.equal(JSON.stringify(geometry()),before);assert.equal(app.run('treeZoom'),1.25);assert.equal(tree.scrollTop,120);
assert.equal(app.run('JSON.stringify([work,view,ops,ui.selection,currentAcd])'),analysis);
app.run("applyTheme('light')");packageColors('light');
app.run("applyTheme('system')");packageColors('light');
app.run("showTab('services');selectService('A');serviceUi.mode='all';renderServices();setServiceZoom(1.25)");
const serviceTree=app.ids.get('services-tree');
for(const theme of ['dark','light']){
  app.run("applyTheme('"+theme+"')");
  for(const [from,to,direction,cyclic] of [['A','B','outgoing',true],['B','A','incoming',true],['Incoming','A','incoming'],['A','Outgoing','outgoing'],['Other','Else','edge']]){
    const edge=serviceTree.querySelectorAll('g[data-service-edge]').find(e=>e.getAttribute('data-service-edge')===JSON.stringify([from,to,'configured_http']));
    const kind=cyclic&&edge.getAttribute('data-service-side')==='right'?'cycle':direction;
    assert.equal(edge.querySelector('path[data-service-line]').getAttribute('stroke'),palettes[theme]['--c-'+kind]);
    assert.equal(edge.querySelector('polygon').getAttribute('fill'),palettes[theme]['--c-'+kind]);
  }
  assert.equal(app.run('serviceUi.zoom'),1.25);assert.equal(app.run('serviceUi.selected'),'A');
}
console.log('Palette: actual CSS direction/cycle colors update on theme changes for package and service SVGs without changing geometry, selection or zoom.');
