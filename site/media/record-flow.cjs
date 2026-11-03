// Record the short viewer flow shown on the home page from a running viewer.
// Same PUPPETEER_MODULE / CHROME environment as capture-views.cjs. Start the
// Commons Collections viewer first (see site/README.md), then:
// node site/media/record-flow.cjs http://127.0.0.1:8816 /tmp/flow.webm
// Puppeteer's screencast needs ffmpeg on PATH; README describes the MP4 encoding.
const path=require('node:path');const {pathToFileURL}=require('node:url');
(async()=>{
 const [collections,out]=process.argv.slice(2);if(!out||!process.env.PUPPETEER_MODULE||!process.env.CHROME)throw new Error('Provide the viewer URL, output .webm path, PUPPETEER_MODULE and CHROME');
 const {default:puppeteer}=await import(pathToFileURL(process.env.PUPPETEER_MODULE));
 const browser=await puppeteer.launch({executablePath:process.env.CHROME,headless:true,args:['--no-sandbox']});
 const pause=ms=>new Promise(r=>setTimeout(r,ms)),errors=[];
 try{
  const page=await browser.newPage();page.on('pageerror',e=>errors.push(e.message));
  await page.emulateMediaFeatures([{name:'prefers-color-scheme',value:'dark'}]);
  await page.setViewport({width:1440,height:960,deviceScaleFactor:1});
  await page.goto(collections+'/#tab=start&lens=problems',{waitUntil:'networkidle0',timeout:300000});
  await page.evaluate(()=>document.getElementById('language-en')?.click());
  await pause(4000);
  // Headless capture has no system pointer; a drawn one makes each step readable.
  await page.evaluate(()=>{const c=document.createElement('div');c.id='flow-pointer';Object.assign(c.style,{position:'fixed',left:'720px',top:'480px',width:'22px',height:'22px',margin:'-11px 0 0 -11px',borderRadius:'50%',background:'rgba(91,156,246,.35)',border:'2px solid #5B9CF6',zIndex:2147483647,pointerEvents:'none',transition:'left .6s ease,top .6s ease,transform .15s'});document.body.appendChild(c);});
  const point=async selector=>{
   const target=(await page.evaluateHandle(selector)).asElement(),bounds=target&&await target.boundingBox();
   if(!bounds)throw new Error('Missing flow target: '+selector);
   const box={x:bounds.x+Math.min(bounds.width/2,60),y:bounds.y+bounds.height/2};
   await page.evaluate(b=>{const c=document.getElementById('flow-pointer');c.style.left=b.x+'px';c.style.top=b.y+'px';},box);await pause(750);
   await page.evaluate(()=>{const c=document.getElementById('flow-pointer');c.style.transform='scale(.7)';setTimeout(()=>c.style.transform='',150);});
  };
  // Each step points at its control, then performs the same action a click or selection would.
  const step=async(selector,action,hold)=>{await point(selector);await page.evaluate(action);await pause(hold);};
  const mapRow=()=>[...document.querySelectorAll('#package-tree g[data-tree-source]')].find(g=>[...g.querySelectorAll('text')].some(t=>t.textContent.trim()==='map'));
  const recorder=await page.screencast({path:out});
  await pause(1800);
  await step(()=>document.getElementById('world-lens-complexity'),()=>document.getElementById('world-lens-complexity').click(),1600);
  await step(()=>document.getElementById('world-kpi'),()=>{const s=document.getElementById('world-kpi');s.value='cbo';s.dispatchEvent(new Event('change',{bubbles:true}));},1800);
  await step(()=>document.querySelector('[data-world-area$="/map"]'),()=>document.querySelector('[data-world-area$="/map"]').dispatchEvent(new MouseEvent('click',{bubbles:true})),2200);
  await step(()=>document.querySelector('#tabs button[data-tab="graph"]'),()=>{location.hash='#tab=graph&scope=org/apache/commons/collections4';},2400);
  await step(mapRow,()=>[...document.querySelectorAll('#package-tree g[data-tree-source]')].find(g=>[...g.querySelectorAll('text')].some(t=>t.textContent.trim()==='map')).dispatchEvent(new MouseEvent('click',{bubbles:true})),2600);  await step(()=>document.querySelector('#tabs button[data-tab="calls"]'),()=>document.querySelector('#tabs button[data-tab="calls"]').click(),1200);
  await step(()=>document.querySelector('#calls-relations button'),()=>document.querySelector('#calls-relations button').click(),2200);
  await step(()=>document.querySelector('#tabs button[data-tab="quality"]'),()=>document.querySelector('#tabs button[data-tab="quality"]').click(),2200);
  await step(()=>document.getElementById('export-summary'),()=>{document.getElementById('export-menu').open=true;},2000);
  await recorder.stop();
  if(errors.length)throw new Error(errors.join('\n'));
  console.log('recorded',path.resolve(out));
 }finally{await browser.close()}
})().catch(e=>{console.error(e);process.exitCode=1});
