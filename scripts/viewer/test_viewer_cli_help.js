const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const app=launch({root:'/example',nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null}},{library:false});
const source=fs.readFileSync('crates/archguard-cli/src/args.rs','utf8').split('enum Command {')[1].split('\n}\n')[0];
const commands=[...source.matchAll(/^    ([A-Z][A-Za-z]+)(?: \{|,)/gm)].map(m=>m[1].replace(/[A-Z]/g,(c,i)=>(i?'-':'')+c.toLowerCase())).concat('help').sort();
const documented=JSON.parse(app.run('JSON.stringify(cliHelpGroups().flatMap(group=>group.commands.map(command=>command[0])))')).sort();
assert.deepEqual(documented,commands,'every actual CLI entry point must be documented exactly once');
const before=app.run('JSON.stringify([data,work,ops,ui.workspace])');
for(const language of ['en','de']){
 app.run(`setViewerLanguage('${language}');openKpiHelp()`);
 const text=app.run('kpiHelpUi.dialog.textContent');
 for(const command of commands)assert.ok(text.includes('archguard '+command+' --help'),command+' has a route to complete options');
 assert.match(text,/archguard serve \. --port 8787/);assert.match(text,/archguard check \./);
 assert.match(text,/quality-history/);assert.match(text,/SARIF/);assert.match(text,/OpenAPI/);
 app.run('closeKpiHelp()');
}
assert.equal(app.run('JSON.stringify([data,work,ops,ui.workspace])'),before);
console.log('CLI help: complete command catalogue matches Rust CLI, DE/EN examples, options references and unchanged analysis passed');
