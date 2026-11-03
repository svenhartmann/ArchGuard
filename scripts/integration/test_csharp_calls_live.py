#!/usr/bin/env python3
"""Real CLI/live Roslyn integration. Requires .NET SDK 8+ and a built CLI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from urllib.request import urlopen
from viewer_payload import payload_from_html as payload



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
        cli = subprocess.run([binary, "calls", str(root), "--format", "json"], capture_output=True, text=True, check=True, timeout=90)
        analysis = json.loads(cli.stdout)
        assert analysis["status"] == "ready" and analysis["files"] == 2, analysis
        assert analysis["analyzers"][0]["backend"] == "roslyn"
        assert analysis["analyzers"][0]["language"] == "csharp"
        assert any(c["to_method"].endswith("#First()") for c in analysis["calls"]), analysis
        server = subprocess.Popen([binary, "serve", str(root), "--port", "0", "--depth", "1"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        try:
            url = server.stdout.readline().strip()
            assert url.startswith("http://127.0.0.1:")

            def read(path=""):
                if not path:
                    # Preserve the fresh compiler-evidence assertions while
                    # the browser's ordinary HTML/status routes stay async.
                    read("__archguard/check")
                with urlopen(url + path, timeout=90) as response:
                    assert response.status == 200
                    return response.read().decode()

            before = payload(read())
            assert before["calls"] == analysis
            version = json.loads(read("__archguard/check"))["version"]
            assert json.loads(read("__archguard/check"))["version"] == version
            stat = project.stat()
            project.write_text(project.read_text().replace(">FIRST<", ">OTHER<"))
            os.utime(project, ns=(stat.st_atime_ns, stat.st_mtime_ns))
            assert project.stat().st_size == stat.st_size
            changed = json.loads(read("__archguard/check"))["version"]
            assert changed != version, "project-only DefineConstants edits invalidate the live semantic model"
            after = payload(read())
            assert after["nodes"] == before["nodes"]
            assert [(e["from"], e["to"]) for e in after["edges"]] == [(e["from"], e["to"]) for e in before["edges"]]
            assert before["semantic"]["projected_calls"] == after["semantic"]["projected_calls"] == 1
            assert any("[semantic call]" in r["raw"] and "service.First()" in r["raw"] for e in before["edges"] for r in e["refs"])
            assert any("[semantic call]" in r["raw"] and "service.Other()" in r["raw"] for e in after["edges"] for r in e["refs"])
            assert not any("service.First()" in r["raw"] for e in after["edges"] for r in e["refs"])
            assert after["calls"]["status"] == "ready"
            assert any(c["to_method"].endswith("#Other()") for c in after["calls"]["calls"])
            assert not any(c["to_method"].endswith("#First()") for c in after["calls"]["calls"])
            assert json.loads(read("__archguard/check"))["version"] == changed
        finally:
            server.terminate()
            server.wait(timeout=10)
        assert not list(root.rglob("obj")) and not list(root.rglob("bin")) and not list(root.rglob("*.dll")), "source projects must not be built/restored"
        assert (root / ".archguard/reports/graph.html").is_file()
        assert all(path.parent == root / ".archguard/reports" for path in root.rglob("*.html")), "reports stay in the project artifact directory"
        print("C# CLI/live: real Roslyn, source exclusions, context-only graph evidence invalidation, stable cache and no project build passed")


if __name__ == "__main__":
    main()
