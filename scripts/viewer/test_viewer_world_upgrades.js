// Upgrades over the landscape: surface search, complexity ranking, coverage
// rollup, bidirectional change lens, path copy with line, image export and the
// jumps from the detail panel into the other workspaces.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'q',kind:'module',parent:null},{id:2,name:'r',kind:'module',parent:null},
 {id:3,name:'p/A.cs',kind:'file',parent:0},{id:4,name:'p/B.cs',kind:'file',parent:0},{id:5,name:'q/C.cs',kind:'file',parent:1},{id:6,name:'r/D.cs',kind:'file',parent:2}];
const entities=[
 {id:'class:p/A.cs',node:'p/A.cs',class_id:'p/A.cs',file:'src/p/A.cs',line:12,max_complexity:31,measured_methods:2,declared_methods:2,coverage:{scope:'file',status:'unchanged_since_import',lines:{covered:40,total:100}}},
 {id:'class:p/B.cs',node:'p/B.cs',class_id:'p/B.cs',file:'src/p/B.cs',line:3,max_complexity:3,measured_methods:1,declared_methods:1,coverage:null},
 {id:'class:q/C.cs',node:'q/C.cs',class_id:'q/C.cs',file:'src/q/C.cs',line:7,max_complexity:2,measured_methods:1,declared_methods:1,coverage:{scope:'file',status:'unchanged_since_import',lines:{covered:100,total:100}}},
 {id:'class:r/D.cs',node:'r/D.cs',class_id:'r/D.cs',file:'src/r/D.cs',line:5,max_complexity:7,measured_methods:1,declared_methods:1,coverage:null}];
const app=launch({root:'/project',nodes,
 edges:[{from:3,to:5,count:1,refs:[]},{from:5,to:4,count:1,refs:[]},{from:5,to:6,count:1,refs:[]}],
 findings:[],cyclicEdges:[],view:{depth:0,visible:null},
 insights:{world:{definition:'architecture-world/v1',revision:'b'.repeat(64),status:'complete',entities}}},{library:false});
app.run("showTab('start')");
const canvas=app.ids.get('world-canvas');
canvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
canvas.getContext=()=>new Proxy({},{get(target,key){return target[key]??(()=>{});}});

// Search dims non-matches, counts them and Enter selects the first hit.
const search=app.ids.get('world-search');
search.value='b.cs';search.dispatch('input');
assert.equal(app.run('worldUi.query'),'b.cs','input filters case-insensitively');
assert.equal(app.run('worldUi.layout.cells.filter(worldMatches).length'),1,'only the p/B surface matches');
assert.match(app.ids.get('world-count').textContent,/1 surfaces match/,'the count line reports matches');
search.dispatch('keydown',{key:'Enter'});
assert.equal(app.run('worldUi.selection?.entity?.class_id'),'p/B.cs','Enter selects the first matching surface');
search.value='';search.dispatch('input');assert.equal(app.run('worldUi.query'),'','clearing the field restores every surface');
app.ids.get('world-reset').click();assert.equal(search.value,'','reset clears the visible search field');

// The complexity lens ranks pins by the largest recorded value and names it.
app.ids.get('world-lens-complexity').click();
const pins=app.ids.get('world-labels').querySelectorAll('button[data-world-area]');
assert.equal(pins[0].getAttribute('data-world-area'),'p','the package with the worst method leads the pins');
assert.match(pins[0].textContent,/31/,'the pin states the recorded maximum');
assert.match(app.ids.get('world-status').textContent,/31.*p/,'the status names the overall maximum and its area');

// The coverage lens rolls the file counters up instead of only counting rows.
app.ids.get('world-lens-coverage').click();
assert.match(app.ids.get('world-status').textContent,/140\/200/,'covered and total lines are summed');
assert.match(app.ids.get('world-status').textContent,/1 surfaces with gaps/,'gapped surfaces are counted');

// The removed change lens leaves no dependency-direction helpers behind.
assert.equal(app.run("typeof worldAffected+','+typeof worldDepends"),'undefined,undefined');
const select=app.ids.get('world-area');select.value='q';select.dispatch('change',{target:select});

// Copy path prefers file:line, falling back to node and area names.
assert.equal(app.run("worldCopyValue({area:'p',entity:{class_id:'p/A.cs',node:'p/A.cs',file:'src/p/A.cs',line:12}},{name:'p'})"),'src/p/A.cs:12');
assert.equal(app.run("worldCopyValue(null,{name:'p'})"),'p');

// Image export downloads the current canvas as PNG or says it is unavailable.
canvas.toDataURL=()=>'data:image/png;base64,AAA';
app.ids.get('world-export').click();
assert.match(app.ids.get('feedback').textContent,/archguard-world-overview\.png/,'export names the file it saved');
canvas.toDataURL=undefined;app.ids.get('world-export').click();
assert.match(app.ids.get('feedback').textContent,/unavailable/,'a browser without canvas export is reported, not silently skipped');

// Detail panel jumps into the other workspaces.
const actionButtons=text=>app.ids.get('world-detail').querySelectorAll('button').filter(b=>b.textContent===text);
app.run("worldUi.selection={area:'p',entity:data.insights.world.entities[0]};worldDetails();");
actionButtons('Class metrics')[0].click();
assert.equal(app.run('ui.workspace'),'quality','class metrics opens the quality workspace');
assert.equal(app.run("engineeringUi.section"),'heatmap','it opens the class heatmap');
assert.equal(app.run("engineeringUi.query"),'p/A.cs','the class is prefilled as the search term');
app.run("showTab('start')");
select.value='p';select.dispatch('change',{target:select});
actionButtons('Quality scope')[0].click();
assert.equal(app.run('ui.workspace'),'quality','quality scope opens the quality workspace');
assert.equal(app.run("engineeringUi.target"),'p','the package becomes the measured scope');
app.run("showTab('start')");
select.value='q';select.dispatch('change',{target:select});
actionButtons('Cycles')[0].click();
assert.equal(app.run("JSON.stringify(cyclesForView().find(c=>cycleKey(c)===ui.cycle)?.nodes.slice().sort())"),JSON.stringify(['p','q']),'the World package cycle opens as the same graph cycle');
// Motion path: with frames available the camera eases into its pose and the
// change arrows flow; the deterministic launches above proved the instant path.
let frames=[],now=1000;
const near=(actual,expected)=>assert.ok(Math.abs(actual-expected)<1e-8,`${actual} != ${expected}`);
const drain=n=>{for(let i=0;i<n&&frames.length;i++){const cb=frames.shift();if(cb)cb(now+=16);}};
const launchMotion=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const motion=launchMotion({root:'/project',nodes,edges:[{from:3,to:5,count:1,refs:[]},{from:5,to:4,count:1,refs:[]},{from:5,to:6,count:1,refs:[]}],
 findings:[],cyclicEdges:[],view:{depth:0,visible:null},
 insights:{world:{definition:'architecture-world/v1',revision:'b'.repeat(64),status:'complete',entities}}},{library:false});
// launch fixes the app globals; motion is only available inside the app realm.
motion.context.requestAnimationFrame=cb=>{frames.push(cb);return frames.length;};
motion.context.cancelAnimationFrame=id=>{frames[id-1]=null;};
motion.context.performance={now:()=>now};
motion.run("showTab('start')");
const motionCanvas=motion.ids.get('world-canvas');
motionCanvas.getBoundingClientRect=()=>({left:0,top:0,width:800,height:600});
motionCanvas.getContext=()=>new Proxy({},{get(target,key){return target[key]??(()=>{});}});
motion.ids.get('world-rotate-right').click();
assert.equal(motion.run('worldUi.camera.rotation'),0,'the queued frame has not landed yet');
assert.ok(frames.length,'the camera tween asked for animation frames');
drain(40);near(motion.run('worldUi.camera.rotation'),Math.PI/12);near(motion.run('worldUi.camera.zoom'),1);
motion.run('worldUi.camera={x:80,y:-40,zoom:3,rotation:1}');
motion.ids.get('world-fit').click();drain(40);
assert.equal(motion.run('JSON.stringify([worldUi.camera.x,worldUi.camera.y,worldUi.camera.zoom])'),'[0,0,1]','fit glides back without touching the orientation');
near(motion.run('worldUi.camera.rotation'),1,'fit preserves the current bearing');
const motionSelect=motion.ids.get('world-area');motionSelect.value='q';motionSelect.dispatch('change',{target:motionSelect});
drain(25);assert.equal(motion.run('worldUi.flow'),0,'a selection in a project World starts no flow animation');
console.log('World upgrades: search, complexity ranking, coverage rollup, path copy, PNG export, cross-view jumps and motion passed');
