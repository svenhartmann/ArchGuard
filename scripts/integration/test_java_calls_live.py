#!/usr/bin/env python3
"""End-to-end Java Calls exports and live updates; requires JDK 17+ and built CLI."""
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
    with tempfile.TemporaryDirectory(prefix="archguard-calls-live-") as temp:
        root = Path(temp)
        (root / "src/p").mkdir(parents=True)
        (root / "src/excluded").mkdir()
        config = root / "archguard.toml"
        config.write_text("derive_layers = false\n[project]\nsource_roots = ['src']\nlanguages = ['java']\nexclude = ['src/excluded/**']\n")
        pom = root / "pom.xml"
        pom.write_text("<project><properties><maven.compiler.release>8</maven.compiler.release></properties></project>\n")
        source = root / "src/p/A.java"
        source.write_text("package p; public class A { public void run(B b) { b.first(); } }\n")
        (root / "src/p/B.java").write_text("package p; public class B { public void first() {} public void other() {} }\n")
        (root / "src/excluded/Ignore.java").write_text("class Ignore { Missing invalid; }\n")
        server = subprocess.Popen([binary, "serve", str(root), "--port", "0", "--depth", "1"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        try:
            url = server.stdout.readline().strip()
            assert url.startswith("http://127.0.0.1:"), "server must print its loopback URL"

            def read(path=""):
                if not path:
                    # HTML is a non-blocking snapshot; this test needs fresh
                    # compiler evidence after each explicit source/context edit.
                    read("__archguard/check")
                with urlopen(url + path, timeout=60) as response:
                    assert response.status == 200
                    return response.read().decode()

            before = payload(read())
            assert before.get("calls", {}).get("status") == "ready", before.get("calls")
            assert before["calls"]["java_release"] == 8, "initial calls use the Maven compiler release"
            assert before["calls"]["files"] == 2, "source roots and exclusions apply to Calls too"
            assert any(c["to_method"] == "p.B#first()" for c in before["calls"]["calls"]), before["calls"]
            version = json.loads(read("__archguard/check"))["version"]
            assert json.loads(read("__archguard/check"))["version"] == version
            stat = source.stat()
            source.write_text(source.read_text().replace("b.first()", "b.other()"))
            os.utime(source, ns=(stat.st_atime_ns, stat.st_mtime_ns))
            assert source.stat().st_size == stat.st_size, "exercise same-size, same-mtime source edits"
            changed = json.loads(read("__archguard/check"))["version"]
            assert changed != version, "method-body edits must refresh Calls even without changed imports"
            after = payload(read())
            assert after["nodes"] == before["nodes"]
            assert [(e["from"], e["to"]) for e in after["edges"]] == [(e["from"], e["to"]) for e in before["edges"]]
            assert before["semantic"]["projected_calls"] == after["semantic"]["projected_calls"] == 1
            assert any("[semantic call]" in r["raw"] and "b.first()" in r["raw"] for e in before["edges"] for r in e["refs"])
            assert any("[semantic call]" in r["raw"] and "b.other()" in r["raw"] for e in after["edges"] for r in e["refs"])
            assert not any("b.first()" in r["raw"] for e in after["edges"] for r in e["refs"])
            assert any(c["to_method"] == "p.B#other()" for c in after["calls"]["calls"])
            assert not any(c["to_method"] == "p.B#first()" for c in after["calls"]["calls"])
            assert after["calls"]["java_release"] == 8
            source_bytes = {path: path.read_bytes() for path in root.rglob("*.java")}
            pom.write_text(pom.read_text().replace("<maven.compiler.release>8<", "<maven.compiler.release>11<"))
            context_version = json.loads(read("__archguard/check"))["version"]
            assert context_version != changed, "Maven context changes must refresh Calls without source edits"
            context_updated = payload(read())
            assert context_updated["calls"]["status"] == "ready", context_updated["calls"]
            assert context_updated["calls"]["java_release"] == 11, "updated Maven release reaches the live Calls payload"
            assert context_updated["nodes"] == after["nodes"] and context_updated["edges"] == after["edges"]
            assert context_updated["calls"]["calls"] == after["calls"]["calls"], "context refresh preserves unchanged call evidence"
            assert {path: path.read_bytes() for path in root.rglob("*.java")} == source_bytes
            assert json.loads(read("__archguard/check"))["version"] == context_version, "unchanged context has a stable revision"
            assert not list(root.rglob("*.class")), "analysis must not generate project bytecode"
            assert (root / ".archguard/reports/graph.html").is_file()
            assert all(path.parent == root / ".archguard/reports" for path in root.rglob("*.html")), "reports stay in the project artifact directory"
        finally:
            server.terminate()
            server.wait(timeout=10)

        for command in ("graph", "cycles"):
            result = subprocess.run([binary, command, str(root), "--format", "html"], capture_output=True, text=True, timeout=60, check=True)
            exported = payload(result.stdout)
            assert exported["calls"]["status"] == "ready"
            assert exported["calls"]["java_release"] == 11
            assert any(c["to_method"] == "p.B#other()" for c in exported["calls"]["calls"])
        print("Java Calls live: source exclusions, content and Maven context invalidation without import changes, stable version, graph/cycles exports and no generated source artifacts passed")


if __name__ == "__main__":
    main()
