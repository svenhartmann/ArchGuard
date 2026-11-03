#!/usr/bin/env python3
"""Enforce ARCHITECTURE.md §3.2 against all manifest dependency kinds.

Normal, build, dev, optional and target-specific workspace dependencies use
the same allowlist. `--no-deps` retains inactive/optional manifest entries;
we deliberately do not rely on the active Cargo resolution graph.
"""
import json
from pathlib import Path
import subprocess
import sys

PREFIX = "archguard-"
ALLOWED = {
    "core": set(),
    "extract": {"core"},
    "analysis": {"core"},
    "report": {"core"},
    "store": {"core"},
    "engine": {"core", "extract", "analysis", "report", "store"},
    "mcp": {"engine"},
    "cli": {"engine", "mcp"},
}


def verify(metadata):
    members = set(metadata.get("workspace_members", []))
    packages = [p for p in metadata.get("packages", []) if p["id"] in members]
    expected = {PREFIX + name for name in ALLOWED}
    if {p["name"] for p in packages} != expected or len(packages) != len(expected):
        raise ValueError("missing, duplicate or unexpected workspace packages")
    by_path = {Path(p["manifest_path"]).resolve().parent: p["name"] for p in packages}
    checked = 0
    for package in packages:
        source = package["name"].removeprefix(PREFIX)
        for dependency in package["dependencies"]:
            target = dependency["name"]
            if dependency.get("path"):
                target = by_path.get(Path(dependency["path"]).resolve(), target)
            if target not in expected:
                continue
            checked += 1
            if target.removeprefix(PREFIX) not in ALLOWED[source]:
                raise ValueError(
                    f"forbidden {package['name']} -> {target} "
                    f"(kind={dependency.get('kind') or 'normal'}, "
                    f"rename={dependency.get('rename')}, optional={dependency.get('optional')}, "
                    f"target={dependency.get('target')})"
                )
    if checked == 0:
        raise ValueError("no workspace edges checked")
    return checked


def main():
    root = Path(__file__).resolve().parents[2]
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
            cwd=root, check=True, capture_output=True, text=True,
        )
        count = verify(json.loads(result.stdout))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"architecture check failed: {error}", file=sys.stderr)
        return 1
    print(f"architecture: {count} workspace dependency entries checked (all kinds)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
