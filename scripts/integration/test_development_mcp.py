#!/usr/bin/env python3
"""Protocol-level architecture entry, bounded atlas pages and real tool routes."""
import json, os, re, select, subprocess, sys, tempfile
from pathlib import Path
binary=Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/archguard').resolve()
with tempfile.TemporaryDirectory(prefix='archguard-development-') as directory:
 root=Path(directory)
 (root/'archguard.toml').write_text("derive_layers=false\n[project]\nlanguages=['ts']\n[modules]\ndepth=0\n")
 for name,body in {'org/demo/search/a.ts':"import '../client/b';\n",'org/demo/search/types.ts':'export const value=1;\n','org/demo/client/b.ts':"import '../search/types';\n"}.items():
  file=root/name;file.parent.mkdir(parents=True,exist_ok=True);file.write_text(body)
 originals={str(f.relative_to(root)):f.read_bytes() for f in root.rglob('*.ts')}
 process=subprocess.Popen([str(binary),'mcp','--root',str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
 ident=0
 def rpc(method,params):
  global ident
  ident+=1;process.stdin.write(json.dumps({'jsonrpc':'2.0','id':ident,'method':method,'params':params})+'\n');process.stdin.flush()
  while True:
   assert select.select([process.stdout],[],[],60)[0], 'MCP timeout'
   line=process.stdout.readline();assert line,process.stderr.read()
   result=json.loads(line)
   if result.get('id')==ident:return result
 def call(name,args):
  response=rpc('tools/call',{'name':name,'arguments':args});assert 'error' not in response,response
  result=response['result'];assert not result.get('isError'),result
  raw=result['content'][0]['text'];assert len(raw.encode())<=4096
  return json.loads(raw)
 try:
  rpc('initialize',{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'entry-test','version':'1'}})
  process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');process.stdin.flush()
  tools={t['name']:t for t in rpc('tools/list',{})['result']['tools']};names=set(tools)
  print('Registered MCP tools:',len(names))
  assert {'development_guide','architecture_atlas','architecture_world'}<=names, 'Missing discoverable development guide and architecture atlas tools'
  world=call('architecture_world',{'limit':1});world_revision=world['metadata']['revision'];world_items=list(world['items']);cursor=world['next_offset']
  while cursor is not None:
   page=call('architecture_world',{'limit':1,'offset':cursor,'expected_revision':world_revision});world_items+=page['items'];cursor=page['next_offset']
  assert len(world_items)==3 and all(row['coverage'] is None for row in world_items),world_items
  scoped=call('architecture_world',{'within':'org/demo/client'});assert len(scoped['items'])==1
  for args in [{'within':'missing'},{'expected_revision':'0'*64},{'path':'/'}]:
   bad=rpc('tools/call',{'name':'architecture_world','arguments':args});assert bad.get('error') or bad['result'].get('isError'),args
  guide=call('development_guide',{'limit':20})
  routes=list(guide['items']);cursor=guide['next_offset']
  while cursor is not None:
   page=call('development_guide',{'limit':20,'offset':cursor});routes+=page['items'];cursor=page['next_offset']
  guide['items']=routes
  assert {r['view'] for r in guide['items']}=={'graph','dsm','calls','services','architecture','assessment','quality','security'}
  for row in guide['items']:
   for route in row['tools']:
    assert route['name'] in names,route
    schema=tools[route['name']]['inputSchema'];properties=schema.get('properties',{})
    arguments=route.get('arguments',{});selectors=route.get('requires',[])+route.get('requires_one_of',[])
    assert set(arguments)<=set(properties),route
    assert set(selectors)<=set(properties),route
    assert set(schema.get('required',[]))<=set(arguments)|set(selectors),route
    assert ('path' in properties)==(route['project_scope']=='selected_project'),route
  for name,args in [('development_guide',{'view':'missing'}),('development_guide',{'mission':'missing'}),('architecture_atlas',{'within':'missing'}),('architecture_atlas',{'expected_revision':'bad'})]:
   bad=rpc('tools/call',{'name':name,'arguments':args});assert bad.get('error') or bad['result'].get('isError'),(name,args)
  atlas=call('architecture_atlas',{'limit':1});revision=atlas['metadata']['revision'];items=atlas['items'];cursor=atlas['next_offset']
  assert atlas['metadata']['scope']=='org/demo';assert atlas['metadata']['cycle_groups']==1
  while cursor is not None:
   page=call('architecture_atlas',{'limit':1,'offset':cursor,'expected_revision':revision});items+=page['items'];cursor=page['next_offset']
  areas=[r for r in items if r['kind']=='area'];assert {r['name'] for r in areas}=={'org/demo/search','org/demo/client'}
  assert sum(r['source_files'] for r in areas)==3
  assert all(r['cyclic'] for r in areas)
  assert len([r for r in items if r['kind']=='connection'])==2
  focused=call('architecture_atlas',{'within':'org/demo/search'});assert focused['metadata']['scope']=='org/demo/search'
  assert focused['metadata']['external_connections']>0
  html=subprocess.check_output([str(binary),'graph',str(root),'--format','html'],text=True)
  payload=json.loads(re.search(r'<script id="archguard-data" type="application/json">([\s\S]*?)</script>',html).group(1))
  assert payload['insights']['world']['entities']==world_items
  assert payload['insights']['world']['revision']==world_revision
  assert payload['developmentPaths']==json.loads(Path('crates/archguard-core/src/development_paths.json').read_text())
  js="""const fs=require('node:fs'),vm=require('node:vm');
const harness=fs.readFileSync('scripts/viewer/test_viewer_state.js','utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\\nlaunch;',{require,console,performance,Blob});
const app=launch(JSON.parse(fs.readFileSync(0,'utf8')),{library:false});
process.stdout.write(app.run('JSON.stringify([architectureAtlasModel(work),architectureAtlasModel(work,"org/demo/search")])'));"""
  frontend=json.loads(subprocess.check_output(['node','-e',js],input=json.dumps(payload),text=True))
  for model,metadata,records in [(frontend[0],atlas['metadata'],items),(frontend[1],focused['metadata'],focused['items'])]:
   for key in ['scope','cycle_groups','external_connections']:assert model[key]==metadata[key],key
   expected={r['name']:{k:v for k,v in r.items() if k not in ['kind','node_kind']} for r in records if r['kind']=='area'}
   actual={r['name']:{k:v for k,v in r.items() if k not in ['kind','src']} for r in model['areas']}
   assert actual==expected,(actual,expected)
   assert model['connections']==[{k:v for k,v in r.items() if k!='kind'} for r in records if r['kind']=='connection']
  (root/'org/demo/search/a.ts').write_text('export const changed=true;\n')
  stale=rpc('tools/call',{'name':'architecture_atlas','arguments':{'expected_revision':revision,'offset':1}})
  assert stale.get('error') or stale['result'].get('isError'),'changed graph must reject old pagination'
  for name in ['development_guide','architecture_atlas']:
   bad=rpc('tools/call',{'name':name,'arguments':{'path':'../outside'}});assert bad.get('error') or bad['result'].get('isError')
  assert (root/'org/demo/client/b.ts').read_bytes()==originals['org/demo/client/b.ts']
  print('Development MCP: discoverable routes, source-backed atlas, cycle semantics, pagination, revision checks and root confinement passed')
 finally:
  process.terminate();process.communicate(timeout=10)
