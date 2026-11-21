"""Shared handbook layout and a static, section-level search index."""
from html import escape
from html.parser import HTMLParser
import json
import re


class PlainText(HTMLParser):
    def __init__(self, markup):
        super().__init__()
        self.parts = []
        self.feed(markup)

    def handle_data(self, data):
        self.parts.append(data)

    def text(self):
        return ' '.join(' '.join(self.parts).split())


def plain(markup):
    return PlainText(markup).text()


def prepare_docs(out, source):
    groups = json.loads((source / 'data/docs-navigation.json').read_text())
    pages = [page for group in groups for page in group['pages']]
    expected = {'index', *(page['slug'] for page in pages)}
    for locale in ('en', 'de'):
        directory = out / ('de/docs' if locale == 'de' else 'docs')
        actual = {path.stem for path in directory.glob('*.html')}
        if actual != expected:
            raise ValueError(f'{locale} handbook/navigation mismatch: {actual ^ expected}')
        prefix = '../../' if locale == 'de' else '../'
        home = 'Dokumentation' if locale == 'de' else 'Documentation'
        search_label = 'Dokumentation durchsuchen' if locale == 'de' else 'Search documentation'
        contents = 'Auf dieser Seite' if locale == 'de' else 'On this page'
        for path in sorted(directory.glob('*.html')):
            text = path.read_text()
            match = re.search(r'<main\b[^>]*>(.*?)</main>', text, re.S)
            if not match:
                raise ValueError(f'Missing main content in {path}')
            body = match[1]
            body = re.sub(r'<nav\b[^>]*class="docs-toc".*?</nav>', '', body, flags=re.S)
            body = re.sub(r'<div class="docs-next">.*?</div>', '', body, flags=re.S)
            title = plain(re.search(r'<h1\b[^>]*>(.*?)</h1>', body, re.S)[1])
            ids = set(re.findall(r'\bid="([^"]+)"', body))

            def add_id(heading):
                attrs, label = heading.groups()
                if re.search(r'\bid=', attrs):
                    return heading[0]
                slug = re.sub(r'[^\w-]+', '-', plain(label).lower()).strip('-') or 'section'
                unique = slug
                suffix = 2
                while unique in ids:
                    unique = f'{slug}-{suffix}'
                    suffix += 1
                ids.add(unique)
                return f'<h2{attrs} id="{unique}">{label}</h2>'

            body = re.sub(r'<h2([^>]*)>(.*?)</h2>', add_id, body, flags=re.S)
            headings = list(re.finditer(r'<h2\b[^>]*id="([^"]+)"[^>]*>(.*?)</h2>', body, re.S))
            toc = ''
            if path.stem != 'index':
                toc = f'<nav class="docs-toc" aria-label="{contents}"><strong>{contents}</strong>'
                toc += ''.join(f'<a href="#{h[1]}">{escape(plain(h[2]))}</a>' for h in headings)
                toc += '</nav>'
                # Lead/title precede the table of contents, instructions follow it.
                pos = headings[0].start() if headings else len(body)
                body = body[:pos] + toc + body[pos:]
            navigation = ''
            for group in groups:
                navigation += f'<div class="docs-nav-group"><h2>{escape(group[locale])}</h2>'
                for item in group['pages']:
                    active = ' aria-current="page"' if item['slug'] == path.stem else ''
                    navigation += f'<a href="{item["slug"]}.html"{active}>{escape(item[locale])}</a>'
                navigation += '</div>'
            next_links = f'<a href="index.html">{home}</a>'
            if path.stem != 'index':
                i = next(i for i, page in enumerate(pages) if page['slug'] == path.stem)
                if i:
                    item = pages[i - 1]
                    label = 'Zurück' if locale == 'de' else 'Previous'
                    next_links = f'<a href="{item["slug"]}.html">{label}: {escape(item[locale])}</a>'
                if i + 1 < len(pages):
                    item = pages[i + 1]
                    label = 'Weiter' if locale == 'de' else 'Next'
                    next_links += f'<a href="{item["slug"]}.html">{label}: {escape(item[locale])}</a>'
            other = 'en' if locale == 'de' else 'de'
            counterpart = f'../../docs/{path.name}' if locale == 'de' else f'../de/docs/{path.name}'
            head = re.search(r'<head>(.*?)</head>', text, re.S)[1]
            if 'docs.js' not in head:
                head += f'<script src="{prefix}docs.js" defer></script>'
            placeholder = 'z. B. Berechtigungen, OTLP, arc42' if locale == 'de' else 'e.g. permissions, OTLP, arc42'
            hint = 'Suche in Anleitungen, Befehlen und Beispielen.' if locale == 'de' else 'Search guides, commands and examples.'
            text = f'''<!doctype html>
<html lang="{locale}"><head>{head}</head><body class="handbook">
<a class="skip" href="#main">{'Zum Inhalt' if locale == 'de' else 'Skip to content'}</a>
<header class="header wrap"><a class="brand" href="../index.html"><span class="mark" aria-hidden="true">A<span>G</span></span> ArchGuard</a><nav aria-label="{'Hauptnavigation' if locale == 'de' else 'Main navigation'}"><a href="../index.html">Website</a><a class="nav-docs" href="index.html">{home}</a><a href="{counterpart}" lang="{other}" hreflang="{other}">{other.upper()}</a></nav></header>
<div class="docs-layout wrap"><aside class="docs-sidebar"><a class="docs-home" href="index.html">{home}</a><details class="docs-navigation" open><summary>{'Kapitel' if locale == 'de' else 'Chapters'}</summary><nav aria-label="{home}">{navigation}</nav></details></aside>
<div class="docs-content"><div class="docs-search"><label for="docs-search">{search_label}</label><input id="docs-search" type="search" placeholder="{placeholder}" autocomplete="off" aria-controls="docs-search-results" aria-describedby="docs-search-status"><p id="docs-search-status" role="status" aria-live="polite">{hint}</p><ol id="docs-search-results" hidden></ol></div>
<main id="main" class="docs-article"><div class="docs-breadcrumb"><a href="index.html">{home}</a><span aria-hidden="true">/</span><span>{escape(title)}</span></div>{body}<nav class="docs-next" aria-label="{'Weiterlesen' if locale == 'de' else 'Continue reading'}">{next_links}</nav></main></div></div>
<footer class="wrap footer"><a class="brand" href="../index.html">ArchGuard</a><p>MIT OR Apache-2.0</p><a href="../about.html">{'Bildnachweise' if locale == 'de' else 'Image sources'}</a><a href="#main">{'Nach oben' if locale == 'de' else 'Back to top'}</a></footer><div id="copy-status" role="status" aria-live="polite"></div></body></html>'''
            path.write_text(text)


def index_docs(out):
    """Index final rendered articles, after conditional repository blocks resolve."""
    for relative in ('docs', 'de/docs'):
        directory = out / relative
        records = []
        for path in sorted(directory.glob('*.html')):
            text = path.read_text()
            article = re.search(r'<main\b[^>]*>(.*?)</main>', text, re.S)[1]
            article = re.sub(r'<nav\b.*?</nav>', '', article, flags=re.S)
            article = re.sub(r'<div class="docs-breadcrumb">.*?</div>', '', article, flags=re.S)
            title = plain(re.search(r'<h1\b[^>]*>(.*?)</h1>', article, re.S)[1])
            parts = list(re.finditer(r'<h2\b[^>]*id="([^"]+)"[^>]*>(.*?)</h2>', article, re.S))
            lead_end = parts[0].start() if parts else len(article)
            records.append({'title': title, 'heading': title, 'url': path.name,
                            'text': plain(article[:lead_end])})
            for i, heading in enumerate(parts):
                end = parts[i + 1].start() if i + 1 < len(parts) else len(article)
                records.append({'title': title, 'heading': plain(heading[2]),
                                'url': f'{path.name}#{heading[1]}',
                                'text': plain(article[heading.end():end])})
        (directory / 'search-index.json').write_text(json.dumps(records, ensure_ascii=False) + '\n')
