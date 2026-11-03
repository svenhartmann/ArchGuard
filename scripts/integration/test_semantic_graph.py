#!/usr/bin/env python3
"""Compiler dependencies must participate in CLI file/package cycles and gates.

Requires JDK 17+, .NET SDK 8+ and a built CLI; no project builds or restores.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/archguard").resolve())
    for language, extension in [("java", "java"), ("cs", "cs")]:
        with tempfile.TemporaryDirectory(prefix="archguard-semantic-graph-") as directory:
            root = Path(directory)
            for package in ("a", "b"):
                (root / "src" / package).mkdir(parents=True)
            (root / "archguard.toml").write_text(
                "derive_layers = false\n[project]\nsource_roots = ['src']\n"
                f"languages = ['{language}']\n[modules]\ndepth = 0\n"
                "[rules]\ndeny_cycles = 'file'\nmax_scc_size = 100\n"
            )
            if language == "java":
                (root / "pom.xml").write_text("<project><properties><maven.compiler.release>17</maven.compiler.release></properties></project>")
                first = "package a; public class A { public static void run() { b.B.run(); } }\n"
                second = "package b; public class B { public static void run() { a.A.run(); } }\n"
            else:
                (root / "Example.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup><Target Name="Forbidden" BeforeTargets="Build"><Error Text="Never execute project builds" /></Target></Project>')
                first = "namespace a; public class A { public static void run() { b.B.run(); } }\n"
                second = "namespace b; public class B { public static void run() { a.A.run(); } }\n"
            (root / f"src/a/A.{extension}").write_text(first)
            source_b = root / f"src/b/B.{extension}"
            source_b.write_text(second)

            def run(*args, code=0):
                result = subprocess.run([binary, args[0], str(root), *args[1:]], capture_output=True, text=True, timeout=90)
                assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
                return json.loads(result.stdout)

            calls = run("calls", "--format", "json")
            assert calls["status"] == "ready", calls
            assert sum(c["resolution"] == "project" for c in calls["calls"]) >= 2
            # Fully qualified calls need no import directives. The import-only
            # pipeline misses this actual cycle; this assertion is the regression.
            file_report = run("cycles", "--level", "file", "--format", "json")
            cycles = [f for f in file_report["findings"] if f["kind"] == "cycle"]
            assert any(set(f["nodes"]) == {f"a/A.{extension}", f"b/B.{extension}"} for f in cycles), file_report
            assert file_report["semantic"]["status"] == "ready"
            assert file_report["semantic"]["projected_calls"] >= 2
            package_report = run("cycles", "--format", "json")
            assert any(set(f["nodes"]) == {"a", "b"} for f in package_report["findings"]), package_report
            run("check", "--format", "json", code=1)
            run("cycles", "--level", "file", "--format", "json", "--fail-on-found", code=1)
            run("scan", "--format", "json")
            graph = json.loads((root / ".archguard/graph.json").read_text())
            assert len(graph["edges"]) == 2, graph
            assert all(any(r["raw"].startswith("[semantic call]") and r["line"] == 1 for r in e["refs"]) for e in graph["edges"])
            assert graph["semantic"] == file_report["semantic"]
            # A warm import cache still has to receive the compiler projection.
            assert run("cycles", "--level", "file", "--format", "json") == file_report
            def git(*args):
                subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True, text=True, timeout=30)

            git("init", "--quiet", "--template=")
            git("add", "src", "archguard.toml", "pom.xml" if language == "java" else "Example.csproj")
            git("-c", "user.name=ArchGuard test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "Compiler-only cycle fixture")
            unchanged = run("check", "--base", "HEAD", "--format", "json")
            assert unchanged["comparison"]["new_findings"] == [], unchanged
            assert unchanged["comparison"]["resolved_findings"] == [], unchanged
            assert unchanged["comparison"]["unchanged_findings"] >= 1, unchanged
            source_b.write_text(second.replace("a.A.run();", ""))
            cleared = run("cycles", "--level", "file", "--format", "json")
            assert not cleared["findings"], cleared
            assert cleared["semantic"]["status"] == "ready"
            assert cleared["semantic"]["projected_calls"] == 1
            run("check", "--format", "json")
            diff = run("check", "--base", "HEAD", "--format", "json")["comparison"]
            assert diff["new_findings"] == [] and len(diff["resolved_findings"]) >= 1, diff
            assert not list(root.rglob("obj")) and not list(root.rglob("bin"))
            assert not list(root.rglob("*.class")) and not list(root.rglob("*.dll"))
            print(f"{language}: compiler-only file and package cycle, gates, evidence, warm cache and body edit passed")


if __name__ == "__main__":
    main()
