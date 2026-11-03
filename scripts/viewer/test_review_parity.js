// Cross-runtime parity: Rust supplies graphs, evidence and expected results.
const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const input=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
const compute=vm.runInNewContext(fs.readFileSync('crates/archguard-report/src/viewer_review.js','utf8')+'\ncomputeReview;');
const actual=JSON.parse(JSON.stringify(compute(input.full,input.view,input.context)));
function close(a,b,path='review') {
  if(typeof a==='number'&&typeof b==='number'){assert.ok(Math.abs(a-b)<1e-9*Math.max(1,Math.abs(b)),path+': '+a+' != '+b);return;}
  if(a&&b&&typeof a==='object'&&typeof b==='object'){
    assert.deepEqual(Object.keys(a).sort(),Object.keys(b).sort(),path+' keys');
    for(const key of Object.keys(b))close(a[key],b[key],path+'.'+key);
  }else assert.deepEqual(a,b,path);
}
close(actual,input.expected);
