// Petclinic acceptance: config/source separation and service drill-down. Synthetic
// trace evidence is injected only into this isolated headless page, never saved.
import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
const [url,executablePath,modulePath,output]=process.argv.slice(2);const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);fs.mkdirSync(output,{recursive:true});
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
try{const p=await browser.newPage(),errors=[];p.on('pageerror',e=>errors.push(String(e)));await p.setViewport({width:1500,height:1100});await p.goto(url,{waitUntil:'load'});const ready=()=>p.waitForFunction(()=>typeof worldUi!=='undefined'&&worldUi.layout&&ui.workspace==='start',{timeout:180000});await ready();
 await p.evaluate(()=>setViewerLanguage('de'));
 await p.select('#world-service-source','code');const code=await p.evaluate(()=>repositoryVisibleConnections().length);assert.ok(code>0);
 const selected=await p.evaluate(()=>repositoryVisibleConnections()[0].from);
 await p.select('#world-service-source','configuration');const configuration=await p.evaluate(()=>repositoryVisibleConnections().length);assert.ok(configuration>0);
 await p.select('#world-service-source','observed');assert.equal(await p.evaluate(()=>repositoryVisibleConnections().length),0,'no traces means unknown traffic, not invented observations');
 await p.select('#world-service-source','all');await p.evaluate(path=>repositorySelectWorld(path),selected);
 await p.click('[data-world-details]');assert.match(await p.$eval('#world-detail',e=>e.textContent),/application\.yml|\.java/);await p.keyboard.press('Escape');
 await p.screenshot({path:path.join(output,'petclinic-connections.png')});
 await Promise.all([p.waitForNavigation({waitUntil:'load'}),p.click('#world-service-open')]);await ready();assert.equal(new URL(p.url()).searchParams.get('project'),selected);assert.ok(await p.evaluate(()=>data.insights.world.entities.length));
 await p.reload({waitUntil:'load'});await ready();assert.equal(new URL(p.url()).searchParams.get('project'),selected);
 await Promise.all([p.waitForNavigation({waitUntil:'load'}),p.click('#world-scope a[href="/#tab=start"]')]);await ready();
 await p.evaluate(()=>{const from=data.services.services.find(s=>s.project);data.services.services.push({id:'trace:test-external',name:'test-external',project:null,compose_file:'',networks:[],profiles:[]});data.services.dependencies.push({from:from.id,to:'trace:test-external',kind:'http_call',evidence:[{file:'.archguard/otel-traces.json',pointer:'/resourceSpans/0',description:'Synthetic UI test: observed calls=1'}]});renderRepositoryWorld();});
 await p.select('#world-service-source','observed');assert.equal(await p.evaluate(()=>repositoryVisibleConnections().length),1);
 await p.evaluate(()=>repositorySelectWorld(repositoryWorldInventory().find(p=>p.name==='test-external').path));assert.equal(await p.$eval('#world-service-open',e=>e.disabled),true);await p.click('[data-world-details]');assert.match(await p.$eval('#world-detail',e=>e.textContent),/OTel|otel-traces|Synthetic/);
 await p.reload({waitUntil:'load'});await ready();assert.equal(await p.evaluate(()=>repositoryWorldInventory().some(p=>p.name==='test-external')),false);
 await p.setViewport({width:390,height:950});await p.screenshot({path:path.join(output,'petclinic-mobile.png')});assert.deepEqual(errors,[]);fs.writeFileSync(path.join(output,'checks.json'),JSON.stringify({code,configuration,unobserved:true,externalTraceUI:true,drilldown:true,reload:true,errors},null,2));console.log('Petclinic evidence filters, real source drill-down, reload and isolated external-trace UI passed',{code,configuration});
}finally{await browser.close();}
