// Exercise the real toolbar with DOM doubles; this is not browser layout QA.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/tmp/tree-options',nodes:[
  {id:0,name:'A',kind:'module',parent:null},{id:1,name:'A/a.ts',kind:'file',parent:0},
  {id:2,name:'B',kind:'module',parent:null},{id:3,name:'B/b.ts',kind:'file',parent:2},
],edges:[{from:1,to:3,count:1,refs:[]},{from:3,to:1,count:1,refs:[]}],view:{depth:0,visible:null},cyclicEdges:[],findings:[]};
const app=launch(fixture,{library:false}),tree=app.ids.get('package-tree');
const options=()=>tree.querySelector('details[data-tree-options]');
const legend=()=>tree.querySelector('details[data-tree-legend]');
const routing=()=>tree.querySelector('button[data-tree-routing]');
const reset=()=>tree.querySelectorAll('button').find(b=>['Reset tree','Baum zurücksetzen'].includes(b.textContent));
const insideDetails=node=>{for(let parent=node.parentElement;parent;parent=parent.parentElement)if(parent.tagName==='DETAILS')return true;return false;};
// Native details emit toggle after their open state changes; model that event.
const toggle=(details,open=true)=>{details.open=open;details.dispatch('toggle');};
assert.ok(options(),'infrequent actions share one native disclosure');
assert.equal(options().open,false);
assert.equal(options().querySelector('summary').textContent,'View');
for(const control of [routing(),reset(),legend(),...tree.querySelectorAll('button[data-tree-all]'),...tree.querySelectorAll('button[data-tree-sort]')])assert.ok(options().contains(control),'secondary controls belong inside the View menu');
assert.equal(legend().open,false);
assert.equal(tree.querySelectorAll('button[data-tree-zoom]').length,4);
for(const control of tree.querySelectorAll('button[data-tree-zoom]'))assert.equal(insideDetails(control),false,'zoom and fit stay directly available');
assert.equal(tree.querySelector('div[data-tree-toolbar]').querySelectorAll('button').filter(b=>!insideDetails(b)).length,4,'only four primary actions remain permanently visible');

app.run("selectNode('A');setPackageTreeZoom(1.25)");
const baseline=app.run('JSON.stringify([data,work,view,ops,ui.selection,ui.edgeMode,treeZoom])');
toggle(options());toggle(legend());assert.equal(legend().open,true);
toggle(options());options().querySelector('summary').focus();
assert.equal(legend().open,true,'the nested legend can remain open without overlapping a second popup');
app.run('renderPackageTree()');
assert.equal(options().open,true);assert.equal(legend().open,true);
assert.equal(app.document.activeElement,options().querySelector('summary'),'rebuilding the tree preserves focus on its options summary');
tree.scrollTop=52;
routing().focus();routing().click();
assert.equal(app.run('treeRouting'),'ordered');
assert.equal(options().open,true,'changing routing does not close the menu');
assert.equal(legend().open,true);
assert.equal(app.document.activeElement,routing(),'keyboard focus follows the replacement routing button');
assert.equal(routing().getAttribute('aria-pressed'),'false');
assert.equal(tree.scrollTop,52);
assert.equal(app.run('JSON.stringify([data,work,view,ops,ui.selection,ui.edgeMode,treeZoom])'),baseline,'routing changes preserve graph, selection, filters and zoom');
app.run("applyTheme('dark')");
assert.equal(options().open,true);
assert.equal(app.document.activeElement,routing(),'an unrelated tree repaint also preserves routing focus');

let prevented=false,stopped=false;
const escape={key:'Escape',target:routing(),preventDefault(){prevented=true;},stopPropagation(){stopped=true;}};
options().dispatch('keydown',escape);
// The DOM double does not bubble events; exercise the actual global Escape
// handler if the options menu fails to consume the event locally.
if(!stopped)for(const handler of app.document.events.keydown||[])handler(escape);
assert.equal(options().open,false,'Escape closes Tree options');
assert.equal(prevented,true);assert.equal(stopped,true,'Escape must not clear graph selection through the global handler');
assert.equal(app.document.activeElement,options().querySelector('summary'),'Escape returns focus to the options summary');
assert.equal(app.run('JSON.stringify([data,work,view,ops,ui.selection,ui.edgeMode,treeZoom])'),baseline,'closing options preserves the selected graph context');
assert.equal(legend().open,true,'closing the parent preserves nested disclosure state');
toggle(options());
options().dispatch('keydown',{key:'Enter',preventDefault(){assert.fail('Enter keeps its native behavior');},stopPropagation(){assert.fail('other keys must not be intercepted');}});
assert.equal(options().open,true);

app.click('language-de');
assert.equal(options().querySelector('summary').textContent,'Ansicht');
assert.equal(routing().textContent,'Eingehend / Ausgehend');assert.equal(reset().textContent,'Baum zurücksetzen');
assert.equal(legend().querySelector('summary').textContent,'Legende');
assert.equal(options().open,true);assert.equal(legend().open,true);
assert.equal(app.run('treeRouting'),'ordered');
assert.equal(app.run('JSON.stringify([data,work,view,ops,ui.selection,ui.edgeMode,treeZoom])'),baseline,'translation does not reset the tree state');
app.click('language-en');
assert.equal(options().querySelector('summary').textContent,'View');
toggle(legend());
assert.equal(options().open,true,'the legend stays inside the open menu');
legend().querySelector('summary').focus();app.run('renderPackageTree()');
assert.equal(app.document.activeElement,legend().querySelector('summary'),'legend focus survives independently of options');
assert.equal(legend().open,true);assert.equal(options().open,true);
app.click('language-de');assert.equal(legend().open,true);assert.equal(options().open,true,'translation preserves the nested legend');
app.click('language-en');
toggle(options());toggle(options(),false);options().querySelector('summary').focus();app.run('renderPackageTree()');
assert.equal(options().open,false,'a deliberately closed menu stays closed');
assert.equal(legend().open,true,'nested legend state survives closing its parent');
assert.equal(app.document.activeElement,options().querySelector('summary'));
toggle(options());reset().focus();reset().click();
assert.equal(app.run('treeZoom'),1);assert.equal(app.run('treeRouting'),'direction','reset restores incoming-left/outgoing-right routing');
assert.equal(options().open,true);assert.equal(app.document.activeElement,reset());

const empty=launch({...fixture,nodes:[],edges:[]},{library:false}),emptyTree=empty.ids.get('package-tree');
const emptyOptions=()=>emptyTree.querySelector('details[data-tree-options]');
toggle(emptyOptions());emptyOptions().querySelector('summary').focus();empty.run('renderPackageTree()');
assert.equal(emptyOptions().open,true);
assert.equal(empty.document.activeElement,emptyOptions().querySelector('summary'),'empty scopes keep disclosure focus too');
console.log('Tree options: compact default toolbar, one nested menu, persistent disclosure/focus, local Escape, routing, reset and DE/EN state passed');
