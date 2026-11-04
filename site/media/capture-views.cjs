// Capture the gallery screenshots in site/assets from running viewers.
// Same PUPPETEER_MODULE / CHROME environment as record-tour.cjs. Start the
// Commons Collections and eShop viewers first (commands in examples/), then:
// node site/media/capture-views.cjs http://127.0.0.1:8816 http://127.0.0.1:8818 site/assets
const path=require('node:path');const {pathToFileURL}=require('node:url');
(async()=>{
 const [collections,eshop,out]=process.argv.slice(2);if(!out||!process.env.PUPPETEER_MODULE||!process.env.CHROME)throw new Error('Provide both viewer URLs, output directory, PUPPETEER_MODULE and CHROME');
 const {default:puppeteer}=await import(pathToFileURL(process.env.PUPPETEER_MODULE));
 const browser=await puppeteer.launch({executablePath:process.env.CHROME,headless:true,args:['--no-sandbox']});
 const pause=ms=>new Promise(r=>setTimeout(r,ms));
 const eshopOverview=async page=>{await page.select('#world-deployment-scope','aspire:src/eShop.AppHost/AppHost.cs');await pause(800);await page.select('#world-service-kind','event');await page.click('#world-service-cycles');await pause(800);if(!(await page.$eval('#world-status',e=>e.textContent)).includes('cyclic'))throw new Error('Expected the inferred event cycle');};
 const eshopEvidence=async page=>{await page.select('#services-kind','event');await page.click('#services-cycles-only');await pause(500);const edge=await page.$('g[data-service-edge]');if(!edge)throw new Error('Missing event edges');await edge.evaluate(e=>e.focus());await page.keyboard.press('Enter');await pause(800);if(!await page.$eval('#sidebar',e=>e.textContent.includes('Source evidence')))throw new Error('Missing source evidence');};
 const metric=kpi=>async page=>{await page.click('#world-lens-complexity');await page.select('#world-kpi',kpi);};
 const insideMap=async page=>{await metric('complexity')(page);await pause(800);await page.$eval('[data-world-area="org/apache/commons/collections4/map"]',e=>e.dispatchEvent(new MouseEvent('click',{bubbles:true})));await page.waitForFunction(()=>document.getElementById('world-status').textContent.includes('/map/'),{timeout:10000});};
 const shots=[
  ['world',collections+'/#tab=start'],
  ...['complexity','cbo','wmc','lcom4'].map(kpi=>['world-'+kpi,collections+'/#tab=start',metric(kpi)]),
  ['world-package',collections+'/#tab=start',insideMap],['graph',collections+'/#tab=graph'],['matrix',collections+'/#tab=dsm'],
  ['calls',collections+'/#tab=calls'],['quality',collections+'/#tab=quality'],
  ['eshop-world',eshop+'/#tab=start',eshopOverview],['eshop-services',eshop+'/#tab=services',eshopEvidence],
 ];
 try{for(const [name,url,act] of shots){
  const page=await browser.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.setViewport({width:1440,height:960,deviceScaleFactor:1});
  await page.goto(url,{waitUntil:'networkidle0',timeout:300000});
  await page.evaluate(()=>document.getElementById('language-en')?.click());
  // Allow analysis results and canvas layout to settle before capturing.
  await pause(4000);if(act){await act(page);await pause(1500);}
  // Periodic source checks show a transient status line; keep a capture taken while it is absent before and after.
  const busy=()=>page.evaluate(()=>/Updating analysis/.test(document.body.innerText));
  for(let attempt=1;;attempt++){
   await page.waitForFunction(()=>!/Updating analysis/.test(document.body.innerText),{timeout:180000,polling:200});
   const image=await page.screenshot();
   if(!(await busy())){require('node:fs').writeFileSync(path.join(out,name+'.png'),image);break;}
   if(attempt>=20)throw new Error(name+': the viewer kept updating its analysis');
  }
  if(errors.length)throw new Error(name+': '+errors.join('\n'));
  console.log('captured',name);await page.close();
 }}finally{await browser.close()}
})().catch(e=>{console.error(e);process.exitCode=1});
