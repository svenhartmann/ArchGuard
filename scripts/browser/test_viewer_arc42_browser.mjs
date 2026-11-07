// Real browser check for chapter selection, rendered Markdown, diagrams, downloads and scrolling.
// node scripts/browser/test_viewer_arc42_browser.mjs REPORT_OR_LOCAL_URL CHROMIUM PUPPETEER_ENTRY OUTPUT_DIR
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const [report, executablePath, modulePath, output] = process.argv.slice(2);
assert.ok(report && executablePath && modulePath && output, 'Provide a report, Chromium, Puppeteer entry and output directory');
const {default: puppeteer} = await import(pathToFileURL(path.resolve(modulePath)).href);
const out = path.resolve(output), kind = 'arc42';
fs.mkdirSync(out, {recursive:true});
const url = /^https?:/.test(report) ? report : pathToFileURL(path.resolve(report)).href;
const browser = await puppeteer.launch({executablePath, headless:true, args:['--no-sandbox']});
const errors=[],checks=[];
try {
  const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));
  const downloads=fs.mkdtempSync(out+'/downloads-'+kind+'-');
  const session=await page.createCDPSession();await session.send('Page.setDownloadBehavior',{behavior:'allow',downloadPath:downloads});
  await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
  for(const [width,height] of [[1500,900],[760,700],[390,640]]) {
    await page.setViewport({width,height});await page.goto(url,{waitUntil:'load'});
    await page.waitForSelector('#architecture-content');
    await page.evaluate(()=>{setViewerLanguage('de');showTab('architecture');});
    const catalogue=await page.evaluate(()=>data.insights.arc42.diagrams?.length?data.insights.arc42.diagrams.map(d=>({id:d.id,chapter:d.chapter,kind:d.kind,shown:d.shown,elements:d.elements.length})):null);
    assert.equal(await page.$$eval('select.arc42-mobile option',n=>n.map(o=>o.value).filter(v=>v!=='diagrams').length),12);
    assert.equal(await page.$$eval('select.arc42-mobile option[value="diagrams"]',n=>n.length),catalogue?1:0,'the Diagrams view follows the chapters when the report has a catalogue');
    await page.select('select.arc42-mobile','5');
    assert.match(await page.$eval('article[data-arc42-chapter] h2',n=>n.textContent),/5\. Bausteinsicht/);
    assert.ok(await page.$('article[data-arc42-chapter] svg'));
    assert.equal(await page.$eval('#tab-architecture',n=>n.scrollTop),0,'Chapter selection keeps the header visible');
    assert.ok(await page.$$eval('g[data-arc42-edge] path',paths=>paths.every(p=>{const b=p.getBBox();return b.y+b.height<=p.ownerSVGElement.height.baseVal.value+1;})),'The scrollable diagram retains all of its edge geometry');
    assert.ok(await page.$$eval('g[data-arc42-edge] path',n=>n.length)>0);
    assert.equal(await page.$$eval('#architecture-content details[data-pattern]',n=>n.length),0);
    assert.doesNotMatch(await page.$eval('#architecture-content',n=>n.textContent),/No target architecture configured/);
    const packageStats=await page.evaluate(()=>{const m=arc42PackageProjection();return {groups:m.allNodes.length,represented:m.nodes.reduce((sum,n)=>sum+n.files,0),files:data.nodes.filter(n=>n.kind==='file').length,total:m.total,edges:data.edges.length};});
    assert.equal(packageStats.represented,packageStats.files);assert.equal(packageStats.total,packageStats.edges);
    await page.click('g[data-arc42-node]');
    assert.ok(await page.$('g[data-arc42-node][aria-pressed="true"]'));
    const selectedEdges=await page.evaluate(()=>{const m=arc42PackageProjection(),id=document.querySelector('g[data-arc42-node][aria-pressed="true"]').getAttribute('data-arc42-node');return m.edges.filter(e=>e.from===id||e.to===id).length;});
    assert.equal(await page.$$eval('g[data-arc42-edge]',n=>n.length),selectedEdges);
    const scopeValue=await page.$eval('select[data-arc42-scope]',n=>n.options[1].value);
    await page.select('select[data-arc42-scope]',scopeValue);
    assert.equal(await page.$eval('select[data-arc42-scope]',n=>n.value),scopeValue);
    await page.select('select[data-arc42-scope]','');
    await page.screenshot({path:out+'/arc42-building-blocks-'+width+'.png'});
    await page.select('select.arc42-mobile','6');
    assert.ok(await page.$('[data-arc42-package-map="calls"] svg'));
    await page.select('select.arc42-mobile','7');
    if(catalogue){
      // ADR 0006: one catalogue figure per deployment file, its C4 source cut from the text.
      const files=catalogue.filter(d=>d.kind==='deployment');assert.ok(files.length>0);
      for(const d of files){const nodes=await page.$$eval('section[data-arc42-diagram="'+d.id+'"] g[data-arc42-node]',n=>n.length);assert.equal(nodes,d.shown?d.elements:0,d.id);}
      assert.doesNotMatch(await page.$eval('article[data-arc42-chapter]',n=>n.textContent),/C4Deployment|Deployment_Node|syntax is not supported/);
    } else {
      const deployments=await page.$$eval('select[data-arc42-deployment] option',nodes=>nodes.map(n=>n.value));
      assert.ok(deployments.length>0);
      for(const file of deployments){await page.select('select[data-arc42-deployment]',file);const expected=await page.evaluate(f=>arc42DeploymentProjection(f).nodes.length,file);assert.equal(await page.$$eval('g[data-arc42-node]',n=>n.length),expected);}
      await page.select('select[data-arc42-deployment]',deployments[0]);
    }
    await page.screenshot({path:out+'/arc42-deployment-'+width+'.png'});
    await page.select('select.arc42-mobile','11');
    assert.ok(await page.$('[data-arc42-package-map="cycles"]'));
    await page.select('select.arc42-mobile','10');
    assert.match(await page.$eval('article[data-arc42-chapter]',n=>n.textContent),/Gemessene Klassenwerte/);
    assert.ok(await page.$$eval('article[data-arc42-chapter] table',n=>n.length)>1);
    await page.evaluate(()=>{setViewerLanguage('en');setViewerLanguage('de');showTab('graph');showTab('architecture');});
    assert.equal(await page.$eval('article[data-arc42-chapter]',n=>n.dataset.arc42Chapter),'10');
    const bounds=await page.$eval('#tab-architecture',n=>{const r=n.getBoundingClientRect();return {left:r.left,width:r.width,top:r.top,height:r.height,sh:n.scrollHeight,ch:n.clientHeight,sw:n.scrollWidth,cw:n.clientWidth,bottom:r.bottom,vh:innerHeight};});
    assert.ok(bounds.sh>bounds.ch,'Fixture must actually overflow');
    if(bounds.sw>bounds.cw+2){console.log(JSON.stringify({bounds,nodes:await page.$$eval('#tab-architecture *',nodes=>nodes.map(n=>{const r=n.getBoundingClientRect();return {tag:n.tagName,id:n.id,cls:n.className,text:n.textContent?.slice(0,100),left:r.left,right:r.right,width:r.width,scrollWidth:n.scrollWidth,clientWidth:n.clientWidth,overflow:getComputedStyle(n).overflowX}}).filter(x=>x.right>innerWidth+2||x.scrollWidth>x.clientWidth+2))},null,2));await page.screenshot({path:out+'/overflow-sequence.png'});}assert.ok(bounds.bottom<=height+2,JSON.stringify(bounds));assert.ok(bounds.sw<=bounds.cw+2,'No horizontal overflow: '+JSON.stringify(bounds));
    await page.mouse.move(bounds.left+bounds.width*.75,bounds.top+bounds.height*.6);
    let end=false;
    for(let i=0;i<100;i++) {
      await page.mouse.wheel({deltaY:900});await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
      if(await page.$eval('#tab-architecture',n=>n.scrollTop>=n.scrollHeight-n.clientHeight-2)){end=true;break;}
    }
    assert.ok(end,'Mouse wheel must reach the bottom at '+width);
    await page.$eval('#tab-architecture',n=>n.scrollTop=0);
    await page.screenshot({path:out+'/arc42-'+kind+'-'+width+'.png'});
    checks.push({width,height,chapters:12,chapter_selection:true,mouse_wheel_reached_end:true,no_horizontal_overflow:true});
    if(width===1500) {
      await page.select('select.arc42-mobile','5');
      const expected=await page.$eval('pre[data-arc42-source]',n=>n.textContent);
      await page.click('button[data-arc42-download]');
      const target=downloads+'/arc42-05.md';
      for(let i=0;i<100&&(!fs.existsSync(target)||fs.statSync(target).size!==Buffer.byteLength(expected));i++)await new Promise(r=>setTimeout(r,50));
      assert.equal(fs.readFileSync(target,'utf8'),expected);
      checks.push({download:'arc42-05.md',matches_source:true});
    }
  }
  assert.deepEqual(errors,[]);
  fs.writeFileSync(out+'/browser-'+kind+'.json',JSON.stringify({url,checks,errors},null,2));
  console.log(JSON.stringify({kind,checks:checks.length,errors}));
} finally {await browser.close();}
