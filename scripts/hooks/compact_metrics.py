"""Compactness ratchet for methods and classes, called by the pre-commit hook.

Usage: python3 compact_metrics.py <archguard> <head-tree|-> <staged-tree>

Reads `archguard calls --format json` for HEAD and for the staged tree. A
method or class fails when it exceeds a ceiling below and is new or got worse
than in HEAD. Existing outliers may stay as they are or shrink, so the gate
can run on today's tree; every touched outlier has to move toward the ceiling.
"""
import json
import os
import re
import subprocess
import sys

# Ceilings: values strictly greater fail. Lines are source lines without
# blanks and comments; methods per class counts free functions per module too.
METHOD_CEILINGS = {
    "source_lines": 60,
    "cyclomatic": 15,
    "cognitive": 20,
    "max_nesting": 4,
    "parameters": 6,
}
CLASS_CEILINGS = {"methods": 20}

GIT_ENV = ("GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE")


def calls(binary, tree):
    env = {k: v for k, v in os.environ.items() if k not in GIT_ENV}
    result = subprocess.run([binary, "calls", tree, "--format", "json"], env=env, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"pre-commit: archguard calls failed for {tree}:\n{result.stderr}")
    return json.loads(result.stdout)


def parameter_count(signature):
    """Top-level parameters of `fn name (a: T, b: U<V, W>)`; `self` counts."""
    start = signature.find("(")
    if start < 0:
        return 0
    depth, count, seen = 0, 0, False
    for ch in signature[start + 1:]:
        if ch in "([<{":
            depth += 1
        elif ch in ")]>}":
            if depth == 0:
                break
            depth -= 1
        elif ch == "," and depth == 0:
            count, seen = count + 1, False
            continue
        if not ch.isspace():
            seen = True
    return count + (1 if seen else 0)


def strip_line(method_id):
    """`file::Class#name@27` -> `file::Class#name`: edits above must not rename it."""
    return re.sub(r"@\d+$", "", method_id)


def larger(a, b):
    return b if a is None else a if b is None else max(a, b)


def measure(report):
    extra = {m["method_id"]: m for m in report.get("details", {}).get("method_metrics", [])}
    methods, classes = {}, {}
    for method in report.get("methods", []):
        detail = extra.get(method["id"], {})
        values = {
            "source_lines": detail.get("source_lines"),
            "cyclomatic": (method.get("complexity") or {}).get("cyclomatic"),
            "cognitive": detail.get("cognitive"),
            "max_nesting": (method.get("complexity") or {}).get("max_nesting"),
            "parameters": parameter_count(method.get("signature", "")),
        }
        key = strip_line(method["id"])
        location = f'{method["file"]}:{method["line"]}'
        if key in methods:
            # Same name twice in one class: keep the larger value per metric.
            values = {k: larger(v, methods[key][0][k]) for k, v in values.items()}
        methods[key] = (values, location)
        cls = classes.setdefault(method["class_id"], ({"methods": 0}, method["file"]))
        cls[0]["methods"] += 1
    return methods, classes


def violations(kind, ceilings, before, after):
    found = []
    for key, (values, location) in sorted(after.items()):
        previous = before.get(key, ({}, None))[0]
        for metric, ceiling in ceilings.items():
            value = values.get(metric)
            if value is None or value <= ceiling:
                continue
            old = previous.get(metric)
            if old is None:
                found.append(f"  {location} {kind} {key}: {metric} {value} > {ceiling} (new)")
            elif value > old:
                found.append(f"  {location} {kind} {key}: {metric} {old} -> {value}, ceiling {ceiling}")
    return found


def main():
    binary, head_tree, staged_tree = sys.argv[1:4]
    before = measure(calls(binary, head_tree)) if head_tree != "-" else ({}, {})
    after = measure(calls(binary, staged_tree))
    found = violations("method", METHOD_CEILINGS, before[0], after[0])
    found += violations("class", CLASS_CEILINGS, before[1], after[1])
    if found:
        print("pre-commit: methods or classes grew past the compactness ceilings:", file=sys.stderr)
        print("\n".join(found), file=sys.stderr)
        print("pre-commit: split them, or keep legacy outliers from growing (scripts/hooks/compact_metrics.py).",
              file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
