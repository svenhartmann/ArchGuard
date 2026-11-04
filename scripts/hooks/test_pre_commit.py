"""Runs the pre-commit hook against throwaway repositories.

Usage: python3 scripts/hooks/test_pre_commit.py [path/to/archguard]
The binary defaults to target/release/archguard, then target/debug/archguard.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
HOOK = ROOT / "scripts/hooks/pre-commit"
CONFIG = ROOT / ".archguard/archguard.toml"
BINARY = None

ACYCLIC = {"lib.rs": "pub mod a;\npub mod b;\n", "a.rs": "pub struct A;\n", "b.rs": "use crate::a::A;\npub struct B(A);\n"}
CYCLE = "use crate::b::B;\npub struct A(Box<B>);\n"


def long_fn(name, lines):
    body = "".join(f"    let v{i} = {i};\n" for i in range(lines))
    return f"pub fn {name}() {{\n{body}}}\n"


def git(repo, *args):
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)


class PreCommitTests(unittest.TestCase):
    def setUp(self):
        self.repo = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.repo)
        git(self.repo, "init", "-q")
        (self.repo / ".archguard").mkdir()
        shutil.copy(CONFIG, self.repo / ".archguard/archguard.toml")
        self.src = self.repo / "crates/archguard-core/src"
        self.src.mkdir(parents=True)
        for name, text in ACYCLIC.items():
            (self.src / name).write_text(text)
        git(self.repo, "add", ".")

    def commit(self):
        git(self.repo, "add", ".")
        git(self.repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--no-verify", "-m", "c")

    def stage(self, name, text):
        (self.src / name).write_text(text)
        git(self.repo, "add", ".")

    def assert_blocked(self, result, *words):
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        for word in words:
            self.assertIn(word, output)

    def run_hook(self):
        env = dict(os.environ, ARCHGUARD_BIN=str(BINARY))
        return subprocess.run([str(HOOK)], cwd=self.repo, env=env, capture_output=True, text=True)

    def test_acyclic_index_passes(self):
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_staged_cycle_blocks_commit(self):
        (self.src / "a.rs").write_text(CYCLE)
        git(self.repo, "add", ".")
        result = self.run_hook()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("cycle", (result.stdout + result.stderr).lower())

    def test_unstaged_cycle_is_not_checked(self):
        (self.src / "a.rs").write_text(CYCLE)
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_new_long_method_blocks_commit(self):
        self.stage("a.rs", "pub struct A;\n" + long_fn("grows", 80))
        self.assert_blocked(self.run_hook(), "grows", "source_lines")

    def test_new_method_with_many_parameters_blocks_commit(self):
        self.stage("a.rs", "pub struct A;\npub fn wide(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8) {}\n")
        self.assert_blocked(self.run_hook(), "wide", "parameters")

    def test_untouched_legacy_method_passes(self):
        self.stage("a.rs", "pub struct A;\n" + long_fn("legacy", 80))
        self.commit()
        self.stage("b.rs", ACYCLIC["b.rs"] + "pub fn small() {}\n")
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_growing_legacy_method_blocks_commit(self):
        self.stage("a.rs", "pub struct A;\n" + long_fn("legacy", 80))
        self.commit()
        self.stage("a.rs", "pub struct A;\n" + long_fn("legacy", 81))
        self.assert_blocked(self.run_hook(), "legacy", "80 -> 81")

    def test_shrinking_legacy_method_passes(self):
        self.stage("a.rs", "pub struct A;\n" + long_fn("legacy", 80))
        self.commit()
        self.stage("a.rs", "pub struct A;\n" + long_fn("legacy", 70))
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_binary_blocks_commit(self):
        env = dict(os.environ, ARCHGUARD_BIN=str(self.repo / "missing"))
        result = subprocess.run([str(HOOK)], cwd=self.repo, env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ARCHGUARD_BIN", result.stderr)


if __name__ == "__main__":
    candidates = [Path(a) for a in sys.argv[1:2]] or [ROOT / "target/release/archguard", ROOT / "target/debug/archguard"]
    BINARY = next((c.resolve() for c in candidates if c.is_file()), None)
    if BINARY is None:
        sys.exit("no archguard binary; build with cargo build or pass its path")
    unittest.main(argv=sys.argv[:1])
