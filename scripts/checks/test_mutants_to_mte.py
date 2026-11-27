import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import mutants_to_mte


def outcome(summary, name, file, span, replacement="0", package="probe"):
    return {
        "scenario": {"Mutant": {"name": name, "package": package, "file": file,
                                "span": span, "replacement": replacement,
                                "genre": "FnValue"}},
        "summary": summary,
        "phase_results": [{"phase": "Build", "duration": 0.5},
                          {"phase": "Test", "duration": 0.25}],
    }


class ConvertTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "src").mkdir()
        # Zeile 1: das Emoji kostet 2 UTF-16-Einheiten, verschiebt also die Spalten.
        (self.root / "src" / "lib.rs").write_text(
            "fn f() -> i32 { \U0001F600 + 1 }\nfn g() {}\n", encoding="utf-8")
        self.addCleanup(self.tmp.cleanup)
        self.pkg = {"probe": self.root}

    def span(self, line, col_start, col_end):
        return {"start": {"line": line, "column": col_start},
                "end": {"line": line, "column": col_end}}

    def convert(self, outcomes):
        return mutants_to_mte.convert({"outcomes": outcomes,
                                       "cargo_mutants_version": "27.1.0"},
                                      self.root, self.pkg)

    def test_status_mapping_and_grouping(self):
        report = self.convert([
            outcome("CaughtMutant", "src/lib.rs:1:15: replace with 0",
                    "src/lib.rs", self.span(1, 15, 16)),
            outcome("UncaughtMutant", "src/lib.rs:1:19: replace with 0",
                    "src/lib.rs", self.span(1, 19, 20)),
            outcome("Unviable", "src/lib.rs:2:10: replace with 0",
                    "src/lib.rs", self.span(2, 10, 11)),
            outcome("Timeout", "src/lib.rs:2:7: replace with 0",
                    "src/lib.rs", self.span(2, 7, 8)),
            outcome("MissingCoverage", "src/lib.rs:2:1: replace with 0",
                    "src/lib.rs", self.span(2, 1, 2)),
        ])
        mutants = report["files"]["src/lib.rs"]["mutants"]
        self.assertEqual([m["status"] for m in mutants],
                         ["Killed", "Survived", "CompileError", "Timeout", "NoCoverage"])
        self.assertEqual(report["schemaVersion"], "2")
        self.assertEqual(report["framework"]["version"], "27.1.0")
        self.assertEqual(mutants[0]["duration"], 750.0)
        self.assertEqual(report["files"]["src/lib.rs"]["language"], "rust")

    def test_utf16_columns_and_source_inline(self):
        # Das Emoji steht als Zeichen 17; Spalten dahinter ruecken um eine UTF-16-Einheit.
        report = self.convert([
            outcome("CaughtMutant", "src/lib.rs:1:21: replace 1 with 0",
                    "src/lib.rs", self.span(1, 21, 22)),
        ])
        mutant = report["files"]["src/lib.rs"]["mutants"][0]
        self.assertEqual(mutant["location"]["start"]["column"], 22)
        self.assertEqual(mutant["location"]["end"]["column"], 23)
        line = report["files"]["src/lib.rs"]["source"].split("\n")[0]
        col = mutant["location"]["start"]["column"]
        encoded = line.encode("utf-16-le")
        self.assertEqual(chr(int.from_bytes(encoded[(col - 1) * 2:col * 2], "little")), "1")

    def test_unknown_summary_and_baseline_are_skipped(self):
        report = self.convert([
            {"scenario": "Baseline", "summary": "Success"},
            outcome("WeirdResult", "src/lib.rs:2:7: replace with 0",
                    "src/lib.rs", self.span(2, 7, 8)),
        ])
        self.assertEqual(report["files"], {})

    def test_span_outside_source_is_skipped(self):
        report = self.convert([
            outcome("CaughtMutant", "src/lib.rs:9:1: replace with 0",
                    "src/lib.rs", self.span(9, 1, 2)),
        ])
        self.assertEqual(report["files"], {})


if __name__ == "__main__":
    unittest.main()
