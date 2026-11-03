// Only injected by `serve`; exported HTML remains an ordinary snapshot.
(() => {
  window.archguardLiveViewer=true;
  if(typeof renderRepositoryNavigation==='function')renderRepositoryNavigation();
  if(typeof renderSidebar==='function')renderSidebar();
  const version = __ARCHGUARD_VERSION__;
  const versionPath = __ARCHGUARD_VERSION_PATH__;
  let disconnected=false,inFlight=false,reloading=false,timer=null,refreshState=null;
  const hidden=()=>document.hidden===true;
  const now=()=>typeof performance!=='undefined'?performance.now():Date.now();
  const text=(en,de)=>typeof t==='function'?t(en,de):en;
  function status(message) {
    const target=document.getElementById('feedback');
    if(target){target.textContent=message;target.title=message===liveMessage()?text('Source checks run in the background and pause in hidden tabs.','Quellprüfungen laufen im Hintergrund und pausieren in ausgeblendeten Tabs.'):'';}
  }
  const liveMessage=()=>text('Live · source checks active','Live · Quellprüfung aktiv');
  function pendingStatus(state) {
    if(refreshState===state)return;
    refreshState=state;
    status(state==='loading'
      ?text('Preparing graph… Analysis runs in the background.','Graph wird vorbereitet… Die Analyse läuft im Hintergrund.')
      :text('Updating analysis in the background. The current graph remains usable.','Analyse wird im Hintergrund aktualisiert. Der aktuelle Graph bleibt bedienbar.'));
  }
  function schedule(delay) {
    if(timer!==null)clearTimeout(timer);
    timer=null;
    if(!hidden()&&!reloading)timer=setTimeout(poll,delay);
  }
  async function poll() {
    timer=null;
    if(hidden()||inFlight||reloading)return;
    inFlight=true;
    const started=now();
    try {
      const response=await fetch(versionPath,{cache:'no-store'});
      if(!response.ok)throw new Error('Server unavailable');
      const current=await response.json();
      if(!current||typeof current.version!=='string')throw new Error('Invalid live status');
      const restored=disconnected;
      disconnected=false;
      if(restored)refreshState=null;
      // A cold/restarted server has no published report yet. Keep its loading
      // shell (or the last good graph) open instead of reloading an empty page.
      if(current.version&&current.version!==version&&!hidden()){
        if(typeof window.archguardCanReload==='function'&&!window.archguardCanReload()){
          status(text('Project changed. Export and clear simulated changes and review drafts, or close the confirmation, to load the updated graph.','Projekt geändert. Simulierte Änderungen und Review-Entwürfe exportieren und löschen oder die Bestätigung schließen, um den aktualisierten Graphen zu laden.'));
        }else{
          reloading=true;window.location.reload();
        }
      }else if(!current.version){
        pendingStatus('loading');
      }else if(current.refreshing===true||current.status==='refreshing'){
        pendingStatus('refreshing');
      }else if(restored){
        refreshState=null;
        status(text('Live connection restored.','Live-Verbindung wiederhergestellt.'));
      }else if(refreshState!==null){
        refreshState=null;
        status(liveMessage());
      }
    }catch(_){
      disconnected=true;
      status(text('Live connection interrupted. Reconnecting automatically…','Live-Verbindung unterbrochen. Automatischer Wiederverbindungsversuch…'));
    }finally{
      inFlight=false;
      // A slow source check must not monopolize the server. Fast projects keep
      // their normal cadence; a completed slow scan gets bounded idle time.
      const elapsed=now()-started;
      schedule(elapsed>=2000?Math.min(10000,Math.ceil(elapsed*2)):1000);
    }
  }
  document.addEventListener('visibilitychange',()=>{
    if(hidden()){if(timer!==null)clearTimeout(timer);timer=null;}
    else if(!inFlight&&!reloading){if(timer!==null)clearTimeout(timer);timer=null;return poll();}
  });
  document.addEventListener('click',event=>{if(['language-de','language-en'].includes(event.target?.id)&&version&&!disconnected&&refreshState===null)status(liveMessage());});
  if(version)status(liveMessage());else pendingStatus('loading');
  schedule(1000);
})();
