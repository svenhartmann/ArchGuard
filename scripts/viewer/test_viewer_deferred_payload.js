// `archguard serve` leaves large payload sections out of the page. Views that
// need them fetch them once for exactly this page revision; nothing else waits.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const context={require,console,performance,Blob};
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{...context});
const compactFixture=vm.runInNewContext(fs.readFileSync('scripts/viewer/test_viewer_compact_calls.js','utf8').split('const original=')[0]+'\n({fixture,compact});',{...context});
const metric=value=>({value,observed:value,status:'ready',evidence:[]});
const codeMetrics={status:'ready',classes:[{class_id:'p.A',node:'src/A.cs',file:'src/A.cs',cbo:metric(3),rfc:metric(4),dit:metric(1),noc:metric(0),wmc:metric(5),lcom4:1}],methods:[]};
const sections={calls:compactFixture.compact,code_metrics:codeMetrics};
const fixture={...compactFixture.fixture,calls:{deferred:'calls'},insights:{code_metrics:{deferred:'code_metrics'}}};
const reloads=[];
const app=launch(fixture,{library:false,live:true,location:{search:'?project=server',hash:'',reload(){reloads.push(true);}}});
const requests=[];let status=200;
app.context.window.archguardLiveVersion='rev1';
app.context.fetch=url=>{requests.push(url);const name=url.split('/')[3];
  return Promise.resolve({status,ok:status===200,json:()=>Promise.resolve(JSON.parse(JSON.stringify(sections[name]))),text:()=>Promise.resolve('changed')});};
const settle=async()=>{for(let i=0;i<5;i++)await new Promise(resolve=>setImmediate(resolve));};
(async()=>{
  assert.equal(requests.length,0,'opening the report fetches nothing');
  assert.equal(app.run("engineeringData('code_metrics')"),null,'a placeholder is never read as measurements');
  app.run("showTab('calls')");
  assert.deepEqual(requests,['/__archguard/payload/calls/rev1?project=server'],'the Calls view requests its section for this page revision');
  assert.match(app.ids.get('calls-relations').textContent,/Loading|geladen/,'the view says it is loading instead of showing no calls');
  await settle();
  assert.equal(app.run('callModel.calls.length'),JSON.parse(app.run('JSON.stringify(decodeCallPayload('+JSON.stringify(sections.calls)+').calls.length)')),'loaded calls are decoded as if embedded');
  assert.doesNotMatch(app.ids.get('calls-relations').textContent,/Loading|geladen/);
  app.run("showTab('graph');showTab('calls')");
  assert.equal(requests.length,1,'a loaded section is not fetched again');
  app.run("showTab('quality')");
  assert.equal(requests[1],'/__archguard/payload/code_metrics/rev1?project=server');
  await settle();
  assert.equal(app.run("engineeringData('code_metrics').classes.length"),1);
  assert.equal(app.run('engineeringUi.section'),'heatmap','quality opens the class heatmap once measurements arrived');
  // A newer analysis on the server reloads the page instead of mixing revisions.
  const stale=launch(fixture,{library:false,live:true,location:{search:'',hash:'',reload(){reloads.push(true);}}});
  stale.context.window.archguardLiveVersion='old';status=409;
  stale.context.fetch=app.context.fetch;stale.run("showTab('calls')");await settle();
  assert.equal(reloads.length,1,'a stale page reloads');
  console.log('Deferred payload: no fetch on load, one request per section and revision, loading states, decoded calls, quality heatmap and stale-page reload passed');
})().catch(error=>{console.error(error);process.exit(1);});
