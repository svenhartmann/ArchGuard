const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const nodes=[{id:0,name:'p',kind:'module',parent:null},{id:1,name:'p/A.java',kind:'file',parent:0},{id:2,name:'q',kind:'module',parent:null},{id:3,name:'q/B.java',kind:'file',parent:2}];
const fixture={root:'/repo',nodes,edges:[{from:1,to:3,count:1,refs:[{file:'p/A.java',line:3,raw:'import q.B;'}]},{from:3,to:1,count:1,refs:[]}],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{world:{definition:'architecture-world/v1',revision:'a'.repeat(64),status:'partial',entities:[{id:'class:A',node:'p/A.java',class_id:'A',max_complexity:3},{id:'class:B',node:'q/B.java',class_id:'B',max_complexity:8}]}}};
const app=launch(fixture,{library:false});
assert.equal(app.run("parseViewerRoute('#tab=matrix&scope=p').tab"),'dsm');
assert.equal(app.run("parseViewerRoute('#world=p').scope"),'p');
assert.equal(app.run("parseViewerRoute('#unrelated-anchor')"),null);
app.run("applyViewerRoute(parseViewerRoute('#tab=graph&node=p%2FA.java'))");
assert.equal(app.run('ui.selection.name'),'p/A.java');
assert.match(app.run('viewerRouteHash()'),/node=p%2FA.java/);
assert.doesNotMatch(app.run('viewerRouteHash()'),/repo|node=1/);
app.run("applyViewerRoute(parseViewerRoute('#tab=start&scope=p&lens=complexity&kpi=cbo'))");
assert.equal(app.run('worldUi.within'),'p');assert.equal(app.run('worldUi.lens'),'complexity');assert.equal(app.run('worldUi.kpi'),'cbo');
app.run("applyViewerRoute(parseViewerRoute('#tab=start&scope=missing&lens=invalid'))");
assert.equal(app.run('worldUi.within'),null);assert.equal(app.run('worldUi.lens'),'problems');
app.run("applyViewerRoute(parseViewerRoute('#tab=matrix&scope=q'))");
assert.equal(app.run('treeMatrixUi.scope'),2);
assert.match(app.run('viewerRouteHash()'),/tab=matrix&scope=q/);
app.run("applyViewerRoute(parseViewerRoute('#tab=graph'))");assert.equal(app.run('ui.selection'),null);
console.log('Viewer navigation: stable named scopes, graph selection, World lens, matrix, legacy links and invalid routes pass');

const reordered=structuredClone(fixture);for(const node of reordered.nodes){node.id+=100;if(node.parent!=null)node.parent+=100;}for(const edge of reordered.edges){edge.from+=100;edge.to+=100;}
const colleague=launch(reordered,{library:false});colleague.run("applyViewerRoute(parseViewerRoute('#tab=graph&node=p%2FA.java'))");assert.equal(colleague.run('ui.selection.name'),'p/A.java');
colleague.run("applyViewerRoute(parseViewerRoute('#tab=matrix&scope=q'))");assert.equal(colleague.run('treeMatrixUi.scope'),102);
app.run("applyViewerRoute(parseViewerRoute('#tab=matrix&cycle=p%2FA.java'))");assert.equal(app.run('treeMatrixUi.cycle'),'C1');assert.match(app.run('viewerRouteHash()'),/cycle=p%2FA.java/);
console.log('Links also resolve after graph IDs change on a colleague machine; matrix cycle uses a stable member path');
