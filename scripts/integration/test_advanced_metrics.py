#!/usr/bin/env python3
"""Real compiler evidence for source maintainability and inheritance metrics."""
import json
from pathlib import Path
import tempfile
import argparse
from test_java_calls import analyze as java
from test_csharp_calls import compile_helper, analyze as csharp

def check(report):
    methods = {m['name']:m for m in report['methods']}
    rows = {m['method_id']:m for m in report['details'].get('method_metrics',[])}
    for name, cognitive in [('simple',0),('nested',6),('ladder',3),('logic',3)]:
        assert methods[name]['id'] in rows, ('missing maintainability evidence',name)
        row = rows[methods[name]['id']]
        assert row['cognitive']==cognitive, (name,row)
        assert row['operators']>=row['distinct_operators']>0, row
        assert row['operands']>=row['distinct_operands']>0, row
        assert row['source_lines']>0
    assert methods['absent']['id'] not in rows
    hierarchy={r['class_id'].split(':')[-1]:r for r in report['details'].get('inheritance',[])}
    child=next(v for k,v in hierarchy.items() if k.endswith('Child'))
    assert child['depth']==2, child
    assert child['base_class'].endswith('Base')

def run(compiler=None):
    check(java({'Metrics.java': '''abstract class Metrics {
      abstract void absent();
      int simple(int x) { return x + 1; }
      int nested(int x) { if(x>0) { while(x>1) { if(x>2) x--; } } return x; }
      int ladder(int x) { if(x>0) return 1; else if(x<0) return 2; else return 3; }
      boolean logic(boolean a, boolean b, boolean c) { if(a && b || c) return true; return false; }
    } class Base {} class Child extends Base {}'''}))
    with tempfile.TemporaryDirectory(prefix='archguard-advanced-') as temp:
        directory=Path(temp); binary,home=compile_helper(directory); root=directory/'project';root.mkdir()
        (root/'App.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
        (root/'Metrics.cs').write_text('''abstract class Metrics {
          public abstract void absent();
          int simple(int x) { return x + 1; }
          int nested(int x) { if(x>0) { while(x>1) { if(x>2) x--; } } return x; }
          int ladder(int x) { if(x>0) return 1; else if(x<0) return 2; else return 3; }
          bool logic(bool a, bool b, bool c) { if(a && b || c) return true; return false; }
        } class Base {} class Child : Base {}''')
        check(csharp(binary,home,root,['Metrics.cs']))
    if compiler:
        from test_typescript_calls import analyze
        result=analyze(compiler,{'Metrics.ts':'''abstract class Metrics {
          abstract absent(): void;
          simple(x:number) { return x + 1; }
          nested(x:number) { if(x>0) { while(x>1) { if(x>2) x--; } } return x; }
          ladder(x:number) { if(x>0) return 1; else if(x<0) return 2; else return 3; }
          logic(a:boolean,b:boolean,c:boolean) { if(a && b || c) return true; return false; }
        } class Root {} class Base extends Root {} class Child extends Base {}'''})
        check(result)
    print('Advanced compiler metrics passed')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',type=Path)
    run(parser.parse_args().compiler)
