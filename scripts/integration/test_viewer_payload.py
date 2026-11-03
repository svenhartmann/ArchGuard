"""HTML test readers accept compact and legacy compiler evidence losslessly."""
import json
import unittest
from viewer_payload import payload_from_html


class ViewerPayloadTest(unittest.TestCase):
    def test_compact_and_legacy(self):
        original = {"calls": {"status": "partial", "classes": [], "methods": [], "calls": [{"file": "ä.cs", "line": 5, "code": '</script><img>', "to_class": None}]}}
        compact = {"calls": {"status": "partial", "classes": [], "methods": [], "calls": [{"file": 0, "line": 5, "code": 1, "to_class": None}], "compact": {"encoding": "string-pool-v1", "strings": ["ä.cs", '</script><img>'], "fields": {"classes": [], "methods": [], "calls": ["file", "code", "to_class"]}}}}
        for data in [original, compact]:
            page = '<script id="archguard-data" type="application/json">' + json.dumps(data).replace('</', '<\\/') + '</script>'
            self.assertEqual(payload_from_html(page), original)

    def test_invalid_compact(self):
        for index in [-1, 2, True, 0.5]:
            model={"classes": [], "methods": [], "calls": [{"file":index}], "compact":{"encoding":"string-pool-v1", "strings":["A.cs"], "fields":{"classes":[],"methods":[],"calls":["file"]}}}
            with self.assertRaisesRegex(ValueError, 'pool'):
                payload_from_html('<script id="archguard-data" type="application/json">'+json.dumps({"calls":model})+'</script>')


if __name__ == '__main__':
    unittest.main()
