// node scripts/browser/test_viewer_inspector_overlay_browser.mjs REPORT CHROMIUM PUPPETEER OUTPUT
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
const errors=[],checks=[];
try{
  const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));
  await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
  const url=/^https?:/.test(report)?report:pathToFileURL(path.resolve(report)).href;
  const visible=()=>page.$eval('#inspector',n=>n.checkVisibility());
  const close='button[data-inspector-action="close"]',pin='button[data-inspector-action="pin"]';
  const state=()=>page.evaluate(()=>{const tree=document.getElementById('package-tree'),rect=document.getElementById('tab-graph').getBoundingClientRect();return {selection:ui.selection,edgeMode:ui.edgeMode,zoom:treeZoom,top:tree.scrollTop,left:tree.scrollLeft,rect:[rect.x,rect.y,rect.width,rect.height]};});
  const settle=()=>page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
  async function clickRow(selector){
    await page.$eval(selector,n=>{const tree=document.getElementById('package-tree'),bounds=tree.getBoundingClientRect(),header=tree.querySelector('[data-tree-toolbar]').getBoundingClientRect(),rect=n.getBoundingClientRect();tree.scrollTop+=rect.y+rect.height/2-(header.bottom+Math.min(24,(bounds.bottom-header.bottom)/2));tree.scrollLeft+=Math.max(0,rect.x+rect.width/2-bounds.right+20);});
    await settle();
    const point=await page.$eval(selector,n=>{const r=n.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};});await page.mouse.click(point.x,point.y);await settle();
  }
  for(const [width,height] of [[1500,900],[900,760],[390,844]]){
    await page.setViewport({width,height});await page.goto(url,{waitUntil:'load'});await page.evaluate(()=>{setViewerLanguage('de');showTab('graph');});
    await settle();assert.equal(await visible(),false);
    const ids=await page.$$eval('g[data-tree-source]',nodes=>nodes.map(n=>n.getAttribute('data-tree-source')).filter(id=>id!=='project'));
    assert.ok(ids.length>1,'fixture has at least two selectable packages');
    const first='g[data-tree-source="'+ids[0]+'"]';
    await clickRow(first+' .tree-node-name');
    assert.equal(await visible(),false,'single click only selects');
    assert.equal((await state()).selection?.kind,'node');
    await page.keyboard.press('Enter');await settle();
    assert.equal(await visible(),true,'Enter after a click opens details');
    const bounds=await page.$eval('#inspector',n=>{const r=n.getBoundingClientRect();return {width:r.width,right:r.right,bottom:r.bottom,top:r.top};});
    assert.equal(bounds.width,Math.min(420,width-48));assert.ok(bounds.right<=width);assert.ok(bounds.bottom<=height);
    await page.keyboard.press('Escape');await settle();assert.equal(await visible(),false);
    await page.evaluate(()=>{setPackageTreeZoom(1.25);const tree=document.getElementById('package-tree');tree.scrollTop=80;tree.scrollLeft=40;});
    const before=await state();
    await page.click('#toggle-inspector');await settle();assert.equal(await visible(),true);assert.deepEqual(await state(),before,'opening must not change geometry or viewport');
    await page.screenshot({path:out+'/details-overlay-'+width+'.png'});
    await page.click(close);await settle();assert.deepEqual(await state(),before,'close button preserves selection, directions, zoom, scroll and geometry');
    await page.click('#toggle-inspector');await page.keyboard.press('Escape');await settle();assert.deepEqual(await state(),before,'Escape also preserves the complete graph state');
    // Real graph interaction while the nonmodal panel is open, on a wide screen.
    if(width===1500){
      await page.evaluate(()=>{setPackageTreeZoom(1);document.getElementById('package-tree').scrollTop=0;document.getElementById('package-tree').scrollLeft=0;});
      const next=await page.$$eval('g[data-tree-source]',(nodes,first)=>nodes.map(n=>n.getAttribute('data-tree-source')).find(id=>id!=='project'&&id!==first),ids[0]);assert.ok(next);
      await page.click('#toggle-inspector');await clickRow('g[data-tree-source="'+next+'"] .tree-node-name');
      assert.equal(await visible(),true);assert.equal(await page.$eval('#sidebar h2',n=>n.textContent),(await state()).selection.name);
      assert.equal(await page.$eval('#inspector',n=>n.contains(document.activeElement)),false,'the drawer must not steal focus from graph selection');
      const live=await state();await page.keyboard.press('Escape');await settle();assert.deepEqual(await state(),live);
      await page.click('#toggle-inspector');const overlay=await state();
      await page.click(pin);await settle();assert.equal(await page.$eval('#main',n=>n.classList.contains('inspector-pinned')),true);
      assert.ok((await state()).rect[2]<overlay.rect[2]-400,'pinning explicitly reserves a column');
      await page.screenshot({path:out+'/details-pinned.png'});
      await page.click(pin);await settle();assert.equal((await state()).rect[2],overlay.rect[2]);
      await page.click(close);
      const edge=await page.$('g[data-tree-edge]');
      assert.ok(edge);await edge.evaluate(n=>n.focus());await page.keyboard.press('Enter');await settle();
      assert.equal(await visible(),true);assert.equal((await state()).selection.kind,'edge');
      await page.keyboard.press('Escape');await page.evaluate(()=>showTab('architecture'));await settle();assert.equal(await visible(),false);
      assert.ok(await page.$('section[data-arc42]'),'the arc42 reader remains available');
    }else{
      await page.click('#toggle-inspector');assert.equal(await page.$eval(pin,n=>n.disabled),true);await page.click(close);
    }
    checks.push({width,height,explicit_open:true,viewport_preserved:true,keyboard:true,pin:width===1500});
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(out+'/checks.json',JSON.stringify({checks,errors},null,2));console.log(JSON.stringify({checks,errors}));
}finally{await browser.close();}
