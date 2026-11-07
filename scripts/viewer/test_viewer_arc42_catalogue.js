// The arc42 diagram catalogue (ADR 0006) in the web reader: generated sections are drawn,
// not shown as source; chapter 7 comes from the catalogue; ticks change only the view and
// yield the [arc42] snippet; the Diagrams view lists the whole catalogue.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
// As arc42_diagrams::section writes it.
const section=(d,source)=>'### '+d.title+'\n\n'+d.note+'\n\n```mermaid\n'+source+'```\n\n';
const deployment={id:'deployment-compose.yaml',chapter:7,title:'C4 · Deployment · compose.yaml',kind:'deployment',note:'Quelle: deklarierte Deployment-Konfiguration.',unit:'Konfigurationsbelege',shown:true,
  elements:[{id:'e0',name:'billing',kind:'container',group:'compose.yaml',cycle:false},{id:'e1',name:'db',kind:'database',technology:'postgresql',group:'compose.yaml',cycle:false},{id:'e2',name:'orders',kind:'container',group:'compose.yaml',cycle:false},{id:'e3',name:'mail',kind:'external',cycle:false}],
  relations:[{from:'e2',to:'e0',kind:'http',count:1,cycle:false},{from:'e2',to:'e1',kind:'startup',count:1,cycle:false},{from:'e0',to:'e1',kind:'data',count:2,cycle:false}]};
const hidden={...deployment,id:'deployment-prod.yml',title:'C4 · Deployment · prod.yml',shown:false,elements:deployment.elements.slice(0,2),relations:[]};
const services={id:'runtime-services',chapter:6,title:'C4 · Kommunikation zwischen Diensten',kind:'communication',note:'Quelle: konfigurierte Verbindungen.',unit:'Konfigurationsbelege',shown:true,
  elements:[{id:'e0',name:'billing',kind:'container',cycle:false},{id:'e1',name:'orders',kind:'container',cycle:false}],relations:[{from:'e1',to:'e0',kind:'grpc',count:3,cycle:false}]};
const blocks={id:'building-blocks',chapter:5,title:'Paketübersicht',kind:'packages',note:'Quelle: vollständiger Quellgraph.',unit:'Quellverbindungen',shown:true,
  elements:[{id:'e0',name:'org/a',kind:'package',cycle:false},{id:'e1',name:'org/b',kind:'package',cycle:false}],relations:[{from:'e0',to:'e1',kind:'dependency',count:1,cycle:false}]};
const sources={
  5:section(blocks,'flowchart LR\n  e0["org/a"]\n  e1["org/b"]\n  e0 -->|1| e1\n'),
  6:section(services,'C4Container\n  title C4 · Kommunikation zwischen Diensten\n  Container(e0, "billing", "Dienst", "")\n  Rel(e1, e0, "gRPC", "3 Konfigurationsbelege")\n'),
  7:section(deployment,'C4Deployment\n  title C4 · Deployment · compose.yaml\n  Deployment_Node(g0, "compose.yaml", "Docker Compose") {\n    Container(e0, "billing", "Dienst", "")\n  }\n  Rel(e2, e0, "HTTP", "1 Konfigurationsbeleg")\n')};
const titles={5:'Bausteinsicht',6:'Laufzeitsicht',7:'Verteilungssicht'};
// As review_cmd.rs embeds a chapter: preamble, heading, generated block.
const chapter=n=>({number:n,title:titles[n],questions:[],markdown:'# arc42\n\nPreamble\n\n## '+n+'. '+titles[n]+'\n\n<!-- archguard:arc42:0'+n+':start -->\n'+sources[n]+'### Rest '+n+'\n\nKept text '+n+'.\n<!-- archguard:arc42:0'+n+':end -->\n'});
const nodes=[{id:0,name:'org',kind:'module',parent:null},{id:1,name:'org/a',kind:'module',parent:0},{id:2,name:'org/a/A.java',kind:'file',parent:1},{id:3,name:'org/b',kind:'module',parent:0},{id:4,name:'org/b/B.java',kind:'file',parent:3}];
const composeServices={services:[{id:'dev::api',name:'api',compose_file:'dev.yml',networks:[],profiles:[]}],dependencies:[],projects:[],cycles:[],complete:true,diagnostics:[]};
const fixture={root:'/repo',nodes,edges:[{from:2,to:4,count:1,refs:[]}],view:{depth:0,visible:null},findings:[],cyclicEdges:[],services:composeServices,
  insights:{arc42:{schema_version:1,scope:'/repo',markdown:'',chapters:[5,6,7].map(chapter),diagrams:[blocks,services,deployment,hidden]}}};
const app=launch(fixture,{library:false});app.run("showTab('architecture')");
const root=app.ids.get('architecture-content'),article=()=>root.querySelector('article[data-arc42-chapter]');
const open=key=>root.querySelectorAll('button[data-arc42-nav]').find(b=>b.getAttribute('data-arc42-nav')===key).click();
const figure=id=>article().querySelectorAll('section[data-arc42-diagram]').find(s=>s.getAttribute('data-arc42-diagram')===id);
const tick=id=>article().querySelectorAll('input[data-arc42-tick]').find(n=>n.getAttribute('data-arc42-tick')===id);
const toggle=(id,on)=>{const box=tick(id);box.checked=on;box.dispatch('change');};
const snippet=()=>article().querySelector('pre[data-arc42-snippet]')?.textContent;

// Chapter 7 draws the catalogue, one figure per deployment file, not data.services.
open('7');
assert.doesNotMatch(article().textContent,/C4Deployment|Deployment_Node/,'the generated C4 source is cut from the chapter text');
assert.doesNotMatch(article().textContent,/Diagram source/,'no "unsupported syntax" note for catalogue diagrams');
assert.match(article().textContent,/Kept text 7\./,'the rest of the chapter stays');
assert.equal(article().querySelector('select[data-arc42-deployment]'),null,'the catalogue replaces the projection from data.services');
assert.equal(figure('deployment-compose.yaml').querySelectorAll('g[data-arc42-node]').length,4);
assert.equal(figure('deployment-compose.yaml').querySelectorAll('g[data-arc42-edge]').length,3);
assert.equal(tick('deployment-compose.yaml').checked,true,'ticks start from the shown flag');
assert.equal(figure('deployment-prod.yml').getAttribute('data-arc42-hidden'),'true','a hidden diagram is one line with its tick');
assert.equal(figure('deployment-prod.yml').querySelectorAll('svg').length,0);assert.equal(tick('deployment-prod.yml').checked,false);
assert.equal(snippet(),undefined,'no snippet before the choice differs from archguard.toml');

// A tick only changes this view and yields the snippet for archguard.toml.
toggle('deployment-compose.yaml',false);
assert.equal(figure('deployment-compose.yaml').getAttribute('data-arc42-hidden'),'true');
assert.equal(figure('deployment-compose.yaml').querySelectorAll('svg').length,0);
assert.equal(snippet(),'[arc42]\nhidden_diagrams = ["deployment-compose.yaml", "deployment-prod.yml"]');
toggle('deployment-prod.yml',true);
assert.equal(figure('deployment-prod.yml').querySelectorAll('g[data-arc42-node]').length,2,'a ticked hidden diagram is drawn from the catalogue');
assert.equal(snippet(),'[arc42]\nhidden_diagrams = ["deployment-compose.yaml"]');
assert.equal(app.run('JSON.stringify(data.insights.arc42.diagrams.map(d=>d.shown))'),'[true,true,true,false]','the report data stays as generated');
toggle('deployment-compose.yaml',true);toggle('deployment-prod.yml',false);assert.equal(snippet(),undefined);

// Chapter 6 draws the communication diagram; chapter 5 keeps its package map with a tick.
open('6');assert.doesNotMatch(article().textContent,/C4Container/);assert.ok(figure('runtime-services').querySelector('svg[data-arc42-graph]'));
open('5');assert.doesNotMatch(article().textContent,/flowchart LR/);
assert.ok(figure('building-blocks').querySelector('section[data-arc42-package-map]'),'the derived package map carries the building-blocks tick');
toggle('building-blocks',false);assert.equal(article().querySelector('section[data-arc42-package-map]'),null);
assert.match(snippet(),/hidden_diagrams = \["building-blocks", "deployment-prod.yml"\]/);
toggle('building-blocks',true);

// The Diagrams view lists the whole catalogue with preview, source and tick.
open('diagrams');
assert.equal(article().getAttribute('data-arc42-chapter'),'diagrams');
assert.deepEqual([...article().querySelectorAll('section[data-arc42-diagram]')].map(s=>s.getAttribute('data-arc42-diagram')),['building-blocks','runtime-services','deployment-compose.yaml','deployment-prod.yml']);
assert.equal(article().querySelectorAll('input[data-arc42-tick]').length,4);
assert.ok(figure('deployment-prod.yml').querySelector('svg[data-arc42-graph]'),'hidden diagrams get a preview here');
const source=article().querySelectorAll('pre[data-arc42-diagram-source]').find(p=>p.getAttribute('data-arc42-diagram-source')==='deployment-compose.yaml');
assert.match(source.textContent,/^C4Deployment\n/,'the source is the section the document contains');
assert.match(figure('deployment-prod.yml').textContent,/appears in the chapter after the diagram is enabled/,'hidden diagrams say where their source comes from');
assert.equal(snippet(),'[arc42]\nhidden_diagrams = ["deployment-prod.yml"]','the view always shows the current snippet');
app.click('language-de');assert.match(article().textContent,/In der Dokumentation/);assert.match(article().textContent,/Diagramme/);app.click('language-en');

// Graphviz model: C4 shapes, a dashed cluster per group, one colour per relation kind.
const graph=JSON.parse(app.run("JSON.stringify(arc42StandardGraph(arc42CatalogueModel(data.insights.arc42.diagrams[2]),null))"));
assert.deepEqual(graph.subgraphs.map(s=>[s.graphAttributes.label,s.graphAttributes.style,s.nodes.length]),[['compose.yaml','dashed',3]]);
assert.deepEqual(graph.nodes.map(n=>n.attributes.style),['filled,dashed'],'the ungrouped external container is dashed');
assert.equal(graph.subgraphs[0].nodes[1].attributes.shape,'cylinder');assert.match(graph.subgraphs[0].nodes[1].attributes.label,/Database · postgresql/);
const [http,startup,data]=graph.edges.map(e=>e.attributes);
assert.equal(new Set([http.color,startup.color,data.color]).size,3);assert.equal(startup.style,'dashed');assert.equal(http.style,'solid');
assert.match(http.xlabel,/^HTTP · 1$/);
const puml=app.run("arc42PlantUml(arc42CatalogueModel(data.insights.arc42.diagrams[2]))");
assert.match(puml,/ContainerDb\(n1, "db", "postgresql"\)/);assert.match(puml,/Container_Ext\(n3, "mail"/);assert.match(puml,/Deployment_Node\(g0, "compose.yaml"/);

// Reports without a catalogue keep chapter 7's projection and have no Diagrams entry.
const old=launch({...fixture,insights:{arc42:{chapters:[5,6,7].map(chapter)}}},{library:false});old.run("showTab('architecture')");
const oldRoot=old.ids.get('architecture-content');oldRoot.querySelectorAll('button[data-arc42-nav]').find(b=>b.getAttribute('data-arc42-nav')==='7').click();
assert.ok(oldRoot.querySelector('select[data-arc42-deployment]'));
assert.ok(!oldRoot.querySelectorAll('button[data-arc42-nav]').some(b=>b.getAttribute('data-arc42-nav')==='diagrams'));
assert.equal(oldRoot.querySelectorAll('input[data-arc42-tick]').length,0);
// A recorded trace is a sequence: participants, lifelines, messages in order, errors marked.
const trace={id:'runtime-trace-0af76519',chapter:6,title:'Sequenz · POST /api/charge · Trace 0af76519',kind:'sequence',note:'Quelle: aufgezeichneter OTLP-Trace.',unit:'Aufrufe',shown:true,
  elements:[{id:'e0',name:'orders',kind:'container',cycle:false},{id:'e1',name:'billing',kind:'container',cycle:false},{id:'e2',name:'shipping',kind:'container',cycle:false}],
  relations:[{from:'e0',to:'e1',kind:'http',count:1,cycle:false,label:'1. HTTP POST /api/charge · 42.0 ms'},{from:'e1',to:'e2',kind:'http',count:1,cycle:false,error:true,label:'2. HTTP POST /api/ship · 25.0 ms · Fehler'}]};
const traced={...fixture,insights:{arc42:{schema_version:1,scope:'/repo',chapters:[{...chapter(6),markdown:chapter(6).markdown.replace(sources[6],section(trace,'sequenceDiagram\n  participant e0 as orders\n'))}],diagrams:[trace]}}};
const seq=launch(traced,{library:false});seq.run("showTab('architecture')");
const seqRoot=seq.ids.get('architecture-content');seqRoot.querySelectorAll('button[data-arc42-nav]').find(b=>b.getAttribute('data-arc42-nav')==='6').click();
const seqFigure=seqRoot.querySelectorAll('section[data-arc42-diagram]').find(s=>s.getAttribute('data-arc42-diagram')===trace.id);
assert.doesNotMatch(seqRoot.querySelector('article[data-arc42-chapter]').textContent,/sequenceDiagram/,'the sequence source is cut from the chapter text');
assert.ok(seqFigure.querySelector('svg[data-arc42-sequence]'),'a trace is drawn as a sequence, not as C4 boxes');
assert.equal(seqFigure.querySelectorAll('g[data-arc42-participant]').length,3);
const messages=[...seqFigure.querySelectorAll('g[data-arc42-message]')];
assert.deepEqual(messages.map(m=>m.textContent),[trace.relations[0].label,trace.relations[1].label],'messages in recorded order with their labels');
assert.deepEqual(messages.map(m=>m.getAttribute('data-arc42-error')),['false','true']);
console.log('arc42 catalogue: sections cut and drawn, chapter 7 from the catalogue, ticks with snippet, Diagrams view, C4 styling and trace sequences passed');
