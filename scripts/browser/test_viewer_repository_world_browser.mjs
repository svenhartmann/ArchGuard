// node scripts/browser/test_viewer_repository_world_browser.mjs URL CHROME PUPPETEER OUTPUT
import assert from 'node:assert/strict';
import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
const [url,executablePath,modulePath,output]=process.argv.slice(2);
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
fs.mkdirSync(output,{recursive:true});
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{
 const page=await browser.newPage(),errors=[],checks=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.setRequestInterception(true);page.on('request',r=>/^https?:/.test(r.url())&&new URL(r.url()).origin!==new URL(url).origin?r.abort():r.continue());
 const ready=()=>page.waitForFunction(()=>typeof worldUi!=='undefined'&&ui.workspace==='start'&&worldUi.layout&&window.archguardLiveViewer,{timeout:180000});
 await page.setViewport({width:1500,height:1050});
 await page.goto(url,{waitUntil:'load'});await ready();
 const projects=await page.evaluate(()=>repositorySelectableProjects().map(p=>({path:p.path,kind:p.kind})));
 assert.ok(projects.length>=4);
 assert.equal(await page.evaluate(()=>data.nodes.length),0,'repository overview does not merge unrelated source graphs');
 assert.equal(await page.$$eval('[data-world-area]',ns=>ns.length),await page.evaluate(()=>repositoryWorldProjects().length));
 assert.equal(await page.$eval('#world-lens-complexity',e=>e.hidden),true);
 await page.evaluate(()=>setViewerLanguage('de'));
 await page.screenshot({path:path.join(output,'services-desktop.png')});
 for(const project of projects.filter(p=>p.kind==='application')){
  if(await page.evaluate(()=>!isRepositoryOverview())){await page.goto(url,{waitUntil:'load'});await ready();}
  console.log('Opening '+project.path);
  if(!checks.length){
   const point=await page.evaluate(name=>{
    const canvas=document.getElementById('world-canvas'),b=canvas.getBoundingClientRect();
    for(const hit of worldUi.hits.filter(h=>h.cell.area===name))for(const triangle of hit.triangles){
     const x=b.left+triangle.reduce((n,p)=>n+p.x,0)/3,y=b.top+triangle.reduce((n,p)=>n+p.y,0)/3;
     if(document.elementFromPoint(x,y)===canvas)return {x,y};
    }
   },project.path);
   assert.ok(point,'a service island is clickable independently of its label');
   await page.mouse.click(point.x,point.y);assert.equal(await page.evaluate(()=>isRepositoryOverview()),true);
   await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.click('#world-service-open')]);
  }else await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.select('#world-area',project.path)]);
  await ready();
  assert.equal(await page.evaluate(()=>repositoryModel().active_project),project.path);
  assert.equal(new URL(page.url()).searchParams.get('project'),project.path);
  assert.ok(await page.evaluate(()=>data.insights.world.entities.length>0),'source project exposes class/file world evidence');
  assert.equal(await page.$eval('#start-world',e=>e.hidden),false);
  assert.ok(await page.$('#world-scope a[href="/#tab=start"]'),'world breadcrumb leads back to repository');
  const entities=await page.evaluate(()=>data.insights.world.entities.length);
  const classes=await page.evaluate(()=>data.insights.world.entities.filter(e=>e.class_id).length);
  // Use actual package pins, then a file pin to open the details overlay.
  let depth=0;
  while(depth<12){
   const area=await page.evaluate(()=>worldUi.model.areas.find(a=>a.entities.length&&a.kind==='module')?.name);
   if(!area)break;
   await page.click('[data-world-area="'+area+'"]');depth++;
  }
  const file=await page.evaluate(()=>worldUi.model.areas.find(a=>a.kind==='file'&&a.entities.length)?.name);
  assert.ok(file,'package drill-down reaches source files');
  await page.click('[data-world-area="'+file+'"]');
  assert.equal(await page.$eval('#world-detail',e=>e.hidden),false);
  const sourcePath=new URL(page.url()).href;
  await page.reload({waitUntil:'load'});await ready();
  assert.equal(new URL(page.url()).searchParams.get('project'),project.path);
  assert.equal(await page.evaluate(()=>worldUi.within),new URL(sourcePath).hash.match(/scope=([^&]+)/)?.[1]?decodeURIComponent(new URL(sourcePath).hash.match(/scope=([^&]+)/)[1]):null,'scope survives a shared/reloaded link');
  await page.screenshot({path:path.join(output,project.path.split('/').pop()+'.png')});
  await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.click('#world-scope a[href="/#tab=start"]')]);await ready();
  assert.equal(await page.evaluate(()=>isRepositoryOverview()),true);
  checks.push({project:project.path,entities,classes,packageLevels:depth,details:true,reload:true,back:true});
  console.log(JSON.stringify(checks.at(-1)));
 }
 await page.setViewport({width:390,height:950});await page.goto(url,{waitUntil:'load'});await ready();
 assert.equal(await page.$$eval('#world-area option',ns=>ns.length),projects.length+1,'mobile picker includes every project even if pins overlap');
 await page.screenshot({path:path.join(output,'services-mobile.png')});
 const first=projects.find(p=>p.kind==='application');
 await Promise.all([page.waitForNavigation({waitUntil:'load'}),page.select('#world-area',first.path)]);await ready();
 assert.equal(await page.evaluate(()=>repositoryModel().active_project),first.path);
 assert.deepEqual(errors,[]);
 fs.writeFileSync(path.join(output,'checks.json'),JSON.stringify({checks,mobile:true,errors},null,2));
 console.log('Repository World: service click, source World, package/file details, deep links, return navigation and mobile passed');
}finally{await browser.close()}
