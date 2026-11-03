import copy
import unittest

from check_architecture import ALLOWED, PREFIX, verify


def metadata():
    packages = []
    for source, targets in ALLOWED.items():
        name = PREFIX + source
        packages.append({
            "name": name, "id": name, "manifest_path": f"/workspace/{source}/Cargo.toml",
            "dependencies": [{"name": PREFIX + target, "kind": None} for target in sorted(targets)],
        })
    return {"workspace_members": [p["id"] for p in packages], "packages": packages}


class ArchitectureTests(unittest.TestCase):
    def test_real_allowlist_is_nonempty(self):
        self.assertEqual(verify(metadata()), 12)

    def test_forbidden_edges_of_every_kind_even_inactive_or_renamed(self):
        for kind in [None, "build", "dev"]:
            for optional in [False, True]:
                data = metadata()
                report = next(p for p in data["packages"] if p["name"] == "archguard-report")
                report["dependencies"].append({
                    "name": "archguard-extract", "rename": "scanner", "kind": kind,
                    "optional": optional, "target": "cfg(windows)", "path": "/workspace/extract",
                })
                with self.subTest(kind=kind, optional=optional), self.assertRaisesRegex(ValueError, "forbidden"):
                    verify(data)

    def test_missing_packages_and_empty_extraction_fail(self):
        for data in [{}, {"workspace_members": [], "packages": []}]:
            with self.assertRaises(ValueError):
                verify(data)
        data = metadata()
        data["packages"].pop()
        with self.assertRaises(ValueError):
            verify(data)
        data = copy.deepcopy(metadata())
        for package in data["packages"]:
            package["dependencies"] = []
        with self.assertRaisesRegex(ValueError, "no workspace edges"):
            verify(data)


if __name__ == "__main__":
    unittest.main()
