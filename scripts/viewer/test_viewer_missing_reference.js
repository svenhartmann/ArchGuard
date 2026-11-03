// A missing direction reference is not a measured zero-degree node.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/tmp/reference-counts',nodes:['A','B','C'].map((name,id)=>({id,name,parent:null,kind:'file',lang:'ts'})),edges:[{from:0,to:1,count:3,refs:[]},{from:1,to:2,count:2,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const app=launch(fixture,{library:false});
const control=mode=>app.ids.get('edge-modes').querySelectorAll('button').find(button=>button.dataset.edges===mode);
const shownEdges=()=>app.run("view.edges.filter(e=>dependencyMatchesSelection(e.from,e.to)).length");
function missingReference(labels,explanation){
  const restore=app.run('ui.edgeMode'),overview=shownEdges();
  for(const [mode,label] of [['incoming',labels[0]],['outgoing',labels[1]],['both',labels[2]]]){
    const button=control(mode);
    assert.equal(button.textContent,label+' (—)','missing reference must not claim a measured zero');
    assert.equal(button.disabled,false,'a direction may be chosen before its reference');
    assert.match(button.title,explanation);
    assert.match(button.getAttribute('aria-label'),explanation,'screen readers receive the reason for the missing count');
    button.click();assert.equal(app.run('ui.edgeMode'),mode,'the chosen direction stays pending until a reference is selected');
    assert.equal(shownEdges(),overview,'a pending direction does not hide the overview');
  }
  control(restore).click();
}
missingReference(['Incoming','Outgoing','Both'],/Choose this direction, then select a package or file/);
assert.equal(control('all').textContent,'All (2)','All retains the actual number of displayed connections');
assert.equal(control('all').disabled,false);
app.click('language-de');
missingReference(['Eingehend','Ausgehend','Beide'],/Richtung wählen, dann ein Paket oder eine Datei auswählen/);
assert.equal(control('all').textContent,'Alle (2)');
app.run("selectNode('A')");
assert.equal(control('incoming').textContent,'Eingehend (0)','an actual zero-degree direction remains zero');
assert.equal(control('outgoing').textContent,'Ausgehend (1)');
assert.equal(control('both').textContent,'Beide (1)');
assert.ok(['incoming','outgoing','both'].every(mode=>!control(mode).disabled));
assert.doesNotMatch(control('incoming').getAttribute('aria-label'),/Richtung wählen, dann ein Paket oder eine Datei auswählen/);
app.run('resetPackageTree()');
missingReference(['Eingehend','Ausgehend','Beide'],/Richtung wählen, dann ein Paket oder eine Datei auswählen/);
app.click('toggle-cycles');
missingReference(['Eingehend','Ausgehend','Beide'],/Richtung wählen, dann ein Paket oder eine Datei auswählen/);
assert.equal(control('all').textContent,'Alle (0)','a real empty cycle filter stays distinguishable from a missing reference');
console.log('Direction counts: absent reference, real zero, All, cycle filtering, reset and DE/EN accessibility passed');
