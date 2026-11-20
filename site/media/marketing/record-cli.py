#!/usr/bin/env python3
"""Record public CLI evidence, replacing only the local checkout prefix with '.'."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from capture import prepare


def normalized(value, root):
    if isinstance(value, dict):
        return {key: normalized(item, root) for key, item in value.items()}
    if isinstance(value, list):
        return [normalized(item, root) for item in value]
    if isinstance(value, str):
        return value.replace(str(root), ".")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--sources", type=Path, required=True)
    args = parser.parse_args()
    site = Path(__file__).resolve().parents[2]
    manifest = json.loads(Path(__file__).with_name("sources.json").read_text())
    projects = {project["id"]: project for project in manifest["projects"]}
    for project, command, expected, name in [
        ("petclinic", "check", 1, "petclinic-check"),
        ("online-boutique", "services", 0, "boutique-services"),
    ]:
        root = prepare(projects[project], args.sources.resolve())
        result = subprocess.run([str(args.cli.resolve()), command, str(root), "--format", "json"],
                                text=True, capture_output=True, timeout=300)
        if result.returncode != expected:
            raise ValueError(f"{command}: expected exit {expected}, got {result.returncode}")
        report = normalized(json.loads(result.stdout), root)
        payload = {
            "scenario": ("Deliberately restrictive local teaching policy; not an upstream defect report."
                         if project == "petclinic" else "Static service discovery; manifest rendering is disabled."),
            "normalization": "Only the local checkout prefix is replaced with '.'; measurements are unchanged.",
            "commit": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
            "cli_version": subprocess.check_output([str(args.cli.resolve()), "--version"], text=True).strip(),
            "exit_code": result.returncode,
            "report": report,
        }
        (site / "assets" / "marketing" / (name + ".json")).write_text(json.dumps(payload, indent=2) + "\n")
    root = (args.sources / "petclinic").resolve()
    base = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    arguments = ["review", str(root), "--base", base, "--format", "pr-comment"]
    result = subprocess.run([str(args.cli.resolve()), *arguments], text=True, capture_output=True, timeout=300)
    if result.returncode not in (0, 1) or not result.stdout.strip():
        raise ValueError(f"review did not produce a PR comment: exit {result.returncode}")
    (site / "assets/marketing/petclinic-pr-comment.md").write_text(normalized(result.stdout, root))
    policy = (root / "archguard.toml").read_text()
    provenance = {
        "kind": "Local generated PR-comment artifact; no GitHub PR was created or posted to.",
        "scenario": "Deliberately restrictive local teaching policy; not an upstream defect report.",
        "base": base, "target_source_commit": base,
        "cli_version": subprocess.check_output([str(args.cli.resolve()), "--version"], text=True).strip(),
        "target": "working tree with unchanged tracked sources and a local illustrative policy",
        "tracked_source_changes": subprocess.check_output(["git", "-C", str(root), "diff", "HEAD", "--name-only"], text=True).splitlines(),
        "policy_file": "archguard.toml", "policy_sha256": hashlib.sha256(policy.encode()).hexdigest(),
        "policy": policy, "command": ["archguard", "review", ".", "--base", base, "--format", "pr-comment"],
        "exit_code": result.returncode,
        "policy_application": "The review command passes its current effective configuration to compare_to_ref for the base comparison.",
        "interpretation": "Review candidates are not a claim of a source-code regression or a real pull request; inspect the raw generated result.",
        "normalization": "Only the local checkout prefix is replaced with '.'; measurements are unchanged.",
    }
    (site / "assets/marketing/petclinic-pr-comment.json").write_text(json.dumps(provenance, indent=2) + "\n")


if __name__ == "__main__":
    main()
