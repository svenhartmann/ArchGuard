"""Accounting regressions; no tokenizer download or external API needed."""
import unittest

from benchmark_context_tokens import count_commands, count_variant, savings, totals


class ContextTokenAccountingTest(unittest.TestCase):
    def test_commands_count_stderr_and_preserve_unicode(self):
        seen = []

        def count(text):
            seen.append(text)
            return len(text)

        self.assertEqual(count_commands([
            {'command': 'rg 架構', 'stdout': '😀\n', 'stderr': 'warning'}
        ], count), 14)
        self.assertEqual(seen, ['rg 架構', '😀\n', 'warning'])

    def test_all_pages_and_identical_source_read_are_counted(self):
        run = {'complete': True, 'error': None, 'calls': [
            {'arguments_text': '{}', 'response_text': '{ "x": "😀" }'},
            {'arguments_text': '{"offset":1}', 'response_text': '{}'},
        ]}
        result = count_variant(run, target_read=10, count=len)
        self.assertEqual(result['calls'], 2)
        self.assertEqual(result['arguments'], 14)
        self.assertEqual(result['responses'], 14)
        self.assertEqual(result['total'], 38)

    def test_incomplete_capture_is_not_reported_as_savings(self):
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            count_variant({'complete': False, 'calls': []}, 0, len)

    def test_savings_retain_negative_values(self):
        self.assertEqual(savings(100, 150), -50.0)
        self.assertEqual(savings(100, 75), 25.0)
        self.assertIsNone(savings(0, 0))

    def test_schema_is_added_once_and_percentages_use_totals(self):
        rows = [{'baseline': 100, 'default': {'total': 150}, 'large': {'total': 80}},
                {'baseline': 900, 'default': {'total': 950}, 'large': {'total': 820}}]
        result = totals(rows, single_schema=20, catalog_schema=200)
        self.assertEqual(result['baseline'], 1000)
        self.assertEqual(result['large']['total'], 900)
        self.assertEqual(result['large']['with_single_schema'], 920)
        self.assertEqual(result['large']['with_catalog'], 1100)
        self.assertEqual(result['large']['saving_percent'], 10.0)
        self.assertEqual(result['default']['saving_percent'], -10.0)


if __name__ == '__main__':
    unittest.main()
