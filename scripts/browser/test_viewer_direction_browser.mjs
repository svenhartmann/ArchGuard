// node scripts/browser/test_viewer_direction_browser.mjs REPORT CHROME PUPPETEER
// Kafka regression: exercise a real mouse click on the expanded namespace row.
import assert from 'node:assert/strict';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[];
 page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);
 page.on('request',r=>/^https?:/.test(r.url())&&!r.url().startsWith('http://127.0.0.1:')?r.abort():r.continue());
 for(const width of [1500,390]){
  await page.setViewport({width,height:1000});
  await page.goto(/^http/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
  await page.evaluate(()=>{showTab('graph');setViewerLanguage('de')});
  const id=await page.evaluate(()=>work.nodes.find(n=>n.name==='org/apache/kafka').id);
  await page.click('[data-tree-source="'+id+'"] .tree-node-name');
  assert.equal(await page.evaluate(()=>ui.selection.name),'org/apache/kafka');
  const internal=await page.evaluate(()=>{const ids=directionReference().ids;return view.edges.filter(e=>ids.has(e.from)&&ids.has(e.to)).length});
  assert.ok(internal>100,'fixture contains many internal edges that previously hid the direction change');
  const counts={};
  for(const mode of ['both','incoming','outgoing','all']){
   await page.click('[data-edges="'+mode+'"]');
   const result=await page.evaluate(()=>{
    const ids=directionReference().ids,shown=new Set(displayedGraphNodes().map(n=>n.id));
    const allowed=e=>ui.edgeMode==='all'||(ui.edgeMode==='incoming'?ids.has(e.to)&&!ids.has(e.from):ui.edgeMode==='outgoing'?ids.has(e.from)&&!ids.has(e.to):ids.has(e.from)||ids.has(e.to));
    const expected=view.edges.filter(e=>shown.has(e.from)&&shown.has(e.to)&&allowed(e)).length;
    const rendered=[...document.querySelectorAll('[data-tree-edge]')].map(n=>{const [from,to]=n.getAttribute('data-tree-edge').split(' ').map(Number);return {from,to}});
    return {mode:ui.edgeMode,selected:ui.selection.name,expected,count:Number(document.querySelector('[data-tree-edge-count]').getAttribute('data-tree-edge-count')),correctPaths:rendered.every(allowed),pressed:document.querySelector('[data-edges="'+ui.edgeMode+'"]').getAttribute('aria-pressed')};
   });
   assert.equal(result.mode,mode);assert.equal(result.pressed,'true');
   assert.equal(result.selected,'org/apache/kafka');
   assert.equal(result.count,result.expected);assert.equal(result.correctPaths,true);
   counts[mode]=result.count;
  }
  assert.ok(counts.incoming<counts.both&&counts.outgoing<counts.both,'internal package edges disappear from both one-direction filters');
  console.log(JSON.stringify({width,internal,counts}));
 }
 assert.deepEqual(errors,[]);
 console.log('Kafka mouse selection: package boundary filters, rendered paths, counts, selection retention and mobile passed');
}finally{await browser.close()}
