import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "hooks"))
import compact_metrics


class ArgvGuardTest(unittest.TestCase):
    """Ein Aufruf mit zu wenigen Argumenten nennt die Usage, nicht einen Traceback."""

    def guard(self, argv):
        old = sys.argv
        sys.argv = argv
        try:
            with self.assertRaises(SystemExit) as caught:
                compact_metrics.main()
            return caught.exception.code
        finally:
            sys.argv = old

    def test_too_few_arguments_name_the_usage(self):
        for argv in [["compact_metrics.py"],
                     ["compact_metrics.py", "archguard"],
                     ["compact_metrics.py", "archguard", "-"]]:
            with self.subTest(argv=argv):
                code = self.guard(argv)
                # Die Usage-Zeile selbst, nicht irgendein Absatz des Docstrings:
                # main() schneidet sie mit __doc__.split("\n\n")[1] heraus, und das
                # bricht still, wenn jemand den Docstring umformatiert.
                self.assertIn("compact_metrics.py <archguard>", code)
                self.assertIn("<head-tree|->", code)
                self.assertIn("<staged-tree>", code)


if __name__ == "__main__":
    unittest.main()
