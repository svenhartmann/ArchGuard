// HTML/CSS and actual locale contracts; this is not visual browser acceptance.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
assert.match(html,/<div class="calls-columns">\s*<section class="calls-outline"/,'the Calls tree is directly visible before relations and evidence');
assert.match(html,/<h2[^>]+data-i18n-de="Aufrufbaum"[^>]*>Call tree<\/h2>/);
for(const id of ['calls-tree-expand','calls-tree-collapse','calls-tree-clear']){
  const tag=html.match(new RegExp('<button[^>]*\\bid="'+id+'"[^>]*>'))?.[0];
  assert.ok(tag,'tree control '+id+' is a keyboard-operable button');
  assert.match(tag,/type="button"/);assert.match(tag,/data-i18n-de=/);
}
assert.match(html,/<p[^>]+id="calls-tree-context"[^>]+role="status"/,'tree focus has a dedicated status');
assert.match(html,/<div[^>]+id="calls-tree"[^>]+aria-label="Packages, types\/modules and functions\/methods"[^>]+data-i18n-aria-label-de="Pakete, Typen\/Module und Funktionen\/Methoden"/);
assert.match(html,/\.calls-columns\s*\{[^}]*grid-template-columns:\s*minmax\([^)]*\)\s+minmax\([^)]*\)\s+minmax\(0,\s*1fr\)/,'wide Calls workspace retains tree, relations and evidence in three shrinkable columns');
assert.match(html,/\.calls-outline[^}]*min-width:\s*0[^}]*overflow:\s*auto/,'tree scrolls locally without widening its workspace');
assert.match(html,/\.call-tree-row\s*\{[^}]*--call-tree-depth/,'rendered semantic depth determines indentation');
const treeRowCss=html.match(/\.call-tree-row\s*\{([^}]*)\}/)[1];
const rowMinimum=Number(treeRowCss.match(/min-width:\s*(\d+)(?:px)?\s*;/)?.[1]);
assert.ok(rowMinimum>=190,'deep package indentation retains a readable row width inside the horizontally scrolling outline');
assert.match(html,/\.call-tree-label\s*\{[^}]*text-overflow:\s*ellipsis/,'long qualified symbols fit the tree');
assert.match(html,/\.call-tree-label\[aria-pressed=true\]/,'identity focus is visibly marked');
assert.match(html,/@media\s*\(max-width:\s*1100px\)[^{]*\{\s*\.calls-columns\s*\{[^}]*grid-template-columns:\s*minmax\([^)]*\)\s+minmax\(0,\s*1fr\)/,'medium viewports have a two-column fallback');
assert.match(html,/@container\s*\(max-width:\s*900px\)[^{]*\{\s*\.calls-columns\s*\{[^}]*overflow:\s*auto/,'two-row layout remains reachable when workspace height is short');
assert.match(html,/@media\s*\(max-width:\s*900px\)[^{]*\{\s*\.calls-columns\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)[^}]*overflow:\s*auto/,'narrow workspace stacks with an accessible scroll area');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=launch({root:'/tmp/calls-tree-layout',nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
for(const [id,en,de] of [['calls-tree-expand','Expand all','Alle aufklappen'],['calls-tree-collapse','Collapse all','Alle zuklappen'],['calls-tree-clear','All elements','Alle Elemente']]){
  assert.equal(app.ids.get(id).textContent,en);
  app.run("setViewerLanguage('de')");assert.equal(app.ids.get(id).textContent,de);
  app.run("setViewerLanguage('en')");assert.equal(app.ids.get(id).textContent,en);
}
app.run("setViewerLanguage('de')");
assert.equal(app.ids.get('calls-tree').getAttribute('aria-label'),'Pakete, Typen/Module und Funktionen/Methoden');
console.log('Calls tree shell: direct placement, native controls, responsive layout, semantic indentation and DE/EN passed');
