import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,library,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
fs.mkdirSync(output,{recursive:true});const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);page.on('request',r=>{if(r.url().includes('@viz-js/viz@3.11.0/lib/viz-standalone.js'))r.respond({status:200,contentType:'application/javascript',headers:{'access-control-allow-origin':'*'},body:fs.readFileSync(library)});else if(r.url().startsWith('http')&&!r.url().startsWith('http://127.0.0.1:'))r.abort();else r.continue();});
 for(const width of [1500,390]){
  await page.setViewport({width,height:1000});await page.goto(/^http/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  await page.evaluate(()=>{setViewerLanguage('de');showTab('architecture')});await page.waitForSelector('[data-arc42-layout="graphviz"]');
  const expected=await page.evaluate(()=>{const m=arc42PackageProjection();return {nodes:m.nodes.length,edges:m.edges.length}});
  assert.equal(await page.$$eval('[data-arc42-node]',n=>n.length),expected.nodes);
  assert.equal(await page.$$eval('[data-arc42-layout] g.edge',n=>n.length),expected.edges);
  assert.ok(await page.$$eval('.arc42-map-controls button',ns=>ns.every(n=>!['‹','›','Alle Beziehungen','Stärkste Beziehungen'].includes(n.textContent))));
  assert.ok(await page.$eval('[data-arc42-layout]',svg=>[...svg.querySelectorAll('g.node')].every((a,i,all)=>all.slice(i+1).every(b=>{const x=a.getBBox(),y=b.getBBox();return x.x+x.width<=y.x||y.x+y.width<=x.x||x.y+x.height<=y.y||y.y+y.height<=x.y}))), 'Graphviz places nodes without overlaps');
  await page.screenshot({path:path.join(output,`arc42-${width}.png`)});
  // Selection only restyles: a dense package map must not run Graphviz again.
  await page.evaluate(async()=>{window.arc42Layouts=0;const viz=await arc42LayoutEngine(),run=viz.renderSVGElement;if(!viz.counted){viz.counted=true;viz.renderSVGElement=function(...args){window.arc42Layouts++;return run.apply(this,args);};}});
  await page.evaluate(()=>{const svg=document.querySelector('[data-arc42-layout]');svg.querySelector('[data-arc42-node]').dispatchEvent(new MouseEvent('click',{bubbles:true}));});await page.waitForSelector('[data-arc42-layout="graphviz"] [data-arc42-node][aria-pressed="true"]');
  assert.ok(await page.$('[data-arc42-node][aria-pressed="true"]'));
  assert.equal(await page.evaluate(()=>window.arc42Layouts),0,'selecting a package reuses the layout');
  assert.equal(await page.$eval('[data-arc42-layout] [data-arc42-node][aria-pressed="true"]',g=>g.querySelector('polygon,path,ellipse')?.getAttribute('stroke')),'#a7c4ff','the selected package is highlighted');
  assert.equal(await page.$$eval('[data-arc42-layout] g.edge',n=>n.length),expected.edges,'selecting a package keeps all edges');
 }
 const sources=await page.evaluate(()=>({packages:arc42PlantUml(arc42PackageProjection()),deployment:arc42PlantUml({scope:'compose.yml',nodes:[{id:'api',name:'API',kind:'deployment'},{id:'db',name:'DB',kind:'deployment'}],edges:[{from:'api',to:'db',kind:'startup'}]})}));
 fs.writeFileSync(path.join(output,'packages.puml'),sources.packages);fs.writeFileSync(path.join(output,'deployment.puml'),sources.deployment);
 // Offline fallback is explicit and remains usable.
 await page.evaluate(()=>{arc42VizPromise=Promise.reject(new Error('offline'));renderArchitecture()});await page.waitForFunction(()=>document.getElementById('architecture-content').textContent.includes('Automatisches Layout nicht verfügbar'));
 assert.ok(await page.$('svg[data-arc42-graph]'));assert.deepEqual(errors,[]);console.log('arc42 browser: actual Graphviz/WASM, non-overlapping packages, selection, desktop/mobile, exports and offline fallback passed');
}finally{await browser.close()}
