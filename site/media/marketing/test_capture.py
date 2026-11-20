"""Regression checks for the capture script's source and configuration guards."""
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("capture", Path(__file__).with_name("capture.py"))
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class SourceGuards(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.sources = Path(self.tmp.name)
        self.repo = self.sources / "fixture"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        (self.repo / "LICENSE").write_text("fixture license\n")
        (self.repo / "source.py").write_text("x = 1\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "LICENSE", "source.py"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "-c", "user.name=Capture Test", "-c",
                        "user.email=capture@example.invalid", "commit", "-qm", "fixture"], check=True)
        self.project = {"id": "fixture", "repository": "https://example.invalid/fixture",
                        "commit": subprocess.check_output(["git", "-C", str(self.repo), "rev-parse", "HEAD"], text=True).strip(),
                        "license_path": "LICENSE", "license_sha256": hashlib.sha256((self.repo / "LICENSE").read_bytes()).hexdigest(),
                        "configuration": 'derive_layers = false\n'}

    def test_pinned_clean_source_gets_only_the_recorded_config(self):
        self.assertEqual(capture.prepare(self.project, self.sources), self.repo)
        self.assertEqual((self.repo / "archguard.toml").read_text(), self.project["configuration"])
        self.assertEqual(capture.prepare(self.project, self.sources), self.repo)

    def test_different_revision_is_rejected(self):
        self.project["commit"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "unexpected revision"):
            capture.prepare(self.project, self.sources)

    def test_edited_sources_are_not_silently_captured(self):
        (self.repo / "source.py").write_text("x = 2\n")
        with self.assertRaisesRegex(ValueError, "tracked source changes"):
            capture.prepare(self.project, self.sources)

    def test_license_hash_must_match_manifest(self):
        self.project["license_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "license file"):
            capture.prepare(self.project, self.sources)

    def test_existing_different_configuration_is_preserved(self):
        config = self.repo / "archguard.toml"
        config.write_text("derive_layers = true\n")
        with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
            capture.prepare(self.project, self.sources)
        self.assertEqual(config.read_text(), "derive_layers = true\n")

    def test_success_without_a_new_png_does_not_reuse_old_capture(self):
        output = self.sources / "old.png"
        output.write_bytes(b"previous reviewed capture")
        with self.assertRaisesRegex(ValueError, "No PNG produced"):
            capture.capture(Path("/usr/bin/true"), self.repo, output, {}, 5)
        self.assertEqual(output.read_bytes(), b"previous reviewed capture")


if __name__ == "__main__":
    unittest.main()
