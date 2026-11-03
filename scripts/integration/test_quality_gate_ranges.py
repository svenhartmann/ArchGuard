#!/usr/bin/env python3
"""Compiler source ranges used by changed-method quality gates."""
import argparse
from pathlib import Path
import tempfile
from test_java_calls import analyze as analyze_java
from test_csharp_calls import compile_helper, analyze as analyze_csharp

SOURCE = """class Example {
    int choose(int value) {
        if (value > 0) return 1;
        return 0;
    }
    int other() { return 2; }
}
"""

def check(result):
    for name, start, end in [("choose", 2, 5), ("other", 6, 6)]:
        method = next(m for m in result["methods"] if m["name"] == name)
        assert method.get("line") == start, method
        assert method.get("end_line") == end, ("compiler must report inclusive method end", method)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--language", choices=["java", "csharp", "all"], default="all")
    args = parser.parse_args()
    if args.language in ("java", "all"):
        check(analyze_java({"Example.java": SOURCE}))
        print("Java exact method declaration ranges passed")
    if args.language in ("csharp", "all"):
        with tempfile.TemporaryDirectory(prefix="archguard-method-ranges-") as tmp:
            directory=Path(tmp)
            binary, home=compile_helper(directory)
            root=directory/"project";root.mkdir()
            (root/"Example.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
            (root/"Example.cs").write_text(SOURCE)
            check(analyze_csharp(binary,home,root,["Example.cs"]))
            print("C# exact method declaration ranges passed")

if __name__ == "__main__":
    main()
