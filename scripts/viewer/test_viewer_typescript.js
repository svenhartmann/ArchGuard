// Compiler module owners expose TS functions without pretending they are classes.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const owner=(name,kind='module')=>({id:'typescript:'+name,name,package:'src',file:'src/'+name+'.ts',graph_node:'src/'+name+'.ts',line:1,kind,language:'typescript'});
const classes=[owner('a'),owner('b'),owner('Store','class')];
const methods=classes.map((c,index)=>({id:c.id+'#run()',class_id:c.id,name:'run',signature:'run()',file:c.file,line:2,end_line:4,is_static:c.kind==='module',complexity:{cyclomatic:index+1,max_nesting:index,definition:'ast-decisions-v1'}}));
const call=(from,to)=>({from_class:methods[from].class_id,from_method:methods[from].id,to_class:methods[to].class_id,to_method:methods[to].id,target_label:'run',file:methods[from].file,line:3,column:1,code:'run();',dispatch:classes[to].kind==='module'?'static':'instance',resolution:'project'});
const fixture={root:'/tmp/ts-view',nodes:classes.map((c,id)=>({id,name:c.graph_node,kind:'file',parent:null,lang:'ts'})),edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},calls:{status:'ready',files:3,classes,methods,calls:[call(0,1),call(1,0),call(0,2),call(2,0)],analyzers:[{language:'typescript',backend:'typescript',status:'ready',files:3,version:'5.8.2',project:'tsconfig.json',diagnostics:[]}],diagnostics:[]}};
const app=launch(fixture,{library:false});
assert.equal(app.run('callModelPrepared'),false);
app.run("showTab('calls')");
for(const pair of [['typescript:a','typescript:b'],['typescript:a','typescript:Store']]){
  const suggestions=JSON.parse(app.run('JSON.stringify(callSuggestions(callRelations().find(r=>r.from==='+JSON.stringify(pair[0])+'&&r.to==='+JSON.stringify(pair[1])+')))'));
  assert.ok(suggestions.every(s=>!/interface|factory/i.test(s.title)),'synthetic module owners must not get class-specific injection/factory prescriptions');
}
assert.match(app.ids.get('calls-diagnostics').textContent,/modules/);
app.click('calls-tree-expand');
assert.equal(app.ids.get('calls-tree').querySelectorAll('small[data-call-owner-kind]').filter(badge=>badge.getAttribute('data-call-owner-kind')==='module').length,2);
app.click('calls-show-rankings');
assert.match(app.ids.get('calls-ranking-coverage').textContent,/module owners.*top-level functions/i);
app.click('calls-show-cycles');
assert.match(app.ids.get('calls-cycles-status').textContent,/modules/);
assert.match(app.ids.get('calls-cycles-coverage').textContent,/module/);
assert.equal(app.ids.get('calls-cycles-list').querySelectorAll('button[data-call-cycle]').length,1);
assert.match(app.run('methodComplexityDescription()'),/TypeScript.*optional.*logical assignment/i);
app.run("setViewerLanguage('de')");
assert.match(app.ids.get('calls-diagnostics').textContent,/Module/);
assert.match(app.ids.get('calls-cycles-status').textContent,/Module/);
assert.match(app.run('methodComplexityDescription()'),/TypeScript.*optionale.*logische Zuweisungen/i);
assert.equal(JSON.parse(app.run('JSON.stringify(exportSnapshot())')).selected_cycle.members.length,3);
console.log('TypeScript Calls UI: module identity, cycles, rankings, labels, metric explanations, DE/EN and no class-only prescriptions passed');
