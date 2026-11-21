#!/usr/bin/env python3
"""Regression checks for static locale output and portable directory links."""
from html.parser import HTMLParser
import json
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parent


class Tables(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.rows = 0
        self.in_body = False
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        if tag == 'tbody':
            self.in_body = True
        if tag == 'tr' and self.in_body:
            self.rows += 1

    def handle_endtag(self, tag):
        if tag == 'tbody':
            self.in_body = False


class BuildTests(unittest.TestCase):
    def build(self, directory, repository=None):
        command = [sys.executable, str(SOURCE / 'build.py'), '--out', str(directory)]
        if repository:
            command += ['--repository', repository]
        subprocess.run(command, check=True, capture_output=True, text=True)

    def test_locales_inventory_and_repository_links(self):
        inventory = json.loads((SOURCE / 'data/languages.json').read_text())
        guides = len(list((SOURCE / 'docs').glob('*.html'))) - 1
        for repository in (None, 'example/archguard'):
            with self.subTest(repository=repository), tempfile.TemporaryDirectory() as directory:
                out = Path(directory) / 'public'
                self.build(out, repository)
                for locale, name in (('en', 'index.html'), ('de', 'de/index.html')):
                    text = (out / name).read_text()
                    self.assertIn(f'<html lang="{locale}"', text)
                    self.assertEqual(Tables(text).rows, len(inventory))
                    for language in inventory:
                        self.assertIn(language['name'], text)
                    self.assertNotIn('@@', text)
                    self.assertIn(f'{len(inventory)} ' + ('languages.' if locale == 'en' else 'Sprachen.'), text)
                    self.assertEqual('https://raw.githubusercontent.com/example/archguard/' in text, bool(repository))
                self.assertIn(f'Browse all {guides} guides', (out / 'index.html').read_text())
                for page in out.rglob('*.html'):
                    self.assertNotIn('@@', page.read_text(), str(page))
                    scripts = re.findall(r'<script src="([^"]*lightbox\.js)" defer></script>', page.read_text())
                    self.assertEqual(len(scripts), 1, str(page))
                    self.assertEqual((page.parent / scripts[0]).resolve(), (out / 'lightbox.js').resolve())
                self.assertTrue((out / 'lightbox.js').is_file())
                for private in ('data', 'media', 'test_build.py', 'README.md', 'build.py', 'check.py'):
                    self.assertFalse((out / private).exists(), private)

    def test_build_preserves_nonempty_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            sentinel = out / 'keep.txt'
            sentinel.write_text('keep')
            result = subprocess.run([sys.executable, str(SOURCE / 'build.py'), '--out', directory], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(sentinel.read_text(), 'keep')

    def test_directory_fragment_targets_index(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out / 'child').mkdir()
            (out / 'index.html').write_text('<a href="child/#present">Child</a>')
            (out / 'child/index.html').write_text('<h1 id="present">Present</h1>')
            (out / 'assets').mkdir()
            for image in (SOURCE / 'assets').glob('*.png'):
                (out / 'assets' / image.name).touch()
            command = [sys.executable, str(SOURCE / 'check.py'), directory]
            subprocess.run(command, check=True, capture_output=True)
            (out / 'index.html').write_text('<a href="child/#missing">Child</a>')
            failed = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn('missing fragment', failed.stderr)


if __name__ == '__main__':
    unittest.main()
