#!/usr/bin/env python3
"""Wandelt cargo-mutants outcomes.json in mutation-testing-elements (v2) um.

Nutzung: python3 scripts/checks/mutants_to_mte.py <outcomes.json> <project-root> <out.json>
Der Importer (archguard mutations --report) verlangt inline source und
eins-basierte UTF-16-Spalten mit exklusivem Ende.
shortcut: killed_by bleibt leer (outcomes.json nennt keine Testnamen pro Mutant);
Testnamen erst mitgeben, wenn cargo-mutants sie maschinenlesbar liefert.
"""
import json
import subprocess
import sys
from pathlib import Path

STATUS_BY_SUMMARY = {
    "CaughtMutant": "Killed",
    "UncaughtMutant": "Survived",
    "Timeout": "Timeout",
    "Unviable": "CompileError",
    "MissingCoverage": "NoCoverage",
    "OtherError": "RuntimeError",
}


def utf16_column(line: str, column: int) -> int:
    """cargo-mutants spaelt in Zeichen; das Schema will UTF-16-Spalten."""
    return sum(len(c.encode("utf-16-le")) // 2 for c in line[: column - 1]) + 1


def package_paths(root: Path) -> dict:
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root, check=True, capture_output=True).stdout)
    return {p["name"]: Path(p["manifest_path"]).parent for p in meta["packages"]}


def convert(outcomes: dict, root: Path, pkg_dirs: dict | None = None) -> dict:
    if pkg_dirs is None:
        pkg_dirs = package_paths(root)
    by_file: dict = {}
    for outcome in outcomes["outcomes"]:
        mutant = outcome.get("scenario")
        if not isinstance(mutant, dict) or "Mutant" not in mutant:
            continue
        mutant = mutant["Mutant"]
        status = STATUS_BY_SUMMARY.get(outcome["summary"])
        if status is None:
            print(f"w skip: unknown summary {outcome['summary']} for {mutant['name']}",
                  file=sys.stderr)
            continue
        pkg = mutant.get("package", "")
        path = pkg_dirs.get(pkg, root) / mutant["file"]
        try:
            source = path.read_text(encoding="utf-8")
        except OSError as e:
            print(f"w skip: source unreadable {path}: {e}", file=sys.stderr)
            continue
        lines = source.split("\n")
        start, end = mutant["span"]["start"], mutant["span"]["end"]
        try:
            location = {
                "start": {"line": start["line"],
                          "column": utf16_column(lines[start["line"] - 1], start["column"])},
                "end": {"line": end["line"],
                        "column": utf16_column(lines[end["line"] - 1], end["column"])},
            }
        except IndexError:
            print(f"w skip: span outside source {mutant['name']}", file=sys.stderr)
            continue
        duration = sum(p["duration"] for p in outcome.get("phase_results", [])
                       if isinstance(p.get("duration"), (int, float)))
        rel = str(path.relative_to(root))
        entry = by_file.setdefault(rel, {
            "language": "rust", "source": source, "mutants": []})
        entry["mutants"].append({
            "id": mutant["name"],
            "mutatorName": mutant["name"].split(": ", 2)[-1],
            "status": status,
            "location": location,
            "description": mutant["name"],
            "replacement": mutant.get("replacement"),
            "duration": round(duration * 1000.0, 1),
        })
    return {
        "schemaVersion": "2",
        "thresholds": {"high": 80, "low": 60},
        "framework": {"name": "cargo-mutants",
                      "version": outcomes.get("cargo_mutants_version")},
        "files": by_file,
    }


def main(argv: list) -> int:
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    outcomes = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
    report = convert(outcomes, Path(argv[2]).resolve())
    Path(argv[3]).write_text(json.dumps(report, indent=1), encoding="utf-8")
    total = sum(len(f["mutants"]) for f in report["files"].values())
    print(f"ok: {len(report['files'])} files, {total} mutants -> {argv[3]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
