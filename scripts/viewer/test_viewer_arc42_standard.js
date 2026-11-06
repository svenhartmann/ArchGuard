const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const base=fs.readFileSync('scripts/viewer/test_viewer_arc42_diagrams.js','utf8').split('const app=launch')[0];
const {launch,fixture}=vm.runInNewContext(base+'\n({launch,fixture})',{require,console,performance,Blob});
const app=launch(fixture,{library:false});
assert.equal(app.run('typeof arc42StandardGraph'),'function','standard diagram model replaces hand-positioned rows');
const model=JSON.parse(app.run('JSON.stringify(arc42StandardGraph(arc42PackageProjection(),null))'));
assert.equal(model.graphAttributes.rankdir,'TB');assert.equal(model.nodes.length,12);assert.ok(model.nodes.some(n=>n.attributes.shape==='folder'));assert.ok(model.edges.some(e=>e.attributes.color==='#ee7d87'));
// Counts stay on every edge as external labels: Graphviz places those after
// layout, while edge `label`s become layout nodes (Elasticsearch: 41 s vs 1.8 s).
assert.ok(model.edges.every(e=>e.attributes.xlabel&&!('label' in e.attributes)),'edge counts are external labels');
const puml=app.run('arc42PlantUml(arc42PackageProjection())');assert.match(puml,/@startuml/);assert.match(puml,/package "/);assert.match(puml,/#ee7d87/);assert.doesNotMatch(puml,/!include/);
const deployment=app.run("arc42PlantUml(arc42DeploymentProjection('dev.yml'))");assert.match(deployment,/Deployment_Node/);assert.match(deployment,/Container/);assert.match(deployment,/startup/);assert.doesNotMatch(deployment,/prod.yml/);
app.run('let evil={nodes:[{id:"x",name:\'quote"\\n!include https://evil.test\\n@enduml\',kind:"package"}],edges:[]};');
assert.equal(app.run('arc42PlantUml(evil).split("\\n").filter(l=>l.startsWith("@enduml")).length'),1);assert.ok(!app.run('arc42PlantUml(evil).split("\\n").some(l=>l.startsWith("!include"))'));
// Rust crates and Maven modules all end in src/main: labels need the shortest unique path suffix.
const twins=JSON.parse(app.run("JSON.stringify(arc42StandardGraph({nodes:[{id:'a',name:'crates/engine/src',kind:'package'},{id:'b',name:'crates/cli/src',kind:'package'},{id:'c',name:'crates/cli',kind:'package'}],edges:[]},null))"));
assert.deepEqual(twins.nodes.map(n=>n.attributes.label.split('\n')[0]),['engine/src','cli/src','cli'],'packages sharing a last segment stay distinguishable');
assert.equal(app.run("arc42ShortNames(['x/a/src','y/a/src','src']).get('x/a/src')"),'x/a/src');
// viz-js copies a subgraph label onto the root unless the root declares its own: the title appeared twice.
const deploymentGraph=JSON.parse(app.run("JSON.stringify(arc42StandardGraph(arc42DeploymentProjection('dev.yml'),null))"));
assert.equal(deploymentGraph.graphAttributes.label,'','the deployment title is drawn once, by its cluster');
// Without relationships dot puts every package into one rank: one unreadable row.
const loose=JSON.parse(app.run("JSON.stringify(arc42StandardGraph({nodes:Array.from({length:9},(_,i)=>({id:'p'+i,name:'p'+i,kind:'package'})),edges:[{from:'p0',to:'p1',count:3}]},null))"));
const spacers=loose.edges.filter(e=>e.attributes.style==='invis');
assert.equal(spacers.length,4,'seven unconnected packages stack into rows of three via invisible edges');
assert.ok(spacers.every(e=>!e.attributes.id&&!e.attributes.xlabel),'layout-only edges carry no count and no selectable id');
assert.ok(!('pack' in loose.graphAttributes),'Graphviz packing drops xlabels, so edge counts would disappear');
assert.equal(loose.edges.filter(e=>e.attributes.xlabel).length,1,'the real relationship keeps its count');
// Colours follow the light/dark theme instead of a fixed dark palette.
app.run("getComputedStyle=()=>({getPropertyValue:k=>({'--panel':'#FFFFFF','--text':'#172033','--muted':'#5B6472','--accent':'#2563EB'})[k]||''})");
const light=JSON.parse(app.run('JSON.stringify(arc42StandardGraph(arc42PackageProjection(),null))'));
assert.equal(light.nodeAttributes.fillcolor,'#FFFFFF');assert.equal(light.nodeAttributes.fontcolor,'#172033');assert.equal(light.edgeAttributes.fontcolor,'#5B6472');
const lightDeployment=JSON.parse(app.run("JSON.stringify(arc42StandardGraph(arc42DeploymentProjection('dev.yml'),null))"));
assert.ok(lightDeployment.edges.length&&lightDeployment.edges.every(e=>e.attributes.color==='#2563EB'),'acyclic edges use the theme accent');
app.run("getComputedStyle=()=>({getPropertyValue:()=>''})");
const before=app.run('JSON.stringify(data)');app.run('arc42StandardGraph(arc42PackageProjection(),null);arc42PlantUml(arc42PackageProjection())');assert.equal(app.run('JSON.stringify(data)'),before);
console.log('arc42 standards: UML packages, directed counts and cycles, isolated C4 deployment, safe PlantUML and immutable evidence passed');
