#!/usr/bin/env python3
"""Real TypeScript compiler tests. Pass --compiler /path/to/typescript/lib/typescript.js."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[2]
HELPER = REPO / "crates/archguard-engine/src/ArchguardCallScanner.cjs"


def analyze(compiler, sources, config=None, project=None, extra=None):
    with tempfile.TemporaryDirectory(prefix="archguard-ts-calls-") as tmp:
        root = Path(tmp)
        for name, text in {**sources, **(extra or {})}.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        if config is not None:
            (root / "tsconfig.json").write_text(json.dumps(config))
        manifest = root / "input.json"
        manifest.write_text(json.dumps({"root": str(root), "compiler": str(compiler), "project": project,
            "sources": [{"path": str(root / name), "file": name, "graph_node": "mapped/" + name,
                         "language": "javascript" if name.endswith((".js", ".jsx")) else "typescript"} for name in sources]}))
        env = {k: v for k, v in os.environ.items() if k not in ("NODE_OPTIONS", "NODE_PATH")}
        result = subprocess.run(["node", str(HELPER), str(manifest)], text=True, capture_output=True, timeout=45, env=env)
        assert result.returncode == 0, result.stderr
        assert not (root / "executed").exists(), "project code/plugins must never execute"
        assert not list(root.glob("*.js.map")), "analysis emits no project output"
        return json.loads(result.stdout)["analysis"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    args = parser.parse_args()
    assert HELPER.is_file(), "TypeScript semantic analysis needs a compiler-backed helper"
    compiler = args.compiler.resolve()
    assert compiler.is_file()
    sources = {
        "api.ts": """export interface Api { save(value: string): void; }
export class Store implements Api {
  save(value: string): void;
  save(value: number): void;
  save(value: string | number): void {}
  static clear() {}
}
export function first(n: number): number { return n ? second(n - 1) : 0; }
export function second(n: number): number { return first(n); }
""",
        "client.ts": """import { Store, Api, first as entry } from './api';
export class Client {
  run(api: Api, store: Store) {
    api.save("hello <ä>&</script>");
    store.save(42);
    Store.clear();
    const created = new Store();
    entry(2);
    // store.fake();
    const text = 'store.fake()';
  }
}
""",
    }
    result = analyze(compiler, sources, {"compilerOptions": {"strict": True, "target": "ES2020"}, "include": ["*.ts"]})
    assert result["status"] == "ready", result["diagnostics"]
    assert result["files"] == 2
    classes = {c["name"]: c for c in result["classes"]}
    assert classes["Store"]["supertypes"] == [classes["Api"]["id"]]
    assert classes["Api"]["kind"] == "interface"
    assert any(c["kind"] == "module" for c in result["classes"]), "top-level functions need explicit module owners"
    assert all(c["graph_node"].startswith("mapped/") for c in classes.values())
    calls = result["calls"]
    assert len(calls) == 7, calls
    assert all(c["resolution"] == "project" for c in calls), calls
    client = [c for c in calls if c["file"] == "client.ts"]
    assert [c["dispatch"] for c in client] == ["interface", "instance", "static", "constructor", "static"]
    assert client[0]["code"] == 'api.save("hello <ä>&</script>")'
    assert client[0]["line"] == 4 and client[0]["column"] == 5
    assert client[0]["to_method"] != client[1]["to_method"], "resolved interface/overload declarations stay distinct"
    assert all(c["from_method"] for c in calls)
    declared = {m["id"] for m in result["methods"]}
    assert all(c["to_method"] in declared for c in calls if c["to_method"]), calls
    assert all(m["end_line"] >= m["line"] for m in result["methods"])
    assert not any(m["name"] == "fake" for m in result["methods"])
    print("TypeScript calls: project symbols, imports/aliases, overloads, interface dispatch, constructors, module functions and exact evidence passed")

    missing = analyze(compiler, {"a.ts": "export function run(value: any) { value.unknown(); missing(); }"})
    assert missing["status"] == "partial"
    assert all(c["resolution"] == "unresolved" and c["to_method"] is None for c in missing["calls"])
    ambiguous = analyze(compiler, {"a.ts": "function a() {} function b() {} export function run(x: boolean) { const f = x ? a : b; f(); }"})
    assert ambiguous["status"] == "partial"
    assert ambiguous["calls"][-1]["resolution"] == "unresolved", "union call signatures must not invent one runtime target"
    print("TypeScript calls: dynamic, missing and union targets remain unresolved")

    metrics = analyze(compiler, {"a.ts": """export function flow(x: number, a: boolean, b: boolean) {
  if (a && b) { while (x > 0) { if (x === 2) x--; x--; } }
  else if (b || a) { for (let i = 0; i < 2; i++) x++; }
  try { switch (x) { case 1: case 2: x++; break; default: x--; } }
  catch (e) { x = a ? 1 : 2; }
  return x;
}
export function enclosing() { const local = () => { if (true) {} }; function nested() { if (true) {} } }
export const arrow = (x: number) => x > 0 ? x : 0;
"""})
    methods = {m["name"]: m for m in metrics["methods"]}
    assert methods["flow"]["complexity"] == {"cyclomatic": 12, "max_nesting": 3, "definition": "ast-decisions-v1"}
    assert methods["enclosing"]["complexity"]["cyclomatic"] == 1
    assert methods["arrow"]["complexity"]["cyclomatic"] == 2
    assert methods["nested"]["complexity"]["cyclomatic"] == 2
    broken = analyze(compiler, {"a.ts": "export function broken() { const a = ; if(true) {} }"})
    assert broken["status"] == "partial" and all("complexity" not in m for m in broken["methods"])
    print("TypeScript complexity: AST decisions, nesting, source ranges, nested scope isolation and parse recovery passed")

    configured = analyze(compiler, {"src/a.ts": "import { b } from '@lib/b'; export function a(){ b(); }", "src/b.ts": "export function b() {}", "excluded.ts": "export function excluded() {}"},
        {"compilerOptions": {"baseUrl": ".", "paths": {"@lib/*": ["src/*"]}, "plugins": [{"name": "./evil.js"}]}, "include": ["src/*.ts"]},
        extra={"evil.js": "require('node:fs').writeFileSync('executed','bad');"})
    assert configured["status"] == "partial", "scanner-visible files excluded by tsconfig remain explicit coverage gaps"
    assert not any(m["name"] == "excluded" for m in configured["methods"])
    assert configured["calls"][0]["resolution"] == "project", configured
    print("TypeScript configuration: paths aliases and project exclusions honored; plugins never execute")

    js = analyze(compiler, {"a.js": "exports.run = function run(x) { return x ? run(x - 1) : 0; };"}, {"compilerOptions": {"allowJs": True, "checkJs": False}, "include": ["*.js"]})
    assert any(a["language"] == "javascript" for a in js["analyzers"])
    assert js["methods"] and all(c["language"] == "javascript" for c in js["classes"])
    no_js = analyze(compiler, {"a.js": "function run() {}"}, {"compilerOptions": {"allowJs": False}})
    assert no_js["status"] in ("partial", "unavailable") and not no_js["methods"]
    print("JavaScript semantics requires actual allowJs source membership")

    namespaces = analyze(compiler, {"a.ts": "namespace Left { export class Same { run(){ this.run(); } } } namespace Right { export class Same { run(){ this.run(); } } }"})
    types = [c for c in namespaces["classes"] if c["kind"] == "class"]
    assert len(types) == 2 and len({c["id"] for c in types}) == 2, "namespace-qualified symbols must not collapse"
    assert len({c["from_class"] for c in namespaces["calls"]}) == 2
    functions = analyze(compiler, {"a.ts": "namespace Left { export function run(){ run(); } } namespace Right { export function run(){ run(); } }"})
    assert len(functions["methods"]) == 2 and len({m["id"] for m in functions["methods"]}) == 2, "same-name namespace functions keep unique symbols"
    print("TypeScript namespaces keep declaration identities separate")

    references = analyze(compiler, {"lib/a.ts": "export function a() {}", "app/b.ts": "import { a } from '../lib/a'; export function b(){ a(); }"},
        {"files": [], "references": [{"path": "lib"}, {"path": "app"}]}, extra={
            "lib/tsconfig.json": json.dumps({"compilerOptions": {"composite": True}, "include": ["*.ts"]}),
            "app/tsconfig.json": json.dumps({"extends": "../base.json", "include": ["*.ts"], "references": [{"path": "../lib"}]}),
            "base.json": json.dumps({"compilerOptions": {"strict": True, "target": "ES2020"}}),
        })
    assert references["status"] == "ready", references["diagnostics"]
    assert len(references["methods"]) == 2 and references["calls"][0]["resolution"] == "project"
    print("TypeScript project references and JSON config inheritance analyzed without a project build")

    same_line = analyze(compiler, {"a.ts": "export function outer(flag:boolean) { if (flag) {function f(){return 1;} f();} else {function f(){return 2;}f();}}"})
    functions = [method for method in same_line["methods"] if method["name"] == "f"]
    assert len(functions) == 2 and len({method["id"] for method in functions}) == 2, functions
    sites = [call for call in same_line["calls"] if call["code"] == "f()"]
    assert len(sites) == 2 and len({call["to_method"] for call in sites}) == 2, sites
    assert all(call["resolution"] == "project" for call in sites)
    print("TypeScript declaration identities distinguish same-line block-local functions")

    mutations = {
        "binding": "export let call = () => 1; call = () => 2; call();",
        "property": "export const holder = {run: () => 1}; holder.run = () => 2; holder.run();",
        "alias": "export const holder = {run: () => 1}; const alias=holder; alias.run = () => 2; holder.run();",
        "receiver": "export let holder = {run: () => 1}; holder = {run: () => 2}; holder.run();",
        "array_assignment": "export let call = () => 1; [call] = [() => 2]; call();",
        "object_assignment": "export let call = () => 1; ({call} = {call: () => 2}); call();",
        "class_method": "class C {run(){return 1;}} const c=new C(); c.run=()=>2; c.run();",
    }
    for name, code in mutations.items():
        changed = analyze(compiler, {"a.ts": code})
        assert changed["status"] == "partial", (name, changed)
        call = changed["calls"][-1]
        assert call["resolution"] == "unresolved" and call["to_method"] is None, (name, call)
        if name == "binding":
            functions = [method for method in changed["methods"] if method["name"] == "call"]
            assert len(functions) == 2 and len({method["id"] for method in functions}) == 2, functions
    for code in ["export const call=()=>1; call();", "export const holder={run:()=>1}; holder.run();", "export function call(){return 1;} call();"]:
        stable = analyze(compiler, {"a.ts": code})
        assert stable["status"] == "ready", stable["diagnostics"]
        assert stable["calls"][-1]["resolution"] == "project", stable["calls"]
    js_changed = analyze(compiler, {"a.js": mutations["binding"]}, {"compilerOptions": {"allowJs": True, "checkJs": False}, "include": ["*.js"]})
    assert js_changed["status"] == "partial" and js_changed["calls"][-1]["resolution"] == "unresolved", js_changed
    print("TypeScript callable writes invalidate inferred targets; stable functions and object methods stay resolved")



if __name__ == "__main__":
    main()
