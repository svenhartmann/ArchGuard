const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const model={schema_version:1,name:'Product responsibilities',status:'partial',severity:'error',require_assignment:true,source_nodes:4,mapped_nodes:2,total_edges:3,evaluated_edges:2,unclassified_edges:1,violating_dependencies:1,unmapped_nodes:1,ambiguous_nodes:1,
  components:[{name:'Web',include:['ui/**'],nodes:['ui/A.ts'],node_count:1},{name:'Domain',include:['domain/**'],nodes:['domain/B.ts'],node_count:1},{name:'Storage',include:['db/**'],nodes:[],node_count:0}],
  allowed_dependencies:[{from:'Web',to:'Domain'},{from:'Domain',to:'Storage'}],
  dependencies:[{from:'Web',to:'Domain',allowed:true,source_edges:1,import_references:3,count_exact:true,evidence:[{file:'ui/A.ts',line:2,raw:'import B;'}],evidence_total:3},{from:'Domain',to:'Web',allowed:false,source_edges:1,import_references:1,count_exact:true,evidence:[{file:'domain/B.ts',line:7,raw:'<img src=x onerror=alert(1)>'}],evidence_total:1}],
  unmapped:['other/C.ts'],ambiguous:[{node:'shared/D.ts',components:['Web','Domain']}],empty_components:['Storage']};
const fixture={root:'/repo',nodes:['ui/A.ts','domain/B.ts','other/C.ts','shared/D.ts'].map((name,id)=>({id,name,parent:null,kind:'file',lang:'ts'})),edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{architecture:{observed:{target_model:model}}}};
const app=launch(fixture);
const mixedSemantic={status:'partial',analyzers:[],projected_calls:1,projected_supertypes:0,skipped_project_calls:0,diagnostics:['Missing project reference']};
const mixed=launch({...fixture,semantic:mixedSemantic});mixed.run("showTab('architecture')");
assert.match(mixed.ids.get('architecture-content').textContent,/dependency evidence records/);
assert.doesNotMatch(mixed.ids.get('architecture-content').textContent,/source-import observations/);
const mixedExport=JSON.parse(mixed.run('JSON.stringify(exportSnapshot())'));
assert.equal(mixedExport.count_unit,'dependency evidence records');assert.deepEqual(mixedExport.semantic,mixedSemantic);
assert.equal(app.run('typeof renderArchitecture'),'function','target architecture needs its own navigable workspace');
app.run("showTab('architecture')");
assert.equal(app.run('ui.workspace'),'architecture');
const content=app.ids.get('architecture-content');
assert.match(content.textContent,/Product responsibilities/);
assert.match(content.textContent,/2 \/ 4/);
assert.match(content.textContent,/1 forbidden/);
assert.match(content.textContent,/partial/i);
const rows=()=>content.querySelectorAll('button[data-architecture-edge]');
assert.equal(rows().length,3,'include allowed but unobserved directions as target policy');
const unused=rows().find(row=>row.textContent.includes('Domain → Storage'));
assert.match(unused.textContent,/Allowed.*not observed/);
rows().find(row=>row.textContent.includes('Domain → Web')).click();
assert.match(content.textContent,/domain\/B.ts:7/);assert.match(content.textContent,/<img src=x onerror=alert\(1\)>/);
assert.equal(content.querySelectorAll('img').length,0,'evidence is rendered as text');
assert.match(content.textContent,/other\/C.ts/);assert.match(content.textContent,/shared\/D.ts.*Web.*Domain/);
const snapshot=app.run('JSON.stringify([data,work,ops])');
app.run("architectureUi.onlyViolations=true;renderArchitecture()");assert.equal(rows().length,1);
app.click('language-de');assert.match(content.textContent,/Nicht erlaubt/);assert.match(content.textContent,/Zuordnung/);
assert.equal(app.run('JSON.stringify([data,work,ops])'),snapshot,'target policy is not changed by filters or locale');
const exported=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(exported.kind,'architecture');assert.equal(exported.edges.length,1);assert.equal(exported.edges[0].allowed,false);
assert.deepEqual(exported.model.allowed_dependencies,model.allowed_dependencies,'full model retained with filtered export');
assert.match(app.run('exportMarkdown(exportSnapshot())'),/Target architecture|Declared target/);
app.run("ops=[{cut_edge:{from:'ui/A.ts',to:'domain/B.ts'}}];renderArchitecture()");
assert.match(content.textContent,/ursprünglichen|Original/i,'target results must not be claimed as simulated compliance');
assert.equal(app.run('architectureModel().violating_dependencies'),1);
const empty=launch({...fixture,insights:{}});empty.run("showTab('architecture')");assert.match(empty.ids.get('architecture-content').textContent,/No arc42 documentation/);
const sampled=launch({...fixture,insights:{architecture:{observed:{target_model:{...model,limitations:['Compiler-only dependencies are outside this import model.'],dependencies:[{...model.dependencies[1],import_references:25,evidence_total:1}]}}}}});
sampled.run("showTab('architecture')");sampled.ids.get('architecture-content').querySelectorAll('button[data-architecture-edge]')[0].click();
assert.match(sampled.ids.get('architecture-content').textContent,/Stored evidence is a sample/,'retained source samples are not complete just because every retained item is shown');
assert.match(sampled.ids.get('architecture-content').querySelectorAll('button[data-action-help]').map(n=>n.getAttribute('data-metric-description')).join(' '),/Compiler-only dependencies are outside this import model/);
assert.ok(JSON.parse(sampled.run('JSON.stringify(exportSnapshot())')).notes.includes('Compiler-only dependencies are outside this import model.'));
const many=launch({...fixture,insights:{architecture:{observed:{target_model:{...model,dependencies:Array.from({length:250},(_,i)=>({...model.dependencies[1],from:'Module '+i})),allowed_dependencies:[]}}}}});
many.run("showTab('architecture')");assert.equal(many.ids.get('architecture-content').querySelectorAll('button[data-architecture-edge]').length,100,'large policies have bounded DOM rows');
many.ids.get('architecture-content').querySelectorAll('button').find(b=>b.textContent==='Show more relationships').click();
assert.equal(many.ids.get('architecture-content').querySelectorAll('button[data-architecture-edge]').length,200);

// Original mapping links must retain source identity when simulations reuse a path.
const navigationNodes=[{id:0,name:'a',kind:'module',parent:null},{id:1,name:'b',kind:'module',parent:null},{id:2,name:'holding',kind:'module',parent:null},{id:3,name:'a/A.ts',kind:'file',parent:0},{id:4,name:'b/A.ts',kind:'file',parent:1}];
const navigation=launch({...fixture,nodes:navigationNodes,insights:{architecture:{observed:{target_model:{...model,components:[{name:'Original A',include:['a/**'],nodes:['a/A.ts'],node_count:1}]}}}}});
navigation.run("ui.simulation=true;recordOp(encodeMove('a/A.ts','holding'));recordOp(encodeMove('b/A.ts','a'));showTab('architecture')");
const originalComponent=navigation.ids.get('architecture-content').querySelectorAll('details[data-architecture-component]')[0];originalComponent.open=true;originalComponent.dispatch('toggle');
const originalLink=originalComponent.querySelectorAll('button').find(button=>button.textContent.startsWith('a/A.ts'));
originalLink.click();
assert.equal(navigation.run('ui.selection.name'),'holding/A.ts','original mapping opens the same source node, not a different file at its old path');
assert.equal(navigation.run('nodeById(view,selectedId).src'),3);
assert.match(originalLink.textContent,/simulated.*holding\/A\.ts/i,'mapping label discloses the current simulated name');

// Every mapping remains reachable without rendering an unbounded initial list.
const components=Array.from({length:101},(_,i)=>({name:'Component '+i,include:['module'+i+'/**'],nodes:['module'+i+'/file.ts'],node_count:1}));
const ambiguous=Array.from({length:51},(_,i)=>({node:'shared/file'+i+'.ts',components:['Component 0','Component 100']}));
const paged=launch({...fixture,insights:{architecture:{observed:{target_model:{...model,components,ambiguous,ambiguous_nodes:51}}}}});
paged.run("showTab('architecture')");const pagedContent=paged.ids.get('architecture-content');
assert.equal(pagedContent.querySelectorAll('details[data-architecture-component]').length,100);
const moreComponents=pagedContent.querySelectorAll('button').find(button=>button.textContent==='Show more components');assert.ok(moreComponents,'components after the first page must remain reachable');moreComponents.click();
assert.equal(pagedContent.querySelectorAll('details[data-architecture-component]').length,101);assert.match(pagedContent.textContent,/Component 100/);
assert.doesNotMatch(pagedContent.textContent,/shared\/file50\.ts/);
const moreAmbiguous=pagedContent.querySelectorAll('button').find(button=>button.textContent==='Show more ambiguous files');assert.ok(moreAmbiguous,'ambiguous assignments after the first page must remain reachable');moreAmbiguous.click();
assert.match(pagedContent.textContent,/shared\/file50\.ts.*Component 0.*Component 100/);

// Markdown includes complete policy and mapping diagnoses independently of the relationship filter.
const policyExport=launch(fixture);policyExport.run("showTab('architecture');architectureUi.onlyViolations=true");
const policyMarkdown=policyExport.run('exportMarkdown(exportSnapshot())');
assert.match(policyMarkdown,/## Target model/);assert.match(policyMarkdown,/require assignment.*true/i);
assert.match(policyMarkdown,/ui\/&#42;&#42;/,'declared mapping patterns are safely exported');
assert.match(policyMarkdown,/other\/C\.ts/,'unmapped files explain partial coverage');
assert.match(policyMarkdown,/shared\/D\.ts.*Web.*Domain/,'ambiguous files retain all candidate components');
assert.match(policyMarkdown,/Components without assigned files: Storage/);
assert.match(policyMarkdown,/\| Domain \| Storage \|/,'unused declared permissions survive relationship filters');
assert.match(policyMarkdown,/complete model.*relationship filters/i);
const filteredRelationships=policyMarkdown.split('## Relationships')[1].split('## Source evidence')[0];
assert.match(filteredRelationships,/Domain.*Web.*forbidden/);assert.doesNotMatch(filteredRelationships,/Web.*Domain.*allowed/);
const contractsModel={...model,violating_interfaces:1,coupling_violations:1,
  components:model.components.map(c=>c.name==='Domain'?{...c,public:['domain/api.ts'],public_nodes:['domain/api.ts'],incoming_components:['Web'],outgoing_components:['Storage'],max_incoming_components:1,max_outgoing_components:0}:c),
  dependencies:model.dependencies.map(d=>d.from==='Web'?{...d,private_accesses:[{from:'ui/A.ts',to:'domain/private.ts',evidence:[{file:'ui/A.ts',line:9,raw:'import "<private>"'}]}]}:d)};
const contractsApp=launch({...fixture,insights:{architecture:{observed:{target_model:contractsModel}}}});
contractsApp.run("showTab('architecture');architectureUi.onlyViolations=true;renderArchitecture()");
const contractsContent=contractsApp.ids.get('architecture-content');
assert.match(contractsContent.textContent,/1 interface violation/);
assert.match(contractsContent.textContent,/1 coupling limit violation/);
assert.equal(contractsContent.querySelectorAll('button[data-architecture-edge]').length,2,'allowed directions with private access must stay in the violations filter');
contractsContent.querySelectorAll('button[data-architecture-edge]').find(b=>b.textContent.includes('Web → Domain')).click();
assert.match(contractsContent.textContent,/Private access/);
assert.match(contractsContent.textContent,/domain\/private\.ts/);
assert.match(contractsContent.textContent,/ui\/A\.ts:9/);
assert.match(contractsContent.textContent,/Outgoing components: 1.*limit: 0/);
const contractsMarkdown=contractsApp.run('exportMarkdown(exportSnapshot())');
assert.match(contractsMarkdown,/domain\/api\.ts/);
assert.match(contractsMarkdown,/domain\/private\.ts/);
assert.match(contractsMarkdown,/Outgoing.*limit/);
contractsApp.click('language-de');assert.match(contractsContent.textContent,/Privater Zugriff/);
console.log('Target architecture UI: policy, interface access, coupling budgets, coverage, evidence, bounded lists, filters, DE/EN and export passed');
