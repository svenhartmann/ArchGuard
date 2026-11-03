#!/usr/bin/env python3
"""End-to-end CLI, history and read-only MCP tests using synthetic Java/Git runs."""
import argparse, hashlib, json, os, select, subprocess, tempfile
from pathlib import Path

def main():
    parser=argparse.ArgumentParser();parser.add_argument('binary',type=Path,nargs='?',default=Path('target/debug/archguard'));parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    binary=args.binary.resolve();out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    env={k:v for k,v in os.environ.items() if not k.startswith('GIT_')};env.update(GIT_CONFIG_GLOBAL=os.devnull,GIT_CONFIG_SYSTEM=os.devnull,GIT_CONFIG_NOSYSTEM='1')
    with tempfile.TemporaryDirectory(prefix='archguard-kpi-live-') as directory:
        root=Path(directory)
        def git(*a,author='Alice'):
            return subprocess.check_output(['git','-C',str(root),'-c','core.hooksPath=/dev/null','-c','commit.gpgsign=false',*a],env={**env,'GIT_AUTHOR_NAME':author,'GIT_COMMITTER_NAME':author,'GIT_AUTHOR_EMAIL':author+'@example.invalid','GIT_COMMITTER_EMAIL':author+'@example.invalid'},stderr=subprocess.DEVNULL)
        def run(command,*a):
            result=subprocess.run([str(binary),command,str(root),*a],capture_output=True,text=True,env=env,timeout=120)
            assert result.returncode==0,(command,result.stderr,result.stdout[:500]);return result.stdout
        for package in ['p','q']:(root/'src'/package).mkdir(parents=True)
        (root/'archguard.toml').write_text("derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['java']\n[modules]\ndepth=0\n")
        (root/'src/p/Base.java').write_text('package p; public class Base { public int base(){return 1;} }')
        source=root/'src/p/A.java'
        source.write_text('package p; public class A extends Base { int state; public int run(int n){if(n>0){while(n>1){if(n>2)n--;else n--;}} return new q.B().value()+state;} }')
        (root/'src/q/B.java').write_text('package q; public class B { public int value(){return 3;} public int back(p.A a){return a.run(0);} }')
        git('init','-q','--template=','-b','main');git('add','.');git('commit','-qm','baseline');revision=git('rev-parse','HEAD').decode().strip()
        def observation(run_id,kind,outcome,seconds):return {'run_id':run_id,'kind':kind,'name':{'test':'SearchTests.findsDocument','build':'compile','runtime':'search latency'}[kind],'revision':revision,'environment':'local-jdk17','producer':'synthetic-fixture/1','timestamp':1700000000,'outcome':outcome,'duration_seconds':seconds}
        runs=[observation('one','test','failed',0.2),observation('two','test','passed',0.1),observation('one','build','passed',10),observation('two','build','passed',14),observation('one','runtime','passed',0.02),observation('two','runtime','passed',0.04)]
        (root/'runs.json').write_text(json.dumps({'schema_version':1,'runs':runs}))
        measured=json.loads(run('metrics','--extended','--report','runs.json','--format','json'))
        assert measured['code']['status']=='ready',measured['code']['diagnostics']
        assert measured['measurements']['flaky_test_percent']==100
        assert all(row['cbo']['value'] is not None for row in measured['code']['classes'])
        a=next(c for c in measured['code']['classes'] if c['class_id']=='p.A');assert a['dit']['value']==2
        before=json.loads(run('quality-history','--record','--tag','before','--format','json'))
        source.write_text('package p; public class A extends Base { int state; public int run(int n){if(n>0)n--;return new q.B().value()+state;} }')
        git('add','src/p/A.java');git('commit','-qm','simpler method',author='Bob')
        after=json.loads(run('quality-history','--record','--tag','after','--format','json'))
        (out/'debug.history.json').write_text(json.dumps(after,indent=2));(out/'debug.metrics.json').write_text(json.dumps(measured,indent=2))
        delta=next(m for m in after['comparison']['modules'] if m['node']=='p')['extended']['cognitive.max']
        assert delta['status']=='comparable' and delta['delta']<0,delta
        assert len(after['series'])==2
        assert after['latest']['operational']['runs']==6
        snapshots={p.relative_to(root).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in (root/'src').rglob('*.java')}
        metrics=json.loads(run('metrics','--extended','--format','json'));(out/'synthetic.metrics.json').write_text(json.dumps(metrics,indent=2));(out/'synthetic.history.json').write_text(json.dumps(after,indent=2))
        (out/'example.html').write_text(run('graph','--format','html'))
        proc=subprocess.Popen([str(binary),'mcp','--root',str(root)],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        sequence=0
        def rpc(method,params):
            nonlocal sequence;sequence+=1;proc.stdin.write(json.dumps({'jsonrpc':'2.0','id':sequence,'method':method,'params':params})+'\n');proc.stdin.flush();assert select.select([proc.stdout],[],[],90)[0],'MCP timeout';reply=json.loads(proc.stdout.readline());assert reply['id']==sequence;return reply
        try:
            rpc('initialize',{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'kpi-test','version':'1'}});proc.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');proc.stdin.flush()
            tools=rpc('tools/list',{})['result']['tools'];assert any(t['name']=='quality_metrics' for t in tools)
            all_rows=[];offset=0;pages=0
            while True:
                reply=rpc('tools/call',{'name':'quality_metrics','arguments':{'offset':offset,'limit':200}})
                assert 'error' not in reply and not reply['result'].get('isError'),reply
                text=reply['result']['content'][0]['text'];assert len(text.encode())<=4096
                value=json.loads(text);all_rows+=value['items'];pages+=1
                if value['next_offset'] is None:break
                assert value['next_offset']>offset;offset=value['next_offset'];assert pages<100
            kinds={r.get('kind') for r in all_rows};assert {'class_metrics','method_metrics','knowledge','measured_runs','metric_coverage'}<=kinds,kinds
            assert all(not r.get('omitted') for r in all_rows)
            query=rpc('tools/call',{'name':'quality_metrics','arguments':{'query':'SearchTests'}});assert 'SearchTests' in query['result']['content'][0]['text']
            for bad in [{'limit':0},{'path':'../outside'}]:
                reply=rpc('tools/call',{'name':'quality_metrics','arguments':bad});assert 'error' in reply or reply['result'].get('isError')
        finally:
            proc.stdin.close();proc.wait(timeout=10)
        assert snapshots=={p.relative_to(root).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in (root/'src').rglob('*.java')}
        result={'cli':True,'compiler_ready':True,'class_metrics':True,'method_metrics':True,'history_delta':delta,'timeline':True,'run_import':True,'mcp_pages':pages,'mcp_kinds':sorted(kinds),'mcp_bytes_bounded':True,'sources_unchanged':True}
        (out/'live.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
if __name__=='__main__':main()
