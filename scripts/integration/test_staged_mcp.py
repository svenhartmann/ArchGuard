#!/usr/bin/env python3
"""Read-only staged architecture pages and revision rejection over MCP stdio."""
import json, select, subprocess, sys, tempfile
from pathlib import Path
binary=Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/archguard').resolve()
with tempfile.TemporaryDirectory(prefix='archguard-staged-mcp-') as directory:
 root=Path(directory)
 def git(*args):return subprocess.check_output(['git','-C',str(root),*args],stderr=subprocess.DEVNULL)
 git('init','-q');git('config','user.name','Fixture');git('config','user.email','fixture@example.invalid');git('config','commit.gpgsign','false')
 (root/'archguard.toml').write_text("derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n")
 (root/'a.ts').write_text("import './b';\n");(root/'b.ts').write_text('export const b=1;\n');git('add','.');git('commit','-qm','base')
 (root/'b.ts').write_text("import './a';\n");git('add','b.ts');(root/'b.ts').write_text('export const unstaged=2;\n');(root/'archguard.toml').write_text('[invalid')
 before=(root/'.git/index').read_bytes()
 process=subprocess.Popen([str(binary),'mcp','--root',str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
 identifier=0
 def rpc(method,params):
  global identifier
  identifier+=1;process.stdin.write(json.dumps({'jsonrpc':'2.0','id':identifier,'method':method,'params':params})+'\n');process.stdin.flush()
  assert select.select([process.stdout],[],[],30)[0],'MCP timeout'
  response=json.loads(process.stdout.readline());assert response['id']==identifier;return response
 def call(**args):return rpc('tools/call',{'name':'staged_architecture_diff','arguments':args})
 try:
  rpc('initialize',{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'staged-test','version':'1'}})
  process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');process.stdin.flush()
  rows=[];offset=0;revision=None
  while True:
   args={'limit':1,'offset':offset}
   if revision:args['expected_revision']=revision
   reply=call(**args);assert 'error' not in reply and not reply['result'].get('isError'),reply
   raw=reply['result']['content'][0]['text'];assert len(raw.encode())<=4096
   page=json.loads(raw);revision=page['metadata']['revision'];rows+=page['items']
   assert page['metadata']['added_edges']==1 and page['metadata']['policy_source']=='index'
   if page['next_offset'] is None:break
   assert page['next_offset']>offset;offset=page['next_offset']
  assert any(r['kind']=='added_edge' and r['from']=='b.ts' and r['to']=='a.ts' for r in rows),rows
  assert any(r['kind']=='new_finding' and r['finding_kind']=='cycle' for r in rows),rows
  assert (root/'.git/index').read_bytes()==before
  git('add','b.ts')
  stale=call(offset=1,expected_revision=revision);assert 'error' in stale or stale['result'].get('isError'),stale
  outside=call(path='../');assert 'error' in outside or outside['result'].get('isError'),outside
  print('MCP staged diff: bounded pages, complete index, new cycle, source evidence, read-only index and stale/outside rejection passed')
 finally:
  process.terminate();process.wait(timeout=10)
