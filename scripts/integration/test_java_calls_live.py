#!/usr/bin/env python3
"""End-to-end Java Calls exports and invalidation; requires JDK 17+ and built CLI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


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

        def cli(command):
            result = subprocess.run([binary, command, str(root), "--format", "json"], capture_output=True, text=True, timeout=120, check=True)
            return json.loads(result.stdout)

        def read():
            return {"calls": cli("calls"), **cli("graph")}

        before = read()
        assert before["calls"]["status"] == "ready", before["calls"]
        assert before["calls"]["java_release"] == 8, "initial calls use the Maven compiler release"
        assert before["calls"]["files"] == 2, "source roots and exclusions apply to Calls too"
        assert any(c["to_method"] == "p.B#first()" for c in before["calls"]["calls"]), before["calls"]
        stat = source.stat()
        source.write_text(source.read_text().replace("b.first()", "b.other()"))
        os.utime(source, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        assert source.stat().st_size == stat.st_size, "exercise same-size, same-mtime source edits"
        after = read()
        assert after["nodes"] == before["nodes"]
        assert [(e["from"], e["to"]) for e in after["edges"]] == [(e["from"], e["to"]) for e in before["edges"]]
        assert before["semantic"]["projected_calls"] == after["semantic"]["projected_calls"] == 1
        assert any("[semantic call]" in r["raw"] and "b.first()" in r["raw"] for e in before["edges"] for r in e["refs"])
        assert any("[semantic call]" in r["raw"] and "b.other()" in r["raw"] for e in after["edges"] for r in e["refs"])
        assert not any("b.first()" in r["raw"] for e in after["edges"] for r in e["refs"])
        assert any(c["to_method"] == "p.B#other()" for c in after["calls"]["calls"]), "method-body edits refresh Calls without changed imports"
        assert not any(c["to_method"] == "p.B#first()" for c in after["calls"]["calls"])
        assert after["calls"]["java_release"] == 8
        source_bytes = {path: path.read_bytes() for path in root.rglob("*.java")}
        pom.write_text(pom.read_text().replace("<maven.compiler.release>8<", "<maven.compiler.release>11<"))
        context_updated = read()
        assert context_updated["calls"]["status"] == "ready", context_updated["calls"]
        assert context_updated["calls"]["java_release"] == 11, "Maven context changes refresh Calls without source edits"
        assert context_updated["nodes"] == after["nodes"] and context_updated["edges"] == after["edges"]
        assert context_updated["calls"]["calls"] == after["calls"]["calls"], "context refresh preserves unchanged call evidence"
        assert {path: path.read_bytes() for path in root.rglob("*.java")} == source_bytes
        assert not list(root.rglob("*.class")), "analysis must not generate project bytecode"
        assert (root / ".archguard/reports/graph.json").is_file(), "exports stay in the project artifact directory"
        print("Java Calls live: source exclusions, content and Maven context invalidation without import changes, graph/calls exports and no generated source artifacts passed")


if __name__ == "__main__":
    main()
