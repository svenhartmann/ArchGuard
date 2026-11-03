// Real Chromium layout regression. Usage:
// node scripts/browser/test_viewer_scrolling.mjs REPORT_HTML CHROMIUM PUPPETEER_ENTRY OUTPUT_DIR
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';

const [report, executablePath, modulePath, output] = process.argv.slice(2);
assert.ok(report && executablePath && modulePath && output, 'Provide report, Chromium, Puppeteer entry and output directory');
const {default: puppeteer} = await import(pathToFileURL(path.resolve(modulePath)).href);
fs.mkdirSync(output, {recursive: true});
const browser = await puppeteer.launch({executablePath, headless: true, args: ['--no-sandbox', '--disable-gpu']});
const failures = [], checks = [], errors = [];
try {
  const page = await browser.newPage();
  page.on('pageerror', error => errors.push(String(error)));
  // Layout tests must not depend on a CDN or contact an external service.
  await page.setRequestInterception(true);
  page.on('request', request => /^https?:/.test(request.url()) ? request.abort() : request.continue());
  await page.goto(pathToFileURL(path.resolve(report)).href, {waitUntil: 'load'});
  await page.waitForSelector('#engineering-section');
  async function check(name, fn) {
    try { await fn(); checks.push(name); }
    catch (error) { failures.push({name, error: error.message}); }
  }
  async function scrollToEnd(content) {
    const state = await page.evaluate(selector => {
      const host = document.querySelector(selector);
      if (!host) throw Error('Missing content: ' + selector);
      const probe = document.createElement('div');
      probe.dataset.scrollProbe = 'true';
      // Stress a real view's scroll boundary with content taller than the viewport.
      probe.style.cssText = 'height:1400px;display:flex;align-items:flex-end;flex-shrink:0';
      probe.textContent = 'Last content';
      host.appendChild(probe);
      let scroller = host;
      while (scroller && scroller.id !== 'main') {
        const style = getComputedStyle(scroller);
        if (/(auto|scroll)/.test(style.overflowY) && scroller.scrollHeight > scroller.clientHeight + 1) break;
        scroller = scroller.parentElement;
      }
      if (!scroller || scroller.id === 'main') throw Error('No bounded scroll area for ' + selector);
      scroller.dataset.scrollTest = 'active';
      scroller.scrollTop = 0;
      const rect = scroller.getBoundingClientRect();
      return {top: rect.top, bottom: rect.bottom, height: rect.height, x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2, windowHeight: innerHeight, documentWidth: document.documentElement.scrollWidth, windowWidth: innerWidth};
    }, content);
    assert.ok(state.height > 40 && state.top >= 0 && state.bottom <= state.windowHeight + 1, 'Scroll area must fit in the visible workspace: ' + JSON.stringify(state));
    assert.ok(state.documentWidth <= state.windowWidth, 'The page must not overflow horizontally');
    await page.mouse.move(state.x, state.y);
    await page.mouse.wheel({deltaY: 400});
    await page.waitForFunction(() => document.querySelector('[data-scroll-test="active"]').scrollTop > 0, {timeout: 2000});
    const end = await page.evaluate(() => {
      const scroller = document.querySelector('[data-scroll-test="active"]');
      const probe = scroller.querySelector('[data-scroll-probe]');
      const rect = scroller.getBoundingClientRect();
      // A shared scroller may contain further panes after this one. Bring this
      // pane's last content into view instead of scrolling past it to the next.
      scroller.scrollTop += probe.getBoundingClientRect().bottom - rect.bottom + 1;
      const content = probe.getBoundingClientRect();
      return {contentBottom: content.bottom, top: rect.top, bottom: rect.bottom, viewport: innerHeight};
    });
    assert.ok(end.contentBottom > end.top && end.contentBottom <= Math.min(end.bottom, end.viewport) + 1, 'Last content must be reachable: ' + JSON.stringify(end));
  }
  async function clean() {
    await page.evaluate(() => {
      document.querySelectorAll('[data-scroll-probe],[data-layout-clone]').forEach(node => node.remove());
      document.querySelectorAll('[data-scroll-test]').forEach(node => { node.scrollTop = 0; delete node.dataset.scrollTest; });
    });
  }
  // Live pages also reserve room for repository navigation and the status
  // footer. In the two-row Calls layout this leaves less than its former
  // 120px + 180px minimum rows. Neither pane may escape its scroll boundary.
  await page.setViewport({width: 1100, height: 600});
  await page.evaluate(() => {
    showTab('calls');
    document.getElementById('feedback').textContent = 'Live';
  });
  await check('1100x600: Calls panes fit inside their shared viewport', async () => {
    const layout = await page.evaluate(() => {
      const columns = document.querySelector('.calls-columns').getBoundingClientRect();
      return {top:columns.top, bottom:columns.bottom, panes:[...document.querySelectorAll('.calls-outline,.calls-master,.calls-detail')].map(n=>{
        const r=n.getBoundingClientRect(); return {top:r.top,bottom:r.bottom};
      })};
    });
    for (const pane of layout.panes) assert.ok(pane.top >= layout.top && pane.bottom <= layout.bottom + 1,
      'Each Calls pane must fit inside the available height: ' + JSON.stringify(layout));
  });
  await page.evaluate(() => { document.getElementById('feedback').textContent = ''; });
  for (const [width, height] of [[1500, 720], [1500, 500], [1100, 720], [760, 600], [390, 640]]) {
    await page.setViewport({width, height});
    await page.evaluate(() => { setViewerLanguage('de'); showTab('quality'); });
    const sections = await page.$$eval('#engineering-section option', options => options.map(option => option.value));
    for (const section of sections) {
      await page.select('#engineering-section', section);
      await check(`${width}: quality/${section} one entry per row`, async () => {
        const boxes = await page.evaluate(section => {
          const list = document.querySelector('#engineering-list');
          const selector = section === 'heatmap' ? 'tr[data-heatmap-class]' : ':scope > article';
          const first = list.querySelector(selector);
          if (!first) throw Error('The fixture needs actual entries for this section');
          // Include single-result sections in the same layout contract.
          while (list.querySelectorAll(selector).length < 3) {
            const clone = first.cloneNode(true); clone.dataset.layoutClone = 'true'; first.parentElement.appendChild(clone);
          }
          return [...list.querySelectorAll(selector)].map(card => {
            const r = card.getBoundingClientRect(); return {left:r.left, right:r.right, top:r.top, bottom:r.bottom};
          });
        }, section);
        for (let i = 1; i < boxes.length; i++) {
          assert.ok(boxes[i].top >= boxes[i-1].bottom, 'Quality cards must follow each other vertically');
          assert.ok(Math.abs(boxes[i].left - boxes[0].left) < 1 && Math.abs(boxes[i].right - boxes[0].right) < 1, 'Quality cards must share the full row width');
        }
      });
      await check(`${width}: quality/${section} wheel and end reachable`, () => scrollToEnd('#engineering-list'));
      await clean();
    }
    await page.select('#engineering-section', 'classes');
    await page.screenshot({path: path.join(output, `quality-${width}-${height}.png`)});
    for (const [tab, contents] of [
      ['architecture', ['#architecture-content']], ['assessment', ['#assessment-content']],
      ['security', ['#security-findings']], ['dsm', ['#dsm-tab']],
      ['graph', ['#package-tree']], ['calls', ['.calls-outline', '.calls-master', '.calls-detail']],
      ['services', ['.services-description', '#services-tree']],
    ]) {
      await page.evaluate(name => showTab(name), tab);
      for (const content of contents) {
        await check(`${width}: ${tab} ${content} wheel and end reachable`, () => scrollToEnd(content));
        await clean();
      }
    }
  }
  assert.deepEqual(errors, [], 'No application errors');
  fs.writeFileSync(path.join(output, 'scrolling.json'), JSON.stringify({checks, failures, errors}, null, 2) + '\n');
  console.log(JSON.stringify({passed:checks.length, failures, errors}));
  assert.deepEqual(failures, []);
} finally { await browser.close(); }
