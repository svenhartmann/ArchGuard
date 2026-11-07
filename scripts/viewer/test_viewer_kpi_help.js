const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=launch({root:'/example',nodes:[{id:0,name:'Example.java',parent:null,kind:'file',lang:'java'}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
assert.equal(app.run('typeof openKpiHelp'),'function','the viewer must provide a language-specific KPI help page');
const before=app.run('JSON.stringify([data,work,ops,ui.selection,ui.workspace])');
app.run('kpiHelpUi.action.click()');
assert.equal(app.run('kpiHelpUi.dialog.open'),true);
assert.equal(app.run('kpiHelpUi.language'),'java','start with a language present in the report');
const languages=JSON.parse(app.run('JSON.stringify(kpiLanguageHelp().map(row=>row.id))'));
assert.deepEqual([...languages].sort(),['csharp','go','java','js','kt','php','py','rs','ts']);
for(const id of languages){
 app.run(`kpiHelpUi.language='${id}';renderKpiHelp()`);
 const text=app.run('kpiHelpUi.detail.textContent');
 assert.match(text,/archguard coverage \. --report/,'every supported language has an import example');
 if(id==='rs')assert.match(text,/abstractness and implemented traits.*rust-analyzer.*ARCHGUARD_TRUST_PROJECT_COMPILER=1.*CBO and RFC stay partial.*DIT and NOC are not applicable/is,'Rust names syntax types, the opt-in resolution and what stays partial');
 else if(['kt','php','py'].includes(id))assert.match(text,/call targets are resolved statically.*CBO, RFC, DIT and NOC stay partial/is,'statically resolved languages state what stays partial');
 else assert.match(text,/archguard calls \. --format json/);
}
assert.doesNotMatch(app.run('kpiHelpUi.dialog.textContent'),/Kotlin[^.]*not currently supported/,'Kotlin is a supported language');
app.run("kpiHelpUi.language='js';renderKpiHelp()");assert.match(app.run('kpiHelpUi.detail.textContent'),/allowJs/);
app.run("setViewerLanguage('de')");
assert.equal(app.run('kpiHelpUi.language'),'js','UI translation retains the selected source language');
assert.match(app.run('kpiHelpUi.dialog.textContent'),/Drittanbieter|Werkzeuge/);
assert.match(app.run('kpiHelpUi.detail.textContent'),/allowJs/);
assert.equal(app.run('kpiHelpUi.action.textContent'),'KPI-Hilfe');
for(const link of app.run('kpiHelpUi.dialog.querySelectorAll("a")')){
 assert.match(link.getAttribute('href'),/^https:\/\//);assert.equal(link.getAttribute('rel'),'noopener noreferrer');
}
app.run('closeKpiHelp()');assert.equal(app.run('kpiHelpUi.dialog.open'),false);
assert.equal(app.run('JSON.stringify([data,work,ops,ui.selection,ui.workspace])'),before,'reading help never changes analysis or navigation');
console.log('KPI help: all supported languages, accurate availability, examples, DE/EN, safe references and unchanged analysis passed');
