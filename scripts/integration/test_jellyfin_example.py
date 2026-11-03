"""Verify the pinned, ignored upstream checkout; no download or build of Jellyfin.

Run explicitly after following examples/jellyfin.md. This integration check is
not in offline CI because upstream sources are deliberately not vendored.
"""
import json
import pathlib
import re
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKOUT = ROOT / "examples/checkouts/jellyfin"
CONFIG = ROOT / "examples/jellyfin.toml"
PIN = "1fbd8739292cce610231be93daf43368733edf63"


def run(*args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True)


def main():
    assert CONFIG.is_file(), "The Jellyfin example needs its own production-source configuration"
    assert run("git", "-C", str(CHECKOUT), "rev-parse", "HEAD").stdout.strip() == PIN
    assert "GNU GENERAL PUBLIC LICENSE" in (CHECKOUT / "LICENSE").read_text()
    sources = [path for path in CHECKOUT.rglob("*.cs") if path.relative_to(CHECKOUT).parts[0] not in {"tests", "fuzz"} and path.name not in {"AssemblyInfo.cs", "SharedVersion.cs"}]
    assert len(sources) == 1767, "Pinned production C# source coverage changed"

    with tempfile.TemporaryDirectory(prefix="archguard-jellyfin-") as temporary:
        output = pathlib.Path(temporary) / "graph.json"
        result = run(str(ROOT / "target/debug/archguard"), "scan", str(CHECKOUT), "--config", str(CONFIG), "--level", "file", "--out", str(output), "--format", "json")
        graph = json.loads(output.read_text())
        report = json.loads(result.stdout)

    verify_graph(graph, report, len(sources))


def verify_graph(graph, report, source_count=1767):
    """Verify saved scan output too, without compiling or rescanning the project."""

    files = [node for node in graph["nodes"] if node["kind"] == "file" and node.get("lang") == "cs"]
    assert len(files) == source_count, "Every production C# source must be present"
    assert all(not node["name"].startswith(("tests/", "fuzz/")) for node in files)
    names = {node["name"] for node in files}
    assert "MediaBrowser/Controller/Entities/BaseItem.cs" in names, "Block namespaces must not fall back to dotted directory names"
    assert "Jellyfin/Api/Controllers/MoviesController.cs" in names
    assert {node["name"] for node in graph["nodes"] if node["parent"] is None} == {"Emby", "Jellyfin", "MediaBrowser"}, "Build metadata must not create directory-based architecture roots"
    assert len(graph["edges"]) > 1000, "The real app must expose a substantial dependency graph"
    references = {"import": 0, "call": 0, "supertype": 0}
    source_cache = {}
    by_id = {node["id"]: node for node in graph["nodes"]}
    for edge in graph["edges"]:
        for ref in edge["refs"]:
            path = CHECKOUT / ref["file"]
            assert path.is_file(), f"Missing source evidence: {path}"
            if path not in source_cache:
                source_cache[path] = path.read_text().splitlines(keepends=True)
            lines = source_cache[path]
            assert 1 <= ref["line"] <= len(lines), ref
            raw = ref["raw"]
            if raw.startswith("[semantic call] column "):
                column, code = raw[len("[semantic call] column "):].split(": ", 1)
                # Roslyn columns are UTF-16 offsets; the expression may span
                # multiple lines and is capped at 1,000 characters by the adapter.
                prefix = lines[ref["line"] - 1].encode("utf-16-le")[:(int(column) - 1) * 2].decode("utf-16-le")
                assert "".join(lines[ref["line"] - 1:])[len(prefix):].startswith(code), ref
                references["call"] += 1
            elif raw.startswith("[semantic supertype] "):
                declared, parent = raw[len("[semantic supertype] "):].split(" -> ", 1)
                assert parent, ref
                short_name = declared.rsplit(".", 1)[-1].split("<", 1)[0]
                # The declaration's first line can be an attribute before the
                # class/interface name; this is declaration evidence, not an
                # invented exact source line for the inheritance syntax.
                declaration = "".join(lines[ref["line"] - 1:ref["line"] + 15])
                assert re.search(r"\b(?:class|interface|struct|record|enum)\s+" + re.escape(short_name) + r"\b", declaration), ref
                references["supertype"] += 1
            else:
                assert lines[ref["line"] - 1].rstrip("\r\n") == raw, ref
                references["import"] += 1
            if raw.startswith("[semantic "):
                assert all(by_id[edge[key]]["kind"] == "file" and not by_id[edge[key]]["name"].endswith("/*") for key in ("from", "to")), edge
            assert path.relative_to(CHECKOUT).parts[0] not in {"tests", "fuzz"}
    assert references["import"] == 4942, "Pinned import evidence must remain intact"
    coverage = graph["semantic"]
    assert coverage["status"] in {"ready", "partial"}, "This semantic integration check requires installed .NET SDK 8+"
    assert references["call"] == coverage["projected_calls"] > 0
    assert references["supertype"] == coverage["projected_supertypes"] > 0
    cycle_pair = {"Emby/Naming/Video/ExtraRuleResolver.cs", "Emby/Naming/Video/VideoResolver.cs"}
    cycle_edges = [edge for edge in graph["edges"] if {by_id[edge[key]]["name"] for key in ("from", "to")} == cycle_pair]
    assert len(cycle_edges) == 2, "Resolved calls must expose the previously missing file cycle"
    assert {ref["line"] for edge in cycle_edges for ref in edge["refs"]} == {28, 81}
    assert any(finding["kind"] == "cycle" and cycle_pair.issubset(finding["nodes"]) for finding in report["findings"])
    assert report["summary"]["sccs"] > 0
    print(f"Jellyfin: {len(files)} C# sources, {len(graph['edges'])} directed edges, {references} verified source references, {report['summary']['sccs']} file SCCs; semantic coverage {coverage['status']}.")


if __name__ == "__main__":
    main()
