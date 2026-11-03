#!/usr/bin/env python3
"""Exercise targeted, bounded change context over actual MCP stdio.

Usage: python3 scripts/integration/test_change_context.py target/debug/archguard [--java]
Uses temporary TypeScript and Git fixtures; --java additionally requires JDK 17+.
No network access, project build or browser is needed.
"""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import select
import subprocess
import tempfile


@contextmanager
def mcp(binary, root, cwd, environment):
    """Use the same JSON-RPC transport for import and compiler-backed fixtures."""
    process = subprocess.Popen(
        [str(binary), "mcp", "--root", str(root)], cwd=cwd, env=environment,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1,
    )
    request_id = 0

    def send(value):
        process.stdin.write(json.dumps({"jsonrpc": "2.0", **value}) + "\n")
        process.stdin.flush()

    def rpc(method, params):
        nonlocal request_id
        request_id += 1
        send({"id": request_id, "method": method, "params": params})
        assert select.select([process.stdout], [], [], 60)[0], "MCP response timeout"
        response = json.loads(process.stdout.readline())
        assert response.get("id") == request_id, response
        return response

    def call(arguments):
        return rpc("tools/call", {"name": "change_context", "arguments": arguments})

    def context(**arguments):
        response = call(arguments)
        assert "error" not in response, response
        assert not response["result"].get("isError"), response
        text = response["result"]["content"][0]["text"]
        result = json.loads(text)
        used = len(text.encode("utf-8"))
        maximum = arguments.get("max_bytes", 4096)
        assert used <= maximum, (used, maximum, result)
        assert result["budget"] == {"max_bytes": maximum, "used_bytes": used}, result
        assert re.fullmatch(r"[a-f0-9]{64}", result["revision"]), result
        assert result["semantic_status"] in {"ready", "partial", "unavailable", "not_applicable", "unknown"}, result
        assert result["returned"] == len(result["items"]), result
        return result

    def rejected(**arguments):
        response = call(arguments)
        assert response.get("error", {}).get("code") == -32602, response
        return response

    try:
        initialized = rpc("initialize", {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "archguard-change-context-test", "version": "1"},
        })
        assert "result" in initialized, initialized
        send({"method": "notifications/initialized"})
        names = {item["name"] for item in rpc("tools/list", {})["result"]["tools"]}
        assert "change_context" in names, "change_context must be advertised over MCP stdio"
        yield context, rejected
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", type=Path, default=Path("target/debug/archguard"))
    parser.add_argument("--java", action="store_true", help="also exercise real Java compiler symbols")
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="archguard-change-context-") as directory:
        base = Path(directory)
        root, outside = base / "project", base / "outside"
        root.mkdir()
        outside.mkdir()
        sentinel = outside / "secret.ts"
        sentinel.write_text("export const secret = 'outside sentinel';\n")
        sources = {
            "src/api/entry.ts": 'import { core } from "../core/core";\nexport const entry = core;\n',
            "src/api/secondLevel.ts": 'import { entry } from "./entry";\nexport const start = entry;\n',
            "src/core/core.ts": 'import { leaf } from "../shared/leaf";\nexport const core = leaf + 1;\n',
            "src/shared/leaf.ts": "export const leaf = 2;\n",
            "src/unrelated/outsider.ts": "export const unrelated = 3;\n",
        }
        for name, content in sources.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        config = (
            'derive_layers = false\n[project]\nsource_roots = ["src"]\nlanguages = ["ts"]\n'
            '[modules]\ndepth = 1\n[rules]\ndeny_cycles = "file"\nmax_scc_size = 100\n'
            '[[dependency_rules]]\nname = "Core cannot depend on API"\n'
            'from = ["core/**"]\nto = ["api/**"]\nseverity = "error"\n'
            'message = "Keep the core independently reusable"\n'
            '[[dependency_rules]]\nname = "Unrelated rule"\n'
            'from = ["elsewhere/**"]\nto = ["other/**"]\nseverity = "warning"\n'
            '[[architecture.goals]]\nname = "Independent core"\npriority = "high"\n'
            'rationale = "Core consumers can release separately"\n'
        )
        (root / "archguard.toml").write_text(config)
        (root / ".gitignore").write_text(".archguard/\n")
        (root / "README.md").write_text("Baseline documentation\n")
        environment = {
            **{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "ArchGuard context test",
            "GIT_AUTHOR_EMAIL": "context@example.invalid",
            "GIT_COMMITTER_NAME": "ArchGuard context test",
            "GIT_COMMITTER_EMAIL": "context@example.invalid",
        }

        def git(*arguments):
            return subprocess.run(
                ["git", *arguments], cwd=root, env=environment,
                capture_output=True, check=True, timeout=30,
            ).stdout

        git("init", "-b", "main", "--template=")
        git("config", "--local", "commit.gpgsign", "false")
        git("add", "src", "archguard.toml", ".gitignore", "README.md")
        git("commit", "-m", "Change context fixture")
        commit = git("rev-parse", "HEAD").decode().strip()

        with mcp(binary, root, outside, environment) as (context, rejected):
            request = {"targets": ["src/core/core.ts"], "max_bytes": 16384}
            first = context(**request)
            assert first["next_offset"] is None, first
            assert context(**request) == first, "unchanged inputs must yield a stable packet"
            items = first["items"]
            assert {(item["kind"], item.get("node")) for item in items if "node" in item} == {
                ("seed", "core/core.ts"),
                ("dependent", "api/entry.ts"),
                ("dependency", "shared/leaf.ts"),
            }, items
            encoded = json.dumps(items)
            assert "outsider.ts" not in encoded and "secondLevel.ts" not in encoded, items
            assert "Core cannot depend on API" in encoded, items
            assert "Unrelated rule" not in encoded, items
            assert "Independent core" in encoded, items
            assert first["coverage"]["semantic_status"] != "ready", first

            entry = next(item for item in items if item.get("node") == "api/entry.ts")
            assert entry["file"] == "src/api/entry.ts" and entry["depth"] == 1, entry
            assert [(edge["from"], edge["to"]) for edge in entry["path"]] == [
                ("api/entry.ts", "core/core.ts")
            ], entry
            evidence = entry["path"][0]["evidence"]
            assert any(ref["file"] == "src/api/entry.ts" and ref["line"] == 1
                       and 'import { core }' in ref["raw"] for ref in evidence), entry
            depth_zero = context(**request, depth=0)
            assert {item["kind"] for item in depth_zero["items"]} <= {"seed", "policy", "cycle", "coverage"}, depth_zero
            depth_two = context(**request, depth=2)
            farther = next(item for item in depth_two["items"] if item.get("node") == "api/secondLevel.ts")
            assert farther["kind"] == "dependent" and farther["depth"] == 2, farther
            assert [(edge["from"], edge["to"]) for edge in farther["path"]] == [
                ("api/secondLevel.ts", "api/entry.ts"), ("api/entry.ts", "core/core.ts")
            ], farther
            assert "outsider.ts" not in json.dumps(depth_two["items"]), depth_two
            logical = context(targets=["core/core.ts"], max_bytes=16384)
            assert logical["items"] == items, logical
            default_budget = context(targets=request["targets"], expected_revision=first["revision"])
            assert default_budget["revision"] == first["revision"], default_budget
            rejected(**request, depth=2, expected_revision=first["revision"])
            rejected(targets=["src/shared/leaf.ts"], expected_revision=first["revision"])

            # Exhausting pages at a fixed revision must reproduce the complete
            # item list without duplicate or missing identities.
            paged, offset = [], 0
            while True:
                page = context(**request, offset=offset, limit=1, expected_revision=first["revision"])
                assert page["omitted_oversized"] == 0 and page["returned"] == 1, page
                paged.extend(page["items"])
                if page["next_offset"] is None:
                    break
                assert page["next_offset"] > offset, page
                offset = page["next_offset"]
            assert paged == items, (paged, items)

            # Same-length body changes with preserved mtime still invalidate the
            # revision, even when no dependency edge changes.
            source = root / "src/core/core.ts"
            previous_stat = source.stat()
            source.write_text(sources["src/core/core.ts"].replace("leaf + 1", "leaf + 9"))
            os.utime(source, ns=(previous_stat.st_atime_ns, previous_stat.st_mtime_ns))
            changed = context(**request)
            assert changed["revision"] != first["revision"], changed
            assert changed["scope"] == first["scope"], (changed, first)
            rejected(**request, offset=1, expected_revision=first["revision"])
            (root / "archguard.toml").write_text(config.replace("Independent core", "Independent releases"))
            configured = context(**request)
            assert configured["revision"] != changed["revision"], configured
            assert "Independent releases" in json.dumps(configured["items"]), configured

            # A multibyte evidence row that cannot fit is explicitly omitted.
            # Cursor progress must continue past it, and metadata must fit too.
            (root / "src/api/entry.ts").write_text(sources["src/api/entry.ts"].replace(";\n", "; // " + "🙂" * 4000 + "\n", 1))
            offset, omitted, small_items = 0, 0, []
            for _ in range(100):
                page = context(targets=["src/core/core.ts"], max_bytes=1024, offset=offset)
                omitted += page["omitted_oversized"]
                small_items.extend(page["items"])
                assert all(row["reason"] == "item_exceeds_byte_budget" for row in page["omissions"]), page
                if page["next_offset"] is None:
                    break
                assert page["next_offset"] > offset, page
                offset = page["next_offset"]
            else:
                raise AssertionError("bounded context pages failed to finish")
            assert omitted > 0, "oversized evidence must be explicitly counted"
            assert all("🙂" not in json.dumps(item, ensure_ascii=False) for item in small_items)

            for invalid in ({}, {"targets": ["src/core/core.ts"], "depth": 4},
                            {"targets": ["src/core/core.ts"], "max_bytes": 1023},
                            {"targets": ["src/core/core.ts"], "max_bytes": 16385},
                            {"targets": ["src/core/core.ts"], "limit": 0}):
                rejected(**invalid)
            for path in ("../outside", str(outside)):
                rejected(path=path, targets=["secret.ts"])
            for target in ("../outside/secret.ts", str(sentinel)):
                rejected(targets=[target])
            (root / "outside-link").symlink_to(outside, target_is_directory=True)
            rejected(path="outside-link", targets=["secret.ts"])
            assert sentinel.read_text() == "export const secret = 'outside sentinel';\n"

            # Git selection includes staged, unstaged, deleted and untracked
            # names. Removed files and documentation remain explicit omissions.
            for name, content in sources.items():
                (root / name).write_text(content)
            (root / "archguard.toml").write_text(config)
            (root / "src/api/entry.ts").write_text(sources["src/api/entry.ts"] + "// staged\n")
            git("add", "src/api/entry.ts")
            (root / "src/core/core.ts").write_text(sources["src/core/core.ts"] + "// unstaged\n")
            (root / "src/shared/leaf.ts").unlink()
            (root / "src/new.ts").write_text("export const fresh = 4;\n")
            (root / "README.md").write_text("Changed documentation\n")
            before = git("status", "--porcelain=v1", "-z")
            diff = context(base="HEAD", depth=0, max_bytes=16384)
            assert diff["selection"]["base_commit"] == commit, diff
            selected = {item.get("file") for item in diff["items"] if item["kind"] == "seed"}
            assert {"src/api/entry.ts", "src/core/core.ts", "src/new.ts"} <= selected, diff
            unresolved = json.dumps([item for item in diff["items"] if item["kind"] == "unresolved_target"])
            assert "src/shared/leaf.ts" in unresolved and "README.md" in unresolved, diff
            assert git("status", "--porcelain=v1", "-z") == before, "context must not mutate the Git index or worktree"
    print("change_context MCP: targeted witnesses/policy, depth, byte budgets, stable revisions, stale pagination, root guards and Git selection passed")
    if args.java:
        java_symbols(binary, environment)


def java_symbols(binary, environment):
    with tempfile.TemporaryDirectory(prefix="archguard-change-context-java-") as directory:
        root = Path(directory)
        (root / "src/physical").mkdir(parents=True)
        (root / "src/client").mkdir(parents=True)
        (root / "src/shared").mkdir(parents=True)
        (root / "archguard.toml").write_text(
            "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['java']\n"
            "[rules]\ndeny_cycles='file'\nmax_scc_size=100\n"
        )
        (root / "pom.xml").write_text(
            "<project><properties><maven.compiler.release>17</maven.compiler.release></properties></project>\n"
        )
        # Namespace and physical directories deliberately differ, and every
        # dependency is a fully qualified call with no import statement.
        (root / "src/physical/Service.java").write_text(
            "package domain;\npublic class Service {\n"
            "  public static void run() { shared.Leaf.touch(); }\n"
            "  public static void overloaded() {}\n"
            "  public static void overloaded(int value) {}\n}\n"
        )
        (root / "src/client/Caller.java").write_text(
            "package client;\npublic class Caller { public static void start() { domain.Service.run(); } }\n"
        )
        (root / "src/shared/Leaf.java").write_text(
            "package shared;\npublic class Leaf { public static void touch() {} }\n"
        )
        (root / "src/Plain.java").write_text("public class Plain { public static void local() {} }\n")
        with mcp(binary, root, root, environment) as (context, rejected):
            packet = context(symbols=["domain.Service.run"], max_bytes=16384)
            assert packet["coverage"]["semantic_status"] == "ready", packet
            symbol = next(item for item in packet["items"] if item["kind"] == "symbol")
            assert symbol["id"] == "domain.Service#run()", symbol
            assert symbol["file"] == "src/physical/Service.java" and symbol["line"] == 3, symbol
            seeds = [item for item in packet["items"] if item["kind"] == "seed"]
            assert len(seeds) == 1 and seeds[0]["node"] == "domain/Service.java", seeds
            dependent = next(item for item in packet["items"] if item.get("node") == "client/Caller.java")
            assert dependent["kind"] == "dependent", dependent
            assert any(ref["file"] == "src/client/Caller.java" and ref["line"] == 2
                       and "[semantic call]" in ref["raw"]
                       for edge in dependent["path"] for ref in edge["evidence"]), dependent
            assert any(item.get("node") == "shared/Leaf.java" and item["kind"] == "dependency"
                       for item in packet["items"]), packet
            exact = context(symbols=[symbol["id"]], max_bytes=16384)
            assert exact["items"] == packet["items"], exact
            physical = context(targets=["src/physical/Service.java"], max_bytes=16384)
            logical = context(targets=["domain/Service.java"], max_bytes=16384)
            assert physical["items"] == logical["items"], (physical, logical)
            assert "ambiguous" in json.dumps(rejected(symbols=["domain.Service.overloaded"])).lower()
            overloaded = context(symbols=["domain.Service#overloaded(int)"], max_bytes=16384)
            assert any(item.get("id") == "domain.Service#overloaded(int)" for item in overloaded["items"]), overloaded
            plain = context(symbols=["Plain.local"], max_bytes=16384)
            assert any(item["kind"] == "symbol" and item["file"] == "src/Plain.java"
                       for item in plain["items"]), plain
            assert not list(root.rglob("*.class")), "semantic context must not build the source project"
    print("change_context Java MCP: qualified names, exact compiler IDs, overload ambiguity, physical/namespace mapping and semantic caller evidence passed")


if __name__ == "__main__":
    main()
