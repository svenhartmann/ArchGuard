// Execute the injected live client, including asynchronous request ordering.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('crates/archguard-cli/src/serve_reload.js','utf8');
function launch({hidden=false,versionPath='/__archguard/version',version='first',language='en'}={}){
  const timers=[],messages={textContent:''},requests=[],listeners=new Map(),navigationFlags=[],sidebarFlags=[];
  const window={reloads:0,location:{reload(){window.reloads++;}}};
  const document={hidden,getElementById:()=>messages,addEventListener:(name,fn)=>listeners.set(name,fn)};
  let now=0,nextId=0,respond,pending;
  vm.runInNewContext(source.replace('__ARCHGUARD_VERSION__',JSON.stringify(version)).replace('__ARCHGUARD_VERSION_PATH__',JSON.stringify(versionPath)),{window,document,t:(en,de)=>language==='de'?de:en,renderRepositoryNavigation(){navigationFlags.push(window.archguardLiveViewer);},renderSidebar(){sidebarFlags.push(window.archguardLiveViewer);},performance:{now:()=>now},
    setTimeout(fn,delay){const id=++nextId;timers.push({id,fn,delay});return id;},
    clearTimeout(id){const index=timers.findIndex(timer=>timer.id===id);if(index>=0)timers.splice(index,1);},
    fetch(url,options){requests.push({url,options});return new Promise(resolve=>{respond=resolve;});},
  });
  const app={window,document,timers,messages,requests,navigationFlags,sidebarFlags,
    start(){pending=timers.shift().fn();assert.equal(timers.length,0,'no overlapping requests while a poll is pending');return pending;},
    visibility(hidden){document.hidden=hidden;const result=listeners.get('visibilitychange')?.();if(result?.then)pending=result;},
    async finish(version,ok=true,elapsed=0){now+=elapsed;respond({ok,json:async()=>version&&typeof version==='object'?version:{version}});await pending;},
    async answer(version,ok=true,elapsed=0){app.start();await app.finish(version,ok,elapsed);},
  };return app;
}
(async()=>{
  const app=launch();assert.equal(app.messages.textContent,'Live · source checks active');assert.equal(app.timers[0].delay,1000);
  assert.equal(app.window.archguardLiveViewer,true,'only the injected server script enables project navigation');
  assert.deepEqual(app.navigationFlags,[true]);assert.deepEqual(app.sidebarFlags,[true],'live enablement refreshes already-rendered inspector links');
  await app.answer('first');assert.equal(app.window.reloads,0);assert.equal(app.timers.length,1);assert.equal(app.timers[0].delay,1000);
  assert.equal(app.requests[0].url,'/__archguard/version');assert.equal(app.requests[0].options.cache,'no-store');
  await app.answer('second');assert.equal(app.window.reloads,1);assert.equal(app.timers.length,0);
  const selected=launch({versionPath:'/__archguard/version?project=apps%2Fsearch%20api'});
  await selected.answer('first');assert.equal(selected.requests[0].url,'/__archguard/version?project=apps%2Fsearch%20api','the live probe must retain the selected project scope');
  const pending=launch();pending.window.archguardCanReload=()=>false;
  await pending.answer('second');assert.equal(pending.window.reloads,0);assert.match(pending.messages.textContent,/simulated changes/);
  pending.window.archguardCanReload=()=>true;await pending.answer('second');assert.equal(pending.window.reloads,1);
  const recovery=launch();await recovery.answer(null,false);assert.equal(recovery.window.reloads,0);assert.match(recovery.messages.textContent,/interrupted/);
  await recovery.answer('first');assert.match(recovery.messages.textContent,/restored/);assert.equal(recovery.timers.length,1);
  const slow=launch();await slow.answer('first',true,2000);assert.equal(slow.timers[0].delay,4000,'slow scans need idle time instead of continuously consuming the server');
  await slow.answer('first',true,800);assert.equal(slow.timers[0].delay,1000,'fast checks retain the normal live cadence');
  await slow.answer('first',true,12000);assert.equal(slow.timers[0].delay,10000,'adaptive delay is bounded');
  const hidden=launch({hidden:true});assert.equal(hidden.timers.length,0);assert.equal(hidden.requests.length,0);
  hidden.visibility(false);assert.equal(hidden.requests.length,1,'returning to a tab probes immediately');await hidden.finish('first');
  hidden.visibility(true);assert.equal(hidden.timers.length,0,'hiding a tab cancels its scheduled poll');
  hidden.visibility(false);assert.equal(hidden.requests.length,2);hidden.visibility(true);await hidden.finish('second');
  assert.equal(hidden.window.reloads,0,'do not reload a hidden tab after its pending request');assert.equal(hidden.timers.length,0);
  hidden.visibility(false);assert.equal(hidden.requests.length,3);await hidden.finish('second');assert.equal(hidden.window.reloads,1);
  const overlap=launch();overlap.start();overlap.visibility(true);overlap.visibility(false);assert.equal(overlap.requests.length,1,'visibility changes do not overlap an active request');
  await overlap.finish('first');assert.equal(overlap.timers.length,1);
  const loading=launch({version:''});
  assert.match(loading.messages.textContent,/Preparing graph/,'the initial shell describes its pending analysis before the first poll');
  await loading.answer({version:'',refreshing:true,status:'loading'});
  assert.equal(loading.window.reloads,0,'an empty revision is not a completed report');
  assert.equal(loading.timers[0].delay,1000,'background analysis keeps status checks responsive');
  await loading.answer({version:'',refreshing:true,status:'loading'});
  assert.equal(loading.window.reloads,0,'pending initial analysis must not cause reload loops');
  await loading.answer({version:'ready-graph',refreshing:false,status:'ready'});
  assert.equal(loading.window.reloads,1,'publish the first completed graph');
  const refresh=launch();
  await refresh.answer({version:'first',refreshing:true,status:'refreshing'});
  assert.equal(refresh.window.reloads,0,'an existing graph stays usable during background refresh');
  assert.match(refresh.messages.textContent,/background/);
  refresh.messages.textContent='User selected a package';
  await refresh.answer({version:'first',refreshing:true,status:'refreshing'});
  assert.equal(refresh.messages.textContent,'User selected a package','identical pending polls must not keep replacing user feedback');
  await refresh.answer({version:'first',refreshing:false,status:'ready'});
  assert.match(refresh.messages.textContent,/Live ·/,'an unchanged completed refresh clears its pending message');
  assert.doesNotMatch(refresh.messages.textContent,/Updating/);
  const lostPage=launch();
  await lostPage.answer({version:'',refreshing:true,status:'loading'});
  assert.equal(lostPage.window.reloads,0,'a restarted server may temporarily have no cached revision');
  await lostPage.answer({version:'first',refreshing:false,status:'ready'});
  assert.equal(lostPage.window.reloads,0,'the restarted server may recover the same report');
  const loadingRecovery=launch({version:''});
  await loadingRecovery.answer({version:'',refreshing:true,status:'loading'});
  await loadingRecovery.answer(null,false);assert.match(loadingRecovery.messages.textContent,/interrupted/);
  await loadingRecovery.answer({version:'',refreshing:true,status:'loading'});
  assert.match(loadingRecovery.messages.textContent,/Preparing graph/,'reconnection while loading clears the disconnection message');
  const backgroundRecovery=launch();
  await backgroundRecovery.answer({version:'first',refreshing:true,status:'refreshing'});
  await backgroundRecovery.answer(null,false);
  await backgroundRecovery.answer({version:'first',refreshing:true,status:'refreshing'});
  assert.match(backgroundRecovery.messages.textContent,/Updating analysis/,'reconnection restores a still-pending refresh message');
  const queued=launch();queued.window.archguardCanReload=()=>false;
  await queued.answer({version:'second',refreshing:true,status:'refreshing'});
  assert.equal(queued.window.reloads,0);assert.match(queued.messages.textContent,/simulated changes/);
  queued.window.archguardCanReload=()=>true;
  await queued.answer({version:'second',refreshing:true,status:'refreshing'});
  assert.equal(queued.window.reloads,1,'an already published new report is loadable while a later check runs');
  const german=launch({version:'',language:'de'});
  assert.match(german.messages.textContent,/Graph wird vorbereitet/);
  await german.answer({version:'',refreshing:true,status:'loading'});
  assert.match(german.messages.textContent,/Hintergrund/);
  const analysisError=launch({version:''});
  await analysisError.answer({version:'error-page',refreshing:false,status:'ready'});
  assert.equal(analysisError.window.reloads,1,'a published error page is loadable and can reconnect normally');
  const errorRecovery=launch({version:'error-page'});
  await errorRecovery.answer({version:'fixed-graph',refreshing:false,status:'ready'});
  assert.equal(errorRecovery.window.reloads,1,'corrected source configuration leaves the error page');
  console.log('live viewer: nonoverlapping polls, bounded slow-scan cooldown, hidden-tab pause, immediate resume, protected simulation and reconnection passed');
})().catch(error=>{console.error(error);process.exitCode=1;});
