#!/usr/bin/env python3
"""Self-checks for view_corpus.py with a fake binary and fake summaries (no corpus needed).

Run: python3 scripts/integration/test_view_corpus.py
"""
import contextlib
import io
import json
import os
import tempfile
import time
import unittest
from pathlib import Path

import view_corpus as vc

JSON = ('{"flow":{"diagnostics":[],"complete":true},"summary":{"screens":1,"actions":1,'
        '"actions_unresolved":0,"endpoints_by_access":{}},"model":{"complete":true}}')


def rec(exit=0, timeout=False, sha="x"):
    return {"exit": exit, "timeout": timeout, "sha256": sha, "counts": {}}


OK = {"ui-flow": rec(), "domain": rec()}


class Tests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="vc-test-"))
        vc.OUT = self.tmp / "out"
        self.corpus = self.tmp / "corpus"
        for app in ("eshop", "my.app"):
            (self.corpus / app).mkdir(parents=True)

    def binary(self, body):
        p = self.tmp / "fake"
        p.write_text(f"#!/bin/sh\n{body}\n")
        p.chmod(0o755)
        return str(p)

    def cli(self, *argv):
        out, code = io.StringIO(), 0
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            try:
                vc.main(list(argv))
            except SystemExit as e:
                code = e.code or 0
        return code, out.getvalue()

    def summary(self, label, data):
        (vc.OUT / label).mkdir(parents=True, exist_ok=True)
        (vc.OUT / label / "summary.json").write_text(json.dumps(data))

    def test_files_do_not_overwrite_each_other(self):
        b = self.binary(f"echo '{JSON}'")
        self.cli("run", "--bin", b, "--label", "a", "--corpus", str(self.corpus))
        files = [p.name for p in (vc.OUT / "a").iterdir() if p.name != "summary.json"]
        self.assertEqual(len(files), 12, sorted(files))  # 2 apps x 2 views x json/err/meta
        self.assertIn("my.app.ui-flow.json", files)

    def test_failed_in_both_is_bad(self):
        s = {"x": {"ui-flow": rec(None, True), "domain": rec()}}
        self.summary("a", s)
        self.summary("b", s)
        code, out = self.cli("compare", "a", "b")
        self.assertEqual(code, 1, out)
        self.assertIn("FAILED", out)
        self.assertEqual(self.cli("compare", "a", "b", "--expect-changed", "x")[0], 0)

    def test_failed_in_one_label_is_bad(self):
        self.summary("a", {"x": OK})
        self.summary("b", {"x": {"ui-flow": rec(1), "domain": rec()}})
        self.assertEqual(self.cli("compare", "a", "b")[0], 1)

    def test_missing_app_is_bad(self):
        self.summary("a", {"x": OK, "y": OK})
        self.summary("b", {"x": OK})
        self.assertEqual(self.cli("compare", "a", "b")[0], 1)
        self.assertEqual(self.cli("compare", "a", "b", "--expect-changed", "y")[0], 0)

    def test_identical_ok(self):
        self.summary("a", {"x": OK})
        self.summary("b", {"x": OK})
        self.assertEqual(self.cli("compare", "a", "b")[0], 0)

    def test_bad_label_and_apps(self):
        for argv in (["--label", "../x"], ["--label", "a/b"], ["--label", "a", "--apps", "../x"]):
            code, _ = self.cli("run", "--bin", "/bin/true", "--corpus", str(self.corpus), *argv)
            self.assertNotEqual(code, 0, argv)

    def test_timeout_kills_process_group(self):
        pidfile = self.tmp / "pid"
        b = self.binary(f"sleep 300 &\necho $! > {pidfile}\nwait")
        self.cli("run", "--bin", b, "--label", "t", "--apps", "eshop", "--corpus", str(self.corpus),
                 "--timeout", "1")
        time.sleep(0.5)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pidfile.read_text()), 0)


if __name__ == "__main__":
    unittest.main()
