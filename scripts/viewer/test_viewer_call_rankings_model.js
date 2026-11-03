'use strict';
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const compute=vm.runInNewContext(fs.readFileSync('crates/archguard-report/src/viewer_call_rankings.js','utf8')+'\ncomputeCallRankings;');
const rank=(calls,classes,methods)=>JSON.parse(JSON.stringify(compute(calls,classes,methods)));
const classes=new Map(['A','B','Unused'].map(id=>[id,{id,name:id}]));
const method=(classId,name)=>({id:classId+'#'+name,class_id:classId,name});
const methods=new Map([method('A','run()'),method('A','helper()'),method('B','fetch()'),method('B','fetch(int)'),method('Unused','run()')].map(item=>[item.id,item]));
const call=(from,to,properties={})=>({from_class:from?.split('#')[0]??'Unknown',from_method:from??null,to_class:to?.split('#')[0]??null,to_method:to??null,target_label:to??'dynamic.call()',dispatch:'instance',resolution:to?'project':'unresolved',file:'A.java',line:10,column:4,code:'target.call();',...properties});
const row=(id,incoming,outgoing,total)=>({id,incoming,outgoing,total});

assert.deepEqual(rank([],classes,methods),{classes:[],methods:[],unresolved_targets:0,missing_callers:0,missing_targets:0},'declarations alone are not call activity');
const first=call('A#run()','B#fetch()');
const sites=[first,{...first,column:11},call('A#run()','B#fetch(int)',{line:11}),call('B#fetch()','A#run()',{line:12}),call('A#run()','A#run()',{line:13}),call('A#run()','A#helper()',{line:14}),call(null,'A#helper()',{from_class:'B',line:15}),call('A#run()','external.Library#fetch()',{resolution:'external',line:16}),call('A#run()',null,{line:17}),call('Unknown#run()','A#run()',{line:18}),Object.fromEntries(Object.entries(first).reverse())];
const before=JSON.stringify([sites,[...classes],[...methods]]);
const result=rank(sites,classes,methods);
assert.deepEqual(result.classes,[row('A',5,8,11),row('B',4,2,6)],'class totals count each involved call-site record once, including initializers');
assert.deepEqual(result.methods,[row('A#run()',3,8,10),row('B#fetch()',3,1,4),row('A#helper()',2,0,2),row('B#fetch(int)',1,0,1)],'overloads retain exact identities and equal incoming counts use stable IDs');
assert.equal(result.unresolved_targets,1,'unresolved dispatch remains visible without invented targets');
assert.equal(result.missing_callers,2,'initializers and absent caller declarations have no invented method');
assert.equal(result.missing_targets,0,'all declared project target methods are represented');
assert.equal(JSON.stringify([sites,[...classes],[...methods]]),before,'ranking must not mutate the source snapshot');
assert.deepEqual(rank(sites.slice().reverse(),classes,methods),result,'input order does not alter counts or ranking');
assert.deepEqual(rank([first,{...first}],classes,methods).classes,[row('B',2,0,2),row('A',0,2,2)],'retain repeated analyzer records consistently with the source tree and evidence view');

const recursive=rank([call('A#run()','A#run()')],classes,methods);
assert.deepEqual(recursive.classes,[row('A',1,1,1)]);
assert.deepEqual(recursive.methods,[row('A#run()',1,1,1)],'one recursive site is one total, with both incoming and outgoing attribution');

for(const [field,value] of Object.entries({file:'Other.java',line:20,column:12,code:'other.call();',dispatch:'static',resolution:'external',target_label:'different target'})){
  const distinct=rank([first,{...first,[field]:value}],classes,methods);
  assert.equal(distinct.classes.find(item=>item.id==='B').incoming,2,'same-pair call sites with distinct '+field+' remain separate');
}
const missing=rank([call('A#missing()','B#implicit()'),call('A#run()','B#fetch()',{from_class:'B',to_class:'A'})],classes,methods);
assert.deepEqual(missing.classes,[row('A',1,1,2),row('B',1,1,2)],'class evidence survives missing or inconsistent method declarations');
assert.deepEqual(missing.methods,[],'unknown constructors and inconsistent method owners never create method rows');
assert.equal(missing.missing_callers,2);
assert.equal(missing.missing_targets,2,'missing project constructors and wrong method owners leave target-method coverage incomplete');
const missingTargets=rank([call('A#run()','B#implicit()'),call('A#run()','Unknown#run()'),call('A#run()',null,{resolution:'project'}),call('A#run()','external.Library#fetch()',{resolution:'external'}),call('A#run()',null)],classes,methods);
assert.equal(missingTargets.missing_targets,3,'count absent project target methods or classes, but separate external and unresolved targets');

const unsafeIds=['__proto__','toString','constructor','<img src=x onerror=alert(1)>','A|B','A\u0000B'];
const safeClasses=new Map(unsafeIds.map(id=>[id,{id}])),safeMethods=new Map(unsafeIds.map(id=>[id+'#run()',{id:id+'#run()',class_id:id}]));
const safeCalls=unsafeIds.map(id=>call(id+'#run()',id+'#run()'));
const safe=rank(safeCalls,safeClasses,safeMethods);
assert.deepEqual(safe.classes.map(item=>item.id),unsafeIds.slice().sort(),'identities are Map keys, with no prototype or delimiter collisions');
assert.ok(safe.classes.every(item=>item.total===1&&item.incoming===1&&item.outgoing===1));
assert.equal(safe.methods.length,unsafeIds.length);
console.log('Call ranking model: static sites, direction, recursion, overloads, missing identities, evidence parity and stable safe identities passed');
