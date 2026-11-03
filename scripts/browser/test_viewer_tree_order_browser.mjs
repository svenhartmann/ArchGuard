// Usage: node scripts/browser/test_viewer_tree_order_browser.mjs REPORT CHROMIUM PUPPETEER OUTPUT
// REPORT uses the 30-file/231-edge fixture from test_viewer_tree_order.js.
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
  await page.setRequestInterception(true);page.on('request',r=>/^https?:/.test(r.url())?r.abort():r.continue());
  for(const width of [1500,760,390]){
    await page.setViewport({width,height:980});await page.goto(pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
    await page.evaluate(()=>{setViewerLanguage('de');showTab('graph');setAllTreeBranches(true);ui.edgeMode='all';ui.cyclesOnly=false;renderAll();});
    assert.equal(await page.$$eval('#package-tree [data-tree-edge]',es=>es.length),231);
    const dataBefore=await page.evaluate(()=>JSON.stringify([data,work,view,sccs,currentAcd,ops]));
    for(const sort of ['alphabetical','connections']){
      if(!await page.$eval('[data-tree-options]',e=>e.open))await page.click('[data-tree-options] > summary');
      await page.click('[data-tree-sort="'+sort+'"]');
      assert.equal(await page.evaluate(()=>treeSort),sort);
      assert.equal(await page.evaluate(()=>document.activeElement.getAttribute('data-tree-sort')),sort);
      assert.equal(await page.$$eval('#package-tree [data-tree-edge]',es=>es.length),231);
      assert.equal(await page.evaluate(()=>JSON.stringify([data,work,view,sccs,currentAcd,ops])),dataBefore);
    }
    await page.keyboard.press('Escape');
    const rows=await page.$$eval('#package-tree [data-tree-source]',es=>es.map(e=>[e.dataset.treeSource,e.getAttribute('transform')]));
    await page.evaluate(()=>selectNode('Api/OrdersController.cs'));
    // At compact widths the Inspector is an intentional overlay; dismiss it
    // through its normal control before clicking the graph underneath it.
    if(width<=1100)await page.click('#toggle-inspector');
    const expected=await page.evaluate(()=>{const id=view.nodes.find(n=>n.name==='Api/OrdersController.cs').id;return {incoming:view.edges.filter(e=>e.to===id).length,outgoing:view.edges.filter(e=>e.from===id).length};});
    for(const mode of ['incoming','outgoing']){
      await page.click('#edge-modes [data-edges="'+mode+'"]');
      assert.equal(await page.$$eval('#package-tree [data-tree-edge]',es=>es.length),expected[mode]);
      assert.deepEqual(await page.$$eval('#package-tree [data-tree-source]',es=>es.map(e=>[e.dataset.treeSource,e.getAttribute('transform')])),rows);
    }
    await page.click('#edge-modes [data-edges=all]');await page.click('#toggle-cycles');
    assert.equal(await page.$$eval('#package-tree [data-tree-edge]',es=>es.length),111,JSON.stringify(await page.evaluate(()=>({mode:ui.edgeMode,cycles:ui.cyclesOnly,selection:ui.selection,shown:displayedGraphNodes().length,view:view.nodes.length}))));
    await page.click('#toggle-cycles');
    assert.equal(await page.evaluate(()=>treeRouting),'direction','directional layout is the default');
    await page.click('[data-tree-options] summary');await page.click('[data-tree-routing]');
    assert.equal(await page.evaluate(()=>treeRouting),'ordered');
    await page.click('[data-tree-routing]');
    assert.equal(await page.evaluate(()=>treeRouting),'direction');
    assert.deepEqual(await page.$$eval('#package-tree [data-tree-source]',es=>es.map(e=>[e.dataset.treeSource,e.getAttribute('transform')])),rows);
    await page.keyboard.press('Escape');assert.equal(await page.$eval('[data-tree-options]',e=>e.open),false);
    await page.evaluate(()=>{selectNode('Storage/OrderRepository.cs');ui.edgeMode='all';renderAll();});
    if(width<=1100&&!(await page.$eval('#inspector',e=>e.classList.contains('panel-hidden'))))await page.click('#toggle-inspector');
    const directions=await page.evaluate(()=>{
      const reference=directionReference(),row=document.querySelector('[data-tree-row]'),bounds=row.getBoundingClientRect();
      return [...document.querySelectorAll('[data-tree-edge]')].map(g=>{
        const [a,b]=g.dataset.treeEdge.split(' ').map(Number),cyclic=cyclicKeys.has(g.dataset.treeEdge),side=g.dataset.treeSide,line=g.querySelector('.tree-edge-line'),box=line.getBoundingClientRect();
        return {side,cyclic,color:line.getAttribute('stroke'),cycleColor:C.cycle,expected:!cyclic&&reference.ids.has(b)&&!reference.ids.has(a)?'left':'right',outside:side==='left'?box.right<bounds.left:box.left>bounds.right};
      });
    });
    assert.ok(directions.some(d=>d.side==='left'),'fixture exercises noncyclic incoming edges');
    assert.ok(directions.some(d=>d.cyclic),'fixture exercises cycles alongside incoming edges');
    assert.ok(directions.every(d=>d.side===d.expected&&d.outside&&(!d.cyclic||d.color===d.cycleColor)),'all tracks stay outside the label column on the correct side, with cycles red on the right');
    await page.mouse.move(1,1);await page.evaluate(()=>{hideContextTooltip();document.activeElement?.blur();});
    if(width===1500)await page.screenshot({path:path.join(out,'tree-in-out-1500.png')});
    await page.evaluate(()=>{focusCycle(cyclesForView()[0]);});
    assert.equal(await page.evaluate(()=>ui.selection.kind),'cycle');
    assert.ok((await page.$$eval('#package-tree [data-tree-edge]',es=>es.length))>0);
    await page.evaluate(()=>{clearCycle();treeRouting='direction';ui.edgeMode='all';ui.cyclesOnly=false;ui.selection=null;reconcileSelection();renderAll();setPackageTreeZoom(1);});
    if(width<=1100&&!(await page.$eval('#inspector',e=>e.classList.contains('panel-hidden'))))await page.click('#toggle-inspector');
    for(const theme of ['light','dark']){
      await page.evaluate(theme=>applyTheme(theme),theme);
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false,'page overflow at '+width);
      const toolbar=await page.$eval('[data-tree-toolbar]',bar=>{const box=bar.getBoundingClientRect();return [...bar.querySelectorAll('button')].filter(b=>b.checkVisibility()&&!b.closest('details')).every(b=>{const r=b.getBoundingClientRect();return r.left>=box.left-1&&r.right<=box.right+1&&r.bottom<=box.bottom+1;});});
      assert.equal(toolbar,true,'visible toolbar controls must wrap within the pane');
      assert.doesNotMatch(await page.$eval('#package-tree',e=>e.textContent),/Umlauf/);
      await page.mouse.move(1,1);await page.evaluate(()=>{hideContextTooltip();document.activeElement?.blur();});
      await page.screenshot({path:path.join(out,'tree-'+theme+'-'+width+'.png')});
      checks.push({width,theme,edges:await page.$$eval('#package-tree [data-tree-edge]',es=>es.length)});
    }
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({checks,errors},null,2));
  console.log(JSON.stringify({views:checks.length,inOut:true,cycles:true,sort:true,evidenceFixture:231,errors}));
}catch(error){const page=(await browser.pages()).at(-1);if(page){await page.screenshot({path:path.join(out,'browser-failure.png')});console.log(await page.evaluate(()=>['#edge-modes [data-edges=all]','#toggle-cycles'].map(selector=>{const el=document.querySelector(selector),r=el.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);return {selector,rect:r.toJSON(),disabled:el.disabled,hit:hit?.outerHTML.slice(0,600)};})));}throw error;}finally{await browser.close();}
