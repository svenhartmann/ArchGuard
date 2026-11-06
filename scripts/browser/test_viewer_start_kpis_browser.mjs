import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
fs.mkdirSync(output,{recursive:true});
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
 for(const width of [1500,760,390]){
  await page.setViewport({width,height:1000});await page.goto(/^https?:/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  await page.evaluate(()=>{setViewerLanguage('de');showTab('start')});
  assert.equal(await page.$$eval('.start-kpis .dash-card',n=>n.length),8);
  assert.ok(await page.evaluate(()=>document.querySelector('.start-kpis').getBoundingClientRect().top>=document.getElementById('start-world').getBoundingClientRect().bottom),'World comes before the KPI cards');
  assert.ok(await page.$$eval('#tabs button',ns=>ns.filter(n=>n.dataset.tab==='assessment').every(n=>getComputedStyle(n).display==='none')));
  assert.ok(await page.$eval('#tab-start',n=>n.scrollWidth<=n.clientWidth+1),'no horizontal overflow');
  await page.screenshot({path:path.join(output,`start-${width}.png`)});
  await page.click('.start-kpis-profile summary');await page.waitForSelector('.start-kpis-profile-grid');assert.match(await page.$eval('.start-kpis-profile-grid',n=>n.textContent),/Verlauf braucht mindestens zwei/);assert.match(await page.$eval('.start-kpis-profile-grid',n=>n.textContent),/Keine messbaren A.I-Punkte/);
  await page.click('.start-kpis-actions button');
  assert.ok(await page.evaluate(()=>{const p=document.getElementById('tab-start'),w=document.getElementById('start-world');return Math.abs(w.getBoundingClientRect().top-p.getBoundingClientRect().top)<2||Math.abs(p.scrollTop-(p.scrollHeight-p.clientHeight))<2}),'World aligns at top or reaches natural end of scroll');
  await page.evaluate(()=>worldOpenArea(worldUi.model.areas.find(a=>a.kind==='module')?.name||worldUi.model.areas[0].name));
  assert.ok(await page.evaluate(()=>{const p=document.getElementById('tab-start'),w=document.getElementById('start-world');return Math.abs(w.getBoundingClientRect().top-p.getBoundingClientRect().top)<2||Math.abs(p.scrollTop-(p.scrollHeight-p.clientHeight))<2}),'World aligns at top or reaches natural end of scroll');
 }
 assert.deepEqual(errors,[]);console.log('Start browser: desktop/tablet/mobile, 8 cards, hidden tabs, no overflow, profile and stable World navigation passed');
}finally{await browser.close()}
