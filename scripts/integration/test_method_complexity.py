#!/usr/bin/env python3
"""Compiler AST metrics: real offline Java/Roslyn tests, no project builds."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

from test_java_calls import HELPER as JAVA_HELPER, JAVAC_API, analyze as analyze_java
from test_csharp_calls import compile_helper, analyze as analyze_csharp


def check(result, name, cyclomatic, nesting):
    methods = [m for m in result["methods"] if m["name"] == name]
    assert len(methods) == 1, (name, methods)
    assert methods[0].get("complexity") == {
        "cyclomatic": cyclomatic,
        "max_nesting": nesting,
        "definition": "ast-decisions-v1",
    }, (name, methods[0].get("complexity"))


def java():
    result = analyze_java({"p/Metrics.java": """package p;
abstract class Metrics {
    abstract void absent();
    void simple() { String ignored = "if && while ?"; /* if (true) {} */ }
    int flow(int x, boolean a, boolean b) {
        if (a && b) { while (x > 0) { if (x == 2) x--; x--; } }
        else if (b || a) { for (int i=0; i<2; i++) x++; }
        try { switch (x) { case 1: case 2: x++; break; default: x--; } }
        catch (RuntimeException e) { x = a ? 1 : 2; }
        return x;
    }
    int choice(int x) { return switch (x) { case 1, 2 -> 1; default -> 0; }; }
    int ternary(boolean a, boolean b) { return a ? (b ? 1 : 2) : 3; }
    int loops(int[] values) { int n=0; for (int value : values) { do { n++; } while(n<value); } return n; }
    void chain(boolean a, boolean b) { if(a) {} else if(b) {} else {} }
    void enclosing() {
        Runnable fn = () -> { if(true) { while(false) {} } };
        class Local { int nested(int x) { return x > 0 ? x : 0; } }
        new Local();
    }
    Missing external(Missing x) { if (x == null) return x; return x; }
}
"""})
    assert result["status"] == "partial", "missing external type remains explicit"
    for args in [("simple",1,0),("flow",12,3),("choice",3,1),("ternary",3,2),
                 ("loops",3,2),("chain",3,1),("enclosing",1,0),("nested",2,1),("external",2,1)]:
        check(result, *args)
    assert "complexity" not in next(m for m in result["methods"] if m["name"] == "absent"), "no body is unavailable, not complexity zero or one"
    broken = analyze_java({"p/Broken.java": "package p; class Broken { void broken() { int x = ; if(true) {} } }"})
    assert broken["status"] == "partial"
    assert all("complexity" not in m for m in broken["methods"]), "syntax recovery cannot claim measured metrics"
    print("Java complexity: decisions, nesting, switch labels, no-body, local/lambda isolation and parse errors passed")


def csharp():
    with tempfile.TemporaryDirectory(prefix="archguard-complexity-cs-") as tmp:
        directory = Path(tmp)
        binary, home = compile_helper(directory)
        root = directory / "project"
        root.mkdir()
        (root / "Metrics.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><Nullable>enable</Nullable></PropertyGroup></Project>')
        (root / "Metrics.cs").write_text("""using System;
abstract class Metrics {
    abstract void absent();
    void simple() { string ignored = "if && while ?"; /* if (true) {} */ }
    int flow(int x, bool a, bool b) {
        if (a && b) { while (x > 0) { if (x == 2) x--; x--; } }
        else if (b || a) { for (int i=0; i<2; i++) x++; }
        try { switch (x) { case 1: case 2: x++; break; default: x--; break; } }
        catch (Exception) { x = a ? 1 : 2; }
        return x;
    }
    int choice(int x) => x switch { 1 or 2 => 1, _ => 0 };
    int ternary(bool a, bool b) => a ? (b ? 1 : 2) : 3;
    int loops(int[] values) { int n=0; foreach (int value in values) { do { n++; } while(n<value); } return n; }
    void chain(bool a, bool b) { if(a) {} else if(b) {} else {} }
    void enclosing() {
        Action fn = () => { if(true) { while(false) {} } };
        int nested(int x) { return x > 0 ? x : 0; }
        nested(1);
    }
    Missing external(Missing x) { if (x == null) return x; return x; }
    string? nulls(Metrics? a, string? b) { b ??= ""; return a?.ToString() ?? b; }
    bool patterns(int n) => n is (>0 and <10) or 20;
    int guards(int n) { try { return n switch { int x when x > 0 => x, _ => 0 }; } catch(Exception) when(n<0) { return 0; } }
}
""")
        result = analyze_csharp(binary, home, root, ["Metrics.cs"])
        assert result["status"] == "partial", "missing external type remains explicit"
        for args in [("simple",1,0),("flow",12,3),("choice",3,1),("ternary",3,2),
                     ("loops",3,2),("chain",3,1),("enclosing",1,0),("nested",2,1),("external",2,1),
                     ("nulls",4,0),("patterns",3,0),("guards",5,1)]:
            check(result, *args)
        assert "complexity" not in next(m for m in result["methods"] if m["name"] == "absent")
        (root / "Body.cs").write_text("partial class Partial { partial void run(int n) { if(n > 0) {} } }")
        (root / "Declaration.cs").write_text("partial class Partial { partial void run(int n); }")
        for source_order in (["Body.cs", "Declaration.cs"], ["Declaration.cs", "Body.cs"]):
            partial = analyze_csharp(binary, home, root, source_order)
            check(partial, "run", 2, 1)
            assert next(m for m in partial["methods"] if m["name"] == "run")["file"] == "Body.cs", "partial metrics need the implementation source location"
        (root / "Metrics.cs").write_text("class Broken { void broken() { int x = ; if(true) {} } }")
        broken = analyze_csharp(binary, home, root, ["Metrics.cs"])
        assert broken["status"] == "partial"
        assert all("complexity" not in m for m in broken["methods"]), "syntax recovery cannot claim measured metrics"
        print("C# complexity: decisions, nesting, modern syntax, no-body, local/lambda isolation and parse errors passed")


def java21():
    """Explicit JDK21+ lane; fails on unsupported JDKs rather than skipping."""
    with tempfile.TemporaryDirectory(prefix="archguard-complexity-java21-") as tmp:
        root = Path(tmp)
        empty = root / "empty"
        empty.mkdir()
        source = root / "Patterns.java"
        source.write_text("""class Patterns {
    int types(Object x) { return switch(x) { case String s -> 1; case Integer n -> 2; default -> 0; }; }
    int nullable(Object x) { return switch(x) { case String s -> 1; case null, default -> 0; }; }
    record Pair(int x, int y) {}
    int record(Object x) { return switch(x) { case Pair(int a, int b) -> a+b; default -> 0; }; }
}
""")
        manifest = root / "sources"
        manifest.write_text("\0".join((str(source), "Patterns.java", "Patterns.java")) + "\0")
        context = root / "context"
        context.write_text("21\0\0")
        result = subprocess.run(
            ["java", "-Xmx512m", "--add-exports", JAVAC_API, "--class-path", str(empty), str(JAVA_HELPER), str(manifest), str(empty), str(context)],
            check=True, capture_output=True, text=True, timeout=45,
            env={k: v for k, v in os.environ.items() if k not in ("JDK_JAVA_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")},
        )
        measured = json.loads(result.stdout)
        assert measured["status"] == "ready", measured["diagnostics"]
        check(measured, "types", 3, 1)
        check(measured, "nullable", 3, 1)
        check(measured, "record", 2, 1)
        assert not list(root.rglob("*.class")), "analysis must not generate project classes"
        print("Java21 complexity: type patterns, record patterns and combined null/default labels passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("language", choices=["java", "java21", "csharp", "all"], default="all", nargs="?")
    selection = parser.parse_args().language
    if selection in ("java", "all"):
        java()
    if selection in ("csharp", "all"):
        csharp()
    if selection == "java21":
        java21()
