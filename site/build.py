#!/usr/bin/env python3
"""Build the static Pages artifact without external dependencies."""
import argparse
import re
import shutil
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out', required=True, type=Path)
parser.add_argument('--repository', default='', help='GitHub owner/repository for the source link')
args = parser.parse_args()
source = Path(__file__).resolve().parent
out = args.out.resolve()
if out == source or source.is_relative_to(out) or out.is_relative_to(source):
    parser.error('output must be outside the site source directory and its parents')
if args.repository and not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repository):
    parser.error('repository must have the form owner/repository')
if out.exists() and any(out.iterdir()):
    parser.error('output directory must be empty; choose a fresh directory')
out.mkdir(parents=True, exist_ok=True)
for name in ('index.html', 'about.html', 'styles.css', 'site.js', 'assets', 'examples', 'docs', 'docs.js'):
    item = source / name
    if item.is_dir():
        shutil.copytree(item, out / name)
    else:
        shutil.copy2(item, out / name)
# Repository links appear only when the repository is known; otherwise they are removed.
for page in out.rglob('*.html'):
    text = page.read_text()
    if args.repository:
        text = text.replace('href="@@REPOSITORY_URL@@" hidden', 'href="https://github.com/' + args.repository + '"')
    else:
        text = re.sub(r'<a class="[^"]*repo-link[^"]*" href="@@REPOSITORY_URL@@" hidden>.*?</a>', '', text)
    page.write_text(text)
(out / '.nojekyll').touch()
print(f'Built static site: {out}')
