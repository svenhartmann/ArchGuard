import assert from 'node:assert/strict';
import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2);const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);fs.mkdirSync(output,{recursive:true});const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.setRequestInterception(true);page.on('request',r=>{if(r.url().startsWith('http')&&!r.url().startsWith('http://127.0.0.1:'))r.abort();else r.continue();});
 for(const width of [1500,390]){
  await page.setViewport({width,height:1100});await page.goto(/^http/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  const initial=await page.evaluate(()=>{setViewerLanguage('de');const start=performance.now();showTab('dsm');return performance.now()-start});
  const columns=await page.$$eval('[data-matrix-name]',ns=>ns.map(n=>n.textContent));assert.ok(columns.length<=12);
  await page.click('.tree-matrix-options > summary');
  const expansion=await page.evaluate(()=>{const start=performance.now();document.querySelector('[data-matrix-expand-all]').click();return performance.now()-start});
  assert.deepEqual(await page.$$eval('[data-matrix-name]',ns=>ns.map(n=>n.textContent)),columns);assert.ok(await page.$$eval('[data-matrix-row]',ns=>ns.length<=120));assert.ok(expansion<2000,'expand all stays responsive');
  await page.select('[data-matrix-cycle-select]','C1');assert.ok(await page.$('[data-matrix-cycle-path]'));assert.ok(await page.$('[data-matrix-path-edge]'));
  await page.click('[data-matrix-focus]');assert.equal(await page.$eval('[data-matrix-focus]',e=>e.getAttribute('aria-pressed')),'true');
  await page.screenshot({path:path.join(output,`matrix-${width}.png`)});
  await page.hover('[data-matrix-path-edge]');
  assert.ok(await page.$('.matrix-column-active'),'hover identifies target column');
  assert.ok(await page.$('.matrix-row-active'),'hover identifies source row');
  await page.click('[data-matrix-path-edge]');await page.waitForSelector('[data-matrix-evidence]');assert.ok(await page.$eval('[data-matrix-evidence]',e=>{const b=e.getBoundingClientRect();return b.top<innerHeight&&b.bottom>0}),'evidence is visible');
  await page.keyboard.press('Escape');assert.equal(await page.$('[data-matrix-evidence]'),null);assert.ok(await page.$('[data-matrix-cycle-path]'),'closing evidence preserves cycle selection');
  await page.evaluate(()=>document.querySelector('[data-matrix-path-row] .tree-matrix-label button').click());assert.ok(await page.evaluate(()=>inspectorIsOpen()),'file names open the existing details overlay');await page.evaluate(()=>setInspectorOpen(false));
  await page.select('[data-matrix-cycle-select]','');
  await page.type('[data-matrix-search]','BrokerSecurityConfigs');
  assert.equal(await page.$eval('[data-matrix-search]',e=>e.value),'BrokerSecurityConfigs');
  assert.equal(await page.evaluate(()=>document.activeElement.hasAttribute('data-matrix-search')),true);
  assert.ok(await page.$$eval('[data-matrix-row]',ns=>ns.length>0&&ns.length<120),'search narrows Kafka with ancestor context');
  const targetId=await page.$eval('[data-matrix-target]',s=>s.options[s.options.length-1].value);
  await page.select('[data-matrix-target]',targetId);
  assert.ok(await page.$eval('thead',head=>!!head.querySelector('.matrix-column-active')),'target picker jumps to and highlights the target column');
  await page.click('[data-matrix-search]',{clickCount:3});await page.type('[data-matrix-search]','no-such-kafka-file-123');
  assert.equal(await page.$$eval('[data-matrix-row]',ns=>ns.length),0);
  console.log(JSON.stringify({width,initialMs:Math.round(initial),expandAllMs:Math.round(expansion),renderedRows:await page.$$eval('[data-matrix-row]',ns=>ns.length)}));
 }
 assert.deepEqual(errors,[]);console.log('Tree matrix browser: Kafka expansion, bounded DOM, stable columns, cycle tracing, evidence, Escape and mobile passed');
}finally{await browser.close()}
