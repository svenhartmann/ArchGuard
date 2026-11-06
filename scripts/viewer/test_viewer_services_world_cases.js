// The services World ("Your services") on real topologies: shown islands and visible
// connections per filter. The cases are shared with the desktop app
// (crates/archguard-desktop/src/start_ui/services/agreement.rs), so both read the same answers.
// node scripts/viewer/test_viewer_services_world_cases.js [archguard binary]
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const path=require('node:path'),{execFileSync}=require('node:child_process');
const root=path.resolve(__dirname,'../..'),binary=process.argv[2]||path.join(root,'target/debug/archguard');
const harness=fs.readFileSync(path.join(root,'scripts/viewer/test_viewer_state.js'),'utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const shared=JSON.parse(fs.readFileSync(path.join(root,'scripts/viewer/services_world_cases.json'),'utf8'));
let count=0;
for(const entry of shared){
  const topology=JSON.parse(execFileSync(binary,['services',path.join(root,entry.topology),'--format','json'],{encoding:'utf8'}));
  const repository={mode:'overview',root:topology.root,active_project:null,projects:topology.projects,excluded_projects:[]};
  const app=launch({root:topology.root,nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{repository},services:topology},{live:true,location:{protocol:'http:',href:'http://localhost:8123/'}});
  assert.equal(app.run('repositoryWorldState.deployment'),entry.default_deployment,entry.topology+': default deployment variant');
  for(const c of entry.cases){
    const state={deployment:c.deployment,source:c.source,kind:c.kind,direction:c.direction,cyclesOnly:c.cycles_only,allProjects:c.all_projects};
    app.run('Object.assign(repositoryWorldState,'+JSON.stringify(state)+');worldUi.selection='+(c.selected===null?'null':JSON.stringify({area:c.selected})));
    assert.deepEqual(JSON.parse(app.run('JSON.stringify(repositoryWorldProjects().map(p=>p.path))')),c.islands,entry.topology+': '+c.case+' (islands)');
    assert.deepEqual(JSON.parse(app.run('JSON.stringify(repositoryVisibleConnections().map(e=>[e.from,e.to,e.kind,e.cyclic,e.evidence.length]))')),c.connections,entry.topology+': '+c.case+' (connections)');
    count++;
  }
}
console.log('Services World shared cases passed: '+count+' cases on '+shared.length+' topologies');
