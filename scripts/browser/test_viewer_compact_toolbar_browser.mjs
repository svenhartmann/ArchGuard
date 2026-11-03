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
 await page.setRequestInterception(true);page.on('request',r=>/^https?:/.test(r.url())?r.abort():r.continue());
 for(const width of [1500,390]){
  await page.setViewport({width,height:1000});await page.goto(pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  await page.evaluate(()=>{setViewerLanguage('de');showTab('graph');setInspectorOpen(false);});
  const primary=await page.$$eval('[data-tree-toolbar] button',ns=>ns.filter(n=>n.checkVisibility()).map(n=>n.getAttribute('data-tree-control')));
  assert.deepEqual(primary,['zoom-out','zoom-in','zoom-reset','zoom-fit']);
  const position=await page.$eval('#package-tree svg',n=>n.getBoundingClientRect().top);
  await page.screenshot({path:path.join(output,'graph-'+width+'.png')});
  await page.click('[data-tree-options] > summary');
  assert.equal(await page.$eval('#package-tree svg',n=>n.getBoundingClientRect().top),position,'menu overlays the graph without shifting it');
  await page.click('[data-tree-legend] > summary');assert.equal(await page.$eval('[data-tree-options]',n=>n.open),true);
  assert.ok(await page.$eval('.tree-options-content',n=>{const r=n.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.bottom<=innerHeight&&n.scrollWidth<=n.clientWidth+1;}),'menu fits desktop/mobile viewport');
  await page.screenshot({path:path.join(output,'graph-menu-'+width+'.png')});
  const state=await page.evaluate(()=>JSON.stringify([ui.selection,ui.edgeMode,treeZoom,ops]));
  await page.keyboard.press('Escape');assert.equal(await page.$eval('[data-tree-options]',n=>n.open),false);
  assert.equal(await page.evaluate(()=>document.activeElement.getAttribute('data-tree-control')),'options');
  assert.equal(await page.evaluate(()=>JSON.stringify([ui.selection,ui.edgeMode,treeZoom,ops])),state);
  await page.click('[data-tree-options] > summary');await page.click('[data-tree-sort="alphabetical"]');assert.equal(await page.evaluate(()=>treeSort),'alphabetical');
  assert.equal(await page.$eval('[data-tree-options]',n=>n.open),true);
  await page.click('[data-tree-options] > summary');await page.click('[data-tree-options] > summary');
  await page.click('#project-name');assert.equal(await page.$eval('[data-tree-options]',n=>n.open),false,'outside click dismisses menu');
 }
 assert.deepEqual(errors,[]);console.log('Compact graph toolbar: four primary actions, nested options/legend, mobile bounds, no layout jump, keyboard focus, Escape and outside dismissal passed');
}finally{await browser.close();}
