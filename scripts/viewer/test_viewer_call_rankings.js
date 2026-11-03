// Ranking counts and navigation use the actual Calls workspace without a browser.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const cls=(id,pkg)=>({id,name:id,package:pkg,file:id+'.java',graph_node:id+'.java',line:1,kind:'class',language:'java'});
const classes=[cls('a.A','a'),cls('b.B','b'),cls('c.C','c')];
const method=(owner,signature)=>({id:owner+'#'+signature,class_id:owner,name:signature.split('(')[0],signature,file:owner+'.java',line:2,is_static:false});
const methods=[method('a.A','run()'),method('a.A','sink()'),method('b.B','serve()'),method('b.B','serve(java.lang.String)'),method('c.C','run()')];
const call=(from,to,line,extra={})=>({from_class:from?.class_id||'a.A',from_method:from?.id||null,to_class:to?.class_id||null,to_method:to?.id||null,target_label:to?.id||'missing()',file:(from?.class_id||'a.A')+'.java',line,column:1,code:'call '+line,dispatch:'instance',resolution:to?'project':'unresolved',...extra});
const calls=[call(methods[0],methods[2],1),call(methods[0],methods[2],1,{column:20}),call(methods[0],methods[3],2),call(methods[2],methods[1],3),call(methods[0],methods[1],4),call(null,methods[2],5),call(methods[4],methods[2],6),call(methods[0],null,7,{resolution:'external',to_class:'SDK',to_method:'SDK#send()'}),call(methods[0],null,8)];
const fixture={root:'/tmp/call-ranks',nodes:classes.map((c,id)=>({id,name:c.graph_node,kind:'file',parent:null,lang:'java'})),edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},calls:{status:'partial',files:3,classes,methods,calls,diagnostics:['Missing <img> dependency']}};
const app=launch(fixture,{library:false});
assert.ok(app.ids.has('calls-show-rankings'),'Calls needs a discoverable Most calls view');
app.run('var rankComputations=0,originalRankings=computeCallRankings;computeCallRankings=function(...args){rankComputations++;return originalRankings(...args);};');
assert.equal(app.run('callModelPrepared'),false,'graph startup must keep optional calls lazy');
app.run("showTab('calls')");assert.equal(app.run('rankComputations'),0,'ordinary connections do not compute hidden rankings');
assert.equal(app.run('callUi.resolution'),'project','a report with project resolutions keeps the project-focused default');

// A report without any resolved target (e.g. Java without a classpath) must still
// list its recorded call sites instead of opening on an empty project-only filter.
const noResolution=launch({...fixture,calls:{status:'partial',files:3,classes,methods,
  calls:fixture.calls.calls.map(c=>({...c,resolution:'unresolved',to_class:null,to_method:null,target_label:c.target_label||'missing()'})),
  diagnostics:['Compiler error limit reached; all call targets remain unresolved.']}},{library:false});
noResolution.run("showTab('calls')");
assert.equal(noResolution.run('callUi.resolution'),'all','without project resolutions the target filter widens to all');
assert.equal(noResolution.ids.get('calls-resolution').value,'all');
assert.ok(noResolution.ids.get('calls-relations').querySelectorAll('button[data-relation]').length>0,
  'unresolved call sites must be listed when no project resolution exists');
app.click('calls-show-rankings');
assert.equal(app.ids.get('calls-ranking').hidden,false);assert.equal(app.ids.get('calls-relations').hidden,true);
assert.equal(app.run('rankComputations'),1);
const rows=()=>app.ids.get('calls-ranking-list').querySelectorAll('button[data-call-rank]');
const id=row=>JSON.parse(row.getAttribute('data-call-rank')).id;
assert.equal(id(rows()[0]),'b.B');assert.equal(rows()[0].getAttribute('data-rank-incoming'),'4');
assert.match(app.ids.get('calls-ranking-status').textContent,/static call sites.*not runtime/i);
assert.match(app.ids.get('calls-ranking-coverage').textContent,/partial|incomplete/i);
const choose=(name,value)=>{app.ids.get(name).value=value;app.ids.get(name).dispatch('change');};
choose('calls-package','*');assert.equal(app.ids.get('calls-package').value,'*','All packages is a real scope');
assert.equal(rows()[0].getAttribute('data-rank-incoming'),'5');
choose('calls-ranking-kind','method');
assert.equal(id(rows()[0]),methods[2].id);assert.equal(rows()[0].getAttribute('data-rank-incoming'),'4');
assert.ok(rows().some(row=>id(row)===methods[3].id),'overloads remain separate');
const rankingBefore=rows().map(row=>[id(row),row.getAttribute('data-rank-incoming')]);
const computationsBefore=app.run('rankComputations');
rows().find(row=>id(row)===methods[3].id).click();
assert.equal(app.run('callUi.element.id'),methods[3].id);
assert.match(app.ids.get('calls-evidence').textContent,/serve\(java.lang.String\)/);
assert.doesNotMatch(app.ids.get('calls-evidence').textContent,/call 1/);
assert.deepEqual(rows().map(row=>[id(row),row.getAttribute('data-rank-incoming')]),rankingBefore,'selection keeps the full ranking available');
assert.equal(app.run('rankComputations'),computationsBefore,'selecting a row reuses scoped aggregation');
assert.ok(app.ids.get('calls-tree').querySelectorAll('button[data-call-element]').some(b=>JSON.parse(b.getAttribute('data-call-element')).id===methods[3].id),'ranked method and its package ancestors open in the tree');
assert.equal(app.run('exportSnapshot().filters.element.id'),methods[3].id,'existing evidence exports retain the exact selected overload');
const graphState=app.run('JSON.stringify([work,view,ops])');
app.run("setViewerLanguage('de')");
assert.equal(app.run('rankComputations'),computationsBefore,'translation does not recompute numeric rankings');
assert.match(app.ids.get('calls-ranking-status').textContent,/Aufrufstellen.*Laufzeit/);
assert.equal(app.run('JSON.stringify([work,view,ops])'),graphState);
app.click('calls-tree-clear');choose('calls-ranking-sort','outgoing');choose('calls-ranking-kind','class');
assert.equal(id(rows()[0]),'a.A');assert.equal(rows()[0].getAttribute('data-rank-outgoing'),'4');
app.click('calls-internal');assert.equal(rows()[0].getAttribute('data-rank-outgoing'),'5','internal calls obey the existing filter');
choose('calls-resolution','all');assert.equal(rows()[0].getAttribute('data-rank-outgoing'),'7');
assert.ok(rows().every(row=>id(row)!=='SDK'),'external and unresolved identities are not invented project classes');
assert.match(app.ids.get('calls-ranking-coverage').textContent,/unaufgelöst/);
app.ids.get('calls-filter').value='serve(java.lang.String)';app.ids.get('calls-filter').dispatch('input');
assert.equal(rows()[0].getAttribute('data-rank-outgoing'),'1','text search affects the ranking');
assert.equal(app.ids.get('calls-ranking-list').querySelectorAll('img').length,0);
const manyClasses=[classes[0],...Array.from({length:35},(_,i)=>cls('a.Target'+i,'a'))],manyMethods=manyClasses.map(c=>method(c.id,'run()'));
const many=launch({...fixture,calls:{...fixture.calls,classes:manyClasses,methods:manyMethods,calls:manyMethods.slice(1).map((m,i)=>call(manyMethods[0],m,i+1))}},{library:false});
many.run("showTab('calls')");many.click('calls-show-rankings');
assert.equal(many.ids.get('calls-ranking-list').querySelectorAll('button[data-call-rank]').length,20);
many.click('calls-ranking-more');assert.equal(many.ids.get('calls-ranking-list').querySelectorAll('button[data-call-rank]').length,35);
const absent=launch({...fixture,calls:{status:'unavailable',files:0,classes:[],methods:[],calls:[],diagnostics:['Compiler unavailable']}},{library:false});
absent.run("showTab('calls')");absent.click('calls-show-rankings');
assert.equal(absent.ids.get('calls-ranking-list').querySelectorAll('button[data-call-rank]').length,0);
assert.match(absent.ids.get('calls-ranking-coverage').textContent,/unavailable/i);

const rankedApp=(types,members,sites)=>{
  const instance=launch({...fixture,nodes:types.map((c,index)=>({id:index,name:c.graph_node,kind:'file',parent:null,lang:c.language})),calls:{status:'ready',files:types.length,classes:types,methods:members,calls:sites,diagnostics:[]}},{library:false});
  instance.run("showTab('calls')");instance.click('calls-show-rankings');return instance;
};
const rankedRows=instance=>Array.from(instance.ids.get('calls-ranking-list').querySelectorAll('button[data-call-rank]'));
const selectValue=(instance,name,value)=>{instance.ids.get(name).value=value;instance.ids.get(name).dispatch('change');};

// Internal calls belong to both directions, but only once to involved sites.
const recursive=rankedApp(classes,methods,[call(methods[0],methods[0],1),call(methods[0],methods[1],2),call(methods[2],methods[0],3)]);
selectValue(recursive,'calls-package','*');selectValue(recursive,'calls-ranking-sort','total');
let recursiveRow=rankedRows(recursive).find(row=>id(row)==='a.A');
assert.equal(recursiveRow.getAttribute('data-rank-total'),'1','the default internal-call filter excludes both recursive and other same-class calls');
recursive.click('calls-internal');
assert.equal(id(rankedRows(recursive)[0]),'a.A','total ordering uses involved call sites');
recursiveRow=rankedRows(recursive)[0];
assert.equal(recursiveRow.getAttribute('data-rank-incoming'),'3');
assert.equal(recursiveRow.getAttribute('data-rank-outgoing'),'2');
assert.equal(recursiveRow.getAttribute('data-rank-total'),'3','each internal record contributes once to the class total');
selectValue(recursive,'calls-ranking-kind','method');
assert.equal(id(rankedRows(recursive)[0]),methods[0].id);
recursiveRow=rankedRows(recursive)[0];
assert.equal(recursiveRow.getAttribute('data-rank-incoming'),'2');
assert.equal(recursiveRow.getAttribute('data-rank-outgoing'),'2');
assert.equal(recursiveRow.getAttribute('data-rank-total'),'3','a recursive method call contributes once, not once per direction');
recursiveRow.click();assert.match(recursive.ids.get('calls-evidence').textContent,/run\(\).*run\(\)/);

// Identical display names never merge compiler identities across languages.
const javaTypes=[cls('java:Shared.Source','shared'),cls('java:Shared.Target','shared')].map((c,index)=>({...c,name:'Shared.'+(index?'Target':'Source')}));
const csharpTypes=javaTypes.map((c,index)=>({...c,id:'csharp:Assembly::'+c.name,language:'csharp',file:(index?'Target':'Source')+'.cs',graph_node:(index?'Target':'Source')+'.cs'}));
const mixedTypes=[...javaTypes,...csharpTypes],mixedMethods=mixedTypes.map(c=>method(c.id,'run()'));
const mixed=rankedApp(mixedTypes,mixedMethods,[call(mixedMethods[0],mixedMethods[1],1),call(mixedMethods[2],mixedMethods[3],2),call(mixedMethods[2],mixedMethods[3],3)]);
selectValue(mixed,'calls-package','*');
assert.deepEqual(rankedRows(mixed).map(id),[csharpTypes[1].id,javaTypes[1].id]);
assert.deepEqual(rankedRows(mixed).map(row=>row.getAttribute('data-rank-incoming')),['2','1']);
selectValue(mixed,'calls-ranking-kind','method');
assert.deepEqual(rankedRows(mixed).map(id),[mixedMethods[3].id,mixedMethods[1].id]);
selectValue(mixed,'calls-language','csharp');
assert.equal(mixed.ids.get('calls-package').value,'*');
assert.deepEqual(rankedRows(mixed).map(id),[mixedMethods[3].id]);
assert.equal(rankedRows(mixed)[0].getAttribute('data-rank-incoming'),'2');
selectValue(mixed,'calls-language','all');
assert.deepEqual(rankedRows(mixed).map(id),[mixedMethods[3].id,mixedMethods[1].id]);
selectValue(mixed,'calls-language','java');
assert.deepEqual(rankedRows(mixed).map(id),[mixedMethods[1].id]);
assert.equal(rankedRows(mixed)[0].getAttribute('data-rank-incoming'),'1');

// All-packages and the valid empty Java package are distinct scopes.
const plainTypes=[cls('PlainSource',''),cls('PlainTarget',''),cls('z.Source','z'),cls('z.Target','z')];
const plainMethods=plainTypes.map(c=>method(c.id,'run()'));
const scope=rankedApp(plainTypes,plainMethods,[call(plainMethods[0],plainMethods[1],1),call(plainMethods[2],plainMethods[3],2)]);
selectValue(scope,'calls-package','*');assert.equal(rankedRows(scope).length,2);
selectValue(scope,'calls-package','');assert.equal(scope.ids.get('calls-package').value,'');
assert.deepEqual(rankedRows(scope).map(id),['PlainTarget']);
selectValue(scope,'calls-package','*');scope.run("openCallsForNode('z.Source.java')");
assert.equal(scope.ids.get('calls-package').value,'z','Inspect calls leaves All packages for the selected source package');
assert.equal(scope.ids.get('calls-filter').value,'z.Source');
assert.deepEqual(rankedRows(scope).map(id),['z.Target']);
selectValue(scope,'calls-package','*');scope.run("openCallsForNode('PlainSource.java')");
assert.equal(scope.ids.get('calls-package').value,'','Inspect calls preserves the empty default package');
assert.equal(scope.ids.get('calls-filter').value,'PlainSource');
assert.deepEqual(rankedRows(scope).map(id),['PlainTarget']);

// Ranked labels themselves are untrusted source strings, not just diagnostics.
const unsafeType={...classes[1],name:'b.<img src=x onerror=alert(1)>'};
const unsafeMethod={...methods[2],signature:'serve(<img src=x onerror=alert(2)>)'};
const unsafe=rankedApp([classes[0],unsafeType],[methods[0],unsafeMethod],[call(methods[0],unsafeMethod,1)]);
assert.ok(rankedRows(unsafe)[0].textContent.includes(unsafeType.name));
assert.equal(unsafe.ids.get('calls-ranking-list').querySelectorAll('img').length,0);
selectValue(unsafe,'calls-ranking-kind','method');
assert.ok(rankedRows(unsafe)[0].textContent.includes(unsafeType.name+'#'+unsafeMethod.signature));
assert.equal(unsafe.ids.get('calls-ranking-list').querySelectorAll('img').length,0);
rankedRows(unsafe)[0].click();assert.ok(unsafe.ids.get('calls-evidence').textContent.includes(unsafeMethod.signature));
assert.equal(unsafe.ids.get('calls-evidence').querySelectorAll('img').length,0);

// A known target type may have no declared method (e.g. an implicit constructor).
const implicit={...methods[2],id:'b.B#implicit()',signature:'implicit()'};
const missingTarget=rankedApp(classes,[methods[0]],[call(methods[0],implicit,1)]);
assert.deepEqual(rankedRows(missingTarget).map(id),['b.B']);
assert.equal(rankedRows(missingTarget)[0].getAttribute('data-rank-incoming'),'1');
assert.match(missingTarget.ids.get('calls-ranking-coverage').textContent,/1 project target sites without a declared method/);
selectValue(missingTarget,'calls-ranking-kind','method');
assert.equal(rankedRows(missingTarget).length,0,'missing method declarations must not turn into ranked method identities');
selectValue(missingTarget,'calls-ranking-sort','outgoing');
assert.deepEqual(rankedRows(missingTarget).map(id),[methods[0].id]);
assert.equal(rankedRows(missingTarget)[0].getAttribute('data-rank-outgoing'),'1','missing target method declarations retain the recorded caller');
missingTarget.run("setViewerLanguage('de')");
assert.match(missingTarget.ids.get('calls-ranking-coverage').textContent,/1 Projekt-Ziel-Aufrufstellen ohne deklarierte Methode/);
console.log('Call rankings UI: lazy cached counts, recursion, languages, package scopes, safe labels, missing targets, overload evidence, paging, coverage and DE/EN passed');
