#!/usr/bin/env python3
"""Real TypeScript CLI/graph/gate integration; no package installation or project execution."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--compiler", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    compiler = args.compiler.resolve()
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update({"ARCHGUARD_TRUST_PROJECT_COMPILER": "1", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})
    with tempfile.TemporaryDirectory(prefix="archguard-ts-live-") as tmp:
        root = Path(tmp)
        (root / "src").mkdir()
        (root / "src/a.ts").write_text("export function first(n: number): number { return n; }\n")
        (root / "src/b.ts").write_text("import { first } from './a'; export function second(n: number): number { return first(n); }\n")
        (root / "archguard.toml").write_text('[project]\nsource_roots=["src"]\nlanguages=["ts"]\n[modules]\ndepth=0\n[semantic.typescript]\ncompiler=' + json.dumps(str(compiler)) + '\n')
        (root / "tsconfig.json").write_text(json.dumps({"compilerOptions": {"strict": True, "target": "ES2020"}, "include": ["src/*.ts"]}))

        def command(*arguments, expected=0):
            result = subprocess.run([str(binary), *arguments], capture_output=True, text=True, timeout=60, env=env)
            assert result.returncode == expected, (arguments, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout)

        before = command("calls", str(root), "--format", "json")
        assert before["status"] == "ready", before
        assert len(before["methods"]) == 2 and len(before["calls"]) == 1
        assert all(m["id"].startswith("typescript:") for m in before["methods"])
        for git_args in [("init", "--quiet", "--template="), ("config", "user.name", "Test"), ("config", "user.email", "test@example.invalid"), ("add", "."), ("commit", "--quiet", "-m", "base")]:
            subprocess.run(["git", *git_args], cwd=root, check=True, capture_output=True, env=env)
        (root / "src/a.ts").write_text("import { second } from './b';\nexport function first(n: number): number { return n > 0 ? second(n - 1) : 0; }\n")
        graph = command("graph", str(root), "--format", "json")
        assert graph["semantic"]["status"] == "ready", graph.get("semantic")
        assert graph["semantic"]["projected_calls"] == 2
        assert len(graph["edges"]) == 2
        fail = command("quality", str(root), "--base", "HEAD", "--max-complexity", "1", "--format", "json", expected=1)["gate"]
        assert fail["status"] == "fail" and len(fail["methods"]) == 1, fail
        assert fail["methods"][0]["cyclomatic"] == 2
        assert fail["methods"][0]["file"] == "src/a.ts", "gate evidence maps stripped compiler paths to physical Git paths"
        passed = command("quality", str(root), "--base", "HEAD", "--max-complexity", "2", "--format", "json")["gate"]
        assert passed["status"] == "pass", passed
        print("TypeScript CLI: real compiler, call identities, semantic graph projection, source roots and changed-method gate pass/fail passed")


if __name__ == "__main__":
    main()
