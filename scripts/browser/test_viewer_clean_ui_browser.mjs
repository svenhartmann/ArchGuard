// Usage: node scripts/browser/test_viewer_clean_ui_browser.mjs REPORT_OR_URL CHROMIUM PUPPETEER OUTPUT
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const url=/^https?:/.test(report)?report:pathToFileURL(path.resolve(report)).href;
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
const errors=[],checks=[];
try{
  const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));
  await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
  for(const [width,height]of [[1500,900],[760,700],[390,640]]){
    await page.setViewport({width,height});await page.goto(url,{waitUntil:'load'});await page.evaluate(()=>setViewerLanguage('de'));
    for(const tab of ['graph','dsm','calls','services','architecture','assessment','quality','security']){
      await page.evaluate(tab=>showTab(tab),tab);
      const result=await page.evaluate(tab=>{
        const visible=n=>!!n&&n.checkVisibility({checkVisibilityCSS:true})&&!n.closest('[hidden]');
        return {tab,summary:visible(document.getElementById('summary')),explorer:visible(document.getElementById('explorer')),inspector:visible(document.getElementById('inspector')),text:[...document.querySelectorAll('p,small,summary')].filter(n=>visible(n)&&n.getBoundingClientRect().top<innerHeight&&n.getBoundingClientRect().bottom>0).map(n=>({id:n.id,text:n.textContent})),horizontalOverflow:document.documentElement.scrollWidth>innerWidth+1};
      },tab);
      assert.equal(result.horizontalOverflow,false,tab+' page overflow at '+width);
      if(['calls','services','security','quality','architecture','assessment'].includes(tab)){assert.equal(result.summary,false,tab+' must hide graph metrics');assert.equal(result.explorer,false,tab+' must hide unrelated explorer');}
      if(tab==='security')assert.equal(result.inspector,false,'avoid duplicate security explanation panel');
      if(tab==='calls')assert.ok((await page.$eval('#calls-status',n=>n.textContent)).length<80);
      if(tab==='quality'){
        assert.equal(await page.$eval('#engineering-scope-options',n=>n.open),false);
        await page.click('#engineering-scope-summary');
        const target=await page.$eval('#engineering-target',n=>[...n.options].find(o=>o.value)?.value);
        assert.ok(target);await page.select('#engineering-target',target);await page.click('#engineering-scope-summary');
        assert.ok((await page.$eval('#engineering-scope-summary',n=>n.textContent)).includes(target));
        await page.click('#engineering-scope-summary');await page.select('#engineering-target','');await page.click('#engineering-scope-summary');
        await page.focus('th span[data-metric-key="cbo"]');assert.match(await page.$eval('#metric-tooltip',n=>n.textContent),/Klassenkopplung/);await page.keyboard.press('Escape');
        await page.evaluate(()=>{document.activeElement?.blur();document.getElementById('tab-quality').scrollTop=0;});
      }
      if(width===1500)await page.screenshot({path:path.join(out,'after-'+tab+'.png')});
      checks.push({width,height,...result});
    }
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({checks,errors},null,2));console.log(JSON.stringify({views:checks.length,errors}));
}finally{await browser.close();}
