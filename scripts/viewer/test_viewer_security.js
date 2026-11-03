// Real Security workspace and exports with DOM doubles; no scanner or network.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});
const finding=(id,packageName,severity='high',extra={})=>({id,package:packageName,installed_version:'1.0',fixed_version:'2.0, 3.1',ecosystem:'npm',severity,title:'Dependency advisory '+id,url:'https://example.invalid/advisory/'+id,targets:['apps/web/package-lock.json'],...extra});
const security={schema_version:1,status:'ready',scanner:'trivy',scanner_version:'0.69.0',scanned_at:'2026-10-28T10:00:00Z',database_updated_at:'2026-10-28T08:00:00Z',fingerprint:'original-lockfiles',packages_scanned:150,targets:[{path:'apps/web/package-lock.json',ecosystem:'npm',packages:130},{path:'api/packages.lock.json',ecosystem:'nuget',packages:25}],findings:[
  finding('CVE-2026-0001','shared','critical'),finding('CVE-2026-0002','shared','high',{fixed_version:null}),
  finding('CVE-2026-0003','<img src=x>{count}','unknown',{url:'javascript:alert(1)',title:'<script>alert(1)</script>'}),
  ...Array.from({length:102},(_,i)=>finding('CVE-2026-'+(1000+i),'package-'+i)),
],diagnostics:['Original diagnostic <img src=x> {count}']};
const fixture={root:"/tmp/security project's $(literal)",nodes:[{id:0,name:'A',kind:'file',parent:null},{id:1,name:'B',kind:'file',parent:null}],edges:[{from:0,to:1,count:1,refs:[]}],view:{depth:0,visible:null},findings:[],cyclicEdges:[],insights:{security}};
const app=launch(fixture,{library:false});
assert.ok(app.ids.has('tab-security'),'the project has a dedicated Security workspace');
app.context.fetch=()=>assert.fail('rendering a stored snapshot must not make network requests');
app.context.XMLHttpRequest=class{constructor(){assert.fail('no background network scanner');}};
app.run("showTab('security')");
assert.equal(app.run('ui.workspace'),'security');
assert.equal(app.ids.get('tab-security').classList.contains('panel-hidden'),false);
assert.equal(app.ids.get('simulation-mode').disabled,true);assert.equal(app.ids.get('repair-cycle').disabled,true);
assert.match(app.ids.get('workspace-description').textContent,/original.*scan/i);
assert.match(app.ids.get('sidebar').textContent,/original.*scan/i);
const rows=()=>app.ids.get('security-findings').querySelectorAll('article[data-security-finding]');
const metric=name=>app.ids.get('security-metrics').querySelectorAll('div[data-security-metric]').find(n=>n.getAttribute('data-security-metric')===name).querySelector('strong').textContent;
assert.equal(metric('packages_scanned'),'150');assert.equal(metric('affected_packages'),'104');assert.equal(metric('findings'),'105');
assert.equal(app.ids.get('security-severities').querySelectorAll('div[data-security-severity]').length,5);
assert.equal(rows().length,100,'initial findings are bounded');
assert.match(app.ids.get('security-count').textContent,/100.*105/);
const coverage=app.ids.get('security-coverage-content');
assert.match(coverage.textContent,/0\.69\.0/);assert.match(coverage.textContent,/2026-10-28T10:00:00Z/);assert.match(coverage.textContent,/2026-10-28T08:00:00Z/);
app.ids.get('security-more').focus();app.click('security-more');assert.equal(rows().length,105);
assert.ok(app.document.activeElement===rows()[100],'Load more focuses the first added finding, even when its button becomes hidden');
assert.match(app.ids.get('security-findings').textContent,/2\.0, 3\.1/,'fixed versions retain the original scanner string');
assert.match(app.ids.get('security-findings').textContent,/No fixed version reported/);
assert.equal(app.ids.get('security-findings').querySelectorAll('img').length,0);
assert.equal(app.ids.get('security-findings').querySelectorAll('script').length,0);
for(const link of app.ids.get('security-findings').querySelectorAll('a'))assert.match(link.getAttribute('href'),/^https?:\/\//);
assert.equal(app.run("safeSecurityUrl('javascript:alert(1)')"),null);
assert.equal(app.run("safeSecurityUrl('data:text/html,x')"),null);
assert.equal(app.run("safeSecurityUrl('//example.invalid')"),null);
assert.equal(app.run("safeSecurityUrl('/relative')"),null);
assert.equal(app.run("safeSecurityUrl('https://')"),null);
assert.equal(app.run("safeSecurityUrl('https://example.invalid/a')"),'https://example.invalid/a');
const evidence=rows()[0].querySelector('details');evidence.open=true;evidence.dispatch('toggle');
assert.match(evidence.textContent,/apps\/web\/package-lock\.json/);
app.run('copyText=text=>{globalThis.copiedSecurityCommand=text;}');
coverage.querySelector('button[data-security-copy]').click();
assert.equal(app.run('copiedSecurityCommand'),"archguard security -- '/tmp/security project'\\''s $(literal)'");

const search=app.ids.get('security-query'),severity=app.ids.get('security-severity');
search.focus();search.value='SHARED';search.dispatch('input');assert.equal(rows().length,2,'search is case-insensitive across package names');
assert.equal(app.document.activeElement,search,'filter rendering preserves the existing input and keyboard focus');
search.value='shared CVE-2026';search.dispatch('input');assert.equal(rows().length,0,'like MCP, a query matches one package name or advisory ID, not an invented concatenation');
search.value='SHARED';search.dispatch('input');
severity.focus();
severity.value='critical';severity.dispatch('change');assert.equal(rows().length,1);
assert.equal(app.document.activeElement,severity,'severity rendering preserves select focus');
assert.equal(metric('findings'),'105','snapshot KPIs are not relabeled as filtered totals');
const before=app.run('JSON.stringify([data,work,view,ops,securityUi])');
app.click('language-de');
assert.equal(search.value,'SHARED');assert.equal(severity.value,'critical');
assert.match(app.ids.get('security-status').textContent,/Scan abgeschlossen/);
assert.match(app.ids.get('security-count').textContent,/angezeigt/);
assert.ok(app.ids.get('tabs').querySelectorAll('button').some(b=>b.dataset.tab==='security'&&b.textContent==='Sicherheit'));
assert.match(app.ids.get('security-findings').textContent,/Kritisch/);
assert.equal(app.run('JSON.stringify([data,work,view,ops,securityUi])'),before,'language changes preserve original sources and filters');
app.click('language-en');
search.value='CVE-2026-0002';search.dispatch('input');severity.value='all';severity.dispatch('change');assert.equal(rows().length,1);
search.value='not-in-this-scan';search.dispatch('input');assert.equal(rows().length,0);
assert.match(app.ids.get('security-findings').textContent,/No findings match/);
search.value='';search.dispatch('input');assert.equal(rows().length,100,'filter changes reset pagination');

const exported=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(exported.kind,'security');assert.equal(exported.findings.length,105,'exports include all matching findings beyond the rendered page');
assert.deepEqual(exported.snapshot,security,'exports retain the full original scan and coverage');
assert.equal(exported.metrics.acd,undefined);assert.equal(exported.simulation_ops,undefined);
const markdown=app.run('exportMarkdown(exportSnapshot())');
assert.match(markdown,/Dependency security/);assert.match(markdown,/package-lock\.json/);assert.match(markdown,/2\.0, 3\.1/);
assert.doesNotMatch(markdown,/<script>|<img|javascript:|```mermaid/);
assert.equal(app.run('exportSvg()'),null,'Security cannot export a hidden package tree');
search.value='SHARED';search.dispatch('input');
const filtered=JSON.parse(app.run('JSON.stringify(exportSnapshot())'));
assert.equal(filtered.findings.length,2);assert.equal(filtered.snapshot.findings.length,105);assert.equal(filtered.filters.query,'SHARED');
app.run("showTab('graph');setSimulation(true);recordOp(encodeCut('A','B'));showTab('security')");
assert.equal(app.run('ops.length'),1);assert.equal(metric('findings'),'105');
assert.deepEqual(JSON.parse(app.run('JSON.stringify(securitySnapshot())')),security,'graph simulation never changes the vulnerability snapshot');
assert.match(app.ids.get('security-snapshot-note').textContent,/simulat/i);
for(const name of ['calls','architecture','services','graph']){app.run('showTab('+JSON.stringify(name)+')');assert.equal(app.run('ui.workspace'),name);}

for(const status of ['not_scanned','partial','stale','error']){
  const other=launch({...fixture,initialTab:'security',insights:{security:{...security,status,packages_scanned:0,findings:[],targets:[]}}},{library:false});
  assert.equal(other.run('ui.workspace'),'security');
  assert.equal(other.ids.get('security-status').getAttribute('data-status'),status);
  assert.doesNotMatch(other.ids.get('security-status').textContent,/Scan completed|clean|secure|safe/i);
  assert.match(other.ids.get('security-findings').textContent,/not establish|No packages|not scanned|failed|unknown/i);
  assert.equal(other.ids.get('security-metrics').querySelectorAll('div[data-security-metric]').find(n=>n.getAttribute('data-security-metric')==='findings').querySelector('strong').textContent,'—');
}
const missing=launch({...fixture,initialTab:'security',insights:{}},{library:false});
assert.equal(missing.ids.get('security-status').getAttribute('data-status'),'not_scanned');
const empty=launch({...fixture,initialTab:'security',insights:{security:{...security,packages_scanned:0,findings:[],targets:[]}}},{library:false});
assert.match(empty.ids.get('security-findings').textContent,/No packages/);
const clear=launch({...fixture,initialTab:'security',insights:{security:{...security,findings:[]}}},{library:false});
assert.match(clear.ids.get('security-findings').textContent,/No vulnerabilities reported.*snapshot/);
const uncovered=launch({...fixture,initialTab:'security',insights:{security:{...security,status:'partial',targets:[{path:'unsupported/package.json',ecosystem:'npm',packages:0}]}}},{library:false});
assert.match(uncovered.ids.get('security-coverage-content').querySelector('details').querySelector('summary').textContent,/Manifest \/ lockfile coverage/,'target rows can include uncovered manifests and must not be labeled as all scanned');
const manyTargets=launch({...fixture,initialTab:'security',insights:{security:{...security,findings:[{...security.findings[0],targets:Array.from({length:102},(_,i)=>'app-'+i+'/package-lock.json')}]}}},{library:false});
const targetEvidence=manyTargets.ids.get('security-findings').querySelector('details');targetEvidence.open=true;targetEvidence.dispatch('toggle');
assert.equal(targetEvidence.querySelectorAll('p').length,100,'evidence lists are also bounded');
targetEvidence.querySelector('button').focus();targetEvidence.querySelector('button').click();
assert.equal(targetEvidence.querySelectorAll('p').length,102);
assert.ok(manyTargets.document.activeElement===targetEvidence.querySelectorAll('p')[100],'loading the last evidence page returns focus to its first added record');
assert.deepEqual(fixture.insights.security,security);
console.log('Security: honest scan status/coverage, KPIs, bounded filters, safe evidence, immutable simulation, DE/EN and full JSON/Markdown export passed');
