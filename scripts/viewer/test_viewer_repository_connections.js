// Repository overview must not imply that unrelated project sources were analyzed together.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const project=(path,kind='application')=>({id:'project:'+path+'/pom.xml',name:'search',path,manifest:path+'/pom.xml',ecosystem:'maven',kind,runtime:kind==='application'?'azure-functions':null,evidence:[]});
const projects=[project('functions/search'),project('functions/import data'),project('tests/browser','tooling'),project('shared/core','library'),project('parent','workspace'),project('.', 'library')];
const services=projects.filter(p=>p.kind==='application').map(p=>({id:p.id,project:p.id,name:p.name,compose_file:'',build_context:p.path,networks:[],profiles:[]}));
const repository={mode:'overview',root:"/repo/company's search",active_project:null,projects,excluded_projects:[]};
const fixture={root:repository.root,nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},initialTab:'calls',insights:{repository},services:{schema_version:1,root:repository.root,monorepo:true,monorepo_reason:'Multiple projects',complete:true,projects,services,compose_files:[],dependencies:[],cycles:[],diagnostics:[]}};

fixture.services.dependencies=[{from:services[0].id,to:services[1].id,kind:'event',evidence:[{file:'Publisher.cs',line:12,description:'OrderCreated'}]},{from:services[1].id,to:services[0].id,kind:'event',evidence:[{file:'Subscriber.cs',line:3,description:'OrderConfirmed'}]}];
fixture.services.cycles=[{kind:'event',nodes:services.map(s=>s.id)}];
const app=launch(fixture,{live:true,location:{protocol:'http:',href:'http://localhost:8123/'}});
assert.equal(app.run('repositoryWorldConnections().length'),2);
// Service islands stay apart so arrows and fixed-size labels remain readable.
app.run("repositoryWorldState.allProjects=true;renderRepositoryWorld()");
assert.ok(app.run('worldUi.layout.areas.length')>1);
assert.equal(app.run('worldUi.layout.areas[1].x-worldUi.layout.areas[0].x-worldUi.layout.areas[0].w'),3,'neighbouring service islands keep a gap of three tiles');
app.run("repositoryWorldState.allProjects=false;renderRepositoryWorld()");
assert.equal(app.run('repositoryWorldConnections().every(e=>e.cyclic)'),true);
app.run("repositorySelectWorld('functions/search')");
assert.equal(app.run('worldUi.selection.area'),'functions/search');
assert.equal(app.run('repositoryVisibleConnections().length'),2);
app.run("repositoryWorldState.direction='incoming'");
assert.equal(app.run('repositoryVisibleConnections().length'),1);
app.run("repositoryWorldState.kind='http_call'");
assert.equal(app.run('repositoryVisibleConnections().length'),0);
app.run("repositoryWorldState.kind='all';repositoryWorldState.direction='both';worldUi.detailsOpen=true;worldDetails()");
assert.match(app.ids.get('world-detail').textContent,/OrderCreated/);
assert.match(app.ids.get('world-detail').textContent,/Publisher.cs/);
assert.match(app.ids.get('world-detail').textContent,/Zoom into service/);
console.log('Repository communication projection, selection, direction, type filters and evidence passed');
app.run(`data.services.dependencies=[
 {from:data.services.services[0].id,to:data.services.services[1].id,kind:'http_call',evidence:[{file:'Client.java',pointer:'spring-client',description:'static client'}]},
 {from:data.services.services[1].id,to:data.services.services[0].id,kind:'http_call',evidence:[{file:'.archguard/otel-traces.json',pointer:'/resourceSpans/0',description:'observed calls=2'}]}
];repositoryWorldState.source='code'`);
assert.equal(app.run('repositoryWorldConnections().length'),1);
assert.equal(app.run('repositoryWorldConnections()[0].cyclic'),false,'the reverse trace edge cannot invent a source-code cycle');
app.run("repositoryWorldState.source='observed'");
assert.equal(app.run('repositoryWorldConnections().length'),1);
assert.equal(app.run('repositoryWorldConnections()[0].cyclic'),false);
app.run("repositoryWorldState.source='all'");
assert.equal(app.run('repositoryWorldConnections().every(e=>e.cyclic)'),true,'combined view explicitly combines evidence');
app.run(`data.services.services.push({id:'trace:external',name:'external',project:null,compose_file:'',profiles:[],networks:[]});data.services.dependencies.push({from:data.services.services[0].id,to:'trace:external',kind:'http_call',evidence:[{file:'.archguard/otel-traces.json',pointer:'/resourceSpans/1',description:'observed calls=1'}]})`);
assert.ok(app.run("repositoryWorldInventory().some(p=>p.external&&p.name==='external')"));
assert.equal(app.run('repositoryWorldConnections().length'),3);
console.log('Source-specific cycles, recorded trace evidence and external service islands passed');
app.run(`data.services.services.push({id:'deploy:second',name:'search-replica',project:data.services.services[0].project,compose_file:'compose.yaml',profiles:[],networks:[]});data.services.dependencies.push({from:data.services.services[0].id,to:'deploy:second',kind:'http_call',evidence:[{file:'Client.go',description:'call between deployments'}]})`);
assert.equal(app.run("repositoryWorldInventory().filter(p=>p.sourcePath==='functions/search').length"),2,'deployments sharing sources keep separate islands');
assert.ok(app.run("repositoryWorldConnections().some(e=>e.serviceTo==='deploy:second')"),'deployment relationship must not disappear into a self edge');
app.run("data.services.complete=false; repositoryWorldState.kind='event'; repositoryWorldState.source='observed'; repositoryWorldControls()");
assert.match(app.ids.get('world-status').textContent,/incomplete|unvollständig/i);
console.log('Deployment identity and incomplete-analysis notice passed');
const variants=structuredClone(fixture);
variants.services.services=[{id:'dev::search',name:'search',project:projects[0].id,compose_file:'dev/compose.yaml',networks:[]},{id:'prod::search',name:'search',project:projects[0].id,compose_file:'prod/compose.yaml',networks:[]}];
variants.services.dependencies=[];
const grouped=launch(variants,{live:true,location:{protocol:'http:',href:'http://localhost:8123/'}});
assert.equal(grouped.run('repositoryDeploymentScopes().length'),2);
grouped.run("repositoryWorldState.deployment='prod/compose.yaml';renderRepositoryWorld()");
assert.equal(grouped.run('worldUi.model.areas.length'),1);
assert.equal(grouped.run('worldUi.model.areas[0].project.id'),'prod::search');
assert.equal(grouped.run('worldUi.model.areas[0].project.sourcePath'),'functions/search');
console.log('Deployment selector isolates variants and retains source drilldown');
// The data-flow button animates packets on demand and only for call or event edges.
let frames=[];
const drain=n=>{for(let i=0;i<n&&frames.length;i++){const cb=frames.shift();if(cb)cb(i*16);}};
const launchFlow=(options={})=>{
  const flowLaunch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
  const flow=flowLaunch(fixture,{live:true,location:{protocol:'http:',href:'http://localhost:8123/'},...options});
  flow.context.requestAnimationFrame=cb=>{frames.push(cb);return frames.length;};
  flow.context.cancelAnimationFrame=id=>{frames[id-1]=null;};
  const arcs=[];
  const canvas=flow.ids.get('world-canvas');canvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
  canvas.getContext=()=>new Proxy({arc(...args){arcs.push(args);}},{get(target,key){return target[key]??(()=>{});},set(target,key,value){target[key]=value;return true;}});
  flow.run("showTab('start');renderStart()");frames=[];
  return {flow,arcs};
};
const {flow,arcs}=launchFlow();
const legendButton=app=>app.ids.get('world-legend').children.find(c=>c.id==='world-service-flow');
const flowButton=()=>legendButton(flow);
assert.equal(flowButton().getAttribute('aria-pressed'),'false','data flow stays off until requested');
drain(5);assert.equal(frames.length,0,'no animation runs while data flow is off');
flowButton().click();
assert.equal(flowButton().getAttribute('aria-pressed'),'true');
assert.match(flow.ids.get('world-status').textContent,/not measured load|keine gemessene Last/);
assert.ok(frames.length,'data flow requests animation frames');
arcs.length=0;drain(3);
assert.ok(flow.run('worldUi.flow')>0);assert.ok(arcs.length>0,'packets are drawn on event connections');
flow.run("repositoryWorldState.kind='configured_reference'");arcs.length=0;drain(3);
assert.equal(arcs.length,0,'configuration references carry no packets');
flow.run("repositoryWorldState.kind='all'");
flowButton().click();
assert.equal(flowButton().getAttribute('aria-pressed'),'false');
const stopped=flow.run('worldUi.flow');drain(5);assert.equal(flow.run('worldUi.flow'),stopped,'turning data flow off stops the animation');
assert.equal(frames.length,0);
const reduced=launchFlow({reducedMotion:true});
assert.equal(legendButton(reduced.flow).disabled,true,'reduced motion disables the animation');
console.log('Data-flow button, packet filtering, stop and reduced-motion handling passed');
// A selected service can follow its data over every hop: where it flows and where it comes from.
const chainProjects=['a','b','c','d','e'].map(name=>project('svc/'+name));
const chainServices=chainProjects.map(p=>({id:p.id,project:p.id,name:p.name,compose_file:'',build_context:p.path,networks:[],profiles:[]}));
const id=name=>'project:svc/'+name+'/pom.xml';
const chain=structuredClone(fixture);
chain.insights.repository={...repository,projects:chainProjects};
Object.assign(chain.services,{projects:chainProjects,services:chainServices,cycles:[],dependencies:[
  {from:id('d'),to:id('a'),kind:'grpc_call',evidence:[]},
  {from:id('a'),to:id('b'),kind:'http_call',evidence:[]},
  {from:id('b'),to:id('c'),kind:'event',evidence:[]},
  {from:id('e'),to:id('c'),kind:'http_call',evidence:[]},
  {from:id('c'),to:id('e'),kind:'configured_reference',evidence:[]},
]});
const paths=launch(chain,{live:true,location:{protocol:'http:',href:'http://localhost:8123/'}});
const shown=()=>paths.run("repositoryVisibleConnections().map(e=>e.from.slice(4)+'>'+e.to.slice(4)).sort().join(' ')");
paths.run("repositorySelectWorld('svc/a')");
paths.run("repositoryWorldState.direction='outgoing'");assert.equal(shown(),'a>b','outgoing keeps direct connections');
paths.run("repositoryWorldState.direction='downstream'");assert.equal(shown(),'a>b b>c','downstream follows every hop');
paths.run("repositorySelectWorld('svc/c');repositoryWorldState.direction='upstream'");assert.equal(shown(),'a>b b>c d>a e>c','upstream collects every source');
paths.run("repositoryWorldState.kind='event'");assert.equal(shown(),'b>c','paths respect the connection type filter');
paths.run("repositoryWorldState.kind='all';worldUi.selection=null");assert.equal(shown(),'a>b b>c c>e d>a e>c','without a selection every connection stays visible');
paths.run("repositorySelectWorld('svc/a');repositoryWorldState.direction='downstream';repositoryWorldControls()");
const direction=paths.ids.get('world-legend').children.flatMap(c=>c.children||[]).find(c=>c.id==='world-service-direction');
assert.ok(direction.children.some(o=>o.value==='downstream')&&direction.children.some(o=>o.value==='upstream'),'the direction filter offers both paths');
assert.match(paths.ids.get('world-status').textContent,/2 connections · where the data flows, all hops|2 Verbindungen · wohin die Daten fließen, alle Stationen/);
console.log('Data-flow paths: downstream and upstream over every hop, type filter and status passed');
