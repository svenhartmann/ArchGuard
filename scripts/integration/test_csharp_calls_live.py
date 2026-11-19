#!/usr/bin/env python3
"""Real CLI Roslyn integration with invalidation. Requires .NET SDK 8+ and a built CLI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/archguard").resolve())
    with tempfile.TemporaryDirectory(prefix="archguard-csharp-live-") as temporary:
        root = Path(temporary)
        (root / "src").mkdir()
        (root / "src/excluded").mkdir()
        (root / "archguard.toml").write_text("derive_layers = false\n[project]\nsource_roots = ['src']\nlanguages = ['cs']\nexclude = ['src/excluded/**']\n")
        project = root / "Example.csproj"
        project.write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><DefineConstants>FIRST</DefineConstants></PropertyGroup><Target Name="Forbidden" BeforeTargets="Build"><Error Text="Project builds must not run" /></Target></Project>')
        (root / "src/A.cs").write_text("""namespace Example;
public class A {
    public void Run(B service) {
#if FIRST
        service.First();
#else
        service.Other();
#endif
    }
}
""")
        (root / "src/B.cs").write_text("namespace Example;\npublic class B { public void First() {} public void Other() {} }\n")
        (root / "src/excluded/Ignore.cs").write_text("class Ignore : IMissing {}")

        def cli(command):
            result = subprocess.run([binary, command, str(root), "--format", "json"], capture_output=True, text=True, check=True, timeout=90)
            return json.loads(result.stdout)

        def read():
            return {"calls": cli("calls"), **cli("graph")}

        before = read()
        analysis = before["calls"]
        assert analysis["status"] == "ready" and analysis["files"] == 2, analysis
        assert analysis["analyzers"][0]["backend"] == "roslyn"
        assert analysis["analyzers"][0]["language"] == "csharp"
        assert any(c["to_method"].endswith("#First()") for c in analysis["calls"]), analysis
        stat = project.stat()
        project.write_text(project.read_text().replace(">FIRST<", ">OTHER<"))
        os.utime(project, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        assert project.stat().st_size == stat.st_size
        after = read()
        assert after["nodes"] == before["nodes"]
        assert [(e["from"], e["to"]) for e in after["edges"]] == [(e["from"], e["to"]) for e in before["edges"]]
        assert before["semantic"]["projected_calls"] == after["semantic"]["projected_calls"] == 1
        assert any("[semantic call]" in r["raw"] and "service.First()" in r["raw"] for e in before["edges"] for r in e["refs"])
        assert any("[semantic call]" in r["raw"] and "service.Other()" in r["raw"] for e in after["edges"] for r in e["refs"])
        assert not any("service.First()" in r["raw"] for e in after["edges"] for r in e["refs"])
        assert after["calls"]["status"] == "ready"
        assert any(c["to_method"].endswith("#Other()") for c in after["calls"]["calls"]), "project-only DefineConstants edits invalidate the semantic model"
        assert not any(c["to_method"].endswith("#First()") for c in after["calls"]["calls"])
        assert read()["calls"] == after["calls"], "an unchanged project gives the same calls"
        assert not list(root.rglob("obj")) and not list(root.rglob("bin")) and not list(root.rglob("*.dll")), "source projects must not be built/restored"
        assert (root / ".archguard/reports/graph.json").is_file(), "exports stay in the project artifact directory"
        print("C# CLI: real Roslyn, source exclusions, context-only graph evidence invalidation, stable result and no project build passed")


if __name__ == "__main__":
    main()
