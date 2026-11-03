"""Verify a pinned, ignored Kafka checkout and saved ArchGuard analysis; no upstream build."""
import json
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CHECKOUT = ROOT / "examples/checkouts/kafka"
PIN = "13f70256db3c994c590e5d262a7cc50b9e973204"


def verify(directory):
    graph = json.loads((directory / "graph.json").read_text())
    report = json.loads((directory / "scan.json").read_text())
    assert graph["semantic"]["status"] in {"ready", "partial"}, graph["semantic"]["diagnostics"]
    assert subprocess.check_output(["git", "-C", str(CHECKOUT), "rev-parse", "HEAD"], text=True).strip() == PIN
    subprocess.run(["git", "-C", str(CHECKOUT), "diff", "--exit-code", "HEAD", "--"], check=True, capture_output=True)
    config = tomllib.loads((CHECKOUT / ".archguard/archguard.toml").read_text())
    paths = [p for root in config["project"]["source_roots"] for p in (CHECKOUT / root).rglob("*.java")]
    assert len(paths) == 3369
    java = [n for n in graph["nodes"] if n["kind"] == "file" and n.get("lang") == "java"]
    assert len(java) == len(paths), "Every selected Java production file must be included"
    assert not any(n.get("lang") not in {None, "java"} for n in graph["nodes"])
    sources = {}
    for path in paths:
        rel = path.relative_to(CHECKOUT).as_posix()
        lines = path.read_text().splitlines()
        sources[rel] = lines
        for root in config["project"]["source_roots"]:
            if rel.startswith(root + "/"):
                sources.setdefault(rel[len(root) + 1:], lines)
                break
    count = 0
    for edge in graph["edges"]:
        for ref in edge["refs"]:
            if ref["raw"].startswith("[semantic "):
                continue
            assert ref["file"] in sources, ref
            assert sources[ref["file"]][ref["line"] - 1] == ref["raw"], ref
            count += 1
    # Independently count SCCs on real Java files, without ArchGuard's algorithm.
    ids = {n["id"] for n in java}
    outgoing = {i: [] for i in ids}; incoming = {i: [] for i in ids}
    for edge in graph["edges"]:
        a, b = edge["from"], edge["to"]
        if a in ids and b in ids:
            outgoing[a].append(b); incoming[b].append(a)
    seen = set(); order = []
    for root in sorted(ids):
        if root in seen:
            continue
        seen.add(root); stack = [(root, iter(outgoing[root]))]
        while stack:
            node, children = stack[-1]
            child = next(children, None)
            if child is None:
                order.append(node); stack.pop()
            elif child not in seen:
                seen.add(child); stack.append((child, iter(outgoing[child])))
    seen = set(); groups = []
    for root in reversed(order):
        if root in seen:
            continue
        members = []; stack = [root]; seen.add(root)
        while stack:
            node = stack.pop(); members.append(node)
            for parent in incoming[node]:
                if parent not in seen:
                    seen.add(parent); stack.append(parent)
        if len(members) > 1:
            groups.append(members)
    assert len(groups) == report["summary"]["sccs"] > 0
    assert max(map(len, groups)) == report["summary"]["largest_scc"]
    by_id = {n["id"]: n for n in graph["nodes"]}
    pair = {"org/apache/kafka/clients/CommonClientConfigs.java", "org/apache/kafka/common/telemetry/internals/ClientTelemetryReporter.java"}
    evidence = []
    for edge in graph["edges"]:
        if {by_id[edge[k]]["name"] for k in ("from", "to")} == pair:
            evidence.append({"from": by_id[edge["from"]]["name"], "to": by_id[edge["to"]]["name"], "refs": edge["refs"]})
    assert len(evidence) == 2, "The documented direct source cycle must exist in both directions"
    result = {"commit": PIN, "production_java_files": len(java), "edges": len(graph["edges"]), "verified_import_references": count,
              "file_cycle_groups": len(groups), "largest_file_cycle_group": max(map(len, groups)), "semantic_status": graph["semantic"]["status"], "cycle_example": evidence}
    (directory / "verification.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "cycle_example"}))


if __name__ == "__main__":
    verify(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / ".archguard/reports/kafka-world")
