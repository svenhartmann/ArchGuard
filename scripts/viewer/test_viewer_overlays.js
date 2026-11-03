// Menus, tooltips and dialogs share one overlay look: elevation, radius and list-style items.
const assert=require('node:assert/strict'),fs=require('node:fs');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
const css=html.slice(html.indexOf('<style>'),html.indexOf('</style>'));
const rule=selector=>{const at=css.indexOf(selector+' {');assert.ok(at>=0,'missing rule '+selector);return css.slice(at,css.indexOf('}',at));};
assert.match(rule(':root'),/--shadow-overlay:/,'light theme defines overlay elevation');
assert.match(rule("html[data-theme='dark']"),/--shadow-overlay:/,'dark theme defines overlay elevation');
const panels=rule('.workspace-menu > .menu-content, .export-menu > div, #metric-tooltip, #services-tooltip, #graph-context-tooltip, #world-tip, dialog');
assert.match(panels,/box-shadow: var\(--shadow-overlay\)/,'every overlay floats above content with the same elevation');
assert.match(panels,/border-radius: var\(--radius\)/,'every overlay uses the same corner radius');
const items=rule('.menu-content .btn-secondary, .export-menu > div > .btn-secondary, .share-actions .btn-secondary');
assert.match(items,/background: transparent/,'menu entries read as list items, not stacked buttons');
assert.match(items,/width: 100%/,'menu entries span the menu width');
assert.match(rule('.export-menu > summary::after'),/content: '⌄'/,'every menu trigger shows the same chevron');
assert.match(rule('.workspace-menu[open] > summary, .export-menu[open] > summary'),/border-color: var\(--accent\)/,'an open menu marks its trigger');
assert.match(rule('dialog::backdrop'),/backdrop-filter/,'dialogs dim and soften the page behind them');
console.log('Overlays: shared elevation, radius, list items, chevrons, open state and dialog backdrop passed');
