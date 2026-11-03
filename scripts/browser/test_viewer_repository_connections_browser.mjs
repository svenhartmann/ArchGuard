// Real-browser acceptance for a served repository with detected communication cycles.
// node scripts/browser/test_viewer_repository_connections_browser.mjs URL CHROME PUPPETEER OUTPUT
import assert from 'node:assert/strict';
import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
const [url,executablePath,modulePath,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
fs.mkdirSync(output,{recursive:true});
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);page.on('request',r=>/^https?:/.test(r.url())&&new URL(r.url()).origin!==new URL(url).origin?r.abort():r.continue());
 const ready=()=>page.waitForFunction(()=>typeof worldUi!=='undefined'&&ui.workspace==='start'&&worldUi.layout&&window.archguardLiveViewer,{timeout:180000});
 await page.setViewport({width:1500,height:1100});await page.goto(url,{waitUntil:'load'});await ready();
 const counts=await page.evaluate(()=>Object.fromEntries([...new Set(repositoryWorldConnections().map(e=>e.kind))].map(k=>[k,repositoryWorldConnections().filter(e=>e.kind===k).length])));
 assert.ok(counts.event>0);assert.ok(counts.http_call>0);assert.ok(counts.grpc_call>0);
 const selected=await page.evaluate(()=>repositoryWorldConnections().find(e=>e.kind==='event'&&e.cyclic).from);
 // A canvas tap selects, leaving the repository URL and camera intact.
 const point=await page.evaluate(name=>{const canvas=document.getElementById('world-canvas'),b=canvas.getBoundingClientRect();for(const hit of worldUi.hits.filter(h=>h.cell.area===name))for(const triangle of hit.triangles){const x=b.left+triangle.reduce((n,p)=>n+p.x,0)/3,y=b.top+triangle.reduce((n,p)=>n+p.y,0)/3;if(document.elementFromPoint(x,y)===canvas)return{x,y};}},selected);
 assert.ok(point);await page.mouse.click(point.x,point.y);
 assert.equal(await page.evaluate(()=>worldUi.selection?.area),selected);
 assert.equal(await page.evaluate(()=>isRepositoryOverview()),true);
 assert.equal(await page.$eval('#world-detail',e=>e.hidden),true);
 await page.select('#world-service-kind','event');
 for(const dir of ['incoming','outgoing']){await page.select('#world-service-direction',dir);assert.ok(await page.evaluate(({dir,selected})=>repositoryVisibleConnections().length>0&&repositoryVisibleConnections().every(e=>e[dir==='incoming'?'to':'from']===selected),{dir,selected}));}
 await page.select('#world-service-direction','both');await page.click('#world-service-cycles');
 assert.ok(await page.evaluate(()=>repositoryVisibleConnections().every(e=>e.cyclic)));
 await page.click('[data-world-details]');assert.equal(await page.$eval('#world-detail',e=>e.hidden),false);
 assert.match(await page.$eval('#world-detail',e=>e.textContent),/\.cs/);
 const camera=await page.evaluate(()=>JSON.stringify(worldUi.camera));await page.keyboard.press('Escape');assert.equal(await page.$eval('#world-detail',e=>e.hidden),true);assert.equal(await page.evaluate(()=>JSON.stringify(worldUi.camera)),camera);
 await page.screenshot({path:path.join(output,'service-connections.png')});
 await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.click('#world-service-open')]);await ready();
 assert.equal(new URL(page.url()).searchParams.get('project'),selected);
 assert.ok(await page.evaluate(()=>data.insights.world.entities.length>0));
 for(let i=0;i<12;i++){const area=await page.evaluate(()=>worldUi.model.areas.find(a=>a.kind==='module'&&a.entities.length)?.name);if(!area)break;await page.click('[data-world-area="'+area+'"]');}
 const file=await page.evaluate(()=>worldUi.model.areas.find(a=>a.kind==='file'&&a.entities.length)?.name);assert.ok(file);await page.click('[data-world-area="'+file+'"]');assert.equal(await page.$eval('#world-detail',e=>e.hidden),false);
 await page.reload({waitUntil:'load'});await ready();assert.equal(new URL(page.url()).searchParams.get('project'),selected);
 await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.click('#world-scope a[href="/#tab=start"]')]);await ready();
 // A real double-click on a service label opens its project too.
 await page.click('[data-world-area="'+selected+'"]');
 await page.click('[data-world-area="'+selected+'"]');
 await page.waitForFunction(expected=>new URL(location.href).searchParams.get('project')===expected,{},selected);await ready();
 assert.equal(new URL(page.url()).searchParams.get('project'),selected);
 await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.click('#world-scope a[href="/#tab=start"]')]);await ready();
 // Wheel beyond maximum enters a service; the source root can zoom back out.
 await page.evaluate(name=>{worldUi.camera.zoom=8;worldUi.hover=worldUi.layout.cells.find(c=>c.area===name);const canvas=document.getElementById('world-canvas');canvas.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true,cancelable:true}));},selected);
 await page.waitForFunction(()=>new URL(location.href).searchParams.has('project'));await ready();
 await page.evaluate(()=>{worldUi.camera.zoom=.4;document.getElementById('world-canvas').dispatchEvent(new WheelEvent('wheel',{deltaY:120,bubbles:true,cancelable:true}));});
 await page.waitForFunction(()=>!new URL(location.href).searchParams.has('project'));await ready();
 await page.setViewport({width:390,height:950});await page.screenshot({path:path.join(output,'service-connections-mobile.png')});
 assert.deepEqual(errors,[]);fs.writeFileSync(path.join(output,'checks.json'),JSON.stringify({counts,selected,filters:true,details:true,escape:true,drilldown:true,reload:true,wheel:true,errors},null,2));
 console.log('Service connections, directional filters, cycle filter, evidence, Esc, package/class drill-down, reload and wheel navigation passed',counts);
}finally{await browser.close();}
