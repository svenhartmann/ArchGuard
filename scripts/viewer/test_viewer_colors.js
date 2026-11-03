// Execute the real graph callbacks: cycles color the whole edge, while
// ordinary edges retain relative direction and simulation states stay visible.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const js=fs.readFileSync('crates/archguard-report/src/viewer.js','utf8');
function declaration(name,source=js){
  const start=source.indexOf('function '+name+'(');if(start<0)return '';
  const lineEnd=source.indexOf('\n',start);
  return source.slice(start,source.slice(start,lineEnd).endsWith('}')?lineEnd:source.indexOf('\n}',start)+2);
}
function accessor(name,next){
  const marker='.'+name+'(',start=js.indexOf(marker)+marker.length,end=js.indexOf(next,start);
  assert.ok(start>=marker.length&&end>start,name+' callback exists');
  const expression=js.slice(start,end).trim();assert.ok(expression.endsWith(')'));
  return expression.slice(0,-1);
}
class Element{
  constructor(){this.children=[];this.style={};this._text='';}
  set textContent(value){this._text=String(value);this.children=[];}
  get textContent(){return this._text+this.children.map(c=>c.textContent).join('');}
  appendChild(node){this.children.push(node);return node;}
  setAttribute(key,value){this[key]=String(value);}
}
const legend=new Element();
const context=vm.createContext({
  viewerLanguage:'en',
  data:{},
  ui:{selection:{kind:'node',name:'A'}},selectedId:0,selectedNodeIds:new Set([0]),selectedLinkKey:null,packageContext:null,
  view:{nodes:['A','B','C'].map((name,id)=>({id,src:id,name,kind:'file'})),edges:[]},
  work:{nodes:[...['A','B','C'].map((name,id)=>({id,name,kind:'file',parent:id===0?3:null})),{id:3,name:'Parent/A',kind:'module',parent:null}]},
  C:{edge:'neutral',incoming:'incoming-blue',outgoing:'outgoing-violet',cycle:'cycle-red',introduced:'new-red',resolved:'resolved-green',label:'text'},
  cyclicKeys:new Set(['0 1','1 0']),introducedKeys:new Set(),resolvedKeys:new Set(),
  document:{getElementById:()=>legend,createElement:()=>new Element()},
  dependencyVisible:()=>true,hasPosition:n=>!!n&&Number.isFinite(n.x)&&Number.isFinite(n.y),
  ARROW_GAP:3,arrowFits:()=>true,
  arrowGeometry:()=>({x:10,y:10,angle:0}),curvePoint:()=>({x:100,y:100}),
  element:(_tag,text)=>{const node=new Element();node.textContent=text;return node;},
});
vm.runInContext(declaration('t',fs.readFileSync('crates/archguard-report/src/viewer_i18n.js','utf8')),context);
for(const name of ['graphEvidenceUnit','edgeWeight','widthFor','linkState','linkStrokeColor','linkArrowColor','isCyclic','childrenOf','subtree','nodeByName','directionReference','edgeReference','buildLegend']){
  const code=declaration(name);if(code)vm.runInContext(code,context);
}
vm.runInContext('globalThis.strokeColor='+accessor('linkColor','.linkVisibility')+';',context);
const a={id:0,name:'A',x:0,y:0},b={id:1,name:'B',x:100,y:0},c={id:2,name:'C',x:200,y:0};
const outgoing={source:a,target:b,count:25,refs:[]},incoming={source:b,target:a,count:2,refs:[]},unrelated={source:b,target:c,count:1,refs:[]};
assert.equal(context.strokeColor(outgoing),'cycle-red','a cycle colors the complete outgoing edge, overriding direction');
assert.equal(context.strokeColor(incoming),'cycle-red','the reciprocal cyclic line is also completely red');
assert.equal(context.strokeColor(outgoing,'left'),'outgoing-violet','left-hand tree cycle legs retain outgoing color');
assert.equal(context.linkArrowColor(incoming,'left'),'incoming-blue','left-hand tree cycle arrows retain incoming color');
assert.equal(context.strokeColor(outgoing,'right'),'cycle-red','right-hand tree cycle legs are highlighted');
assert.equal(context.strokeColor(unrelated),'neutral','All mode does not label unrelated edges as incoming or outgoing');
vm.runInContext('globalThis.paintLink='+accessor('linkCanvasObject',';\n\n// double-click')+';',context);
function arrowColor(link){
  const fills=[],canvas=new Proxy({},{get(target,key){if(key==='fill')return ()=>fills.push(target.fillStyle);return target[key]??(()=>{});}});
  context.paintLink(link,canvas,1);
  assert.equal(fills.length,2,'color checks cover both endpoint and on-curve arrows');
  assert.ok(fills.every(color=>color===fills[0]),'all arrowheads retain the same direction/cycle state');
  return fills[0];
}
assert.equal(arrowColor(outgoing),'cycle-red','cycle state remains visible on the outgoing arrowhead');
assert.equal(arrowColor(incoming),'cycle-red','cycle state remains visible on the incoming arrowhead');
context.resolvedKeys.add('0 1');assert.equal(context.strokeColor(outgoing),'resolved-green','resolved status applies to the complete edge even with a selected reference');assert.equal(arrowColor(outgoing),'resolved-green');
for(const side of ['left','right'])assert.equal(context.strokeColor(outgoing,side),'resolved-green','resolved simulation remains green on either tree side');
context.resolvedKeys.clear();context.introducedKeys.add('0 1');assert.equal(context.strokeColor(outgoing),'new-red');assert.equal(arrowColor(outgoing),'new-red');
assert.equal(context.strokeColor(outgoing,'left'),'outgoing-violet','introduced cycles also keep their left-hand direction color');
assert.equal(context.strokeColor(outgoing,'right'),'new-red','introduced cycles are highlighted on the right');
context.introducedKeys.clear();context.cyclicKeys.clear();
context.packageContext={name:'Parent/A',ids:new Set([0]),edges:new Set(['0 1','1 0','1 2'])};
for(const [link,color] of [[outgoing,'outgoing-violet'],[incoming,'incoming-blue'],[unrelated,'neutral']]){
  assert.equal(context.strokeColor(link),color,'coarse package context retains direction colors when no current-scope cycle exists');
  assert.equal(arrowColor(link),color);
}
context.resolvedKeys.add('0 1');
assert.equal(context.strokeColor(outgoing),'resolved-green','resolved simulation edges override stale package-level cycle context');
assert.equal(arrowColor(outgoing),'resolved-green');
context.resolvedKeys.clear();context.introducedKeys.add('0 1');
assert.equal(context.strokeColor(outgoing),'new-red','introduced cycle status overrides generic package-level cycle context');assert.equal(arrowColor(outgoing),'new-red');
context.introducedKeys.clear();context.packageContext=null;
assert.equal(context.strokeColor(outgoing),'outgoing-violet');assert.equal(arrowColor(outgoing),'outgoing-violet');
assert.equal(context.strokeColor(incoming),'incoming-blue');assert.equal(arrowColor(incoming),'incoming-blue');
context.buildLegend();assert.match(legend.textContent,/Incoming.*Outgoing/);assert.match(legend.textContent,/Lines and arrowheads:/);assert.match(legend.textContent,/package-level/i);assert.match(legend.textContent,/A/);
context.ui.selection={kind:'node',name:'B'};context.selectedId=1;context.selectedNodeIds=new Set([1]);
assert.equal(context.strokeColor(outgoing),'incoming-blue');assert.equal(context.strokeColor(incoming),'outgoing-violet','selecting the other endpoint reverses the relative directions');
context.ui.selection={kind:'edge',from:'A',to:'B'};context.selectedId=null;
assert.equal(context.strokeColor(outgoing),'neutral','edge selection does not invent a node-relative direction');
context.ui.selection.reference='A';
assert.equal(context.strokeColor(outgoing),'outgoing-violet','edge inspection keeps the retained reference direction');assert.equal(arrowColor(outgoing),'outgoing-violet');
assert.equal(context.strokeColor(incoming),'incoming-blue');assert.equal(arrowColor(incoming),'incoming-blue');
context.buildLegend();assert.match(legend.textContent,/Reference: A/);assert.match(legend.textContent,/Incoming.*Outgoing/);
delete context.ui.selection.reference;
context.cyclicKeys.add('0 1');assert.equal(context.strokeColor(outgoing),'cycle-red','no node selection keeps the existing state colors');
context.ui.selection=null;context.buildLegend();assert.doesNotMatch(legend.textContent,/Incoming.*Outgoing/);
assert.match(legend.textContent,/cycle/i);
context.cyclicKeys.clear();context.packageContext={name:'Parent/A',ids:new Set([0]),edges:new Set(['1 2'])};
assert.equal(context.strokeColor(outgoing),'outgoing-violet','an opened package is the direction reference without a separate node selection');
assert.equal(context.strokeColor(incoming),'incoming-blue');
assert.equal(context.strokeColor(unrelated),'neutral','an external context path stays neutral without a selected endpoint');
assert.equal(arrowColor(unrelated),'neutral');
context.buildLegend();assert.match(legend.textContent,/Inside: A/);assert.match(legend.textContent,/Incoming.*Outgoing/);assert.match(legend.textContent,/Lines and arrowheads:/);
context.ui.selection={kind:'edge',from:'A',to:'B'};
assert.equal(context.strokeColor(outgoing),'neutral','edge selection has no implicit package direction reference');
assert.equal(context.strokeColor(unrelated),'neutral','inspecting a coarse context edge does not invent a current-scope cycle');
context.packageContext=null;
context.ui.selection={kind:'node',name:'<img src=x onerror=alert(1)>'};context.selectedId=0;context.selectedNodeIds=new Set([0]);context.C.incoming='dark-incoming';context.C.outgoing='dark-outgoing';
context.view.nodes[0].name=context.ui.selection.name;
assert.equal(context.strokeColor(incoming),'dark-incoming');assert.equal(arrowColor(incoming),'dark-incoming');
assert.equal(context.strokeColor(outgoing),'dark-outgoing');assert.equal(arrowColor(outgoing),'dark-outgoing');
context.buildLegend();assert.match(legend.textContent,/<img/,'selection labels stay text');
// A simulation can change status sets without another selection. The normal
// graph-controls refresh must update the same legend after that rebuild.
const controls=new Map();
context.document.getElementById=id=>{if(id==='edge-legend')return legend;if(!controls.has(id))controls.set(id,new Element());return controls.get(id);};
context.document.querySelectorAll=()=>[];
context.view={nodes:[],edges:[]};context.renderedNodes=[];context.renderedLinks=[];context.isolated=null;
context.ui.diagram='network';
for(const name of ['displayedGraphNodes','renderWorkspaceDescription','renderGraphPresentation'])vm.runInContext(declaration(name),context);
context.resolvedKeys.add('0 1');vm.runInContext(declaration('renderGraphControls'),context);context.renderGraphControls();
assert.match(legend.textContent,/resolved/,'simulation updates refresh the legend even without selecting another node');
console.log('viewer colors: complete cycle/status colors, package-level return paths, direction colors, resolved simulations, reversed selection, themes and accurate legend passed');
