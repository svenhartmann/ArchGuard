// Execute the actual generated HTML/JS with the project's deterministic DOM harness.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),path=require('node:path');
const file=process.argv[2],synthetic=process.argv.includes('--synthetic');assert.ok(file,'Pass workspace HTML and optionally --synthetic');
const html=fs.readFileSync(file,'utf8'),payload=JSON.parse(html.match(/<script id="archguard-data" type="application\/json">([\s\S]*?)<\/script>/)[1]);
const js=[...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(match=>match[1]).find(text=>text.includes('function renderCalls()'));
assert.ok(js.includes('function renderWorkspace()'));
const source=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0].replace(/^const js=.*$/m,'const js=LIVE_JS;').replace(/^const html=.*$/m,'const html=LIVE_HTML;');
const launch=vm.runInNewContext(source+'\nlaunch;',{require,console,performance,Blob,LIVE_JS:js,LIVE_HTML:html});
const start=performance.now(),app=launch(payload,{library:false}),startup=performance.now()-start;
const workspace=payload.insights.workspace,sources=payload.insights.workspace_sources,impact=payload.insights.workspace_impact;
assert.ok(workspace);assert.ok(sources);
assert.equal(app.run('callModelPrepared'),false,'workspace startup leaves the shared compiler-call pool untouched');
assert.equal(app.ids.get('workspace-list').children.length,0,'closed project workspace stays lazy');
app.run("showTab('services')");app.ids.get('workspace-panel').open=true;app.ids.get('workspace-panel').dispatch('toggle');
assert.ok(app.ids.get('workspace-status').textContent.includes(workspace.status));
assert.equal(app.ids.get('workspace-list').querySelectorAll('article').length,Math.min(workspace.dependencies.length,20));
if(workspace.dependencies.length){
  const project=workspace.dependencies[0].from;
  app.ids.get('workspace-project').value=project;app.ids.get('workspace-project').dispatch('change');
  app.ids.get('workspace-direction').value='outgoing';app.ids.get('workspace-direction').dispatch('change');
  assert.equal(app.ids.get('workspace-list').querySelectorAll('article').length,Math.min(workspace.dependencies.filter(edge=>edge.from===project).length,20));
  app.ids.get('workspace-project').value='';app.ids.get('workspace-project').dispatch('change');
}
const section=value=>{app.ids.get('workspace-section').value=value;app.ids.get('workspace-section').dispatch('change');};
section('sources');assert.equal(app.ids.get('workspace-list').querySelectorAll('article').length,Math.min(sources.graph.edges.length,20));
assert.equal(app.run('callModelPrepared'),false,'source dependency browsing does not decode Calls');
section('tests');
if(impact?.targets?.length){
  assert.equal(app.ids.get('workspace-target').value,'@reported');
  const recorded=JSON.parse(app.run('JSON.stringify(workspaceTestImpact())'));
  assert.deepEqual(recorded.targets,impact.targets);
  assert.deepEqual(recorded.selected.map(row=>row.test_id).sort(),impact.selected.map(row=>row.test_id).sort());
  if(impact.targets.length===1){
    const target=impact.targets[0],file=sources.files.find(file=>file.path===target||file.id===target),project=workspace.projects.find(project=>project.id===target);
    if(file||project){
      app.ids.get('workspace-target').value=file?file.id:'@project:'+project.id;app.ids.get('workspace-target').dispatch('change');
      const calculated=JSON.parse(app.run('JSON.stringify(workspaceTestImpact())'));
      assert.deepEqual(calculated.affected_files.slice().sort(),impact.affected_files.slice().sort(),'UI and Rust agree on cross-project reachability');
      assert.deepEqual(calculated.selected.map(row=>row.id).sort(),impact.selected.map(row=>row.test_id).sort(),'UI and Rust agree on mapped test recommendations');
      assert.equal(calculated.unknown_tests,impact.unknown_tests);assert.equal(calculated.safe_to_skip_other_tests,false);
      assert.equal(calculated.status,impact.status,'UI and Rust agree on incomplete test coverage');
    }
  }
}
const exported=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(exported.workspace.dependencies.length,workspace.dependencies.length);
assert.equal(exported.workspace_sources.files.length,sources.files.length);
assert.equal(exported.workspace_sources.calls.calls.length,sources.call_summary.calls,'explicit export restores complete shared call evidence');
assert.equal(app.run('callModelPrepared'),true);
const markdown=app.run('exportMarkdown(exportSnapshot())');assert.match(markdown,/Project workspace/);assert.match(markdown,/Source|source/);
for(const diagnostic of sources.diagnostics||[])assert.ok(markdown.includes(app.run('exportMarkdownCell('+JSON.stringify(diagnostic)+')')));
app.run("showTab('graph')");app.run("showTab('calls')");assert.equal(app.run('callModelPrepared'),true);
app.run("showTab('services');setViewerLanguage('de')");assert.equal(app.ids.get('workspace-heading').textContent,'Projektverbund');
if(synthetic){
  assert.equal(workspace.projects.length,3);assert.equal(sources.files.length,3);assert.ok(workspace.cycles.length>0);assert.ok(sources.cycles.length>0);
  assert.equal(sources.files.filter(file=>file.name==='lib/Shared.java').length,2);
  assert.equal(impact.selected.length,2);assert.equal(impact.unknown_tests,1);
  assert.ok(exported.workspace_sources.calls.calls.some(call=>call.from_class==='project:app/pom.xml::app.Main'&&call.to_class==='project:lib/pom.xml::lib.Shared'));
}
const result={file:path.basename(file),synthetic,actualEmbeddedJavaScript:true,projects:workspace.projects.length,files:sources.files.length,checks:['lazy startup','qualified project directions','source dependencies','shared call export','normal Graph and Calls views','Rust/UI test-impact parity when target supplied','DE/EN'],startupHarnessMs:Number(startup.toFixed(2)),environment:'Node DOM harness; no browser layout or paint'};
fs.writeFileSync(path.join(path.dirname(file),path.basename(file,'.html')+'-html-verification.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result,null,2));
