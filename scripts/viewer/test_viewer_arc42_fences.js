// Code fences in arc42 chapters: any info string opens a block, only a bare fence closes it.
// The same cases run in the desktop app (architecture_ui/markdown.rs).
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const render=markdown=>{
  const chapters=[{number:5,title:'Bausteinsicht',markdown,questions:[]}];
  const app=launch({root:'/tmp/arc42',nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},insights:{arc42:{schema_version:1,scope:'/tmp/arc42',markdown,chapters}}},{library:false});
  app.run("showTab('architecture')");
  const article=app.ids.get('architecture-content').querySelector('article[data-arc42-chapter]');
  return {code:Array.from(article.querySelectorAll('pre'),p=>p.textContent),paragraphs:Array.from(article.querySelectorAll('p'),p=>p.textContent)};
};
const fenced=render('## 5. Bausteinsicht\n\n```c++\nint a;\n```\nprose\n\n```js title="x"\nlet b;\n```\n\n```shell-session\n$ ls\n```');
assert.deepEqual(fenced.code,['int a;','let b;','$ ls'],'non-word info strings open a code block, the bare fence closes it');
assert.ok(fenced.paragraphs.includes('prose'),'prose between the blocks stays a paragraph');
const open=render('## 5. Bausteinsicht\n\ntext\n\n```rust\nfn a() {}\n\nmore');
assert.deepEqual(open.code,['fn a() {}\n\nmore'],'an unclosed fence runs to the end of the chapter');
const comments=render('## 5. Bausteinsicht\n\n<!-- gone -->\n```html\n<!-- kept -->\n```\nend <!-- gone -->');
assert.deepEqual(comments.code,['<!-- kept -->'],'comments inside a code block stay');
assert.ok(comments.paragraphs.includes('end '),'comments in prose are removed');
console.log('arc42 fences: info strings, bare closing fence, unclosed fence and comments in code passed');
