import contextlib, io, json, sys, tempfile, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import view_corpus as vc

COND = {"role": "admin"}


def flow(screen_access=None, ep="undeclared", extra=None):
    f = {"screens": [{"id": "s", "route": "/s", "access": screen_access}],
         "transitions": [], "actions": [], "endpoints": [{"method": "GET", "path": "/a", "access": ep}]}
    f.update(extra or {})
    return f


def run(base, head, out=None):
    with tempfile.TemporaryDirectory() as t:
        dirs = []
        for n, f in (("b", base), ("h", head)):
            (Path(t) / n).mkdir()
            (Path(t) / n / "app.ui-flow.json").write_text(json.dumps({"flow": f}))
            dirs.append(str(Path(t) / n))
        try:
            with contextlib.redirect_stdout(out or io.StringIO()):
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

    def test_legacy_null_prints_no_count_noise(self):
        out = io.StringIO()
        run(flow(None), flow("undeclared", extra={"role_conventions": [{"a": 1}]}), out)
        self.assertIn("app: no change", out.getvalue())

    def test_unrecognised_is_exit_2(self):
        f = flow(COND, {"open": None})
        self.assertEqual(run(f, f), 2)

    def test_old_unit_condition_is_protected(self):
        f = flow("authenticated", "undeclared")
        self.assertEqual(run(f, f), 0)

    def test_shared_key_twin_hides_move(self):
        ep = lambda a: {"method": "GET", "path": "/a", "access": a}
        base = flow(extra={"endpoints": [ep("open"), ep({"protected": COND})]})
        head = flow(extra={"endpoints": [ep("open"), ep("open")]})
        self.assertEqual(run(base, head), 1)
        self.assertEqual(run(base, base), 0)

    def test_enforced_open_layer_makes_the_effective_condition_weaker(self):
        layer = lambda how, state: {"endpoint_file": "F", "endpoint_line": 1, "endpoint_method": "GET", "endpoint_path": "/a",
                                    "how": how, "state": state}
        ep = {"method": "GET", "path": "/a", "file": "F", "line": 1, "access": {"protected": COND}}
        base = flow(extra={"endpoints": [ep]})
        ok = flow(extra={"endpoints": [ep], "service_layers": [layer("enforced", {"protected": COND}), layer("hint", "open")]})
        bad = flow(extra={"endpoints": [ep], "service_layers": [layer("enforced", "open")]})
        out = io.StringIO()
        self.assertEqual(run(base, ok, out), 0)
        self.assertIn("{'enforced': 1, 'hint': 1}", out.getvalue())
        self.assertEqual(run(base, bad), 1)


if __name__ == "__main__":
    unittest.main()
