// Template contracts for a compact workspace; this does not measure a browser layout.
const assert=require('node:assert/strict'),fs=require('node:fs');
const html=fs.readFileSync('crates/archguard-report/src/viewer.html','utf8');
const css=html.match(/<style>([\s\S]*?)<\/style>/)[1];
function declarations(selector){
  const escaped=selector.replace(/[.*+?^${}()|[\]\\]/g,'\\$&'),result=new Map();
  for(const match of css.matchAll(new RegExp('(?:^|})\\s*'+escaped+'\\s*\\{([^}]+)\\}','g'))){
    for(const declaration of match[1].split(';')){
      const split=declaration.indexOf(':');if(split>0)result.set(declaration.slice(0,split).trim(),declaration.slice(split+1).trim());
    }
  }
  return result;
}
assert.ok(parseFloat(declarations('#topbar').get('min-height'))<=56,'the top bar must not reserve the former 78px');
assert.ok(parseFloat(declarations('.workspace-bar').get('min-height'))<=44,'breadcrumbs and tabs share a compact workspace bar');
assert.notEqual(declarations('.kpi-details').get('flex-basis'),'100%','closed More KPIs must not force a full extra row');
assert.equal(declarations('.kpi-details[open]').get('flex-basis'),'100%','expanded KPIs still have room for every metric');
assert.equal(declarations('.kpi-details[open]').get('order'),'1','expanded KPI content follows the metric and report-status row');
assert.equal(declarations('.report-status').get('margin-left'),'auto','report status uses spare inline space');
assert.equal(declarations('.graph-status').get('display'),'flex');
assert.equal(declarations('.graph-status').get('flex-wrap'),'wrap','scope context wraps on narrow screens instead of clipping');
assert.equal(declarations('#feedback:empty').get('display'),'none','an empty feedback footer reserves no diagram height');
assert.ok(parseFloat(declarations('.btn').get('min-height'))>=32,'compaction preserves clickable control sizes');
assert.match(html,/<button id="workspace-info"[^>]+aria-label="View help"[^>]*>\?<\/button>/,'view explanations have a keyboard-accessible help button');
assert.match(html,/<small id="workspace-description" hidden>/,'long view explanation is not permanent UI prose');
const vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=launch({root:'/tmp/help',nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
app.ids.get('workspace-info').dispatch('focus');assert.match(app.ids.get('metric-tooltip').textContent,/tree|dependencies/i,'focus exposes the actual view explanation');
const scope=html.match(/<details id="graph-scope-details"[^>]*>([\s\S]*?)<\/details>/);
assert.ok(scope,'long filter explanations have a native disclosure');
assert.match(scope[1],/id="graph-hint"/);
assert.match(scope[1],/id="graph-visibility"/,'detailed hidden-node and edge counts are available on demand');
assert.doesNotMatch(scope[1],/id="(?:edge-reference|graph-status-brief|graph-filter-feedback)"/,'reference, brief visibility counts and empty feedback remain visible without expanding help');
for(const id of ['edge-reference','graph-status-brief'])assert.match(html,new RegExp('<div id="'+id+'" role="status" aria-live="polite"'));
assert.match(html,/<div class="graph-status">[\s\S]*?id="edge-reference"[\s\S]*?id="graph-status-brief"/);
assert.equal(declarations('#main').get('min-height'),'0','the remaining body height belongs to the workspace');
assert.equal(declarations('.workspace-help, .tree-legend-details').get('position'),'static','help panels must anchor to the whole toolbar, not a narrow summary label');
assert.equal(declarations('.workspace-bar').get('position'),'relative');
assert.ok(Number(declarations('.workspace-bar').get('z-index'))>Number(declarations('.package-tree-legend').get('z-index')),'View info must paint above the graph toolbar');
assert.ok(Number(declarations('.workspace-bar').get('z-index'))<Number(declarations('#explorer, #inspector').get('z-index')),'workspace menus remain behind open compact Explorer/Inspector drawers');
assert.ok(Number(declarations('.workspace-bar').get('z-index'))>Number(declarations('#graph-options').get('z-index')),'View info stays above display options');
const popup=declarations('.workspace-help > small, .tree-legend-content');
assert.equal(popup.get('max-width'),'min(320px, calc(100% - 20px))','help width is bounded by its pane rather than the entire browser');
assert.equal(declarations('.tree-legend-content').get('left'),'10px','Legend stays within the tree pane even when its summary sits near the right edge');
assert.equal(declarations('.workspace-help > small').get('right'),'10px');
console.log('Workspace density: compact header, inline closed KPIs, visible scope/reference, optional explanations and retained control sizes passed');
