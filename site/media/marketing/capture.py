#!/usr/bin/env python3
"""Reproduce the pinned marketing captures with a snapshot-enabled desktop build.

This uses fresh public Git clones. It never runs a project's build or installs its
dependencies. The provided application must be built with --features snapshot.
Existing source directories are reused only at the exact pinned revision.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
SITE = HERE.parents[1]


def run(argv, **kwargs):
    return subprocess.run(argv, check=True, text=True, **kwargs)


def prepare(project, sources):
    checkout = sources / project["id"]
    if not checkout.exists():
        checkout.mkdir(parents=True)
        run(["git", "init", "--quiet", str(checkout)])
        run(["git", "-C", str(checkout), "remote", "add", "origin", project["repository"] + ".git"])
        run(["git", "-C", str(checkout), "fetch", "--depth", "1", "origin", project["commit"]])
        run(["git", "-C", str(checkout), "checkout", "--detach", "FETCH_HEAD"])
    actual = run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True).stdout.strip()
    if actual != project["commit"]:
        raise ValueError(f"{checkout.name}: unexpected revision; use a new sources directory")
    dirty = run(["git", "-C", str(checkout), "diff", "HEAD", "--name-only"], capture_output=True).stdout
    if dirty.strip():
        raise ValueError(f"{checkout.name}: tracked source changes; use a new sources directory")
    digest = hashlib.sha256((checkout / project["license_path"]).read_bytes()).hexdigest()
    if digest != project["license_sha256"]:
        raise ValueError(f"{checkout.name}: license file does not match recorded source")
    for additional in project.get("additional_license_files", []):
        digest = hashlib.sha256((checkout / additional["path"]).read_bytes()).hexdigest()
        if digest != additional["sha256"]:
            raise ValueError(f"{checkout.name}: additional license file does not match recorded source")
    if project.get("configuration"):
        config = checkout / "archguard.toml"
        if config.exists() and config.read_text() != project["configuration"]:
            raise ValueError(f"{checkout.name}: refusing to overwrite a different configuration")
        config.write_text(project["configuration"])
    return checkout


def capture(app, checkout, output, env, timeout):
    with tempfile.TemporaryDirectory(prefix="archguard-marketing-config-") as cfg:
        output.parent.mkdir(parents=True, exist_ok=True)
        # Validate a new render before replacing a previously reviewed capture.
        # A program that exits successfully without a PNG must not reuse a stale file.
        temporary = Path(cfg) / "capture.png"
        environment = {**os.environ, "ARCHGUARD_DESKTOP_CONFIG_DIR": cfg,
                       "ARCHGUARD_DESKTOP_LANG": "en", "ARCHGUARD_DESKTOP_SNAPSHOT_MS": "1500",
                       "ARCHGUARD_DESKTOP_SIZE": "1480x920",
                       "ARCHGUARD_DESKTOP_SNAPSHOT": str(temporary), **env}
        # No external project compiler and no Helm/Kustomize rendering opt-in.
        environment.pop("ARCHGUARD_TRUST_PROJECT_COMPILER", None)
        environment.pop("ARCHGUARD_RENDER_MANIFESTS", None)
        run([str(app), str(checkout)], env=environment, timeout=timeout,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not temporary.is_file() or temporary.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
            raise ValueError("No PNG produced; build archguard-desktop with --features snapshot")
        # The temporary directory and destination can be on different volumes.
        output.write_bytes(temporary.read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", required=True, type=Path)
    parser.add_argument("--sources", required=True, type=Path)
    parser.add_argument("--only", help="Comma-separated capture IDs")
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    manifest = json.loads((HERE / "sources.json").read_text())
    projects = {p["id"]: p for p in manifest["projects"]}
    selected = set(args.only.split(",")) if args.only else None
    definitions = json.loads((HERE / "captures.json").read_text())
    known = {c["id"] for c in definitions}
    if selected and selected - known:
        parser.error("unknown capture IDs: " + ", ".join(sorted(selected - known)))
    for definition in definitions:
        if selected and definition["id"] not in selected:
            continue
        checkout = prepare(projects[definition["project"]], args.sources.resolve())
        target = SITE / "assets" / "marketing" / (definition["id"] + ".png")
        capture(args.app.resolve(), checkout, target, definition["environment"], args.timeout)
        print(definition["id"], hashlib.sha256(target.read_bytes()).hexdigest(), flush=True)


if __name__ == "__main__":
    main()
