#!/usr/bin/env python3
"""Compiler source evidence for type-only dependencies and instance state."""
import argparse
import tempfile
from pathlib import Path
from test_java_calls import analyze as java
from test_csharp_calls import compile_helper, analyze as csharp
from test_typescript_calls import analyze as typescript

def check(report):
    details=report.get('details')
    assert details and details['definition']=='source-types-state/v1', 'typed field/signature/state evidence is absent'
    classes={c['id']:c for c in report['classes']}
    uses=details['type_uses']
    for role in ('field','parameter','return'):
        assert any(u['role']==role and u['resolution']=='project' and classes[u['to_class']]['name'].split('.')[-1]=='Port' for u in uses),(role,uses)
    service=next(c for c in report['classes'] if c['name'].split('.')[-1]=='Service')
    state=next(s for s in details['class_states'] if s['class_id']==service['id'])
    assert state['status']=='ready',state
    fields={f['id']:f for f in state['fields']}
    assert {f['name'] for f in fields.values()} >= {'left','right','shared'}
    assert any(f['name']=='shared' and f['is_static'] for f in fields.values())
    methods={m['id']:m for m in report['methods']}
    accesses={(methods[a['method']]['name'],fields[a['field']]['name']) for a in state['accesses'] if a['field'] in fields}
    assert {('one','left'),('two','right'),('three','left')} <= accesses,accesses
    assert ('shadow','left') not in accesses,'local shadow must not become a field access'
    assert all(a['file'] and a['line']>0 for a in state['accesses'])
    print('field, parameter, return, static exclusion metadata and resolved field accesses verified')

def main():
    p=argparse.ArgumentParser();p.add_argument('language',choices=['java','csharp','typescript']);p.add_argument('--compiler',type=Path);args=p.parse_args()
    if args.language=='java':
        result=java({'p/Port.java':'package p; public interface Port {}','q/Service.java':'''package q;
public class Service {
 p.Port port; int left, right; static int shared;
 public Service(p.Port port) { this.port=port; }
 p.Port identity(p.Port arg) { return arg; }
 int one(){return left;} int two(){return right;} int three(){return this.left;}
 int shadow(){int left=1;return left;}
}'''})
    elif args.language=='csharp':
        with tempfile.TemporaryDirectory(prefix='archguard-details-cs-') as directory:
            binary,home=compile_helper(Path(directory));root=Path(directory)/'source';root.mkdir()
            (root/'App.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
            (root/'Port.cs').write_text('namespace p; public interface Port {}')
            (root/'Service.cs').write_text('''namespace q;
public class Service {
 p.Port port; int left, right; static int shared;
 public Service(p.Port port) { this.port=port; }
 p.Port identity(p.Port arg) { return arg; }
 int one(){return left;} int two(){return right;} int three(){return this.left;}
 int shadow(){int left=1;return left;}
 int current { get { return left; } } int viaProperty(){return current;}
}''')
            result=csharp(binary,home,root,['Port.cs','Service.cs'])
    else:
        assert args.compiler
        result=typescript(args.compiler,{'port.ts':'export interface Port { value:number; }','service.ts':'''import {Port} from './port';
export class Service {
 port:Port; left=0; right=0; static shared=0;
 constructor(port:Port){this.port=port;}
 identity(arg:Port):Port{return arg;}
 one(){return this.left;} two(){return this.right;} three(){return this.left;}
 shadow(){let left=1;return left;}
 get current(){return this.left;} viaProperty(){return this.current;}
}'''},{'compilerOptions':{'strict':True,'target':'ES2020'},'include':['*.ts']})
    if args.language=='typescript':
        aliases=typescript(args.compiler,{'options.ts':'export type Options={value:number};','client.ts':"import type {Options} from './options'; export class Client { value:Options; constructor(value:Options){this.value=value;} }"},{'compilerOptions':{'strict':True},'include':['*.ts']})
        uses=[u for u in aliases['details']['type_uses'] if u['role']=='field']
        assert uses and uses[0]['resolution']=='project','a source type alias is a project dependency, not an external type'
        target=next(c for c in aliases['classes'] if c['id']==uses[0]['to_class'])
        assert target['file']=='options.ts'
    check(result)
    if args.language in ('csharp','typescript'):
        states=result['details']['class_states'];state=next(s for s in states if any(f['name']=='left' for f in s['fields']))
        field=next(f for f in state['fields'] if f['name']=='current')
        methods={m['id']:m for m in result['methods']}
        users={methods[a['method']]['name'] for a in state['accesses'] if a['field']==field['id']}
        assert 'viaProperty' in users and ('get_current' in users or 'current' in users),users
if __name__=='__main__':main()
