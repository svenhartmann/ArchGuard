'use strict';
// Durable graph experiments, not executable source-code refactorings. This
// module loads after the viewer and reuses its single SimOp implementation.
const SCENARIO_SCHEMA='archguard-refactoring-scenario/v1';
const SCENARIO_MAX_BYTES=1024*1024,SCENARIO_MAX_OPS=256,SCENARIO_MAX_SAVED=50;
var scenarioBaseCut=null; // hoisted: startCut can use it during viewer startup
let scenarioPending=null,scenarioSourcePromise=null;
function scenarioCompare(a,b){return a<b?-1:a>b?1:0;}
function scenarioBytes(text){return new Blob([text]).size;}
function scenarioProject(){return typeof data.root==='string'?data.root.replace(/\\/g,'/').replace(/\/+$/,'')||'/':'';}
function scenarioStorageKey(){const project=scenarioProject();if(!project)throw Error(t('This report has no project identity.','Dieser Bericht hat keine Projektkennung.'));return 'archguard-scenarios/v1:'+encodeURIComponent(project);}
function scenarioKeys(value,keys,label){
  if(!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).some(key=>!keys.includes(key))||keys.some(key=>!Object.prototype.hasOwnProperty.call(value,key)))throw Error(t('Invalid scenario {field}.','Ungültiges Szenariofeld {field}.',{field:label}));
}
function scenarioString(value,maximum,label,empty=false){
  if(typeof value!=='string'||value.length>maximum||value.includes('\0')||(!empty&&!value.trim()))throw Error(t('Invalid scenario {field}.','Ungültiges Szenariofeld {field}.',{field:label}));return value;
}
function scenarioValidate(value){
  scenarioKeys(value,['schema','name','description','project','source','scope','ops'],'document');
  if(value.schema!==SCENARIO_SCHEMA)throw Error(t('Unsupported scenario version.','Nicht unterstützte Szenarioversion.'));
  const name=scenarioString(value.name,120,'name').trim(),description=scenarioString(value.description,4000,'description',true),project=scenarioString(value.project,4096,'project');
  scenarioKeys(value.source,['fingerprint','nodes','edges'],'source');
  if(!/^sha256:[a-f0-9]{64}$/.test(value.source.fingerprint)||![value.source.nodes,value.source.edges].every(n=>Number.isSafeInteger(n)&&n>=0))throw Error(t('Invalid dependency-graph baseline.','Ungültige Abhängigkeitsgraph-Basis.'));
  scenarioKeys(value.scope,['nodes'],'scope');
  if(!Array.isArray(value.scope.nodes)||value.scope.nodes.length>20000)throw Error(t('Invalid saved analysis scope.','Ungültiger gespeicherter Analyseumfang.'));
  const scope=value.scope.nodes.map(name=>scenarioString(name,4096,'scope node'));
  if(new Set(scope).size!==scope.length)throw Error(t('Duplicate nodes in the saved scope.','Doppelte Knoten im gespeicherten Umfang.'));
  if(!Array.isArray(value.ops)||value.ops.length>SCENARIO_MAX_OPS)throw Error(t('A scenario supports at most {count} changes.','Ein Szenario unterstützt höchstens {count} Änderungen.',{count:SCENARIO_MAX_OPS}));
  const changes=value.ops.map(op=>{
    if(!op||typeof op!=='object'||Array.isArray(op)||Object.keys(op).length!==1)throw Error(t('Invalid simulated change.','Ungültige simulierte Änderung.'));
    if(op.cut_edge){scenarioKeys(op.cut_edge,['from','to'],'cut_edge');return encodeCut(scenarioString(op.cut_edge.from,4096,'from'),scenarioString(op.cut_edge.to,4096,'to'));}
    if(op.move_node){scenarioKeys(op.move_node,['node','new_parent'],'move_node');return encodeMove(scenarioString(op.move_node.node,4096,'node'),scenarioString(op.move_node.new_parent,4096,'new_parent'));}
    throw Error(t('This viewer supports cut_edge and move_node scenarios.','Dieser Viewer unterstützt Szenarien mit cut_edge und move_node.'));
  });
  return {schema:SCENARIO_SCHEMA,name,description,project,source:{fingerprint:value.source.fingerprint,nodes:value.source.nodes,edges:value.source.edges},scope:{nodes:scope},ops:changes};
}
function scenarioSource(){
  if(scenarioSourcePromise)return scenarioSourcePromise;
  scenarioSourcePromise=(async()=>{
    if(typeof crypto==='undefined'||!crypto.subtle||typeof TextEncoder==='undefined')throw Error(t('Scenario fingerprints require browser cryptography support.','Szenario-Prüfsummen benötigen die Kryptografie-Unterstützung des Browsers.'));
    // Ignore volatile dense IDs and unrelated compiler payloads. Fingerprint
    // source import structure, counts and evidence, including source locations.
    const names=new Map(data.nodes.map(node=>[node.id,node.name]));
    const nodes=data.nodes.map(node=>[node.name,node.kind,node.lang||null,node.parent==null?null:names.get(node.parent)]).sort((a,b)=>scenarioCompare(a[0],b[0]));
    const edges=data.edges.map(edge=>[names.get(edge.from),names.get(edge.to),edge.count??null,(edge.refs||[]).map(ref=>[ref.file,ref.line,ref.raw]).sort((a,b)=>scenarioCompare(JSON.stringify(a),JSON.stringify(b)))]).sort((a,b)=>scenarioCompare(JSON.stringify(a),JSON.stringify(b)));
    const hash=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(JSON.stringify({nodes,edges})));
    return {fingerprint:'sha256:'+Array.from(new Uint8Array(hash),byte=>byte.toString(16).padStart(2,'0')).join(''),nodes:nodes.length,edges:edges.length};
  })();
  return scenarioSourcePromise;
}
async function scenarioSnapshot(name,description=''){
  const source=await scenarioSource();
  return scenarioValidate({schema:SCENARIO_SCHEMA,name,description,project:scenarioProject(),source,scope:{nodes:view.nodes.map(node=>node.name).sort()},ops});
}
function scenarioReadStore(){
  try{
    const raw=localStorage.getItem(scenarioStorageKey());if(!raw)return {items:[],error:null};
    if(scenarioBytes(raw)>SCENARIO_MAX_BYTES*2)throw Error(t('Saved scenarios exceed the storage limit.','Gespeicherte Szenarien überschreiten das Speicherlimit.'));
    const values=JSON.parse(raw);if(!Array.isArray(values)||values.length>SCENARIO_MAX_SAVED)throw Error(t('Invalid saved scenario collection.','Ungültige gespeicherte Szenariosammlung.'));
    const items=values.map(scenarioValidate);
    if(new Set(items.map(item=>item.name)).size!==items.length||items.some(item=>item.project!==scenarioProject()))throw Error(t('Saved scenarios belong to another project or contain duplicate names.','Gespeicherte Szenarien gehören zu einem anderen Projekt oder enthalten doppelte Namen.'));
    return {items:items.sort((a,b)=>scenarioCompare(a.name,b.name)),error:null};
  }catch(error){return {items:[],error:t('Local storage unavailable or invalid: {reason}. JSON export remains available.','Lokaler Speicher nicht verfügbar oder ungültig: {reason}. JSON-Export bleibt verfügbar.',{reason:error.message})};}
}
function scenarioStatus(message){document.getElementById('scenario-status').textContent=message;}
function scenarioPersist(items){
  try{
    if(items.length>SCENARIO_MAX_SAVED)throw Error(t('At most {count} saved scenarios per project.','Höchstens {count} gespeicherte Szenarien pro Projekt.',{count:SCENARIO_MAX_SAVED}));
    const text=JSON.stringify(items.sort((a,b)=>scenarioCompare(a.name,b.name)));
    if(scenarioBytes(text)>SCENARIO_MAX_BYTES*2)throw Error(t('Saved scenarios exceed the storage limit.','Gespeicherte Szenarien überschreiten das Speicherlimit.'));
    localStorage.setItem(scenarioStorageKey(),text);return true;
  }catch(error){scenarioStatus(t('Local storage could not save this scenario: {reason}. Export JSON to keep it.','Der lokale Speicher konnte dieses Szenario nicht speichern: {reason}. Zum Aufbewahren JSON exportieren.',{reason:error.message}));return false;}
}
function scenarioRender(){
  const select=document.getElementById('scenario-list'),selected=select.value,stored=scenarioReadStore();select.textContent='';
  const empty=element('option',t('Choose a saved scenario','Gespeichertes Szenario auswählen'));empty.value='';select.appendChild(empty);
  for(const item of stored.items){const option=element('option',item.name+' · '+t('{count} changes','{count} Änderungen',{count:item.ops.length}));option.value=item.name;select.appendChild(option);}
  select.value=stored.items.some(item=>item.name===selected)?selected:'';
  for(const id of ['scenario-load','scenario-delete'])document.getElementById(id).disabled=!select.value;
  scenarioStatus(stored.error||t('{count} saved scenarios for this project. Import previews a scenario; Save stores it in this browser.','{count} gespeicherte Szenarien für dieses Projekt. Import zeigt eine Vorschau; Speichern legt sie in diesem Browser ab.',{count:stored.items.length}));
}
function scenarioOpen(){if(isRepositoryOverview())return;closeWorkspaceMenu('refactor-menu',true);scenarioRender();document.getElementById('scenario-dialog').showModal();document.getElementById('scenario-name').focus();}
function scenarioCanReload(){return !scenarioPending&&!document.getElementById('scenario-dialog').open;}
function cancelScenarioAction(){
  if(!scenarioPending)return;scenarioPending=null;
  document.getElementById('action-title').textContent=t('Confirm simulation','Simulation bestätigen');
  document.getElementById('confirm-action').textContent=t('Apply simulation','Simulation anwenden');
}
function scenarioConfirm(description,action){
  scenarioPending=action;ui.pending={scenario_action:true};pendingRepair=null;ui.previousFocus=document.activeElement;
  document.getElementById('action-title').textContent=t('Confirm scenario action','Szenarioaktion bestätigen');
  document.getElementById('action-description').textContent=description+t(' Source files remain unchanged.',' Quelldateien bleiben unverändert.');
  document.getElementById('confirm-action').textContent=t('Confirm','Bestätigen');
  document.getElementById('action-dialog').showModal();document.getElementById('cancel-action').focus();
}
function confirmScenarioAction(){
  if(!scenarioPending)return false;const action=scenarioPending;cancelScenarioAction();closeDialog();
  try{action();}catch(error){scenarioStatus(error.message);feedback(error.message);}return true;
}
function scenarioProbe(document){
  const probe=baseline();
  for(const [index,op] of document.ops.entries()){
    const count=probe.edges.length;
    try{applyOp(probe,op);if(op.cut_edge&&probe.edges.length===count)throw Error(t('No dependency matches this cut.','Zu dieser Trennung passt keine Abhängigkeit.'));}
    catch(error){throw Error(t('Scenario change {index}: {reason}','Szenarioänderung {index}: {reason}',{index:index+1,reason:error.message}));}
  }
  const byName=new Map(probe.nodes.map(node=>[node.name,node.id])),cut=new Set();
  for(const name of document.scope.nodes){if(!byName.has(name))throw Error(t('Saved scope node is missing: {name}','Gespeicherter Umfangsknoten fehlt: {name}',{name}));cut.add(byName.get(name));}
  if(!cut.size&&probe.nodes.length)throw Error(t('The saved analysis scope is empty.','Der gespeicherte Analyseumfang ist leer.'));
  for(const id of cut)if(ancestorsOf(probe,id).some(parent=>cut.has(parent)))throw Error(t('Saved scope nodes overlap.','Gespeicherte Umfangsknoten überschneiden sich.'));
  return cut;
}
async function scenarioRequestLoad(value){
  if(isRepositoryOverview())return false;
  try{
    const snapshot=scenarioValidate(value);scenarioProbe(snapshot);const source=await scenarioSource();
    const foreign=snapshot.project!==scenarioProject(),stale=source.fingerprint!==snapshot.source.fingerprint;
    const description=t('Load “{name}” with {count} changes and replace {current} current changes? The saved analysis scope will be restored.','„{name}“ mit {count} Änderungen laden und {current} aktuelle Änderungen ersetzen? Der gespeicherte Analyseumfang wird wiederhergestellt.',{name:snapshot.name,count:snapshot.ops.length,current:ops.length})
      +(foreign?t(' This scenario comes from a different project location. Confirm only if this is the intended checkout.',' Dieses Szenario stammt aus einem anderen Projektpfad. Bestätige nur, wenn dies die gewünschte Arbeitskopie ist.'):'')
      +(stale?t(' The dependency graph or its evidence has changed. All changes were validated against the current graph; no old KPI results are reused.',' Der Abhängigkeitsgraph oder seine Belege haben sich geändert. Alle Änderungen wurden am aktuellen Graphen geprüft; alte KPI-Ergebnisse werden nicht übernommen.'):'');
    scenarioConfirm(description,()=>{
      const cut=scenarioProbe(snapshot); // check again at the actual commit boundary
      ops=snapshot.ops.map(op=>op.cut_edge?encodeCut(op.cut_edge.from,op.cut_edge.to):encodeMove(op.move_node.node,op.move_node.new_parent));
      opBatches=ops.length?[ops.length]:[];appliedRepair=null;repairComparison=null;pendingRepair=null;
      scenarioBaseCut=cut;trail=[];treeBaseCollapses.clear();treeLocation=null;navigationAtRoot=false;isolated=null;ui.cycle=null;
      ui.simulation=true;document.getElementById('simulation-mode').setAttribute('aria-pressed','true');
      recompute();renderAll(false);
      document.getElementById('scenario-name').value=snapshot.name;document.getElementById('scenario-description').value=snapshot.description;
      scenarioStatus(t('Scenario “{name}” loaded as a graph preview. Save explicitly to keep it here.','Szenario „{name}“ als Graphvorschau geladen. Zum Aufbewahren hier ausdrücklich speichern.',{name:snapshot.name}));
      feedback(t('Scenario preview loaded; source files were not edited.','Szenariovorschau geladen; Quelldateien wurden nicht bearbeitet.'));
    });return true;
  }catch(error){scenarioStatus(error.message);feedback(error.message);return false;}
}
async function scenarioSaveCurrent(){
  try{
    const snapshot=await scenarioSnapshot(document.getElementById('scenario-name').value,document.getElementById('scenario-description').value),stored=scenarioReadStore();
    if(stored.error){scenarioStatus(stored.error);return false;}
    const confirmOverwrite=existing=>{
      const reviewedVersion=JSON.stringify(existing),description=existing.description.slice(0,160)+(existing.description.length>160?'…':'');
      scenarioConfirm(t('Replace the saved scenario “{name}” with the current {count} changes? Saved version: {savedCount} changes. {description}','Gespeichertes Szenario „{name}“ durch die aktuellen {count} Änderungen ersetzen? Gespeicherte Version: {savedCount} Änderungen. {description}',{name:snapshot.name,count:snapshot.ops.length,savedCount:existing.ops.length,description}),()=>save(reviewedVersion));return true;
    };
    const save=(reviewedVersion=null)=>{
      const latest=scenarioReadStore();if(latest.error){scenarioStatus(latest.error);return false;}
      const existing=latest.items.find(item=>item.name===snapshot.name);
      // Another tab may create or replace the same name after the initial read
      // or while its confirmation is open. Approval belongs to that version.
      if(existing&&JSON.stringify(existing)!==reviewedVersion)return confirmOverwrite(existing);
      if(!scenarioPersist([...latest.items.filter(item=>item.name!==snapshot.name),snapshot]))return false;
      scenarioRender();document.getElementById('scenario-list').value=snapshot.name;scenarioSelectionChanged();scenarioStatus(t('Saved “{name}” in this browser. Export JSON for version control or another browser.','„{name}“ in diesem Browser gespeichert. Für Versionskontrolle oder einen anderen Browser JSON exportieren.',{name:snapshot.name}));return true;
    };
    const existing=stored.items.find(item=>item.name===snapshot.name);if(existing)return confirmOverwrite(existing);
    return save();
  }catch(error){scenarioStatus(error.message);return false;}
}
async function scenarioImportText(text){
  try{if(typeof text!=='string'||scenarioBytes(text)>SCENARIO_MAX_BYTES)throw Error(t('Scenario JSON is too large (maximum 1 MiB).','Szenario-JSON ist zu groß (höchstens 1 MiB).'));return await scenarioRequestLoad(scenarioValidate(JSON.parse(text)));}
  catch(error){scenarioStatus(t('Could not import scenario: {reason}','Szenario konnte nicht importiert werden: {reason}',{reason:error.message}));return false;}
}
async function scenarioExportCurrent(){
  try{
    const snapshot=await scenarioSnapshot(document.getElementById('scenario-name').value,document.getElementById('scenario-description').value),text=JSON.stringify(snapshot,null,2)+'\n';
    if(scenarioBytes(text)>SCENARIO_MAX_BYTES)throw Error(t('Scenario JSON exceeds 1 MiB. Reduce the scope or number of changes.','Szenario-JSON überschreitet 1 MiB. Umfang oder Zahl der Änderungen reduzieren.'));
    const blob=new Blob([text],{type:'application/json'}),link=element('a');link.href=URL.createObjectURL(blob);link.download='archguard-scenario.json';link.click();URL.revokeObjectURL(link.href);
    scenarioStatus(t('Scenario exported. Keep this JSON in version control to share or review it.','Szenario exportiert. Diese JSON-Datei lässt sich über Versionskontrolle teilen und prüfen.'));return true;
  }catch(error){scenarioStatus(error.message);return false;}
}
function scenarioSelectionChanged(){
  const selected=document.getElementById('scenario-list').value,item=scenarioReadStore().items.find(item=>item.name===selected);
  for(const id of ['scenario-load','scenario-delete'])document.getElementById(id).disabled=!item;
  if(item){document.getElementById('scenario-name').value=item.name;document.getElementById('scenario-description').value=item.description;}
}
function scenarioDeleteSelected(){
  const name=document.getElementById('scenario-list').value,stored=scenarioReadStore();if(!stored.items.some(item=>item.name===name))return;
  scenarioConfirm(t('Delete the saved scenario “{name}” from this browser? The active preview is kept.','Gespeichertes Szenario „{name}“ aus diesem Browser löschen? Die aktive Vorschau bleibt erhalten.',{name}),()=>{const latest=scenarioReadStore();if(latest.error){scenarioStatus(latest.error);return;}if(scenarioPersist(latest.items.filter(item=>item.name!==name)))scenarioRender();});
}
document.getElementById('scenario-open').addEventListener('click',scenarioOpen);
document.getElementById('scenario-close').addEventListener('click',()=>{document.getElementById('scenario-dialog').close();document.getElementById('refactor-summary').focus();});
document.getElementById('scenario-dialog').addEventListener('close',()=>document.getElementById('refactor-summary').focus());
document.getElementById('scenario-save').addEventListener('click',scenarioSaveCurrent);
document.getElementById('scenario-list').addEventListener('change',scenarioSelectionChanged);
document.getElementById('scenario-load').addEventListener('click',()=>{const selected=document.getElementById('scenario-list').value,item=scenarioReadStore().items.find(item=>item.name===selected);if(item)scenarioRequestLoad(item);});
document.getElementById('scenario-delete').addEventListener('click',scenarioDeleteSelected);
document.getElementById('scenario-export').addEventListener('click',scenarioExportCurrent);
document.getElementById('scenario-import').addEventListener('click',()=>document.getElementById('scenario-file').click());
document.getElementById('scenario-file').addEventListener('change',async event=>{
  const file=event.target.files?.[0];event.target.value='';if(!file)return;
  if(file.size>SCENARIO_MAX_BYTES){scenarioStatus(t('Scenario JSON is too large (maximum 1 MiB).','Szenario-JSON ist zu groß (höchstens 1 MiB).'));return;}
  try{await scenarioImportText(await file.text());}catch(error){scenarioStatus(t('Could not read scenario file: {reason}','Szenariodatei konnte nicht gelesen werden: {reason}',{reason:error.message}));}
});
for(const language of ['de','en'])document.getElementById('language-'+language).addEventListener('click',()=>{if(document.getElementById('scenario-dialog').open)scenarioRender();});
