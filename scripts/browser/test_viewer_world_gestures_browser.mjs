// node scripts/browser/test_viewer_world_gestures_browser.mjs REPORT CHROME PUPPETEER OUTPUT
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const [report,executablePath,modulePath,output]=process.argv.slice(2),out=path.resolve(output);fs.mkdirSync(out,{recursive:true});
const {default:puppeteer}=await import(pathToFileURL(path.resolve(modulePath)).href);
const browser=await puppeteer.launch({executablePath,headless:true,args:['--no-sandbox']});
const near=(a,b)=>assert.ok(Math.abs(a-b)<.03,`${a} != ${b}`),errors=[],checks=[];
try{
  const page=await browser.newPage();page.on('pageerror',e=>errors.push(String(e)));
  await page.setRequestInterception(true);page.on('request',r=>{const u=new URL(r.url());if(u.protocol.startsWith('http')&&u.hostname!=='127.0.0.1')r.abort();else r.continue();});
  await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'reduce'}]);
  const cdp=await page.createCDPSession(),pose=()=>page.evaluate(()=>({...worldUi.camera}));
  const touch=async(type,points)=>{
    await cdp.send('Input.dispatchTouchEvent',{type,touchPoints:points.map(p=>({...p,radiusX:3,radiusY:3,force:1}))});
    // Chrome coalesces touch moves at frame boundaries; wait for delivery.
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  };
  const fit=()=>page.evaluate(()=>document.getElementById('world-reset').click());
  const frame=async()=>{await page.$eval('#world-canvas',n=>n.scrollIntoView({block:'center'}));return page.$eval('#world-canvas',n=>{const b=n.getBoundingClientRect();return {x:b.left,y:b.top,width:b.width,height:b.height};});};
  for(const width of [1500,390]){
    await page.setViewport({width,height:1000,hasTouch:true,isMobile:width===390});
    await page.goto(/^(https?|file):/.test(report)?report:pathToFileURL(path.resolve(report)).href,{waitUntil:'load'});
    await page.evaluate(()=>{showTab('start');setViewerLanguage('de');});await page.waitForFunction(()=>worldUi.hits.length>0);
    const evidence=await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,ops,worldUi.layout]));
    const box=await frame(),cx=box.x+box.width/2,cy=box.y+box.height/2;
    let points=[{id:1,x:cx-30,y:cy},{id:2,x:cx+30,y:cy}];
    const scrollBefore=await page.evaluate(()=>[scrollX,scrollY,document.getElementById('tab-start').scrollTop,visualViewport.scale]);
    await touch('touchStart',points);
    assert.equal(await page.$eval('#world-labels',n=>n.hidden),true,'both contacts start a gesture');
    points=[{id:1,x:cx-60,y:cy},{id:2,x:cx+60,y:cy}];await touch('touchMove',points);
    near((await pose()).zoom,2);near((await pose()).rotation,0);
    points=[{id:1,x:cx,y:cy-60},{id:2,x:cx,y:cy+60}];await touch('touchMove',points);
    const turned=await pose();near(turned.zoom,2);near(turned.rotation,Math.PI/2);
    points=points.map(p=>({...p,x:p.x+16,y:p.y+12}));await touch('touchMove',points);
    const moved=await pose();near(moved.x,turned.x+16);near(moved.y,turned.y+12);
    await touch('touchEnd',[points[1]]);assert.deepEqual(await pose(),moved,'two contacts to one has no jump');
    points=[{...points[0],x:points[0].x+13,y:points[0].y+7}];await touch('touchMove',points);
    near((await pose()).x,moved.x+13);near((await pose()).y,moved.y+7);await touch('touchEnd',[]);
    assert.equal(await page.evaluate(()=>worldUi.selection),null);assert.equal(await page.evaluate(()=>worldUi.within),null);
    assert.equal(await page.$eval('#world-labels',n=>n.hidden),false);
    assert.deepEqual(await page.evaluate(()=>[scrollX,scrollY,document.getElementById('tab-start').scrollTop,visualViewport.scale]),scrollBefore,'gesture does not scroll or zoom the browser');
    await page.screenshot({path:path.join(out,'touch-rotated-'+width+'.png')});
    // Pointer cancellation restores labels and a subsequent gesture works.
    await touch('touchStart',[{id:1,x:cx-30,y:cy},{id:2,x:cx+30,y:cy}]);await touch('touchCancel',[]);
    assert.equal(await page.$eval('#world-labels',n=>n.hidden),false);
    const cancelled=await pose();await touch('touchStart',[{id:1,x:cx,y:cy}]);await touch('touchMove',[{id:1,x:cx+10,y:cy+8}]);await touch('touchEnd',[]);
    near((await pose()).x,cancelled.x+10);near((await pose()).y,cancelled.y+8);
    await fit();await frame();
    // A package pin is also a gesture surface, while a stationary tap retains
    // the existing direct navigation behavior.
    const pin=await page.$eval('#world-labels button',n=>{const b=n.getBoundingClientRect();return {name:n.getAttribute('data-world-area'),x:b.left+b.width/2,y:b.top+b.height/2};});
    await touch('touchStart',[{id:1,x:pin.x,y:pin.y},{id:2,x:pin.x+45,y:pin.y+10}]);
    await touch('touchMove',[{id:1,x:pin.x-10,y:pin.y},{id:2,x:pin.x+55,y:pin.y+10}]);await touch('touchEnd',[]);
    assert.equal(await page.evaluate(()=>worldUi.within),null,'pin-origin pinch never opens its package');assert.ok((await pose()).zoom>1);
    await fit();await frame();
    const tapPin=await page.$eval('#world-labels button',n=>{const b=n.getBoundingClientRect();return {name:n.getAttribute('data-world-area'),x:b.left+b.width/2,y:b.top+b.height/2};});
    await page.touchscreen.tap(tapPin.x,tapPin.y);
    assert.equal(await page.evaluate(()=>worldUi.within),tapPin.name,'a pin tap opens the package once');
    assert.equal(await page.evaluate(()=>worldUi.history.length),1);await fit();
    if(width===1500){
      const area=await frame(),start={x:area.x+30,y:area.y+110};
      await page.evaluate(()=>{
        window.worldMenus=[];window.worldImageExports=[];
        document.addEventListener('contextmenu',e=>window.worldMenus.push({prevented:e.defaultPrevented,button:e.button,ctrl:e.ctrlKey}));
        const click=HTMLAnchorElement.prototype.click;
        HTMLAnchorElement.prototype.click=function(){if(this.download){window.worldImageExports.push(this.download);return;}return click.call(this);};
      });
      for(const target of ['terrain','pin']){
        const point=target==='terrain'?start:await page.$eval('#world-labels button',n=>{const r=n.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};});
        await page.mouse.click(point.x,point.y,{button:'right'});
        assert.equal(await page.evaluate(()=>worldUi.selection),null);assert.equal(await page.evaluate(()=>worldUi.within),null);
        await page.keyboard.down('Control');await page.mouse.click(point.x,point.y);await page.keyboard.up('Control');
        assert.equal(await page.evaluate(()=>worldUi.selection),null);assert.equal(await page.evaluate(()=>worldUi.within),null);
      }
      const menus=await page.evaluate(()=>worldMenus);
      assert.ok(menus.length>=2&&menus.every(e=>e.prevented),'terrain and pins suppress the native Save image menu');
      // Check during the drag, before pointerup restores gesture visibility.
      const pins=()=>page.$$eval('#world-labels button',nodes=>nodes.filter(n=>!n.hidden).map(n=>({name:n.getAttribute('data-world-area'),x:n.style.left,y:n.style.top})));
      for(const modifier of ['Shift','Control']){
        await page.keyboard.down(modifier);await page.mouse.move(start.x,start.y);await page.mouse.down();
        await page.mouse.move(start.x+50,start.y+10,{steps:4});
        assert.equal(await page.$eval('#world-labels',n=>n.hidden),false,modifier+' rotation keeps labels visible');assert.ok((await pins()).length>0);
        await page.mouse.up();await page.keyboard.up(modifier);await fit();await frame();
      }
      await page.keyboard.down('Control');await page.mouse.move(start.x,start.y);await page.mouse.down();await page.mouse.move(start.x+50,start.y+10,{steps:4});await page.mouse.up();await page.keyboard.up('Control');
      near((await pose()).rotation,.4);near((await pose()).x,0);near((await pose()).y,0);await fit();await frame();
      await page.evaluate(()=>document.getElementById('world-canvas').addEventListener('contextmenu',e=>{window.worldContextPrevented=e.defaultPrevented;},{once:true}));
      const beforeRotation=await pins();
      await page.mouse.move(start.x,start.y);await page.mouse.down({button:'right'});await page.mouse.move(start.x+95,start.y+20,{steps:6});
      assert.equal(await page.$eval('#world-labels',n=>n.hidden),false,'right-drag keeps labels visible before release');
      const duringRotation=await pins();assert.ok(duringRotation.length>0);assert.notDeepEqual(duringRotation,beforeRotation,'visible pins follow the rotated terrain');
      await page.screenshot({path:path.join(out,'mouse-rotation-labels.png')});await page.mouse.up({button:'right'});
      const mouse=await pose();near(mouse.rotation,.76);near(mouse.x,0);near(mouse.y,0);assert.equal(await page.evaluate(()=>worldUi.selection),null);
      assert.equal(await page.evaluate(()=>window.worldContextPrevented),true);
      await page.mouse.down();await page.mouse.move(start.x+130,start.y+35,{steps:3});await page.mouse.up();
      near((await pose()).x,35);near((await pose()).y,15);near((await pose()).rotation,.76);
      await page.click('#world-reset');await frame();
      const node=await page.$eval('#world-labels button',n=>n.getAttribute('data-world-area'));await page.click('#world-labels button');
      assert.equal(await page.evaluate(()=>worldUi.within),node,'mouse pin click remains navigable');await fit();
      await frame();
      const surface=await page.evaluate(()=>{
        const canvas=document.getElementById('world-canvas'),b=canvas.getBoundingClientRect();
        for(const h of worldUi.hits.slice().reverse())for(const triangle of h.triangles){
          const x=b.left+triangle.reduce((s,p)=>s+p.x,0)/3,y=b.top+triangle.reduce((s,p)=>s+p.y,0)/3;
          if(document.elementFromPoint(x,y)===canvas)return {x,y,area:h.cell.area};
        }
      });
      assert.ok(surface);const menusBeforePrimary=await page.evaluate(()=>worldMenus.length);await page.mouse.click(surface.x,surface.y);
      assert.equal(await page.evaluate(()=>worldMenus.length),menusBeforePrimary,'normal primary click does not open a browser menu');
      assert.equal(await page.evaluate(()=>worldUi.selection?.area),surface.area,'single click still selects terrain');
      await page.mouse.click(surface.x,surface.y,{count:2});
      assert.equal(await page.evaluate(()=>worldUi.within),surface.area,'double-click still opens terrain packages');await fit();
      assert.deepEqual(await page.evaluate(()=>worldImageExports),[],'clicking and dragging never trigger image export');
      await page.click('#world-export');assert.equal(await page.evaluate(()=>worldImageExports.length),1,'explicit PNG export still works');
    }
    assert.equal(await page.evaluate(()=>JSON.stringify([data.nodes,data.edges,ops,worldUi.layout])),evidence);
    checks.push({width,pinch:true,twist:true,pan:true,pinGestures:true,tap:true,cancel:true,mouse:width===1500});
  }
  // With animation enabled, direct manipulation takes ownership immediately.
  await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'no-preference'}]);await page.reload({waitUntil:'load'});
  await page.evaluate(()=>showTab('start'));const box=await frame(),x=box.x+25,y=box.y+100;
  await page.evaluate(()=>worldCameraTo({zoom:3},1000));await touch('touchStart',[{id:1,x,y}]);const interrupted=await pose();
  await new Promise(resolve=>setTimeout(resolve,1100));assert.deepEqual(await pose(),interrupted,'old animation cannot overwrite the gesture camera');await touch('touchEnd',[]);
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'browser.json'),JSON.stringify({checks,animationInterruption:true,errors},null,2));console.log(JSON.stringify({checks,animationInterruption:true,errors}));
}finally{await browser.close();}
