// Usage: node scripts/benchmarks/benchmark_viewer.js /tmp/report-payload.json [--out /tmp/measurement.json]
// Executes the actual application in the existing DOM harness. These timings
// exclude browser layout/paint and are measurements, not pass/fail thresholds.
const fs=require('node:fs'),vm=require('node:vm'),{performance}=require('node:perf_hooks');
const input=process.argv[2];if(!input)throw new Error('Supply an existing HTML report or extracted JSON payload');
const raw=fs.readFileSync(input,'utf8');
const json=raw.trimStart().startsWith('<')?raw.match(/<script id="archguard-data" type="application\/json">([\s\S]*?)<\/script>/)?.[1]:raw;
if(!json)throw new Error('HTML has no archguard-data payload');
const timings={};
function measure(name,callback){const start=performance.now();const value=callback();timings[name]=Math.round((performance.now()-start)*10)/10;return value;}
const data=measure('jsonParseMs',()=>JSON.parse(json));
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=measure('harnessLaunchMs',()=>launch(data,{library:false}));timings.applicationBootMs=Math.round(app.loadMs*10)/10;
measure('expandAllMs',()=>app.run('setAllTreeBranches(true)'));
measure('collapseAllMs',()=>app.run('setAllTreeBranches(false)'));
const reference=data.nodes.find(node=>node.name==='MediaBrowser/Controller')||data.nodes.find(node=>node.kind==='module');
if(reference){
  measure('selectPackageMs',()=>app.run('selectNode('+JSON.stringify(reference.name)+')'));
  measure('expandSelectedTreeMs',()=>app.run('setAllTreeBranches(true)'));
}
measure('callsTabMs',()=>app.run("showTab('calls')"));
measure('callsGermanMs',()=>app.click('language-de'));
measure('callsEnglishMs',()=>app.click('language-en'));
const result={input,environment:'Node DOM harness; no browser layout or paint',payloadBytes:Buffer.byteLength(json),compactCalls:!!data.calls?.compact,nodes:data.nodes.length,edges:data.edges.length,callSites:app.run('callModel.calls.length'),timings};
const output=JSON.stringify(result,null,2)+'\n';
const out=process.argv.indexOf('--out');if(out>=0){if(!process.argv[out+1])throw new Error('--out needs a path');fs.writeFileSync(process.argv[out+1],output);}
process.stdout.write(output);
