// node scripts/browser/test_viewer_kpi_help_browser.mjs REPORT CHROME PUPPETEER OUTPUT
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2),out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']}),errors=[],checks=[];
try{
 const page=await browser.newPage();page.on('pageerror',error=>errors.push(String(error)));
 const external=[];await page.setRequestInterception(true);page.on('request',request=>{if(/^https?:/.test(request.url())){external.push(request.url());request.abort();}else request.continue();});
 for(const width of [1500,390]){
  await page.setViewport({width,height:1000});await page.goto(pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  const before=await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,work,ops,ui.selection,ui.workspace])),requests=external.length;
  await page.click('#settings-summary');await page.click('#kpi-help-open');assert.equal(await page.$eval('#kpi-help-dialog',node=>node.open),true);assert.equal(await page.$eval('#settings-menu',node=>node.open),false,'the settings menu closes behind the dialog');
  assert.equal(await page.evaluate(()=>window.archguardCanReload()),false,'live refresh waits while help is open');
  await page.click('#kpi-help-dialog [aria-label=Deutsch]');
  assert.match(await page.$eval('#kpi-help-title',node=>node.textContent),/Sprachen/);
  assert.equal(await page.$$eval('[data-kpi-language-row]',nodes=>nodes.length),8);
  assert.ok(await page.$eval('.kpi-help-body',node=>node.scrollWidth<=node.clientWidth+1),'the help page has no horizontal overflow');
  await page.screenshot({path:path.join(out,'kpi-help-overview-'+width+'.png')});
  for(const [id,expected] of [['go',/Go SDK/],['java',/JDK 17/],['csharp',/Roslyn/],['ts',/TypeScript/],['js',/allowJs/],['py',/coverage.py/],['php',/Xdebug/],['rs',/cargo-llvm-cov/]]){
   await page.click('[data-kpi-language="'+id+'"]');
   const text=await page.$eval('[data-kpi-detail]',node=>node.textContent);assert.match(text,expected);assert.match(text,/archguard coverage/);
   if(['py','php','rs'].includes(id))assert.match(text,/keine compilerbasierten/);
  }
  await page.click('[data-kpi-language="js"]');
  await page.screenshot({path:path.join(out,'kpi-help-javascript-'+width+'.png')});
  await page.evaluate(()=>{window.copiedHelp=[];Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>copiedHelp.push(text)}});});
  await page.click('[data-kpi-detail] .kpi-help-command button');assert.match((await page.evaluate(()=>copiedHelp))[0],/archguard calls/);
  await page.click('#kpi-help-dialog [aria-label=English]');
  assert.equal(await page.$eval('[data-kpi-detail]',node=>node.getAttribute('data-kpi-detail')),'js');
  assert.match(await page.$eval('#kpi-help-title',node=>node.textContent),/Languages/);
  for(let i=0;i<12;i++){await page.keyboard.press('Tab');assert.equal(await page.evaluate(()=>kpiHelpUi.dialog.contains(document.activeElement)),true,'keyboard focus stays inside help');}
  const links=await page.$$eval('#kpi-help-dialog a',nodes=>nodes.map(node=>({href:node.href,rel:node.rel,target:node.target})));
  assert.ok(links.every(link=>link.href.startsWith('https://')&&link.rel==='noopener noreferrer'&&link.target==='_blank'));
  await page.keyboard.press('Escape');assert.equal(await page.$eval('#kpi-help-dialog',node=>node.open),false);
  assert.equal(await page.evaluate(()=>document.activeElement.id),'settings-summary','Escape restores focus to the visible menu that opened help');
  assert.equal(await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,work,ops,ui.selection,ui.workspace])),before);
  assert.equal(external.length,requests,'reading help works offline without external requests');
  await page.click('#settings-summary');await page.click('#kpi-help-open');await page.click('[data-kpi-help-close]');assert.equal(await page.$eval('#kpi-help-dialog',node=>node.open),false);
  checks.push({width,languages:8,translation:true,copy:true,keyboard:true,offline:true});
 }
 assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({checks,errors},null,2));console.log(JSON.stringify({checks,errors}));
}finally{await browser.close();}
