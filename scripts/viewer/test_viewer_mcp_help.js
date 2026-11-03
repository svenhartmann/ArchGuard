const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=launch({root:'/example',nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
const files=['lib.rs','review_kpi_tools.rs','review_decision_tools.rs','pr_comment_tools.rs'];
const names=files.flatMap(file=>[...fs.readFileSync('crates/archguard-mcp/src/'+file,'utf8').matchAll(/#\[tool\([\s\S]*?\)\]\s*async fn (\w+)/g)].map(m=>m[1])).sort();
const entries=JSON.parse(app.run('JSON.stringify(mcpHelpGroups().flatMap(g=>g.tools))'));
assert.deepEqual(entries.map(e=>e.name).sort(),names,'every registered MCP tool, including optional graph_query, documented once');
const before=app.run('JSON.stringify([data,work,ops,ui.workspace])');
for(const language of ['en','de']){
 app.run("setViewerLanguage('"+language+"');openKpiHelp()");
 const text=app.run('kpiHelpUi.dialog.textContent');
 for(const entry of JSON.parse(app.run('JSON.stringify(mcpHelpGroups().flatMap(g=>g.tools))'))){assert.ok(text.includes(entry.name));assert.equal(typeof entry.arguments,'object');assert.equal(typeof entry.prompt,'string',entry.name+' has a natural-language agent prompt');assert.ok(entry.prompt.length>40);assert.ok(text.includes(entry.prompt));}
 assert.equal(app.run("kpiHelpUi.dialog.querySelectorAll('div[data-mcp-prompt]').length"),names.length);
 assert.equal(app.run("kpiHelpUi.dialog.querySelector('details[data-mcp-setup]').open"),false);
 assert.match(text,/archguard mcp --root/);assert.match(text,/tools\/call/);assert.match(text,/tools\/list/);
 assert.match(text,/expected_revision/);assert.match(text,/next_offset/);assert.match(text,/review_decisions/);
 app.run('closeKpiHelp()');
}
assert.equal(app.run('JSON.stringify([data,work,ops,ui.workspace])'),before);
if(process.argv[2]){
 const tools=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
 for(const tool of tools){
  const entry=entries.find(e=>e.name===tool.name);assert.ok(entry);
  const schema=tool.inputSchema;
  for(const key of schema.required||[])assert.ok(key in entry.arguments,tool.name+' requires '+key);
  for(const [key,value] of Object.entries(entry.arguments)){
   assert.ok(key in schema.properties,tool.name+' unknown argument '+key);
   const field=schema.properties[key],type=field.type;
   if(typeof type==='string')assert.equal(Array.isArray(value)?'array':typeof value,type,tool.name+'.'+key);
   if(field.enum)assert.ok(field.enum.includes(value));
  }
 }
 assert.deepEqual(entries.filter(e=>e.name!=='graph_query').map(e=>e.name).sort(),tools.filter(e=>e.name!=='graph_query').map(e=>e.name).sort());
}
console.log('MCP help: all tool names, DE/EN examples, pagination, unchanged analysis and optional live schemas passed');
