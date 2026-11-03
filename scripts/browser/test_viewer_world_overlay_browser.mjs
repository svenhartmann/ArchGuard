// Real-browser regression: node clicks, nonmodal World details, keyboard and stable camera/layout.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[],checks=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
 for(const width of [1500,390]){
  await page.setViewport({width,height:950});await page.goto(/^https?:/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  await page.evaluate(()=>{setViewerLanguage('de');showTab('start');const entity=worldUi.model.areas.flatMap(a=>a.entities).find(e=>e.class_id&&e.max_complexity!=null),source=nodeByName(data,entity.node);worldOpenArea(nodeById(data,source.parent).name);document.getElementById('world-canvas').scrollIntoView({block:'center'});});
  const before=await page.$eval('#world-canvas',n=>n.getBoundingClientRect().toJSON());
  const hit=await page.evaluate(()=>{const canvas=document.getElementById('world-canvas'),b=canvas.getBoundingClientRect();for(const hit of worldUi.hits.slice().reverse())if(hit.cell.entities.length===1&&hit.cell.entities[0].class_id)for(const triangle of hit.triangles){const x=b.left+triangle.reduce((v,p)=>v+p.x,0)/3,y=b.top+triangle.reduce((v,p)=>v+p.y,0)/3;if(document.elementFromPoint(x,y)===canvas)return {x,y};}return null;});
  assert.ok(hit,'a selectable class surface is visible');await page.mouse.click(hit.x,hit.y);
  assert.equal(await page.$eval('#world-detail',n=>n.hidden),false,'class click opens World details');
  assert.ok(await page.$('#world-detail [data-inspector-metrics]'));
  const after=await page.$eval('#world-canvas',n=>n.getBoundingClientRect().toJSON());assert.deepEqual(after,before,'overlay does not resize or move the landscape');
  await page.click('#world-lens-complexity');
  assert.equal(await page.$eval('[data-world-context]',n=>n.getAttribute('data-world-context')),'complexity');
  assert.ok(await page.$('#world-detail [data-world-method]'));
  assert.equal(await page.$eval('[data-world-background]',n=>n.open),false,'generic dependency details start collapsed');
  await page.screenshot({path:out+'/world-complexity-context-'+width+'.png'});
  await page.select('#world-kpi','wmc');assert.match(await page.$eval('[data-world-context]',n=>n.textContent),/WMC/);
  await page.click('#world-lens-coverage');assert.equal(await page.$eval('[data-world-context]',n=>n.getAttribute('data-world-context')),'coverage');
  await page.click('#world-lens-problems');assert.equal(await page.$eval('[data-world-context]',n=>n.getAttribute('data-world-context')),'problems');
  const selection=await page.evaluate(()=>worldUi.selection),camera=await page.evaluate(()=>JSON.stringify(worldUi.camera));
  await page.$$eval('#world-detail details',nodes=>nodes.forEach(n=>n.open=true));
  const bounds=await page.$eval('#world-detail',n=>{const b=n.getBoundingClientRect();return {left:b.left,right:b.right,width:b.width,height:b.height,scroll:n.scrollHeight,client:n.clientHeight};});
  assert.ok(bounds.left>=0&&bounds.right<=width+1&&bounds.width<=420);assert.ok(bounds.scroll>bounds.client,'long details scroll inside the overlay');
  await page.$eval('#world-detail',n=>n.scrollTop=n.scrollHeight);assert.ok(await page.$eval('#world-detail',n=>n.scrollTop)>0,'overlay content can actually scroll');await page.$eval('#world-detail',n=>n.scrollTop=0);
  await page.screenshot({path:out+'/world-details-'+width+'.png'});
  await page.keyboard.press('Escape');assert.equal(await page.$eval('#world-detail',n=>n.hidden),true);
  assert.deepEqual(await page.evaluate(()=>worldUi.selection),selection);assert.equal(await page.evaluate(()=>JSON.stringify(worldUi.camera)),camera);
  await page.focus('#world-canvas');await page.keyboard.press('Enter');assert.equal(await page.$eval('#world-detail',n=>n.hidden),false);
  await page.click('#world-detail .world-detail-title button');assert.equal(await page.$eval('#world-detail',n=>n.hidden),true);
  await page.click('[data-world-details]');assert.equal(await page.$eval('#world-detail',n=>n.hidden),false);
  await page.evaluate(()=>{const area=worldUi.model.areas.find(a=>a.entities.some(e=>e.id!==worldUi.selection.entity?.id)),entity=area.entities.find(e=>e.id!==worldUi.selection.entity?.id);worldUi.selection={area:area.name,entity};worldDetails();});
  assert.notDeepEqual(await page.evaluate(()=>worldUi.selection),selection);assert.equal(await page.$eval('#world-detail',n=>n.hidden),false);
  await page.keyboard.press('Escape');
  if(width===1500){
   const source=await page.evaluate(()=>{const name=worldUi.selection.entity.node;showTab('graph');revealNode(name);setInspectorOpen(false);return nodeByName(work,name);});
   const selector='g[data-tree-source="'+source.id+'"]';await page.waitForSelector(selector);await page.$eval(selector,n=>n.scrollIntoView({block:'center'}));await page.click(selector);
   assert.equal(await page.evaluate(()=>inspectorIsOpen()),true,'actual class/file click in tree opens graph details');
   assert.ok((await page.$eval('#sidebar',n=>n.textContent)).includes(source.name));
   await page.screenshot({path:out+'/graph-class-details.png'});
  }
  checks.push({width,click_opens:true,escape_preserves_selection:true,stable_camera_and_layout:true});
 }
 assert.deepEqual(errors,[]);fs.writeFileSync(out+'/checks.json',JSON.stringify({checks,errors},null,2));console.log(JSON.stringify({checks,errors}));
}finally{await browser.close();}
