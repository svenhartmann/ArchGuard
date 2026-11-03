#!/usr/bin/env python3
"""Exercise quality CLI imports and exported HTML with explicitly synthetic data.

Usage: python3 scripts/integration/test_engineering_quality_live.py [BINARY] [--out DIRECTORY]
No servers, project builds, test runners or external analyzers are started.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

from viewer_payload import payload_from_html


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path, nargs="?", default=Path("target/debug/archguard"))
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update({"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})

    with tempfile.TemporaryDirectory(prefix="archguard-synthetic-quality-") as temporary:
        root = Path(temporary)
        inputs = {}

        def write(path, content):
            inputs[path] = content
            destination = root / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(content, encoding="utf-8")

        def run(command, *arguments, expected=0, as_json=True):
            process = subprocess.run(
                [str(binary), command, str(root), *arguments, "--format", "json" if as_json else "html"],
                capture_output=True, text=True, timeout=90, env=environment,
            )
            assert process.returncode == expected, (command, arguments, process.returncode, process.stderr, process.stdout[:1000])
            return json.loads(process.stdout) if as_json else process.stdout

        write("archguard.toml", "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['py']\nexclude=[]\n[modules]\ndepth=1\n[rules]\ndeny_cycles='off'\n")
        with (root / "archguard.toml").open("a") as config:
            config.write("[architecture.model]\nname='Synthetic component contracts'\nseverity='error'\n[[architecture.model.components]]\nname='Application'\ninclude=['pkg/a.py','pkg/c.py']\n[[architecture.model.components]]\nname='Domain'\ninclude=['pkg/b.py','pkg/d.py']\npublic=['pkg/b.py']\nmax_incoming_components=0\n[[architecture.model.allowed_dependencies]]\nfrom='Application'\nto='Domain'\n")
        inputs["archguard.toml"] = (root / "archguard.toml").read_text()
        sources = {
            "src/pkg/a.py": "import pkg.b\nvalue_a = pkg.b.value_b\n",
            "src/pkg/b.py": "value_b = 1\n",
            "src/pkg/c.py": "import pkg.d\nvalue_c = pkg.d.value_d\n",
            "src/pkg/d.py": "value_d = 2\n",
        }
        for path, source in sources.items():
            write(path, source)
        write("compose.yaml", "services:\n  api:\n    image: synthetic-api\n  web:\n    image: synthetic-web\n    environment:\n      API_URL: http://api:8080\n")
        cohesion = run("cohesion")
        package = next(row for row in cohesion["packages"] if row["node"] == "pkg")
        assert package["split_candidate"] is True and package["internal_dependencies"] == 2, package
        assert sorted(len(group["files"]) for group in package["groups"]) == [2, 2]

        write("junit.xml", "<testsuite name='synthetic-unit'><testcase classname='TestA' name='a' time='0.125'/><testcase classname='TestB' name='b' time='2.5'><failure message='Synthetic assertion failure'/></testcase><testcase name='unknown'><skipped/></testcase></testsuite>")
        write("test-map.json", json.dumps({"schema_version": 1, "tests": [
            {"class_name": "TestA", "name": "a", "sources": [{"path": "src/pkg/a.py", "lines": [1, 2]}]},
            {"class_name": "TestB", "name": "b", "sources": [{"path": "src/pkg/b.py", "lines": [1]}]},
        ]}))
        tests = run("test-evidence", "--report", "junit.xml", "--mapping", "test-map.json", "--target", "pkg/b.py")
        assert tests["report"]["tests"][0]["duration_seconds"] == 0.125
        assert tests["impact"]["unmapped_tests"] == 1
        assert {test["reason"] for test in tests["impact"]["selected"]} == {"mapped_changed_source", "mapped_static_dependent"}, tests

        mutation_source = sources["src/pkg/b.py"]
        column = mutation_source.index("1") + 1
        write("mutation-report.json", json.dumps({"schemaVersion": "2", "thresholds": {"high": 80, "low": 60}, "framework": {"name": "Synthetic fixture", "version": "1"}, "files": {
            "src/pkg/b.py": {"language": "python", "source": mutation_source, "mutants": [
                {"id": str(index), "mutatorName": "SyntheticNumber", "status": status, "location": {"start": {"line": 1, "column": column}, "end": {"line": 1, "column": column + 1}}, "replacement": "2"}
                for index, status in enumerate(["Killed", "Survived"])
            ]}
        }}))
        mutations = run("mutations", "--report", "mutation-report.json")
        assert mutations["summary"]["killed_percent"] == 50.0
        assert mutations["files"][0]["status"] == "matches_reported_source"

        write("cpd.xml", "<pmd-cpd version='1.0.0' pmdVersion='7.3.0'><duplication lines='1' tokens='5'><file path='src/pkg/b.py' line='1'/><file path='src/pkg/d.py' line='1'/><codefragment>value_b = 1</codefragment></duplication></pmd-cpd>")
        duplicates = run("duplicates", "--report", "cpd.xml")
        assert duplicates["status"] == "ready" and len(duplicates["groups"][0]["occurrences"]) == 2

        def coverage(all_covered):
            classes = []
            for index, (path, source) in enumerate(sources.items()):
                lines = "".join(f'<line number="{line}" branch="false" hits="{1 if all_covered or (index % 2 == 0 and line == 1) else 0}"/>' for line in range(1, len(source.splitlines()) + 1))
                classes.append(f'<class filename="{path}"><lines>{lines}</lines></class>')
            return '<coverage><packages><package><classes>' + ''.join(classes) + '</classes></package></packages></coverage>'

        def quality(count):
            return json.dumps({"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "SyntheticLint", "version": "1"}}, "results": [
                {"ruleId": "FIXTURE", "level": "warning", "message": {"text": "Synthetic example finding; not a real project defect"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": path}, "region": {"startLine": 1}}}]}
                for path in ["src/pkg/a.py", "src/pkg/c.py"][:count]
            ]}]})

        for tag, complete, findings in [("synthetic-before", False, 2), ("synthetic-after", True, 1)]:
            write("coverage.xml", coverage(complete))
            imported_coverage = run("coverage", "--report", "coverage.xml")
            assert imported_coverage["status"] == "ready", imported_coverage
            write("quality.sarif", quality(findings))
            run("quality", "--report", "quality.sarif")
            run("quality-history", "--record", "--tag", tag)
        history = run("quality-history")
        assert len(history["records"]) == 2
        assert history["records"][0]["source_sha256"] == history["records"][1]["source_sha256"], "Synthetic measurements changed; production source did not"
        delta = next(row for row in history["comparison"]["modules"] if row["node"] == "pkg")
        assert delta["line_coverage"]["status"] == "unverified" and delta["line_coverage"]["delta"] > 0, delta
        assert delta["active_findings"]["delta"] == -1
        assert delta["mean_complexity"]["delta"] is None, "Python has no fabricated compiler measurement"

        services = run("services")
        provider = next(service["id"] for service in services["services"] if service["name"] == "api")
        consumer = next(service["id"] for service in services["services"] if service["name"] == "web")
        spec = {"openapi": "3.1.0", "info": {"title": "Synthetic fixture API", "version": "1"}, "paths": {
            "/items": {"get": {"responses": {"200": {"description": "OK"}}}},
            "/items/{id}": {"get": {"responses": {"200": {"description": "OK"}}}},
        }}
        write("api.json", json.dumps(spec))
        write("api-usage.json", json.dumps({"schema_version": 1, "usages": [{"consumer": consumer, "operation": "GET /items"}]}))
        run("contracts", "--report", "api.json", "--service", provider, "--usage", "api-usage.json")
        del spec["paths"]["/items"]
        write("api.json", json.dumps(spec))
        contracts = run("contracts", "--report", "api.json", "--service", provider)
        removed = next(change for change in contracts["contracts"][0]["changes"] if change["kind"] == "operation_removed")
        assert removed["operation"] == "GET /items" and removed["consumers"] == [consumer], removed

        review = run("review", "--save")
        categories = {f["category"] for f in review["findings"]}
        assert {"architecture", "tests", "mutations", "duplication"} <= categories, categories
        duplicate_finding = next(f for f in review["findings"] if f["category"] == "duplication")
        assert set(duplicate_finding["evidence"]["current_source_hashes"]) == {"pkg/b.py", "pkg/d.py"}
        page = run("graph", as_json=False)
        data = payload_from_html(page)
        for key in ["cohesion", "quality_history", "test_evidence", "mutations", "duplicates", "contracts"]:
            assert key in data["insights"], f"Missing actual HTML insight {key}"
        insights = data["insights"]
        assert any(row["node"] == "pkg" and row["split_candidate"] for row in insights["cohesion"]["packages"])
        assert len(insights["quality_history"]["records"]) == 2
        assert len(insights["test_evidence"]["tests"]) == 3
        assert insights["mutations"]["summary"]["killed_percent"] == 50.0
        assert len(insights["duplicates"]["groups"]) == 1
        model = insights["architecture"]["observed"]["target_model"]
        assert model["violating_dependencies"] == 0 and model["violating_interfaces"] == 1 and model["coupling_violations"] == 1, model
        assert insights["review_session"]["status"] == "current", insights["review_session"]
        assert insights["contracts"]["contracts"][0]["changes"][0]["operation"] == "GET /items"
        assert 'data-tab="quality"' in page and 'id="contracts-panel"' in page
        assert 'function renderEngineeringQuality(' in page and 'function renderApiContracts(' in page
        measured = [method for method in data.get("calls", {}).get("methods", []) if method.get("complexity")]
        assert all(method.get("end_line", 0) >= method["line"] for method in measured)
        result = {
            "dataset": "Synthetic fixtures only; not measurements from a real application",
            "source_files": len(sources), "cohesion_groups": [len(group["files"]) for group in package["groups"]],
            "tests": len(tests["report"]["tests"]), "recommended_tests": len(tests["impact"]["selected"]), "unmapped_tests": tests["impact"]["unmapped_tests"],
            "killed_percent": mutations["summary"]["killed_percent"], "history_records": len(history["records"]), "history_delta": delta,
            "contract_change": removed, "html_insights": ["cohesion", "quality_history", "test_evidence", "mutations", "duplicates", "contracts"],
            "duplicate_groups": len(duplicates["groups"]), "review_categories": sorted(categories),
            "compiler_measured_methods": len(measured), "html_bytes": len(page.encode("utf-8")),
            "checks": "Built CLI imports, saved-snapshot reads and actual embedded HTML payload; no browser layout/paint verification",
        }
        if args.out:
            output = args.out.resolve()
            output.mkdir(parents=True, exist_ok=True)
            (output / "example.html").write_text(page, encoding="utf-8")
            (output / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            # Preserve labeled, reproducible input files separately from the generated viewer.
            for path, content in inputs.items():
                destination = output / "synthetic-inputs" / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text(content, encoding="utf-8")
            (output / "synthetic-inputs" / "SYNTHETIC.txt").write_text("Synthetic integration-test inputs, not real code-quality findings. Re-run the script to reproduce the two-stage imports.\n", encoding="utf-8")
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
