#!/usr/bin/env python3
"""Check the built site's local links, fragments and image metadata."""
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

root = Path(sys.argv[1]).resolve()

class Document(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.path, self.ids, self.links = path, set(), []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if 'id' in values:
            assert values['id'] not in self.ids, f'{self.path}: duplicate id {values["id"]}'
            self.ids.add(values['id'])
        if tag == 'img':
            assert values.get('alt'), f'{self.path}: missing image description'
            assert values.get('width') and values.get('height'), f'{self.path}: missing image dimensions'
        if tag == 'video':
            assert values.get('aria-label'), f'{self.path}: missing video description'
            assert values.get('width') and values.get('height'), f'{self.path}: missing video dimensions'
        for attr in ('href', 'src', 'poster'):
            if values.get(attr):
                self.links.append(values[attr])

documents = {path.resolve(): Document(path) for path in root.rglob('*.html')}
assert documents, 'No HTML pages found'
checked = 0
for path, document in documents.items():
    assert '@@' not in path.read_text(), f'{path}: unresolved template token'
    for link in document.links:
        parsed = urlsplit(link)
        if parsed.scheme or parsed.netloc:
            assert parsed.scheme == 'https', f'{path}: unexpected external URL {link}'
            continue
        assert not parsed.path.startswith('/'), f'{path}: root-relative link breaks project Pages: {link}'
        target = (path.parent / unquote(parsed.path)).resolve() if parsed.path else path
        assert target.is_relative_to(root), f'{path}: link escapes published site: {link}'
        assert target.exists(), f'{path}: missing target {link}'
        if parsed.fragment and target in documents:
            assert unquote(parsed.fragment) in documents[target].ids, f'{path}: missing fragment {link}'
        checked += 1
for name in ('world', 'world-complexity', 'world-cbo', 'world-wmc', 'world-lcom4', 'world-package', 'graph', 'matrix', 'calls', 'quality', 'eshop-world', 'eshop-services', 'services'):
    assert (root / 'assets' / f'{name}.png').is_file(), f'Missing gallery screenshot: {name}'
print(f'Validated {len(documents)} HTML pages and {checked} local links/assets.')
