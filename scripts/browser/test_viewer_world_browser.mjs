// node scripts/browser/test_viewer_world_browser.mjs REPORT CHROME PUPPETEER OUTPUT
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2),out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href),browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox'],userDataDir:path.join(out,'browser-profile')});
const errors=[],checks=[];
try{const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
for(const [width,height]of [[1500,1000],[760,800],[390,700]]){
 await page.goto('about:blank');await page.setViewport({width,height,deviceScaleFactor:width===760?2:1});await page.goto(/^(https?|file):/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});await page.evaluate(()=>setViewerLanguage('de'));
 assert.equal(await page.evaluate(()=>ui.workspace),'start');assert.equal(await page.evaluate(()=>callModelPrepared),false);
 await page.waitForFunction(()=>worldUi.hits.length>0);const original=await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,ops]));
 const entry=await page.evaluate(()=>({scope:worldUi.within,cycleGroups:worldUi.model.cycle_groups,entities:worldUi.layout.cells.reduce((n,c)=>n+c.entities.length,0)}));
 const linked=new URL(await page.url()).hash;if(linked.startsWith('#world='))assert.equal(entry.scope,decodeURIComponent(linked.slice(7)));
 await page.screenshot({path:path.join(out,'world-entry-'+width+'.png'),fullPage:false});
 const initialGeometry=await page.evaluate(()=>JSON.stringify(worldUi.hits.map(h=>h.ps)));
 await page.click('#world-rotate-right');assert.ok(await page.evaluate(()=>worldUi.camera.rotation>0));
 assert.notEqual(await page.evaluate(()=>JSON.stringify(worldUi.hits.map(h=>h.ps))),initialGeometry,'rotation changes actual hit geometry');
 const bearing=await page.evaluate(()=>worldUi.camera.rotation);await page.click('#world-zoom-in');assert.equal(await page.evaluate(()=>worldUi.camera.rotation),bearing);await page.click('#world-fit');assert.equal(await page.evaluate(()=>worldUi.camera.rotation),bearing);
 const dragPoint=await page.$eval('#world-canvas',e=>{const b=e.getBoundingClientRect();return {x:b.left+20,y:b.top+100};});
 const beforeRotation=await page.evaluate(()=>worldUi.camera);await page.mouse.move(dragPoint.x,dragPoint.y);await page.keyboard.down('Shift');await page.mouse.down();await page.mouse.move(dragPoint.x+85,dragPoint.y+15,{steps:6});await page.mouse.up();await page.keyboard.up('Shift');
 assert.ok(await page.evaluate(old=>worldUi.camera.rotation>old, beforeRotation.rotation));assert.equal(await page.evaluate(()=>worldUi.camera.x),beforeRotation.x);assert.equal(await page.evaluate(()=>worldUi.selection),null);
 await page.screenshot({path:path.join(out,'world-rotated-'+width+'.png'),fullPage:false});
 const orbit=await page.$eval('.world-orbit',e=>{const b=e.getBoundingClientRect();return {left:b.left,right:b.right};});assert.ok(orbit.left>=0&&orbit.right<=width);
 await page.click('#world-orientation');assert.ok(Math.abs(await page.evaluate(()=>worldUi.camera.rotation))<.001);
 await page.focus('#world-canvas');await page.keyboard.press('e');assert.ok(await page.evaluate(()=>worldUi.camera.rotation>0));await page.keyboard.press('q');assert.ok(Math.abs(await page.evaluate(()=>worldUi.camera.rotation))<.001);await page.click('#world-reset');
 const box=await page.$eval('#world-canvas',e=>{const b=e.getBoundingClientRect();return {x:b.x,y:b.y,w:b.width,h:b.height};});
 await page.evaluate(()=>document.getElementById('world-canvas').addEventListener('wheel',e=>{globalThis.lastWorldWheel={x:e.clientX,y:e.clientY,rect:e.target.getBoundingClientRect().toJSON()};},{once:true}));
 const point={x:Math.round(box.w*.7),y:Math.round(box.h*.5)};await page.mouse.move(box.x+point.x,box.y+point.y);await page.mouse.wheel({deltaY:-130});
 await page.waitForFunction(()=>worldUi.camera.zoom>1);
 const wheel=await page.evaluate(()=>lastWorldWheel),anchor={x:wheel.x-wheel.rect.left-wheel.rect.width/2,y:wheel.y-wheel.rect.top-wheel.rect.height/2};let cam=await page.evaluate(()=>worldUi.camera);assert.ok(Math.abs((anchor.x-cam.x)/cam.zoom-anchor.x)<.01,JSON.stringify({width,cam,anchor}));assert.ok(Math.abs((anchor.y-cam.y)/cam.zoom-anchor.y)<.01,JSON.stringify({width,cam,anchor}));
 await page.mouse.down();await page.mouse.move(box.x+point.x+50,box.y+point.y+25,{steps:4});await page.mouse.up();assert.equal(await page.evaluate(()=>worldUi.selection),null,'drag does not select');assert.ok((await page.evaluate(()=>worldUi.camera.x))>cam.x+40);
 await page.click('#world-fit');assert.equal(await page.evaluate(()=>worldUi.camera.zoom),1);assert.equal(await page.evaluate(()=>worldUi.camera.x),0);
 await page.click('#world-zoom-in');assert.ok(await page.evaluate(()=>worldUi.camera.zoom>1));await page.click('#world-zoom-out');assert.ok(Math.abs(await page.evaluate(()=>worldUi.camera.zoom)-1)<.001);
 await page.focus('#world-canvas');await page.keyboard.press('+');assert.ok(await page.evaluate(()=>worldUi.camera.zoom>1));await page.keyboard.press('0');assert.equal(await page.evaluate(()=>worldUi.camera.zoom),1);
 const clickPoint=await page.evaluate(()=>{const canvas=document.getElementById('world-canvas'),b=canvas.getBoundingClientRect();for(const hit of worldUi.hits.slice().reverse())for(const triangle of hit.triangles){const x=b.left+triangle.reduce((v,p)=>v+p.x,0)/3,y=b.top+triangle.reduce((v,p)=>v+p.y,0)/3;if(document.elementFromPoint(x,y)===canvas)return {x,y,ids:hit.cell.entities.map(e=>e.id)};}return null;});assert.ok(clickPoint,'visible terrain can be selected');await page.mouse.click(clickPoint.x,clickPoint.y);const selected=await page.evaluate(()=>worldUi.selection);assert.ok(selected);assert.ok(selected.entity?clickPoint.ids.includes(selected.entity.id):selected.members.some(e=>clickPoint.ids.includes(e.id)));await page.click('#world-reset');
 for(const [lens,labels] of [['problems',['Zyklus / Befund','Klassen / Dateien']],['complexity',['Gering → hoch','Unbekannt']],['coverage',['Testlücken','Abgedeckt','Unbekannt']]]){
  await page.click('#world-lens-'+lens);assert.deepEqual(await page.$$eval('#world-legend button',nodes=>nodes.map(n=>n.textContent)),labels);
  const boxes=await page.$$eval('#world-legend button',nodes=>nodes.map(n=>{const b=n.getBoundingClientRect();return {x:b.x,right:b.right,width:b.width};}));assert.ok(boxes.every(b=>b.x>=0&&b.right<=width&&b.width>0),'legend wraps inside the viewport');
  await page.focus('#world-legend button');assert.equal(await page.$eval('#metric-tooltip',n=>n.hidden),false);assert.ok((await page.$eval('#metric-tooltip',n=>n.textContent)).length>30);await page.keyboard.press('Escape');
  await page.click('#world-legend button');assert.equal(await page.$eval('#metric-tooltip',n=>n.hidden),false);await page.keyboard.press('Escape');
 }
 assert.equal(await page.$('#world-lens-change'),null);
 await page.click('#world-lens-problems');
 const shape=await page.evaluate(()=>JSON.stringify(worldUi.layout.cells.map(c=>[c.x,c.y,c.z])));
 for(const lens of ['coverage','complexity','problems']){await page.click('#world-lens-'+lens);assert.equal(await page.evaluate(()=>JSON.stringify(worldUi.layout.cells.map(c=>[c.x,c.y,c.z]))),shape,'lens preserves positions');}
 await page.evaluate(()=>{copyText=text=>{globalThis.worldCopied=text;};});
 const area=await page.evaluate(()=>worldUi.model.areas.find(a=>a.entities.length)?.name);await page.select('#world-area',area);await page.click('[data-world-details]');assert.equal(await page.$eval('#world-detail',e=>e.hidden),false);
 await page.$$eval('#world-detail button',buttons=>buttons.find(b=>b.textContent==='KI-Kontext').click());const copied=JSON.parse(await page.evaluate(()=>worldCopied));assert.equal(copied.tools[0].name,'architecture_world');assert.equal(copied.tools[0].arguments.within,area);
 const entity=await page.$('[data-world-entity]');if(entity){await entity.click();assert.ok(await page.evaluate(()=>worldUi.selection.entity));}
 const module=await page.evaluate(()=>worldUi.model.areas.find(a=>a.kind==='module'&&a.entities.length)?.name);if(module){await page.select('#world-area',module);await page.$$eval('#world-detail button',buttons=>buttons.find(b=>b.textContent==='Erkunden').click());assert.equal(await page.evaluate(()=>worldUi.within),module);}
 await page.click('#world-reset');assert.equal(await page.evaluate(()=>worldUi.selection),null);assert.equal(await page.evaluate(()=>worldUi.within),null);
 // Direct package labels, nested breadcrumbs and actual canvas double-clicks.
 const packages=await page.$$eval('#world-labels [data-world-kind="module"]',nodes=>nodes.filter(n=>!n.hidden).map(n=>n.getAttribute('data-world-area')));
 const namedPackages=await page.evaluate(()=>worldUi.model.areas.filter(a=>['Emby','MediaBrowser'].includes(a.name)).map(a=>a.name));
 for(const name of namedPackages)assert.ok(packages.includes(name),name+' remains visible for direct package navigation');
 const targets=packages.filter(n=>/^(Emby|MediaBrowser)(\/|$)/.test(n));if(!targets.length&&packages.length)targets.push(packages[0]);
 for(const name of targets){
  await page.click(`[data-world-area=${JSON.stringify(name)}]`);assert.equal(await page.evaluate(()=>worldUi.within),name,'label opens its package');
  assert.equal(await page.evaluate(()=>worldUi.camera.zoom),1);assert.equal(await page.$eval('#world-detail',n=>n.hidden),true);
  assert.equal(await page.$eval('#world-scope [aria-current="page"]',n=>n.getAttribute('data-world-scope')),name);
  assert.equal(await page.evaluate(()=>document.activeElement.id),'world-scope');
  const child=await page.$$eval('#world-labels [data-world-kind="module"]',nodes=>nodes.find(n=>!n.hidden)?.getAttribute('data-world-area'));
  if(child){
   await page.click(`[data-world-area=${JSON.stringify(child)}]`);assert.equal(await page.evaluate(()=>worldUi.within),child);
   await page.click('#world-zoom-in');const nestedCamera=await page.evaluate(()=>worldUi.camera);
   await page.click(`#world-scope [data-world-scope=${JSON.stringify(name)}]`);assert.equal(await page.evaluate(()=>worldUi.within),name);
   await page.click('#world-back');assert.equal(await page.evaluate(()=>worldUi.within),child);assert.deepEqual(await page.evaluate(()=>worldUi.camera),nestedCamera,'back restores package zoom');
   const crumbs=await page.$$eval('#world-scope button',nodes=>nodes.map(n=>{const b=n.getBoundingClientRect();return {left:b.left,right:b.right};}));assert.ok(crumbs.every(b=>b.left>=0&&b.right<=width),'breadcrumbs stay inside narrow viewports');
   await page.screenshot({path:path.join(out,'world-package-'+name.replaceAll('/','-')+'-'+width+'.png'),fullPage:false});
  }
  await page.click('#world-scope [data-world-scope=""]');assert.equal(await page.evaluate(()=>worldUi.within),null);
 }
 const doublePoint=await page.evaluate(()=>{const canvas=document.getElementById('world-canvas'),b=canvas.getBoundingClientRect();for(const hit of worldUi.hits.slice().reverse()){if(!worldUi.model.areas.some(a=>a.name===hit.cell.area&&a.kind==='module'))continue;for(const triangle of hit.triangles){const x=b.left+triangle.reduce((v,p)=>v+p.x,0)/3,y=b.top+triangle.reduce((v,p)=>v+p.y,0)/3;if(document.elementFromPoint(x,y)===canvas)return {x,y,area:hit.cell.area};}}return null;});
 if(packages.length){assert.ok(doublePoint,'package terrain is reachable');await page.mouse.click(doublePoint.x,doublePoint.y,{count:2});await page.waitForFunction(name=>worldUi.within===name,{},doublePoint.area);assert.equal(await page.evaluate(()=>worldUi.within),doublePoint.area,'double-click opens the terrain package');}
 await page.click('#world-reset');assert.equal(await page.evaluate(()=>worldUi.history.length),0);assert.equal(await page.$eval('#world-back',n=>n.disabled),true);
 await page.evaluate(()=>applyTheme('dark'));await page.screenshot({path:path.join(out,'world-dark-'+width+'.png'),fullPage:false});await page.evaluate(()=>applyTheme('light'));await page.screenshot({path:path.join(out,'world-light-'+width+'.png'),fullPage:false});
 await page.focus('#world-help');assert.ok((await page.$eval('#metric-tooltip',e=>e.textContent)).includes('Mausrad'));await page.keyboard.press('Escape');
 assert.equal(await page.$('#start-tools'),null);await page.click('#tabs [data-tab="quality"]');assert.equal(await page.evaluate(()=>ui.workspace),'quality');await page.evaluate(()=>showTab('start'));
 const bounds=await page.$eval('#tab-start',e=>{e.scrollTop=e.scrollHeight;return {sw:e.scrollWidth,cw:e.clientWidth,top:e.scrollTop,max:e.scrollHeight-e.clientHeight};});assert.ok(bounds.sw<=bounds.cw+1&&bounds.top>=bounds.max-1,JSON.stringify(bounds));
 assert.equal(await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,ops])),original);checks.push({width,entry,zoom:true,pan:true,rotation:true,controls:true,stableLayout:true,clipboard:true,scroll:true,packages:targets,breadcrumbs:true,doubleClick:true});
}
assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({checks,errors},null,2));console.log(JSON.stringify({checks,errors}));
}finally{await browser.close();}
