"""Exercise the built CLI and actual MCP stdio transport in temporary roots.

Usage: python3 scripts/integration/smoke_cli.py target/debug/archguard [--db]
The optional DB checks require a binary built with --features db.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--db", action="store_true")
    args = parser.parse_args()
    binary = args.binary.resolve()
    repository = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="archguard-smoke-") as directory:
        base = Path(directory)
        root = base / "project"
        root.mkdir()
        outside = base / "outside"
        outside.mkdir()
        (outside / "sentinel").write_text("unchanged")
        (root / "A.ts").write_text('import { b } from "./B";\nexport const a = 1;\n')
        (root / "B.ts").write_text('import { a } from "./A";\nexport const b = 2;\n')
        (root / "entry").mkdir()
        (root / "entry/index.ts").write_text('import { a } from "../A";\nexport const start = a;\n')
        (root / "CODEOWNERS").write_text('* @example/core\n/entry/ @example/cli\n')
        (root / ".gitignore").write_text('.archguard/\narchitecture.md\n')
        (root / "archguard.toml").write_text(
            'derive_layers = false\n[project]\nlanguages = ["ts"]\n'
            '[modules]\ndepth = 0\n[rules]\ndeny_cycles = "file"\nmax_scc_size = 1\n'
            '[[architecture.goals]]\nname = "Independent releases"\npriority = "high"\n'
            'rationale = "Teams can ship separately"\n'
        )

        # Only the temporary repository receives identity/configuration. Ignore
        # user/system configuration and templates, including signing and hooks.
        git_environment = {
            **{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "ArchGuard smoke", "GIT_AUTHOR_EMAIL": "smoke@example.invalid",
            "GIT_COMMITTER_NAME": "ArchGuard smoke", "GIT_COMMITTER_EMAIL": "smoke@example.invalid",
        }

        def git(*arguments):
            return subprocess.run(
                ["git", *arguments], cwd=root, env=git_environment,
                text=True, capture_output=True, check=True, timeout=30,
            ).stdout.strip()

        git("init", "-b", "main", "--template=")
        git("config", "--local", "user.name", "ArchGuard smoke")
        git("config", "--local", "user.email", "smoke@example.invalid")
        git("config", "--local", "commit.gpgsign", "false")
        git("add", "A.ts", "B.ts", "entry/index.ts", "CODEOWNERS", ".gitignore", "archguard.toml")
        git("commit", "-m", "Smoke baseline")
        commits = [git("rev-parse", "HEAD")]
        for revision in range(2):
            for name in ("A.ts", "B.ts", "entry/index.ts"):
                source = root / name
                source.write_text(source.read_text() + f"// Shared revision {revision}\n")
            git("add", "A.ts", "B.ts", "entry/index.ts")
            git("commit", "-m", f"Shared revision {revision}")
            commits.append(git("rev-parse", "HEAD"))
        commit = git("rev-parse", "HEAD")

        def cli(*arguments, code=0):
            result = subprocess.run(
                [str(binary), *map(str, arguments)], cwd=base,
                env=git_environment, text=True, capture_output=True, timeout=60,
            )
            assert result.returncode == code, (arguments, result.returncode, result.stderr)
            return result.stdout

        report = json.loads(cli("check", root, "--format", "json", code=1))
        assert report["summary"]["status"] == "fail"
        assert any(f["kind"] == "metric_threshold" for f in report["findings"])
        assert (root / ".archguard/cache.json").is_file()
        assert json.loads(cli("check", root, "--format", "json", code=1)) == report
        html = cli("graph", root, "--format", "html")
        assert 'id="archguard-data"' in html and 'id="action-dialog"' in html

        # Execute the real viewer's export handler, then feed its downloaded
        # JSON to the real CLI parser/engine. No browser or source edits needed.
        export = base / "viewer-ops.json"
        environment = {**os.environ, "VIEWER_EXPORT_PATH": str(export)}
        subprocess.run(
            ["node", "scripts/viewer/test_viewer_state.js"], cwd=repository,
            env=environment, check=True, capture_output=True, text=True, timeout=60,
        )
        assert json.loads(export.read_text()) == [{"cut_edge": {"from": "A.ts", "to": "B.ts"}}]
        simulation = json.loads(cli("simulate", root, "--ops", export, "--format", "json"))
        assert simulation["scc_count_before"] == 1, simulation
        assert simulation["scc_count_after"] == 0, simulation

        priorities = json.loads(cli("hotspots", root, "--level", "file", "--top", "3", "--format", "json"))
        assert priorities["git"]["status"] == "available", priorities
        assert priorities["summary"]["mapped_files"] == 3, priorities
        assert {row["name"] for row in priorities["nodes"]} == {"A.ts", "B.ts", "entry/index.ts"}
        assert all(row["changes"] == 3 for row in priorities["nodes"]), priorities
        co_change = json.loads(cli("co-change", root, "--min-shared", "3", "--min-ratio", "1", "--format", "json"))
        assert len(co_change["pairs"]) == 1, co_change
        assert set(co_change["pairs"][0]["evidence_commits"]) == set(commits), co_change
        ownership = json.loads(cli("ownership", root, "--format", "json"))
        assert ownership["complete"] and ownership["owned_files"] == 3, ownership
        assert ownership["cross_owner_dependencies"] == 1, ownership
        performance = json.loads(cli("performance", root, "--format", "json"))
        assert performance["entries"][0]["name"] == "entry/index.ts", performance
        assert performance["entries"][0]["reachable_files"] == 3, performance
        cli("doc", root, "--out", "architecture.md")
        document = (root / "architecture.md").read_text()
        assert document.startswith("# Architecture\n") and "Independent releases" in document
        cli("doc", root, "--out", "architecture.md", "--check")
        assert (root / "architecture.md").read_text() == document
        cli("snapshot", root)
        base_check = json.loads(cli("check", root, "--base", "HEAD", "--fail-on-acd-regression", "0", "--format", "json"))
        assert base_check["findings"] == [], base_check
        assert base_check["comparison"]["base_commit"] == commit, base_check
        assert git("status", "--porcelain") == "", "analysis must not change tracked files"

        process = subprocess.Popen(
            [str(binary), "mcp", "--root", str(root)], cwd=outside,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=git_environment, text=True, bufsize=1,
        )
        request_id = 0

        def send(value):
            process.stdin.write(json.dumps({"jsonrpc": "2.0", **value}) + "\n")
            process.stdin.flush()

        def rpc(method, params):
            nonlocal request_id
            request_id += 1
            send({"id": request_id, "method": method, "params": params})
            assert select.select([process.stdout], [], [], 15)[0], "MCP response timeout"
            response = json.loads(process.stdout.readline())
            assert response.get("id") == request_id, response
            return response

        def tool(name, arguments):
            return rpc("tools/call", {"name": name, "arguments": arguments})

        def value(response):
            assert "error" not in response, response
            assert not response["result"].get("isError"), response
            return json.loads(response["result"]["content"][0]["text"])

        try:
            initialized = rpc("initialize", {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "archguard-smoke", "version": "1"},
            })
            assert "result" in initialized, initialized
            send({"method": "notifications/initialized"})
            names = {item["name"] for item in rpc("tools/list", {})["result"]["tools"]}
            assert len(names) == (47 if args.db else 46), names
            assert {"workspace_context", "review_context", "architecture_patterns", "architecture_findings", "architecture_finding"} <= names, names
            assert {"hotspots", "co_change", "ownership", "performance", "compare",
                    "architecture_context", "architecture_document", "service_topology", "semantic_model", "security_findings", "test_coverage", "quality_findings",
                    "package_cohesion", "quality_history", "quality_metrics", "test_impact", "mutation_findings", "duplication_findings", "api_contracts"} <= names, names
            for name, snapshot in [("test_impact", "tests.json"), ("mutation_findings", "mutation.json"), ("duplication_findings", "duplicates.json"), ("api_contracts", "contracts.json")]:
                evidence = value(tool(name, {"path": "."}))
                assert evidence["status"] == "not_imported" and evidence["total"] == 0, evidence
                assert not (root / ".archguard" / snapshot).exists(), f"{name} must not import evidence"
                assert "error" in tool(name, {"path": "../outside"})
            hypotheses = value(tool("architecture_patterns", {"path": ".", "limit": 1}))
            assert hypotheses["status"] == "assessed" and hypotheses["metadata"]["assessments_total"] >= 6, hypotheses
            assert hypotheses["next_offset"] is not None
            assert "error" in tool("architecture_patterns", {"path": "../outside"})
            assert not (root / ".archguard/patterns.json").exists()
            history = value(tool("quality_history", {"path": "."}))
            assert history["status"] == "not_recorded" and history["total"] == 0, history
            review = value(tool("review_context", {"path": "."}))
            assert review["status"] == "not_recorded" and review["total"] == 0, review
            assert not (root / ".archguard/review.json").exists(), "MCP must not record a review"
            assert "error" in tool("review_context", {"path": "../outside"})
            cohesion = value(tool("package_cohesion", {"path": "."}))
            workspace = value(tool("workspace_context", {"path": "."}))
            assert workspace["metadata"]["sources_requested"] is False, workspace
            assert "error" in tool("workspace_context", {"path": "../outside"})
            assert "error" in tool("workspace_context", {"targets": ["A.ts"]})
            assert cohesion["metadata"]["packages_total"] == 2, cohesion
            assert "error" in tool("quality_history", {"path": "../outside"})
            assert "error" in tool("package_cohesion", {"path": "../outside"})
            quality = value(tool("quality_findings", {"path": "."}))
            assert quality["status"] == "not_imported" and quality["total"] == 0, quality
            assert quality["source_revision_verified"] is False, quality
            assert not (root / ".archguard/quality.json").exists(), "MCP must not create a quality report"
            assert "error" in tool("quality_findings", {"path": "../outside"})
            coverage = value(tool("test_coverage", {"path": "."}))
            assert coverage["status"] == "not_imported" and coverage["total"] == 0, coverage
            assert coverage["source_revision_verified"] is False, coverage
            assert not (root / ".archguard/coverage.json").exists(), "MCP must not create a test report"
            assert "error" in tool("test_coverage", {"path": "../outside"})
            security = value(tool("security_findings", {"path": "."}))
            assert security["status"] == "not_scanned", security
            assert security["findings_total"] == 0 and security["coverage_scope"] == "whole_report", security
            assert not (root / ".archguard/security.json").exists(), "MCP must not start a dependency scan"
            assert "error" in tool("security_findings", {"path": "../outside"})
            semantic = value(tool("semantic_model", {"path": ".", "kind": "calls", "language": "typescript"}))
            assert semantic["status"] == "unavailable" and semantic["total"] == 0, semantic
            assert semantic["analyzers"][0]["language"] == "typescript", semantic
            assert semantic["analyzers"][0]["status"] == "unavailable", semantic
            assert semantic["next_offset"] is None, semantic
            assert "error" in tool("semantic_model", {"path": "../outside"})
            checked = value(tool("check", {"path": "."}))
            assert checked["status"] == report["summary"]["status"]
            assert checked["summary"]["status"] == checked["status"]
            scanned = value(tool("scan", {"path": ".", "level": "file"}))
            assert scanned["edges"] == 3, scanned
            cycle = next(f for f in report["findings"] if f["kind"] == "cycle")
            details = value(tool("cycle_get", {"cycle_id": cycle["id"]}))
            assert details["sampling"]["complete"] is False, details
            assert details["edges"], details
            mcp_priorities = value(tool("hotspots", {"path": ".", "level": "file", "top": 3}))
            assert mcp_priorities["summary"] == priorities["summary"], mcp_priorities
            assert [(row["name"], row["score"]) for row in mcp_priorities["nodes"]] == [
                (row["name"], row["score"]) for row in priorities["nodes"]
            ], mcp_priorities
            mcp_co_change = value(tool("co_change", {"path": ".", "min_shared": 3, "min_ratio": 1}))
            assert mcp_co_change["pairs"] == co_change["pairs"], mcp_co_change
            mcp_ownership = value(tool("ownership", {"path": "."}))
            assert mcp_ownership["complete"] and mcp_ownership["cross_owner_dependencies"] == 1, mcp_ownership
            mcp_performance = value(tool("performance", {"path": "."}))
            assert mcp_performance["entries"] == performance["entries"], mcp_performance
            context = value(tool("architecture_context", {"path": "."}))
            assert context["intent"]["goals"][0]["name"] == "Independent releases", context
            assert context["intent"]["goals"][0]["priority"] == "high", context
            assert context["observed"]["source_files"] == 3, context
            generated = value(tool("architecture_document", {"path": "."}))
            assert generated["markdown"] == document, generated
            assert (root / "architecture.md").read_text() == document, "MCP document generation is read-only"
            compared = value(tool("compare", {"path": ".", "base": "HEAD"}))
            assert compared["new_findings"] == [] and compared["resolved_findings"] == [], compared
            assert compared["base_commit"] == commit and compared["acd_delta"] == 0, compared

            # Explicit imports are separate from read-only MCP requests. Evidence
            # lives in ignored .archguard reports and never changes production code.
            reports = root / ".archguard/reports"
            reports.mkdir(exist_ok=True)
            (reports / "junit.xml").write_text("<testsuite name='unit'><testcase name='b works' time='0.25'/></testsuite>")
            (reports / "mapping.json").write_text(json.dumps({"schema_version": 1, "tests": [{"name": "b works", "sources": [{"path": "B.ts", "lines": [2]}]}]}))
            cli("test-evidence", root, "--report", reports / "junit.xml", "--mapping", reports / "mapping.json")
            affected = value(tool("test_impact", {"path": ".", "targets": ["A.ts"]}))
            assert affected["metadata"]["selected_tests"] == 1, affected
            assert any(item.get("duration_seconds") == 0.25 for item in affected["items"]), affected
            assert affected["metadata"]["safe_to_skip_other_tests"] is False
            (reports / "mutations.json").write_text(json.dumps({"schemaVersion": "2", "thresholds": {"high": 80, "low": 60}, "files": {"B.ts": {"language": "typescript", "source": (root / "B.ts").read_text(), "mutants": [{"id": "b-value", "mutatorName": "NumberLiteral", "status": "Survived", "location": {"start": {"line": 2, "column": 18}, "end": {"line": 2, "column": 19}}}]}}}))
            cli("mutations", root, "--report", reports / "mutations.json")
            mutations = value(tool("mutation_findings", {"path": "."}))
            assert mutations["status"] == "ready" and mutations["metadata"]["summary"]["survived"] == 1, mutations
            (reports / "cpd.xml").write_text("<pmd-cpd><duplication lines='1' tokens='8'><file path='A.ts' line='1'/><file path='B.ts' line='1'/><codefragment>import</codefragment></duplication></pmd-cpd>")
            cli("duplicates", root, "--report", reports / "cpd.xml")
            duplicate_path = root / ".archguard/duplicates.json"
            duplicate_bytes = duplicate_path.read_bytes()
            duplicate_rows, offset = [], 0
            while True:
                duplicates = value(tool("duplication_findings", {"path": ".", "limit": 1, "offset": offset}))
                assert duplicates["status"] == "ready" and duplicates["metadata"]["occurrences_total"] == 2, duplicates
                assert len(json.dumps(duplicates).encode()) <= 4096
                duplicate_rows.extend(duplicates["items"])
                if duplicates["next_offset"] is None:
                    break
                assert duplicates["next_offset"] > offset
                offset = duplicates["next_offset"]
            assert {r["occurrence"]["path"] for r in duplicate_rows if r["kind"] == "duplicate_location"} == {"A.ts", "B.ts"}
            assert duplicate_path.read_bytes() == duplicate_bytes, "MCP reads never rewrite imports"
            filtered = value(tool("duplication_findings", {"path": ".", "query": "B.TS"}))
            assert sum(r["kind"] == "duplicate_location" for r in filtered["items"]) == 2, filtered
            cli("quality-history", root, "--record", "--tag", "smoke")
            history = value(tool("quality_history", {"path": "."}))
            assert history["metadata"]["records_total"] == 1, history

            recorded = json.loads(cli("review", root, "--base", "HEAD", "--save", "--format", "json"))
            record_path = root / ".archguard/review.json"
            record_bytes = record_path.read_bytes()
            review = value(tool("review_context", {"path": ".", "limit": 1}))
            assert review["status"] == "current", review
            assert review["metadata"]["summary"] == recorded["summary"], review
            assert review["metadata"]["review_revision"] == recorded["revision"], review
            assert review["next_offset"] is not None, review
            following = value(tool("review_context", {"path": ".", "offset": review["next_offset"], "limit": 1,
                                                       "expected_revision": review["metadata"]["revision"]}))
            assert following["status"] == "current", following
            assert record_path.read_bytes() == record_bytes
            assert not (root / ".archguard/review-decisions.json").exists(), "MCP must not create decisions"
            original_source = (root / "B.ts").read_text()
            (root / "B.ts").write_text(original_source + "// Change after the recorded review\n")
            try:
                stale = value(tool("review_context", {"path": "."}))
                assert stale["status"] == "stale", stale
                assert record_path.read_bytes() == record_bytes, "MCP preserves the historical review"
                assert "error" in tool("review_context", {"path": ".", "offset": review["next_offset"],
                                                           "expected_revision": review["metadata"]["revision"]})
            finally:
                (root / "B.ts").write_text(original_source)

            # Compose-only services use their own evidence model, including over MCP.
            deployment = root / "deployment"
            (deployment / "one").mkdir(parents=True)
            (deployment / "two").mkdir()
            (deployment / "compose.yaml").write_text(
                "services:\n  one:\n    build: ./one\n    environment:\n      TWO_URL: http://two:8080/api\n"
                "  two:\n    build: ./two\n    environment:\n      ONE_URL: http://one:8080/api\n"
            )
            topology = json.loads(cli("services", deployment, "--format", "json", "--fail-on-found", code=1))
            assert topology["monorepo"] and len(topology["cycles"]) == 1, topology
            mcp_topology = value(tool("service_topology", {"path": "deployment"}))
            assert mcp_topology["dependencies"] == topology["dependencies"], mcp_topology
            assert mcp_topology["cycles"] == topology["cycles"], mcp_topology
            assert not mcp_topology.get("truncated"), mcp_topology
            assert "error" in tool("service_topology", {"path": "../outside"})

            contract_reports = deployment / ".archguard/reports"
            contract_reports.mkdir(parents=True, exist_ok=True)
            api_file = contract_reports / "api.json"
            api_file.write_text(json.dumps({"openapi": "3.0.3", "info": {"title": "Smoke API", "version": "1"}, "paths": {"/items": {"get": {"responses": {"200": {"description": "ok"}}}}}}))
            provider = topology["services"][0]["id"]
            cli("contracts", deployment, "--report", api_file, "--service", provider)
            api_file.write_text(json.dumps({"openapi": "3.0.3", "info": {"title": "Smoke API", "version": "2"}, "paths": {}}))
            cli("contracts", deployment, "--report", api_file, "--service", provider)
            api = value(tool("api_contracts", {"path": "deployment"}))
            assert api["metadata"]["changes_total"] == 1, api
            assert any(item["kind"] == "api_change" for item in api["items"]), api

            # An uncommitted new dependency joins the entry to the existing SCC.
            # Reuse the live server so this also exercises cache invalidation.
            changed_source = 'import { a } from "./A";\nimport { start } from "./entry/index";\nexport const b = 2;\n'
            (root / "B.ts").write_text(changed_source)
            changed = value(tool("compare", {"path": ".", "base": "HEAD"}))
            assert changed["new_findings"] and changed["resolved_findings"], changed
            assert changed["acd_delta"] > 0, changed
            stale_mutations = value(tool("mutation_findings", {"path": "."}))
            assert stale_mutations["status"] == "stale", stale_mutations
            assert stale_mutations["metadata"]["summary"]["killed_percent"] is None, stale_mutations
            stale_duplicates = value(tool("duplication_findings", {"path": "."}))
            assert stale_duplicates["status"] == "stale", stale_duplicates
            assert duplicate_path.read_bytes() == duplicate_bytes
            changed_check = json.loads(cli("check", root, "--base", "HEAD", "--fail-on-acd-regression", "0", "--format", "json", code=1))
            assert any(item["id"] == "metric-acd-regression" for item in changed_check["findings"]), changed_check
            snapshot_check = json.loads(cli("check", root, "--fail-on-acd-regression", "0", "--format", "json", code=1))
            assert any(item["id"] == "metric-acd-regression" for item in snapshot_check["findings"]), snapshot_check
            cli("doc", root, "--out", "architecture.md", "--check", code=1)
            assert (root / "architecture.md").read_text() == document, "drift check must not rewrite the document"
            assert (root / "B.ts").read_text() == changed_source, "comparison must not change working-tree source"
            assert git("diff", "--cached", "--name-only") == "", "comparison must not stage source changes"
            for path in ("../outside", str(outside)):
                denied = tool("scan", {"path": path})
                assert denied.get("error", {}).get("code") == -32602, denied
            assert list(outside.iterdir()) == [outside / "sentinel"]
            assert (outside / "sentinel").read_text() == "unchanged"

            if args.db:
                db = root / ".archguard/graph.db"
                missing = base / "missing/graph.db"
                cli("db", "query", "RETURN 1", "--db", missing, code=3)
                assert not missing.parent.exists()
                cli("db", "sync", root, "--db", db)
                digest = hashlib.sha256(db.read_bytes()).hexdigest()
                queried = json.loads(cli("db", "query", "MATCH (m:module) RETURN m.name", "--db", db, "--format", "json"))
                assert queried["rows"], queried
                cli("db", "query", "CREATE (:module {id: 'unsafe'})", "--db", db, code=2)
                assert hashlib.sha256(db.read_bytes()).hexdigest() == digest
                result = value(tool("graph_query", {"cypher": "MATCH (m:module) RETURN m.name", "db": ".archguard/graph.db"}))
                assert result["rows"], result
                assert hashlib.sha256(db.read_bytes()).hexdigest() == digest
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)

    print("CLI/MCP stdio, review/insight tools, Git comparison/ACD gate, document drift, cache, viewer export and root scope passed" + ("; real read-only DB passed" if args.db else ""))


if __name__ == "__main__":
    main()
