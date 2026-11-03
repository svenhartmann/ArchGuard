// node scripts/browser/test_viewer_development_browser.mjs REPORT_OR_URL CHROMIUM PUPPETEER OUTPUT
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2),out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href),browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
const errors=[],checks=[];
try{const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
for(const [width,height]of [[1500,900],[760,700],[390,640]]){
 await page.setViewport({width,height});await page.goto(/^http/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});await page.evaluate(()=>setViewerLanguage('de'));
 assert.equal(await page.evaluate(()=>ui.workspace),'start','source projects open the new entry');
 assert.equal(await page.evaluate(()=>callModelPrepared),false,'start does not eagerly decode compiler calls');
 const original=await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,ops]));const world=await page.$eval('#start-world',node=>!node.hidden);const areaSelector=world?'[data-world-area]':'[data-atlas-area]';const areas=await page.$$eval(areaSelector,nodes=>nodes.length);assert.ok(areas>0);
 await page.screenshot({path:path.join(out,'start-'+width+'.png')});
 if(width===1500){await page.evaluate(()=>applyTheme('dark'));await page.screenshot({path:path.join(out,'start-dark.png')});await page.evaluate(()=>applyTheme('light'));}
 await page.click(areaSelector);assert.ok(await page.evaluate(()=>startUi.within||worldUi.within||worldUi.selection||ui.workspace==='graph'),'area opens its scope or file');
 await page.evaluate(()=>{startUi.within=null;showTab('start');});
 const visited=new Set();
 assert.equal(await page.$('#start-tools'),null);assert.equal(await page.$('[data-development-view]'),null);
 const views=await page.$$eval('#tabs [data-tab]',nodes=>nodes.map(n=>n.dataset.tab).filter(view=>view!=='start'));assert.equal(views.length,8);
  for(const view of views){
   await page.click('#tabs [data-tab="'+view+'"]');assert.equal(await page.evaluate(()=>ui.workspace),view);visited.add(view);
   await page.evaluate(()=>{globalThis.entryCopied=null;copyText=text=>{globalThis.entryCopied=text;};});await page.click('#workspace-mcp');
   const command=JSON.parse(await page.evaluate(()=>entryCopied));assert.equal(command.name,'development_guide');assert.equal(command.arguments.view,view);assert.ok(command.arguments.path);
   await page.focus('#workspace-info');assert.equal(await page.$eval('#metric-tooltip',n=>n.hidden),false);assert.ok((await page.$eval('#metric-tooltip',n=>n.textContent)).length>20);await page.keyboard.press('Escape');
   if(view==='quality'){await page.focus('#engineering-help');assert.match(await page.$eval('#metric-tooltip',n=>n.textContent),/Quell|Mess|Kennzahl/);await page.keyboard.press('Escape');assert.equal(await page.$eval('#engineering-note',n=>n.checkVisibility()),false);}
   await page.evaluate(()=>showTab('start'));
  }
 assert.equal(visited.size,8);assert.equal(await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,ops])),original);
 const bounds=await page.$eval('#tab-start',n=>{n.scrollTop=n.scrollHeight;return {sw:n.scrollWidth,cw:n.clientWidth,top:n.scrollTop,max:n.scrollHeight-n.clientHeight,bottom:n.getBoundingClientRect().bottom};});
 assert.ok(bounds.sw<=bounds.cw+1&&bounds.bottom<=height+1&&bounds.top>=bounds.max-1,JSON.stringify(bounds));
 checks.push({width,areas,views:visited.size,clipboard:true,help:true,scroll:true});
}
assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({checks,errors},null,2));console.log(JSON.stringify({checks,errors}));
}finally{await browser.close();}
