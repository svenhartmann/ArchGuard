const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'pkg',kind:'module',parent:null},{id:1,name:'pkg/A.ts',kind:'file',parent:0,lang:'ts'},{id:2,name:'pkg/B.ts',kind:'file',parent:0,lang:'ts'}];
const pack={node:'pkg',files:2,internal_dependencies:1,incoming_dependencies:0,outgoing_dependencies:1,internal_share:0.5,split_candidate:true,co_change_status:'not_checked',cross_group_co_changes:0,reason:'Review <img src=x> boundaries',groups:[{files:['pkg/A.ts'],internal_dependencies:1,incoming_dependencies:0,outgoing_dependencies:1,boundaries:[{from:'pkg/A.ts',to:'outside/B.ts',evidence:[{file:'pkg/A.ts',line:2,raw:'import b'}]}]}]};
const metric={status:'unverified',before:50,after:70,delta:20,reason:'producer revision unverified'};
const fixture={root:'/tmp/engineering',nodes,edges:[{from:1,to:2,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{
cohesion:{status:'partial',packages:[pack],diagnostics:['Observed graph only']},
quality_history:{status:'ready',records:[{path:'quality-history/a.json',timestamp:1700000000,git_commit:'abcdef',source_sha256:'abc',modules:1}],latest:{producer_revision:'unverified',modules:[{node:'pkg',files:2,coverage_files:1,coverage_status:'partial',semantic_status:'partial',declared_methods:5,measured_methods:3,findings_status:'stale',line_coverage:{covered:7,total:10},mean_complexity:3,max_complexity:4,active_findings:2}]},comparison:{modules:[{node:'pkg',status:'compared',line_coverage:metric,mean_complexity:{...metric,before:4,after:3,delta:-1},max_complexity:{...metric,before:5,after:4,delta:-1},active_findings:{...metric,before:3,after:2,delta:-1}}]}}, 
test_evidence:{status:'ready',tests:[{id:'t1',name:'changes A <svg>',suite:'suite',outcome:'passed',duration_seconds:2.5,mappings:[{path:'pkg/A.ts',node:'pkg/A.ts',lines:[1],status:'unchanged_since_import'}]},{id:'t2',name:'unknown mapping',suite:'suite',outcome:'passed',duration_seconds:null,mappings:[]}]},
mutations:{status:'ready',summary:{total:2,killed:1,survived:1,killed_percent:50},files:[{path:'pkg/A.ts',node:'pkg/A.ts',status:'matches_reported_source',mutants:[{id:'m1',status:'Survived',mutator_name:'Conditional',location:{start:{line:2,column:1},end:{line:2,column:4}},original:'a>b',replacement:'true'},{id:'m2',status:'Killed',mutator_name:'Conditional',location:{start:{line:3,column:1},end:{line:3,column:4}},original:'a<b',replacement:'false'}]}]},
contracts:{status:'ready',contracts:[{provider:'orders',mapped_service:true,source_path:'api.json',source_status:'unchanged_since_import',comparison_status:'compared',usage_status:'not_imported',operations:[{id:'GET /orders'}],usages:[],configured_callers:['shop'],changes:[{operation:'GET /orders',kind:'operation_removed',severity:'breaking',message:'removed',consumers:['declared-shop']}],diagnostics:[]}],limitations:['Configured is not observed']}
}};
fixture.insights.quality_history.comparison.modules.push({node:'old-package',status:'removed',line_coverage:{status:'removed',before:30,after:null,delta:null},mean_complexity:{status:'removed',before:3,after:null,delta:null},max_complexity:{status:'removed',before:4,after:null,delta:null},active_findings:{status:'removed',before:2,after:null,delta:null}});
const app=launch(fixture,{library:false});
assert.ok(app.ids.has('tab-quality'),'quality evidence needs a central workspace');
assert.equal(app.run('callModelPrepared'),false);
assert.equal(app.ids.get('engineering-list').children.length,0,'quality cards stay lazy at graph startup');
app.run("showTab('quality')");
assert.equal(app.run('ui.workspace'),'quality');
assert.match(app.ids.get('engineering-list').textContent,/pkg/);
assert.match(app.ids.get('engineering-list').textContent,/50/);
assert.equal(app.ids.get('engineering-list').querySelectorAll('img').length,0);
assert.equal(app.ids.get('engineering-list').querySelectorAll('button[data-engineering-node]').length,1,'closed group evidence must stay lazy');
app.ids.get('engineering-section').value='history';app.ids.get('engineering-section').dispatch('change');
assert.match(app.ids.get('engineering-list').textContent,/unverified/i);
assert.match(app.ids.get('engineering-list').textContent,/Coverage: incomplete report · 1 \/ 2 files/,'partial coverage denominator stays visible');
assert.match(app.ids.get('engineering-list').textContent,/3.*5/s,'measured/declared methods stay visible');
assert.match(app.ids.get('engineering-list').textContent,/old-package/,'removed modules remain visible in history');
app.ids.get('engineering-target').value='pkg/A.ts';app.ids.get('engineering-target').dispatch('change');
assert.match(app.ids.get('engineering-list').textContent,/pkg/,'file selection retains containing package history');
app.ids.get('engineering-target').value='';app.ids.get('engineering-target').dispatch('change');
app.ids.get('engineering-section').value='tests';app.ids.get('engineering-section').dispatch('change');
assert.match(app.ids.get('engineering-list').textContent,/2.5/);
app.ids.get('engineering-target').value='pkg/B.ts';app.ids.get('engineering-target').dispatch('change');
assert.match(app.ids.get('engineering-list').textContent,/changes A/,'test mapped to dependent A is a conservative suggestion for B');
assert.match(app.ids.get('engineering-note').textContent,/unknown|incomplete|omission/i);
app.ids.get('engineering-section').value='mutations';app.ids.get('engineering-section').dispatch('change');
app.ids.get('engineering-target').value='';app.ids.get('engineering-target').dispatch('change');
assert.match(app.ids.get('engineering-list').textContent,/Survived/);
assert.equal(app.run('callModelPrepared'),false,'quality does not decode Calls');
const snapshot=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(snapshot.kind,'quality');assert.equal(snapshot.mutations.files[0].mutants.length,2);
assert.match(app.run('exportMarkdown(exportSnapshot())'),/Survived/);
assert.doesNotMatch(app.run('exportMarkdown(exportSnapshot())'),/<svg/);
app.run("setViewerLanguage('de')");assert.equal(app.ids.get('engineering-heading').textContent,'Softwarequalität');
app.run("showTab('services')");assert.match(app.ids.get('contracts-list').textContent,/GET \/orders/);
assert.match(app.ids.get('contracts-list').textContent,/declared-shop/);
assert.match(app.ids.get('contracts-list').textContent,/shop/);
const ex=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));assert.equal(ex.contracts.contracts.length,1);
const absent=launch({...fixture,insights:{}},{library:false});absent.run("showTab('quality')");
absent.ids.get('engineering-section').value='history';absent.ids.get('engineering-section').dispatch('change');
assert.match(absent.ids.get('engineering-note').textContent,/quality-history.*--record/);
const many=launch({...fixture,insights:{...fixture.insights,cohesion:{status:'ready',packages:Array.from({length:45},(_,i)=>({...pack,node:'p'+i}))}}},{library:false});
many.run("showTab('quality')");assert.equal(many.ids.get('engineering-list').querySelectorAll('article').length,20);
many.click('engineering-more');assert.equal(many.ids.get('engineering-list').querySelectorAll('article').length,40);
assert.equal(JSON.parse(many.run('JSON.stringify(exportSnapshot())')).cohesion.packages.length,45);
const firstContract=launch({...fixture,insights:{contracts:{...fixture.insights.contracts,contracts:[{...fixture.insights.contracts.contracts[0],comparison_status:'no_baseline',changes:[]}]}}},{library:false});firstContract.run("showTab('services')");
assert.match(firstContract.ids.get('contracts-list').textContent,/GET \/orders/,'first import exposes endpoint inventory before a comparison exists');
const longReason='x'.repeat(9000);const huge=launch({...fixture,insights:{cohesion:{status:'ready',packages:[{...pack,reason:longReason}]}}},{library:false});huge.run("showTab('quality')");
assert.match(huge.ids.get('engineering-list').textContent,/shortened/i,'bounded text discloses truncation');
assert.equal(JSON.parse(huge.run('JSON.stringify(exportSnapshot())')).cohesion.packages[0].reason,longReason);
const incomplete=launch({...fixture,insights:{
  cohesion:{status:'partial',packages:[],diagnostics:['Unresolved source dependencies']},
  test_evidence:{status:'stale',report_path:'reports/junit.xml',mapping_path:'reports/test-map.json',imported_at_unix:1700000000,tests:[],diagnostics:['Mapped test source changed']},
  mutations:{status:'partial',report_path:'reports/mutation.json',producer:'Stryker',producer_version:'9.0',imported_at_unix:1700000001,files:[],summary:{killed_percent:null},diagnostics:['Pending mutants are unmeasured']}
}},{library:false});incomplete.run("showTab('quality')");
const incompleteMarkdown=incomplete.run('exportMarkdown(exportSnapshot())');
assert.match(incompleteMarkdown,/## Package cohesion\s+Status: partial/);
assert.match(incompleteMarkdown,/## Quality history\s+Status: not&#95;recorded/,'missing history cannot look like an empty successful measurement');
assert.match(incompleteMarkdown,/## Test evidence\s+Status: stale/);
assert.match(incompleteMarkdown,/## Mutation evidence\s+Status: partial/);
assert.match(incompleteMarkdown,/reports\/junit\.xml/);assert.match(incompleteMarkdown,/reports\/test-map\.json/);
assert.match(incompleteMarkdown,/reports\/mutation\.json/);assert.match(incompleteMarkdown,/Stryker.*9\.0/);
assert.match(incompleteMarkdown,/1700000001/);assert.match(incompleteMarkdown,/Mapped test source changed/);
const missingMarkdown=absent.run('exportMarkdown(exportSnapshot())');
assert.match(missingMarkdown,/## Test evidence\s+Status: not&#95;imported/);
assert.match(missingMarkdown,/## Mutation evidence\s+Status: not&#95;imported/);
const partialContract=launch({...fixture,insights:{contracts:{status:'stale',diagnostics:['Declared consumer lost its service mapping'],limitations:['Declared usage is not a trace'],contracts:[{
  ...fixture.insights.contracts.contracts[0],mapped_service:false,usage_status:'changed_since_import',usage_path:'api-usage.json',source_sha256:'source-hash',usage_sha256:'usage-hash',imported_at_unix:1700000002,
  usages:[{consumer:'old-shop',operation:'GET /orders',mapped_service:false}],diagnostics:['OpenAPI additionalOperations are not analyzed: /orders']
}]}}},{library:false});partialContract.run("showTab('services')");
const contractMarkdown=partialContract.run('exportMarkdown(exportSnapshot())');
assert.match(contractMarkdown,/Provider mapping: unmapped/);
assert.match(contractMarkdown,/Usage mapping: changed&#95;since&#95;import/);
assert.match(contractMarkdown,/api-usage\.json/);assert.match(contractMarkdown,/source-hash/);assert.match(contractMarkdown,/usage-hash/);
assert.match(contractMarkdown,/Configured callers \(operation usage unverified\): shop/);
assert.match(contractMarkdown,/old-shop.*GET \/orders.*unmapped/);
assert.match(contractMarkdown,/additionalOperations are not analyzed/);
assert.match(contractMarkdown,/Declared consumer lost its service mapping/);
const removedChildren=launch({...fixture,insights:{...fixture.insights,quality_history:{...fixture.insights.quality_history,comparison:{modules:[
  ...fixture.insights.quality_history.comparison.modules,{node:'pkg/removed-child',status:'removed'},{node:'pkg2/removed-child',status:'removed'}
]}}}},{library:false});removedChildren.run("showTab('quality');engineeringUi.section='history';engineeringUi.target='pkg';renderEngineeringQuality()");
assert.match(removedChildren.ids.get('engineering-list').textContent,/pkg\/removed-child/,'package history retains removed descendants');
assert.doesNotMatch(removedChildren.ids.get('engineering-list').textContent,/pkg2\/removed-child/,'history matching respects path segment boundaries');
removedChildren.run("engineeringUi.target='pkg/A.ts';renderEngineeringQuality()");
assert.doesNotMatch(removedChildren.ids.get('engineering-list').textContent,/removed-child/,'a file target does not include removed sibling packages');
const manyTargets=launch({...fixture,nodes:Array.from({length:2001},(_,id)=>({id,name:'pkg/File'+id+'.ts',kind:'file',parent:null,lang:'ts'})),edges:[],insights:{test_evidence:{status:'ready',tests:[{id:'late-test',name:'Late test',suite:'suite',outcome:'passed',duration_seconds:1,mappings:[{path:'pkg/File2000.ts',node:'pkg/File2000.ts',lines:[],status:'unchanged_since_import'}]}]}}},{library:false});
manyTargets.run("showTab('quality');engineeringUi.section='tests';renderEngineeringQuality()");
assert.ok(manyTargets.ids.has('engineering-target-query'),'target search reaches files beyond the bounded selector');
assert.match(manyTargets.ids.get('engineering-target-note').textContent,/2000.*2001/,'the initial target limit is visible');
const targetSearch=manyTargets.ids.get('engineering-target-query');targetSearch.value='File2000';targetSearch.dispatch('input');
assert.ok(manyTargets.ids.get('engineering-target').children.some(option=>option.value==='pkg/File2000.ts'));
assert.equal(manyTargets.ids.get('engineering-query').value,'','target search does not change the result filter');
manyTargets.ids.get('engineering-target').value='pkg/File2000.ts';manyTargets.ids.get('engineering-target').dispatch('change');
assert.match(manyTargets.ids.get('engineering-list').textContent,/Late test/);
targetSearch.value='File1';targetSearch.dispatch('input');
assert.equal(manyTargets.ids.get('engineering-target').value,'pkg/File2000.ts');
assert.ok(manyTargets.ids.get('engineering-target').children.some(option=>option.value==='pkg/File2000.ts'),'selected target remains available outside the search');
targetSearch.value='';targetSearch.dispatch('input');
assert.ok(manyTargets.ids.get('engineering-target').children.length<=2002,'target options remain bounded including global and selected choices');
manyTargets.run("setViewerLanguage('de')");assert.match(manyTargets.ids.get('engineering-target-note').textContent,/Ziele|Auswahl/);
console.log('Engineering quality workspace: lazy evidence, cohesion, honest history, test impact/cost, mutation outcomes, contract consumers, safe exports and DE/EN passed');
