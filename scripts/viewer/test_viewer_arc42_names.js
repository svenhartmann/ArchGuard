// Building-block names in the arc42 Architecture view: relative to the shown blocks, without
// generic source roots, unique. The cases are shared with the desktop app (architecture_ui/names.rs).
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const cases=JSON.parse(fs.readFileSync('scripts/viewer/arc42_block_names.json','utf8'));
// This repository's shape: every crate a root, its source root below it, one sub-package.
const nodes=[
  {id:0,name:'crates/archguard-cli',kind:'module',parent:null},{id:1,name:'crates/archguard-cli/src',kind:'module',parent:0},{id:2,name:'crates/archguard-cli/src/main.rs',kind:'file',parent:1,lang:'rust'},
  {id:3,name:'crates/archguard-engine',kind:'module',parent:null},{id:4,name:'crates/archguard-engine/src',kind:'module',parent:3},{id:5,name:'crates/archguard-engine/src/lib.rs',kind:'file',parent:4,lang:'rust'},
  {id:6,name:'crates/archguard-engine/src/semantic_calls',kind:'module',parent:4},{id:7,name:'crates/archguard-engine/src/semantic_calls/mod.rs',kind:'file',parent:6,lang:'rust'},
  {id:8,name:'crates/archguard-engine/src/semantic_calls/rust',kind:'module',parent:6},{id:9,name:'crates/archguard-engine/src/semantic_calls/rust/mod.rs',kind:'file',parent:8,lang:'rust'},
];
const chapters=[5].map(number=>({number,title:'Building block view',markdown:'## 5. Building block view\n\nSource evidence',questions:[]}));
const app=launch({root:'/repo',nodes,edges:[{from:2,to:5,count:1,refs:[]},{from:7,to:5,count:1,refs:[]}],view:{depth:0,visible:null},findings:[],cyclicEdges:[],insights:{arc42:{chapters}}},{library:false});
for(const c of cases){const short=JSON.parse(app.run('JSON.stringify([...arc42ShortNames('+JSON.stringify(c.names)+').values()])'));assert.deepEqual(short,c.labels,c.case);}
app.run("showTab('architecture')");const content=app.ids.get('architecture-content');
const cards=()=>Array.from(content.querySelectorAll('g[data-arc42-node]'),g=>({title:g.querySelectorAll('title')[0].textContent,name:g.querySelectorAll('text').filter(t=>t.getAttribute('class')==='arc42-map-name').map(t=>t.textContent).join('')}));
const levels=()=>Array.from(content.querySelectorAll('button[data-arc42-level-scope]'),b=>b.querySelectorAll('span')[0].textContent);
assert.deepEqual(levels(),['Project overview','archguard-cli','archguard-engine/semantic_calls','archguard-engine'],'the level list names crates, not their source roots');
assert.deepEqual(cards().map(c=>c.name),['archguard-engine/semantic_calls','archguard-cli','archguard-engine'],'map boxes show the same names');
assert.deepEqual(cards().map(c=>c.title),['crates/archguard-engine/src/semantic_calls','crates/archguard-cli/src','crates/archguard-engine/src'],'the full path stays the title');
assert.deepEqual(Array.from(content.querySelectorAll('select[data-arc42-scope]')[0].children,o=>o.textContent),['Project overview','archguard-cli','archguard-engine','archguard-engine/semantic_calls','archguard-engine/semantic_calls/rust'],'the scope picker uses the names');
assert.ok(content.querySelectorAll('p').some(p=>p.textContent.startsWith('archguard-cli → archguard-engine · 1')),'relationships use the names');
const standard=JSON.parse(app.run('JSON.stringify(arc42StandardGraph(arc42PackageProjection(),null))'));
assert.deepEqual(standard.nodes.map(n=>n.attributes.label.split('\n')[0]),['archguard-engine/semantic_calls','archguard-cli','archguard-engine'],'the laid-out diagram uses the names');
assert.ok(app.run('arc42PlantUml(arc42PackageProjection())').includes('"crates/archguard-cli/src"'),'the PlantUML export keeps the full path');
assert.deepEqual(JSON.parse(app.run("JSON.stringify([arc42NameLines('archguard-engine'),arc42NameLines('archguard-engine/semantic_calls'),arc42NameLines('a'.repeat(50))])")),[['archguard-engine'],['archguard-engine/','semantic_calls'],['a'.repeat(24),'a'.repeat(24)+'…']],'box names break after a slash, as diagram::name_lines');
content.querySelectorAll('button[data-arc42-level-scope]').find(b=>b.getAttribute('data-arc42-level-scope')==='4').click();
assert.deepEqual(cards().map(c=>c.name),['semantic_calls','archguard-engine / direct files','Other project packages'],'inside a block the names are relative to it');
content.querySelectorAll('g[data-arc42-node]').find(g=>g.getAttribute('data-arc42-node')==='6').click();
assert.equal(content.querySelectorAll('strong').map(s=>s.textContent).find(s=>s.includes('semantic')),'semantic_calls','the selection shows the name');
content.querySelectorAll('button[data-arc42-level-scope]').find(b=>b.getAttribute('data-arc42-level-scope')==='6').click();
assert.deepEqual(levels(),['Project overview','archguard-cli','archguard-engine/semantic_calls','rust','archguard-engine'],'level 3 is named below its parent');
assert.deepEqual(Array.from(content.querySelectorAll('nav[data-arc42-breadcrumb]')[0].querySelectorAll('button'),b=>b.textContent),['1 · Project overview','2 · archguard-engine/semantic_calls'],'breadcrumbs use the level names');
console.log('arc42 names: shared cases, level list, map boxes, titles, scope picker, relations, Graphviz labels and selection passed');
