// Usage: node scripts/browser/test_viewer_tree_kafka_browser.mjs REPORT CHROME PUPPETEER OUTPUT
// Uses the real pinned Kafka example, including all 16,108 directed dependencies.
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2),out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href),browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
const results=[],errors=[];
try{
 const page=await browser.newPage();page.setDefaultTimeout(90000);page.setDefaultNavigationTimeout(90000);page.on('pageerror',e=>errors.push(String(e)));
 await page.setViewport({width:1500,height:980});await page.setRequestInterception(true);page.on('request',r=>/^https?:/.test(r.url())?r.abort():r.continue());
 await page.goto(pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});await page.click('[data-tab=graph]');
 const settle=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 const stats=()=>page.$eval('#package-tree',tree=>({rows:tree.querySelectorAll('[data-tree-source]').length,edges:tree.querySelectorAll('[data-tree-edge]').length,total:Number(tree.querySelector('svg').getAttribute('data-tree-edge-count')),dom:tree.querySelectorAll('*').length,scroll:tree.scrollTop}));
 // A collapsed package has callers in direct files hidden by the initial mixed cut.
 const pkg=await page.evaluate(()=>work.nodes.find(n=>n.name==='org/apache/kafka/controller/errors').id);
 await page.click('[data-tree-source="'+pkg+'"]');
 for(const mode of ['incoming','outgoing']){
  await page.click('#edge-modes [data-edges='+mode+']');
  const result=await stats();assert.ok(result.total>0,mode+' must reveal related file endpoints');assert.equal(result.edges,result.total);results.push({step:'package-'+mode,...result});
 }
 await page.evaluate(()=>{ui.selection=null;reconcileSelection();ui.edgeMode='all';renderAll();});
 await page.click('[data-tree-options] > summary');const started=Date.now();await page.click('[data-tree-all=expand]');await page.keyboard.press('Escape');await settle();results.push({step:'expand',wallMs:Date.now()-started,...await stats()});
 const baseline=await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,view,sccs,currentAcd,ops]));
 assert.equal((await stats()).total,16108);assert.ok((await stats()).rows<100);assert.ok((await stats()).dom<10000);
 for(const fraction of [.2,.5,.9]){
  await page.$eval('#package-tree',(tree,fraction)=>{tree.scrollTop=(tree.scrollHeight-tree.clientHeight)*fraction;},fraction);await settle();
  const result=await stats();assert.ok(result.rows<110&&result.dom<10000,'scrolling must retain a bounded scene');assert.equal(result.total,16108);results.push({step:'scroll-'+fraction,...result});
 }
 const deepPackage=await page.evaluate(()=>{const n=treeModelCache.layout.nodes.find(n=>n.name==='org/apache/kafka/streams/state/internals');document.getElementById('package-tree').scrollTop=n.y-180;ui.edgeMode='both';window.contextAggregations=0;const original=aggregate;window.restoreAggregate=()=>aggregate=original;aggregate=function(...args){contextAggregations++;return original(...args);};return n.src;});await settle();
 const packageStarted=Date.now();await page.click('[data-tree-source="'+deepPackage+'"]');await settle();
 const builds=await page.evaluate(()=>{restoreAggregate();return contextAggregations;});assert.ok(builds<=30,'unrelated history cuts must not be reaggregated');
 assert.equal((await stats()).total,2901,'all detailed and historical package-cycle paths are preserved');results.push({step:'expanded-package',wallMs:Date.now()-packageStarted,aggregations:builds,...await stats()});
 for(const [mode,count] of [['incoming',218],['outgoing',1143]]){await page.click('#edge-modes [data-edges='+mode+']');await settle();assert.equal((await stats()).total,count);}
 const file=await page.evaluate(()=>{const n=treeModelCache.layout.nodes.find(n=>n.name==='org/apache/kafka/clients/CommonClientConfigs.java');document.getElementById('package-tree').scrollTop=n.y-180;return n.src;});await settle();
 await page.click('[data-tree-source="'+file+'"]');await settle();
 assert.equal(await page.evaluate(()=>ui.selection.name),'org/apache/kafka/clients/CommonClientConfigs.java');
 for(const [mode,count] of [['incoming',55],['outgoing',7]]){
  await page.evaluate(()=>{const original=renderPackageTree;window.lastTreeRenderMs=0;renderPackageTree=function(...args){const start=performance.now();try{return original(...args);}finally{lastTreeRenderMs=performance.now()-start;renderPackageTree=original;}};});
  await page.click('#edge-modes [data-edges='+mode+']');await settle();const result=await stats();
  assert.equal(result.total,count);assert.equal(result.edges,count,'selected endpoint keeps all matching edges visibly mounted');
  assert.ok(await page.$eval('#package-tree .is-selected[data-tree-source]',row=>{const r=row.getBoundingClientRect(),tree=document.getElementById('package-tree').getBoundingClientRect();return r.top>=tree.top&&r.bottom<=tree.bottom;}),'selection stays in view');
  results.push({step:'file-'+mode,renderMs:await page.evaluate(()=>lastTreeRenderMs),...result});
 }
 await page.$eval('#package-tree .is-selected[data-tree-source]',node=>node.focus());await page.keyboard.press('End');await settle();
 assert.equal(await page.evaluate(()=>document.activeElement.getAttribute('data-tree-source')),await page.evaluate(()=>String(treeModelCache.layout.nodes.at(-1).src)),'keyboard reaches unmounted rows');
 await page.keyboard.press('Home');await settle();assert.equal(await page.evaluate(()=>document.activeElement.getAttribute('data-tree-source')),'project');
 // Explicit selection of an offscreen file also reveals it, without changing the graph cut.
 await page.evaluate(()=>{document.getElementById('package-tree').scrollTop=0;ui.selection=null;reconcileSelection();renderPackageTree();selectNode('org/apache/kafka/clients/CommonClientConfigs.java');});await settle();
 assert.ok((await stats()).scroll>0);assert.equal((await stats()).edges,7);
 await page.click('#edge-modes [data-edges=all]');await page.click('[data-tree-zoom=fit]');await settle();
 assert.ok((await stats()).dom<16000,'fit uses simplified rows and combined strokes, not a full detailed DOM');assert.equal((await stats()).total,16108);
 results.push({step:'fit',zoom:await page.evaluate(()=>treeZoom),...await stats()});
 await page.screenshot({path:path.join(out,'kafka-tree-fit.png')});
 await page.click('[data-tree-zoom=reset]');await settle();
 // SVG export temporarily mounts the complete scene, then restores windowing.
 const exported=await page.evaluate(()=>{const svg=exportSvg();return {rows:(svg.match(/data-tree-source=/g)||[]).length,edges:(svg.match(/data-tree-edge=/g)||[]).length};});
 assert.equal(exported.rows,await page.evaluate(()=>treeModelCache.layout.nodes.length));assert.equal(exported.edges,16108);assert.ok((await stats()).rows<110);
 assert.equal(await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,view,sccs,currentAcd,ops])),baseline,'scroll, filters, zoom and export preserve all source evidence and analysis');
 await page.evaluate(()=>selectNode('org/apache/kafka/clients/CommonClientConfigs.java'));await page.click('#edge-modes [data-edges=incoming]');await settle();
 await page.mouse.move(1,1);await page.screenshot({path:path.join(out,'kafka-tree-incoming.png')});
 assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({results,exported,errors},null,2));console.log(JSON.stringify({results,exported,errors}));
}catch(error){console.error(JSON.stringify({results,errors}));throw error;}finally{await browser.close();}
