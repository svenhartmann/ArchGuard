'use strict';
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const scc=fs.readFileSync('crates/archguard-report/src/viewer_scc.js','utf8');
const compute=vm.runInNewContext(scc+'\n'+fs.readFileSync('crates/archguard-report/src/viewer_call_cycles_model.js','utf8')+'\ncomputeCallCycles;');
const plain=value=>JSON.parse(JSON.stringify(value));
const classes=new Map(['api.A','impl.B','other.C','unused.D'].map(id=>[id,{id,name:id,package:id.split('.')[0]}]));
const method=(classId,name)=>({id:classId+'#'+name,class_id:classId,name});
const methods=new Map([method('api.A','run()'),method('api.A','sink()'),method('impl.B','run()'),method('impl.B','other()'),method('impl.B','run(int)'),method('other.C','run()')].map(item=>[item.id,item]));
const call=(from,to,properties={})=>({from_class:from?.split('#')[0]??'Unknown',from_method:from??null,to_class:to?.split('#')[0]??null,to_method:to??null,target_label:to??'dynamic.call()',dispatch:'instance',resolution:to?'project':'unresolved',file:'A.java',line:10,column:4,code:'target.call();',...properties});
const model=sites=>plain(compute(sites,classes,methods));
const checkWitness=group=>{
  assert.equal(group.witness[0],group.witness.at(-1),'a witness is a closed directed path');
  assert.ok(group.witness.length>=2);
  const pairs=new Set(group.edges.map(edge=>JSON.stringify([edge.from,edge.to])));
  for(let i=1;i<group.witness.length;i++)assert.ok(pairs.has(JSON.stringify(group.witness.slice(i-1,i+1))),'every witness step has recorded evidence');
  assert.ok(group.witness.every(id=>group.members.includes(id)));
  assert.equal(group.call_sites,group.edges.reduce((sum,edge)=>sum+edge.calls.length,0));
};

assert.deepEqual(model([]),{classes:[],methods:[],unresolved_targets:0,missing_methods:0});
assert.deepEqual(model([call('api.A#run()','impl.B#run()')]).classes,[],'one-way dependencies are not cycles');

// A cycle between types does not prove a cyclic path between their methods.
const aggregateSites=[call('api.A#run()','impl.B#run()'),call('impl.B#other()','api.A#sink()')];
const aggregate=model(aggregateSites);
assert.equal(aggregate.classes.length,1);
assert.deepEqual(aggregate.classes[0].members,['api.A','impl.B']);
assert.equal(aggregate.classes[0].id,JSON.stringify(['class',['api.A','impl.B']]));
assert.equal(aggregate.classes[0].kind,'class');
assert.deepEqual(aggregate.methods,[],'different caller/target methods must not become a synthetic method cycle');
checkWitness(aggregate.classes[0]);

const first=call('api.A#run()','impl.B#run()');
const sites=[first,{...first,column:12},{...first},call('impl.B#run()','other.C#run()',{file:'B.java',line:20}),call('other.C#run()','api.A#run()',{file:'C.java'}),call('api.A#run()','other.C#run()',{line:11}),call('impl.B#other()','impl.B#run(int)'),call('impl.B#run(int)','impl.B#other()')];
const before=JSON.stringify([sites,[...classes],[...methods]]);
const result=model(sites);
assert.equal(result.classes.length,1);
assert.deepEqual(result.classes[0].members,['api.A','impl.B','other.C']);
assert.equal(result.classes[0].edges.length,4,'retain all internal edges, beyond a single witness');
assert.equal(result.classes[0].call_sites,6,'duplicate analyzer records remain evidence');
assert.equal(result.methods.length,2,'independent same-class mutual calls form a method group');
assert.deepEqual(result.methods[0].members,['api.A#run()','impl.B#run()','other.C#run()']);
assert.deepEqual(result.methods[1].members,['impl.B#other()','impl.B#run(int)'],'overloads preserve exact identities');
for(const group of [...result.classes,...result.methods])checkWitness(group);
assert.equal(JSON.stringify([sites,[...classes],[...methods]]),before,'source records and declaration maps remain untouched');
assert.deepEqual(model(sites.slice().reverse()),result,'source order does not change groups, witnesses, evidence order or IDs');
assert.deepEqual(plain(compute(sites,new Map([...classes].reverse()),new Map([...methods].reverse()))),result,'declaration order does not change cycle identities');
const originals=compute(sites,classes,methods).classes[0].edges.flatMap(edge=>edge.calls);
assert.ok(originals.every(record=>sites.includes(record)),'evidence references original records');

const recursion=model([call('api.A#run()','api.A#run()')]);
assert.deepEqual(recursion.classes,[],'an ordinary internal class call is not a class dependency cycle');
assert.deepEqual(recursion.methods[0].members,['api.A#run()']);
assert.deepEqual(recursion.methods[0].witness,['api.A#run()','api.A#run()']);
assert.equal(recursion.methods[0].call_sites,1);

const invalid=[call('api.A#missing()','impl.B#missing()'),call('impl.B#run()','api.A#run()',{from_class:'api.A',to_class:'impl.B'}),call(null,'api.A#run()',{from_class:'impl.B'}),call('api.A#run()','Unknown#run()'),call('api.A#run()','impl.B#run()',{resolution:'external'}),call('impl.B#run()','api.A#run()',{resolution:'unresolved'}),call('api.A#run()',null,{resolution:'project'})];
const incomplete=model(invalid);
assert.equal(incomplete.classes.length,1,'known class endpoints remain valid without declared methods');
assert.equal(incomplete.classes[0].call_sites,3,'unknown, external and unresolved class edges are excluded');
assert.deepEqual(incomplete.methods,[],'absent declarations or wrong ownership never create method edges');
assert.equal(incomplete.missing_methods,5,'count project records with either missing or inconsistent method endpoint once');
assert.equal(incomplete.unresolved_targets,1,'unresolved records remain visible as a coverage limitation');
assert.deepEqual(model([first,call('impl.B#run()','api.A#run()',{resolution:'external'})]).classes,[],'an external record cannot close a project cycle');
assert.deepEqual(model([first,call('impl.B#run()','api.A#run()',{resolution:'unresolved'})]).classes,[],'an unresolved record cannot close a project cycle');

// Safe map keys and compiler identities cannot collide, including identical labels.
const ids=['__proto__','constructor','a|b','a\u0000b','java:Shared.Type','csharp:Assembly::Shared.Type'];
const safeClasses=new Map(ids.map(id=>[id,{id,name:'Same'}]));
const safeMethods=new Map(ids.map(id=>[id+'#run()',{id:id+'#run()',class_id:id,name:'run'}]));
const safeSites=ids.flatMap((id,index)=>index%2?[]:[call(id+'#run()',ids[index+1]+'#run()'),call(ids[index+1]+'#run()',id+'#run()')]);
const safe=plain(compute(safeSites,safeClasses,safeMethods));
assert.equal(safe.classes.length,3);assert.equal(safe.methods.length,3);
assert.equal(new Set(safe.classes.map(group=>group.id)).size,3);
for(const group of [...safe.classes,...safe.methods])checkWitness(group);

// Large single SCC, a deep acyclic chain and many separate SCCs must be stack-safe.
const count=10000,largeClasses=new Map(),largeMethods=new Map(),largeCalls=[];
for(let i=0;i<count;i++){
  const id='Type'+String(i).padStart(5,'0');largeClasses.set(id,{id});
  const member=method(id,'run()');largeMethods.set(member.id,member);
  if(i)largeCalls.push(call('Type'+String(i-1).padStart(5,'0')+'#run()',member.id));
}
const chain=compute(largeCalls,largeClasses,largeMethods);
assert.equal(chain.classes.length,0);assert.equal(chain.methods.length,0);
largeCalls.push(call('Type09999#run()','Type00000#run()'));
const deep=compute(largeCalls,largeClasses,largeMethods);
assert.equal(deep.classes.length,1);assert.equal(deep.methods.length,1);
assert.equal(deep.classes[0].members.length,count);
assert.equal(deep.methods[0].witness.length,count+1);
checkWitness(deep.methods[0]);
const pairs=largeCalls.filter((_,i)=>i%2===0).flatMap(site=>[site,{...site,from_class:site.to_class,to_class:site.from_class,from_method:site.to_method,to_method:site.from_method}]);
const separate=compute(pairs,largeClasses,largeMethods);
assert.equal(separate.classes.length,count/2);assert.equal(separate.methods.length,count/2);
assert.ok(separate.classes.every(group=>group.call_sites===2&&group.members.length===2));
console.log('Call cycles model: distinct class/method SCCs, recursion, full evidence, exact identities, coverage, stable witnesses and 10k stack-safe traversal passed');
