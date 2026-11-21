import json, sys, tempfile, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import view_corpus as vc

COND = {"role": "admin"}


def flow(screen_access=None, ep="undeclared", extra=None):
    f = {"screens": [{"id": "s", "route": "/s", "access": screen_access}],
         "transitions": [], "actions": [], "endpoints": [{"method": "GET", "path": "/a", "access": ep}]}
    f.update(extra or {})
    return f


def run(base, head):
    with tempfile.TemporaryDirectory() as t:
        dirs = []
        for n, f in (("b", base), ("h", head)):
            (Path(t) / n).mkdir()
            (Path(t) / n / "app.ui-flow.json").write_text(json.dumps({"flow": f}))
            dirs.append(str(Path(t) / n))
        try:
            vc.main(["access-diff", *dirs])
        except SystemExit as e:
            return e.code


class T(unittest.TestCase):
    def test_same_is_clean(self):
        f = flow(COND, {"protected": COND})
        self.assertEqual(run(f, f), 0)

    def test_protected_to_open(self):
        self.assertEqual(run(flow(COND, {"protected": COND}), flow(COND, "open")), 1)

    def test_new_screen_open(self):
        self.assertEqual(run(flow(), flow("open", extra={"screens": [{"id": "s", "access": "open"}]})), 1)

    def test_legacy_null_to_undeclared(self):
        new = flow("undeclared", extra={"role_conventions": []})
        self.assertEqual(run(flow(None), new), 0)

    def test_unrecognised_is_exit_2(self):
        f = flow(COND, {"open": None})
        self.assertEqual(run(f, f), 2)

    def test_old_unit_condition_is_protected(self):
        f = flow("authenticated", "undeclared")
        self.assertEqual(run(f, f), 0)


if __name__ == "__main__":
    unittest.main()
