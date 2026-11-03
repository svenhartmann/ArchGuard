// node scripts/browser/test_viewer_world_fullscreen_browser.mjs REPORT CHROME PUPPETEER
import assert from 'node:assert/strict';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);page.on('request',r=>/^https?:/.test(r.url())&&!r.url().startsWith('http://127.0.0.1:')?r.abort():r.continue());
 for(const size of [{width:1920,height:1440},{width:1500,height:1000}]){
  await page.setViewport(size);
  await page.goto(/^http/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  await page.evaluate(()=>{showTab('start');worldUi.camera={x:12,y:8,zoom:1.3,rotation:.4};worldDraw()});
  const measure=()=>page.evaluate(()=>({height:document.getElementById('world-canvas').clientHeight,width:document.getElementById('world-canvas').clientWidth,camera:JSON.stringify(worldUi.camera)}));
  const before=await measure();
  await page.click('#world-fullscreen');await page.waitForFunction(()=>!!document.fullscreenElement);
  await page.waitForFunction(()=>document.getElementById('world-fullscreen').getAttribute('aria-pressed')==='true');
  const after=await measure();
  assert.ok(after.height>before.height+30,'fullscreen expands the landscape beyond its normal capped height');
  assert.ok(after.width>=before.width);assert.equal(after.camera,before.camera);
  assert.ok(await page.evaluate(()=>{const host=document.getElementById('start-world').getBoundingClientRect(),tab=document.getElementById('tab-start').getBoundingClientRect();return host.bottom<=tab.bottom+1&&host.top>=tab.top}),'world fits the available viewport');
  assert.ok(await page.$eval('#world-fullscreen',e=>{const r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight}),'exit control stays reachable');
  await page.click('#world-help');assert.ok(await page.evaluate(()=>!document.getElementById('metric-tooltip').hidden),'help remains available in fullscreen');
  await page.keyboard.press('Escape');
  if(await page.evaluate(()=>!!document.fullscreenElement))await page.click('#world-fullscreen');
  await page.waitForFunction(()=>!document.fullscreenElement);
  const restored=await measure();assert.equal(restored.height,before.height);assert.equal(restored.camera,before.camera);
  console.log(JSON.stringify({size,before,after}));
 }
 assert.deepEqual(errors,[]);console.log('World fullscreen: larger canvas, full available width, preserved camera, help, exit and restored layout passed');
}finally{await browser.close()}
