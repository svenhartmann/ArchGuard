#!/usr/bin/env python3
"""Exercise project-qualified workspace CLI analysis with a real Java compiler.

Usage: python3 scripts/integration/test_workspace_live.py [BINARY] [--out DIRECTORY]
       [--project EXISTING_REPOSITORY]

Creates a synthetic Maven cycle and an unrelated duplicate Java type. No Maven
build, test runner, server or browser is started. Existing repositories are only
analyzed; artifacts are written exclusively to --out, never into that repository.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from viewer_payload import payload_from_html


def hashes(root, *, all_files=False):
    ignored = {".git", "node_modules", "target", "build", "dist", "bin", "obj", "vendor", ".venv", "venv", "__pycache__"}
    extensions = {".java", ".cs", ".fs", ".vb", ".ts", ".tsx", ".js", ".jsx", ".rs", ".py", ".php", ".kt", ".toml", ".csproj", ".fsproj", ".vbproj"}
    names = {"pom.xml", "package.json", "tsconfig.json", "compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml", "Directory.Build.props", "Directory.Build.targets", "Directory.Packages.props"}
    result = {}
    for directory, children, files in os.walk(root, followlinks=False):
        children[:] = sorted(name for name in children if name not in ignored and not (Path(directory) / name).is_symlink())
        for name in sorted(files):
            path = Path(directory) / name
            if path.is_symlink() or not path.is_file():
                continue
            if all_files or path.suffix in extensions or name in names:
                result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", type=Path, default=Path("target/debug/archguard"))
    parser.add_argument("--out", type=Path)
    parser.add_argument("--project", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update({"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})
    output = args.out.resolve() if args.out else None
    if args.project and output and (output == args.project.resolve() or args.project.resolve() in output.parents):
        raise SystemExit("--out must be outside the existing repository")

    def cli(command, root, *options, html=False):
        process = subprocess.run([str(binary), command, str(root), *map(str, options), "--format", "html" if html else "json"], capture_output=True, text=True, timeout=240, env=environment)
        assert process.returncode == 0, (command, options, process.returncode, process.stderr, process.stdout[:1000])
        return process.stdout if html else json.loads(process.stdout)

    with tempfile.TemporaryDirectory(prefix="archguard-workspace-live-") as temporary:
        root = Path(temporary)
        inputs = {}

        def put(relative, content):
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            inputs[relative] = content

        for name, dependency in [("app", "lib"), ("lib", "app"), ("unrelated", None)]:
            dependencies = f"<dependencies><dependency><groupId>synthetic</groupId><artifactId>{dependency}</artifactId><version>1</version></dependency></dependencies>" if dependency else ""
            put(f"{name}/pom.xml", f"<project><modelVersion>4.0.0</modelVersion><groupId>synthetic</groupId><artifactId>{name}</artifactId><version>1</version><properties><maven.compiler.release>17</maven.compiler.release></properties>{dependencies}</project>\n")
            put(f"{name}/archguard.toml", "[project]\nsource_roots=['src/main/java']\nlanguages=['java']\nexclude=[]\n[semantic.java]\nrelease=17\n")
        put("app/src/main/java/app/Main.java", "package app; public class Main { public int run() { return lib.Shared.value(); } public static int base() { return 1; } }\n")
        put("lib/src/main/java/lib/Shared.java", "package lib; public class Shared { public static int value() { return app.Main.base(); } }\n")
        put("unrelated/src/main/java/lib/Shared.java", "package lib; public class Shared { public static int value() { return 2; } }\n")
        for project, source in [("app", "src/main/java/app/Main.java"), ("lib", "src/main/java/lib/Shared.java"), ("unrelated", None)]:
            put(f"{project}/junit.xml", "<testsuite name='synthetic-unit'><testcase name='same test name' time='0.25'/></testsuite>\n")
            options = ["--report", "junit.xml"]
            if source:
                put(f"{project}/test-map.json", json.dumps({"schema_version": 1, "tests": [{"name": "same test name", "sources": [{"path": source}]}]}) + "\n")
                options += ["--mapping", "test-map.json"]
            cli("test-evidence", root / project, *options)

        before = hashes(root, all_files=True)
        target = "lib/src/main/java/lib/Shared.java"
        report = cli("workspace", root, "--sources", "--target", target)
        workspace, sources, impact = report["workspace"], report["sources"], report["impact"]
        app, lib, unrelated = [f"project:{name}/pom.xml" for name in ("app", "lib", "unrelated")]
        assert len(workspace["projects"]) == 3
        assert {(edge["from"], edge["to"]) for edge in workspace["dependencies"]} == {(app, lib), (lib, app)}, workspace
        assert any(set(group) == {app, lib} for group in workspace["cycles"]), workspace["cycles"]
        assert len(sources["files"]) == 3 and len({file["id"] for file in sources["files"]}) == 3
        duplicate = [file for file in sources["files"] if file["name"] == "lib/Shared.java"]
        assert len(duplicate) == 2 and duplicate[0]["id"] != duplicate[1]["id"], duplicate
        owners = {file["id"]: file["project"] for file in sources["files"]}
        nodes = {node["id"]: node["name"] for node in sources["graph"]["nodes"]}
        source_pairs = {(owners.get(nodes[edge["from"]]), owners.get(nodes[edge["to"]])) for edge in sources["graph"]["edges"]}
        assert {(app, lib), (lib, app)} <= source_pairs, ("Requires an installed JDK supporting release 17", sources["diagnostics"], sources["calls"]["diagnostics"])
        assert all(unrelated not in pair for pair in source_pairs), source_pairs
        classes = {row["id"]: owners.get(row["graph_node"]) for row in sources["calls"]["classes"]}
        cross_calls = [(classes[row["from_class"]], classes[row["to_class"]]) for row in sources["calls"]["calls"] if classes.get(row["from_class"]) and classes.get(row.get("to_class")) and classes[row["from_class"]] != classes[row["to_class"]]]
        assert {(app, lib), (lib, app)} <= set(cross_calls), sources["calls"]
        assert all(unrelated not in pair for pair in cross_calls), cross_calls
        assert any({owners[node] for node in group} == {app, lib} for group in sources["cycles"]), sources["cycles"]
        assert {test["project"] for test in impact["selected"]} == {app, lib}, impact
        assert impact["unknown_tests"] == 1 and impact["safe_to_skip_other_tests"] is False, impact
        assert len({test["test_id"] for test in impact["selected"]}) == 2, impact
        assert {test["reason"] for test in impact["selected"]} == {"mapped_changed_source", "mapped_static_dependent"}, impact

        page = cli("workspace", root, "--sources", "--target", target, html=True)
        payload = payload_from_html(page)
        assert payload["insights"]["workspace"] == workspace
        assert payload["insights"]["workspace_impact"] == impact
        assert "calls" not in payload["insights"]["workspace_sources"], "HTML shares the top-level call payload"
        assert payload["insights"]["workspace_sources"]["call_summary"]["calls"] == len(sources["calls"]["calls"])
        assert 'id="workspace-panel"' in page and "function renderWorkspace()" in page
        assert hashes(root, all_files=True) == before, "Workspace JSON and HTML analysis must not change source, config, or imported snapshots"
        result = {
            "dataset": "Synthetic Maven dependency cycle and unrelated duplicate type; no Maven build performed",
            "projects": len(workspace["projects"]), "declared_dependencies": len(workspace["dependencies"]), "project_cycles": len(workspace["cycles"]),
            "files": len(sources["files"]), "source_dependencies": len(sources["graph"]["edges"]), "source_cycles": len(sources["cycles"]),
            "compiler_calls": len(sources["calls"]["calls"]), "cross_project_calls": len(cross_calls), "source_status": sources["status"],
            "recommended_tests": len(impact["selected"]), "unknown_tests": impact["unknown_tests"], "all_fixture_file_hashes_unchanged": True,
            "html_bytes": len(page.encode("utf-8")), "checks": "Actual built CLI, explicit test imports, installed Java compiler, scoped identities, call evidence, impact and embedded HTML payload; no browser rendering check",
        }
        if output:
            output.mkdir(parents=True, exist_ok=True)
            (output / "example.html").write_text(page, encoding="utf-8")
            (output / "analysis.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            (output / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            for relative, content in inputs.items():
                destination = output / "synthetic-inputs" / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text(content, encoding="utf-8")
            (output / "synthetic-inputs" / "SYNTHETIC.txt").write_text("Deliberate cyclic Maven dependencies for analysis tests. Not a buildable production architecture; no Maven build or project tests were executed.\n", encoding="utf-8")
        print(json.dumps(result, indent=2))

    if args.project:
        root = args.project.resolve()
        before = hashes(root)
        report = cli("workspace", root, "--sources")
        page = cli("workspace", root, "--sources", html=True)
        assert hashes(root) == before, "Existing repository source/config/manifest content changed"
        sources = report["sources"]
        assert len({row["id"] for row in sources["files"]}) == len(sources["files"])
        known_projects = {project["id"] for project in report["workspace"]["projects"]}
        assert all(row["id"] and row["project"] in known_projects for row in sources["files"])
        result = {"root": str(root), "projects": len(report["workspace"]["projects"]), "declared_dependencies": len(report["workspace"]["dependencies"]), "files": len(sources["files"]), "source_status": sources["status"], "source_and_config_files_hashed": len(before), "hashes_unchanged": True}
        if output:
            (output / "real-workspace.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            (output / "real-workspace.html").write_text(page, encoding="utf-8")
            (output / "real-verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
