// node scripts/browser/test_viewer_class_heatmap_browser.mjs REPORT_OR_LOCAL_URL CHROMIUM PUPPETEER_ENTRY OUTPUT_DIR
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2);
assert.ok(report&&executablePath&&modulePath&&output);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const url=/^https?:/.test(report)?report:pathToFileURL(path.resolve(report)).href;
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
const checks=[],errors=[];
try{
  const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));
  await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
  for(const [width,height]of [[1500,900],[760,700],[390,640]]){
    await page.setViewport({width,height});await page.goto(url,{waitUntil:'load'});
    await page.evaluate(()=>{setViewerLanguage('de');showTab('quality');});
    assert.equal(await page.$eval('#engineering-section',n=>n.value),'heatmap');
    const count=await page.$$eval('tr[data-heatmap-class]',n=>n.length);assert.ok(count>0&&count<=20);
    assert.equal(await page.evaluate(()=>callModelPrepared),false);
    await page.focus('th span[data-metric-key="cbo"]');
    if(await page.$eval('#metric-tooltip',n=>n.hidden)){
      await page.screenshot({path:path.join(out,'tooltip-focus-failure-'+width+'.png')});
      console.log(JSON.stringify({width,focus:await page.evaluate(()=>({active:document.activeElement?.outerHTML,scroll:document.getElementById('tab-quality').scrollTop,target:metricTooltipTarget?.outerHTML}))}));
    }
    assert.equal(await page.$eval('#metric-tooltip',n=>n.hidden),false,'Focused metric tooltip must stay visible at '+width);
    assert.match(await page.$eval('#metric-tooltip',n=>n.textContent),/Klassenkopplung/);
    const tip=await page.$eval('#metric-tooltip',n=>{const r=n.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};});
    assert.ok(tip.left>=0&&tip.right<=width+1&&tip.top>=0&&tip.bottom<=height+1,JSON.stringify(tip));
    await page.keyboard.press('Escape');assert.equal(await page.$eval('#metric-tooltip',n=>n.hidden),true);
    await page.focus('button[data-metric-state]');
    assert.match(await page.$eval('#metric-tooltip',n=>n.textContent),/Coupling Between Objects/);
    await page.keyboard.press('Escape');
    const selected=await page.$eval('button[data-class-select]',n=>n.getAttribute('data-class-select'));
    await page.click('button[data-class-select]');
    assert.equal(await page.$eval('article[data-class-metrics]',n=>n.getAttribute('data-class-metrics')),selected);
    await page.click('button[data-class-calls]');
    assert.deepEqual(await page.evaluate(()=>({tab:ui.workspace,id:callUi.element?.id})),{tab:'calls',id:selected});
    await page.evaluate(()=>{showTab('quality');[...document.querySelectorAll('button')].find(n=>n.textContent==='Zur Heatmap').click();});
    if(!await page.$eval('#engineering-more',n=>n.hidden)){
      await page.click('#engineering-more');assert.ok(await page.$$eval('tr[data-heatmap-class]',n=>n.length)>count);
    }
    const bounds=await page.$eval('#tab-quality',n=>{n.scrollTop=0;const r=n.getBoundingClientRect();return {left:r.left,top:r.top,width:r.width,height:r.height,bottom:r.bottom,sw:n.scrollWidth,cw:n.clientWidth};});
    assert.ok(bounds.bottom<=height+1&&bounds.sw<=bounds.cw+1,'Quality must fit the viewport: '+JSON.stringify(bounds));
    await page.mouse.move(bounds.left+bounds.width*.8,bounds.top+bounds.height*.7);let end=false;
    for(let i=0;i<80;i++){
      await page.mouse.wheel({deltaY:800});await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
      if(await page.$eval('#tab-quality',n=>n.scrollTop>=n.scrollHeight-n.clientHeight-2)){end=true;break;}
    }
    assert.ok(end,'Last class must be reachable by wheel');
    const horizontal=await page.$eval('.class-heatmap-scroll',n=>{n.scrollLeft=n.scrollWidth;return {left:n.scrollLeft,max:n.scrollWidth-n.clientWidth};});
    assert.ok(horizontal.left>=horizontal.max-1,'Last metric column must be reachable');
    await page.evaluate(()=>{document.querySelector('.class-heatmap-scroll').scrollLeft=0;document.getElementById('tab-quality').scrollTop=0;});
    await page.screenshot({path:path.join(out,'heatmap-'+width+'.png')});
    if(width===1500){await page.evaluate(()=>applyTheme('dark'));await page.screenshot({path:path.join(out,'heatmap-dark.png')});await page.evaluate(()=>applyTheme('light'));}
    checks.push({width,height,exact_class_navigation:true,tooltip_keyboard_and_escape:true,tooltip_inside_viewport:true,vertical_end_reached:true,horizontal_end_reached:true});
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({url,checks,errors},null,2));console.log(JSON.stringify({checks:checks.length,errors}));
}finally{await browser.close();}
