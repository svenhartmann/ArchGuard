"""Offline discovery must not run the optional upstream integration check."""
import importlib.util
import pathlib
import unittest
from unittest import mock


class JellyfinExampleImportTest(unittest.TestCase):
    def test_import_does_not_inspect_checkout_or_launch_commands(self):
        path = pathlib.Path(__file__).with_name("test_jellyfin_example.py")
        spec = importlib.util.spec_from_file_location("jellyfin_example_import_check", path)
        module = importlib.util.module_from_spec(spec)
        with (
            mock.patch("subprocess.run", side_effect=AssertionError("import must not launch commands")),
            mock.patch.object(pathlib.Path, "is_file", side_effect=AssertionError("import must not inspect checkout files")),
            mock.patch.object(pathlib.Path, "read_text", side_effect=AssertionError("import must not read checkout files")),
            mock.patch.object(pathlib.Path, "rglob", side_effect=AssertionError("import must not walk the checkout")),
        ):
            spec.loader.exec_module(module)
        self.assertTrue(callable(module.main))


if __name__ == "__main__":
    unittest.main()
