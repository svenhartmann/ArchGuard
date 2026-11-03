// Exercise translations declared in the real HTML and the real download code.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
for(const id of ['export-json','export-svg','clear-search','explorer-expand-all','explorer-collapse-all','repair-cycle','cancel-action']){
  const tag=html.match(new RegExp('<[^>]+\\bid="'+id+'"[^>]*>'))?.[0];
  assert.ok(tag?.includes('data-i18n-de='),id+' declares a German UI translation');
}
// Translating an entire container would replace its controls/icons with text.
for(const match of html.matchAll(/<([a-z][\w-]*)\b([^>]*\bdata-i18n-de="[^"]*"[^>]*)>([\s\S]*?)<\/\1>/g)){
  assert.doesNotMatch(match[3],/<[a-z]/i,'text translation is limited to leaf elements: '+match[0]);
}
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const fixture={root:'/tmp/All',nodes:[{id:0,name:'All',kind:'module',parent:null},{id:1,name:'All/External.java',kind:'file',parent:0}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const app=launch(fixture,{library:false});
assert.equal(app.ids.get('export-json').textContent,'JSON data + evidence');
app.run("setViewerLanguage('de')");
assert.match(app.ids.get('toggle-files').textContent,/Dateien/,'graph file-filter state follows the selected language');
assert.equal(app.ids.get('export-json').textContent,'JSON-Daten + Belege');
assert.equal(app.ids.get('clear-search').textContent,'Suche löschen');
assert.equal(app.ids.get('filter').getAttribute('placeholder'),'Module oder Dateien suchen…');
assert.equal(app.ids.get('calls-filter').getAttribute('aria-label'),'Typen, Module oder Funktionen filtern');
assert.equal(app.ids.get('explorer-expand-all').getAttribute('title'),'Alle Pakete in diesem Bericht aufklappen und die Suche löschen');
assert.match(html,/<button id="export-ops"[\s\S]*?<svg[\s\S]*?<span data-i18n-de="Änderungen exportieren">Export changes<\/span>[\s\S]*?<\/button>/,'only the label beside the export icon is translated');
const nodes=app.run('JSON.stringify(data.nodes)');
assert.equal(app.run('rootName'),'All','project names remain untranslated');
assert.equal(nodes,JSON.stringify(fixture.nodes),'source node names remain untranslated');
const translated=app.staticNodes.filter(node=>node.getAttribute('data-i18n-de')!==null);
assert.ok(translated.length>=60,'all static UI regions have explicit translation markers');
for(const node of translated)assert.equal(node.textContent,node.getAttribute('data-i18n-de'));
app.run("setViewerLanguage('en')");
assert.equal(app.ids.get('export-json').textContent,'JSON data + evidence');
assert.equal(app.ids.get('filter').getAttribute('placeholder'),'Search modules or files…');
assert.equal(app.ids.get('explorer-expand-all').getAttribute('title'),'Expand all packages in this report and clear search');
const before=app.run('JSON.stringify(exportSnapshot())');
app.run("setViewerLanguage('de')");
assert.equal(app.run('JSON.stringify(exportSnapshot())'),before,'language does not change export data, enums, or evidence');
app.run("showTab('dsm')");
app.click('export-svg');
assert.match(app.ids.get('feedback').textContent,/SVG-Export.*Baum.*Dienste/);
assert.equal(app.graph.downloads.length,0,'unavailable SVG feedback does not initiate a download');
app.click('export-json');
assert.match(app.ids.get('feedback').textContent,/archguard-dsm\.json.*exportiert/);
assert.equal(app.graph.downloads.length,1);
app.run("setViewerLanguage('en')");
app.click('export-svg');
assert.match(app.ids.get('feedback').textContent,/SVG export is available/);
const graphApp=launch({...fixture,nodes:[
  {id:0,name:'All',kind:'module',parent:null},{id:1,name:'All/External.java',kind:'file',parent:0},
  {id:2,name:'Both',kind:'module',parent:null},{id:3,name:'Both/Incoming.java',kind:'file',parent:2}
],edges:[{from:1,to:3,count:1,refs:[]},{from:3,to:1,count:1,refs:[]}]},{library:false});
graphApp.click('language-de');
assert.match(graphApp.ids.get('search-count').textContent,/\d+ von 4 Einträgen angezeigt/);
assert.match(graphApp.ids.get('graph-visibility').textContent,/Knoten angezeigt.*Abhängigkeiten.*ausgeblendet/);
const explorerRows=()=>graphApp.ids.get('module-list').querySelectorAll('button[data-node]');
const packageRow=explorerRows().find(node=>node.dataset.node==='0');
assert.match(packageRow.getAttribute('aria-label'),/^All · Paket/);
const disclosure=graphApp.ids.get('module-list').querySelectorAll('button[data-toggle]')[0];
assert.match(disclosure.getAttribute('aria-label'),/All.*(auf|zu)klappen/);
packageRow.click();
const modes=()=>graphApp.ids.get('edge-modes').querySelectorAll('button');
for(const [mode,label] of [['incoming','Eingehend'],['outgoing','Ausgehend'],['both','Beide'],['all','Alle']]){
  const control=modes().find(button=>button.dataset.edges===mode);
  assert.match(control.textContent,new RegExp('^'+label+' \\(\\d+\\)$'));
  control.click();
  assert.equal(graphApp.run('ui.edgeMode'),mode,'translated direction buttons retain stable state values');
}
assert.match(modes().find(button=>button.dataset.edges==='incoming').title,/Verbindungen von außerhalb nach All/);
graphApp.ids.get('filter').value='External';graphApp.ids.get('filter').dispatch('input');
graphApp.run('searchTimer()'); // The deterministic harness returns the debounced callback.
assert.equal(graphApp.ids.get('search-count').textContent,'1 Ergebnisse für „External“');
const fileRow=explorerRows().find(node=>node.dataset.node==='1');
assert.match(fileRow.getAttribute('aria-label'),/^All\/External\.java · Datei/);
assert.match(graphApp.ids.get('module-list').querySelectorAll('button[data-toggle]')[0].title,/Suche.*Zweige/);
graphApp.click('language-en');
assert.equal(graphApp.ids.get('search-count').textContent,'1 results for “External”');
assert.match(modes().find(button=>button.dataset.edges==='incoming').textContent,/^Incoming \(\d+\)$/);
assert.match(explorerRows().find(node=>node.dataset.node==='1').getAttribute('aria-label'),/^All\/External\.java · file/);
console.log('Static DE/EN UI: visible labels, accessible attributes, source names, reversible switching and export feedback passed');
