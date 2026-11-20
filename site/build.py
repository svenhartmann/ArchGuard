#!/usr/bin/env python3
"""Build the static Pages artifact without external dependencies."""
import argparse
from html import escape
import json
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
for name in ('index.html', 'about.html', 'styles.css', 'site.js', 'assets', 'examples', 'docs', 'docs.js',
             'de', 'for', 'compare', 'stories', 'languages'):
    item = source / name
    if not item.exists():
        continue
    if item.is_dir():
        shutil.copytree(item, out / name)
    else:
        shutil.copy2(item, out / name)
# Repository links and the release install commands appear only when the repository
# is known; otherwise they are removed and <!--norepo--> blocks (source build) stay.
keep, drop = ('repo', 'norepo') if args.repository else ('norepo', 'repo')
languages = json.loads((source / 'data/languages.json').read_text())
assert languages and len({row['name'] for row in languages}) == len(languages)
guide_count = len(list((source / 'docs').glob('*.html'))) - 1


def language_table(locale):
    headers = ('Language', 'Imports', 'Analysis and limits', 'Required tools') if locale == 'en' else (
        'Sprache', 'Imports', 'Analyse und Grenzen', 'Benötigte Werkzeuge')
    included = 'Included' if locale == 'en' else 'Enthalten'
    head = ''.join(f'<th scope="col">{label}</th>' for label in headers)
    rows = ''.join('<tr><th scope="row">' + escape(row['name']) + '</th><td>' + included +
                   '</td><td>' + escape(row[locale]['analysis']) + '</td><td>' +
                   escape(row[locale]['tools']) + '</td></tr>' for row in languages)
    return f'<div class="table-wrap"><table><thead><tr>{head}</tr></thead><tbody>{rows}</tbody></table></div>'


for page in out.rglob('*.html'):
    text = page.read_text()
    locale = 'de' if '<html lang="de"' in text else 'en'
    text = text.replace('@@LANGUAGE_COUNT@@', str(len(languages)))
    text = text.replace('@@LANGUAGE_TABLE@@', language_table(locale))
    text = text.replace('@@GUIDE_COUNT@@', str(guide_count))
    text = re.sub(rf'<!--{drop}-->.*?<!--/{drop}-->', '', text, flags=re.S)
    text = re.sub(rf'<!--/?{keep}-->', '', text)
    if args.repository:
        text = text.replace('href="@@REPOSITORY_URL@@" hidden', 'href="https://github.com/' + args.repository + '"')
        text = text.replace('@@REPOSITORY@@', args.repository)
    else:
        text = re.sub(r'<a class="[^"]*repo-link[^"]*" href="@@REPOSITORY_URL@@" hidden>.*?</a>', '', text)
    evidence = 'Fehlende Evidenz ist keine Null.' if locale == 'de' else 'Missing evidence is not a zero.'
    if evidence not in text:
        text = text.replace('</footer>', f'<p class="evidence-motto">{evidence}</p></footer>')
    page.write_text(text)
(out / '.nojekyll').touch()
print(f'Built static site: {out}')
