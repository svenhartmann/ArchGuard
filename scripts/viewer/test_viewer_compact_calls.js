// The HTML transport may pool long symbols; the source model/export stays exact.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const strings=['C# <type>','src/A.cs','P','p.A#Run()','Run()','Run','csharp','class','public','p.A','csharp:external:Assembly::External.B','External.B.Save()','static','external','x.Save("</script>ä");'];
const compact={status:'partial',java_release:0,files:1,diagnostics:['diagnostics stay literal'],analyzers:[],classes:[{id:9,name:0,package:2,file:1,graph_node:1,line:1,kind:7,language:6,visibility:8,is_abstract:false,supertypes:[10]}],methods:[{id:3,class_id:9,name:5,signature:4,file:1,line:2,is_static:false}],calls:[{from_class:9,from_method:3,to_class:10,to_method:null,target_label:11,dispatch:12,resolution:13,file:1,line:2,column:9,code:14}],compact:{encoding:'string-pool-v1',strings,fields:{classes:['id','name','package','file','graph_node','kind','language','visibility','supertypes'],methods:['id','class_id','name','signature','file'],calls:['from_class','from_method','to_class','to_method','target_label','dispatch','resolution','file','code']}}};
// Empty arrays must not classify a nested object-array column as pooled.
// Real compiler reports mix empty/nonempty metadata on partial declarations,
// while older rows may omit either field or explicitly carry null.
strings.push('p.B','B','src/B.cs','p.C','C','p.D','D');
const originalClass={...compact.classes[0]};
const supertypeSite={target:strings[10],file:'src/B.cs',line:17};
compact.classes=[
  {...originalClass,supertype_sites:[],declaration_files:[]},
  {...originalClass,id:15,name:16,file:17,graph_node:17,supertype_sites:[supertypeSite],declaration_files:[1,17]},
  {...originalClass,id:18,name:19,supertype_sites:null,declaration_files:null},
  {...originalClass,id:20,name:21},
];
compact.compact.fields.classes.push('declaration_files');
const expected=JSON.parse(JSON.stringify(compact));delete expected.compact;
for(const [table,fields] of Object.entries(compact.compact.fields))for(const row of expected[table])for(const field of fields)if(row[field]!=null)row[field]=Array.isArray(row[field])?row[field].map(id=>strings[id]):strings[row[field]];
const fixture={root:'/tmp/compact',nodes:[{id:0,name:'src/A.cs',kind:'file',parent:null}],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}};
const original=JSON.stringify(compact),app=launch({...fixture,calls:compact});
assert.equal(app.run('data.calls.compact?.encoding'),'string-pool-v1','graph startup preserves compact optional calls until requested');
app.run("showTab('calls')");
assert.deepEqual(JSON.parse(app.run('JSON.stringify(callModel)')),expected,'decode source identities, evidence, nullable fields and metadata without changing semantics');
const decodedClasses=JSON.parse(app.run('JSON.stringify(callModel.classes)'));
assert.deepEqual(decodedClasses[0].supertype_sites,[]);assert.deepEqual(decodedClasses[0].declaration_files,[]);
assert.deepEqual(decodedClasses[1].supertype_sites,[supertypeSite],'nested target IDs and declaration locations stay literal');
assert.deepEqual(decodedClasses[1].declaration_files,['src/A.cs','src/B.cs'],'string-array metadata still decodes the pool');
assert.equal(decodedClasses[2].supertype_sites,null);assert.equal(decodedClasses[2].declaration_files,null);
assert.equal(Object.hasOwn(decodedClasses[3],'supertype_sites'),false);assert.equal(Object.hasOwn(decodedClasses[3],'declaration_files'),false);
assert.equal(JSON.stringify(compact),original,'caller-owned transport fixture remains unmodified');
assert.equal(app.ids.get('archguard-data').textContent,'','release parsed JSON text instead of retaining another full report in the DOM');
app.run("showTab('calls');callUi.resolution='external';renderCalls()");
const exported=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(exported.edges[0].evidence[0].code,strings[14]);assert.equal(exported.edges[0].to,strings[10]);
assert.deepEqual(JSON.parse(app.run('JSON.stringify(data.calls)')),expected);
assert.doesNotMatch(app.ids.get('calls-evidence').textContent,/string-pool-v1/);assert.equal(app.ids.get('calls-evidence').querySelectorAll('img').length,0);
const legacy=launch({...fixture,calls:expected});assert.deepEqual(JSON.parse(legacy.run('JSON.stringify(callModel)')),expected);
for(const malformed of [
 {...compact,compact:{...compact.compact,encoding:'future'}},
 {...compact,compact:{...compact.compact,strings:null}},
 {...compact,calls:[{...compact.calls[0],from_class:999999}]},
 {...compact,calls:[{...compact.calls[0],from_class:-1}]},
 {...compact,calls:[{...compact.calls[0],from_class:1.5}]},
 {...compact,compact:{...compact.compact,fields:{...compact.compact.fields,classes:[...compact.compact.fields.classes,'supertype_sites']}}},
]){
  const broken=launch({...fixture,calls:malformed}),before=broken.run('JSON.stringify(data.calls)');
  assert.throws(()=>broken.run('decodeCallPayload({...data.calls})'),/call|compact|pool|encoding/i,'corrupt transport must never fabricate targets');
  broken.run("showTab('calls')");
  assert.equal(broken.run('callModel.status'),'unavailable');
  assert.match(broken.ids.get('calls-status').textContent,/could not be read/i,'corruption is explicit when optional calls are requested');
  assert.equal(broken.run('JSON.stringify(data.calls)'),before,'failed optional decoding retains the original transport');
  assert.equal(broken.run('callClasses.size+callMethods.size'),0);
  broken.run("showTab('graph')");assert.equal(broken.run('work.nodes.length'),fixture.nodes.length,'corrupt optional calls do not block the dependency graph');
}
console.log('Compact Calls: lossless pooled symbols, nested declaration/supertype metadata, exports, legacy reports, released JSON and corruption guards passed');
