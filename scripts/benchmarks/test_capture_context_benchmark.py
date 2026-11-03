import importlib.util
import json
from pathlib import Path
import unittest
import tempfile

SOURCE = Path(__file__).resolve().with_name('capture_context_benchmark.py')
spec = importlib.util.spec_from_file_location('capture_context_benchmark', SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def page(offset, items, next_offset, omissions=(), revision='a' * 64, total=3):
    return json.dumps({
        'revision': revision, 'total': total, 'offset': offset,
        'items': items, 'next_offset': next_offset,
        'omitted_oversized': len(omissions), 'omissions': list(omissions),
        'omitted_metadata': [], 'returned': len(items),
        'processed': len(items) + len(omissions),
        'semantic_status': 'partial',
    })


class CaptureTests(unittest.TestCase):
    def test_manifest_uses_physical_file_not_logical_reference_alias(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            (root / 'source.ts').write_text('export const n = 1;')
            result = module.source_manifest(root, [{'kind': 'dependency', 'path': [
                {'evidence': [{'file': 'source.ts', 'reference_file': 'alias.ts'}]}
            ]}])
            self.assertEqual([row['file'] for row in result], ['source.ts'])

    def test_pages_are_all_counted_and_revision_bound(self):
        requests = []
        def call(arguments):
            requests.append(arguments.copy())
            if arguments['offset'] == 0:
                return page(0, [{'kind': 'seed'}], 1)
            self.assertEqual(arguments['expected_revision'], 'a' * 64)
            return page(1, [{'kind': 'dependency'}, {'kind': 'coverage'}], None)
        result = module.collect_pages(call, 'src/a.ts', 4096)
        self.assertTrue(result['complete'])
        self.assertEqual(len(result['calls']), 2)
        self.assertEqual(len(result['items']), 3)
        self.assertEqual([x['offset'] for x in requests], [0, 1])

    def test_omission_retries_same_offset_and_counts_failed_response(self):
        requests = []
        def call(arguments):
            requests.append(arguments.copy())
            if arguments['max_bytes'] == 4096:
                return page(0, [], 1, [{'index': 0, 'bytes': 5000}])
            self.assertEqual(arguments['expected_revision'], 'a' * 64)
            return page(0, [{'kind': 'seed'}, {'kind': 'cycle'}, {'kind': 'coverage'}], None)
        result = module.collect_pages(call, 'src/a.ts', 4096)
        self.assertTrue(result['complete'])
        self.assertEqual(len(result['calls']), 2)
        self.assertEqual([x['offset'] for x in requests], [0, 0])
        self.assertEqual([x['max_bytes'] for x in requests], [4096, 16384])
        self.assertEqual(len(result['items']), 3)

    def test_unrecoverable_omission_is_never_complete(self):
        result = module.collect_pages(
            lambda args: page(0, [], 1, [{'index': 0, 'bytes': 20000}]),
            'src/a.ts', 4096,
        )
        self.assertFalse(result['complete'])
        self.assertEqual(len(result['calls']), 2)
        self.assertIn('omission', result['error'])

    def test_changed_revision_does_not_mix_context(self):
        calls = []
        def call(arguments):
            calls.append(arguments)
            return (page(0, [{'kind': 'seed'}], 1) if len(calls) == 1
                    else page(1, [{'kind': 'dependency'}], None, revision='b' * 64))
        result = module.collect_pages(call, 'src/a.ts', 4096)
        self.assertFalse(result['complete'])
        self.assertIn('revision', result['error'])
        self.assertEqual(len(result['calls']), 2)

    def test_nonadvancing_cursor_is_reported_instead_of_looping(self):
        result = module.collect_pages(
            lambda args: page(0, [{'kind': 'seed'}], 0), 'src/a.ts', 4096,
        )
        self.assertFalse(result['complete'])
        self.assertIn('cursor', result['error'])


if __name__ == '__main__':
    unittest.main()
