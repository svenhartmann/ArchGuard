#!/usr/bin/env python3
"""Verify that published captures match their recorded provenance and pages."""
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import struct
from urllib.parse import urlsplit


SITE = Path(__file__).resolve().parents[2]
manifest = json.loads((SITE / "assets/marketing/provenance.json").read_text())
projects = {p["id"]: p for p in manifest["projects"]}
for project in projects.values():
    assert len(project["commit"]) == 40
    assert len(project["license_sha256"]) == 64
    assert project["repository"].startswith("https://github.com/")
for capture in manifest["captures"]:
    assert capture["project"] in projects
    path = SITE / capture["asset"]
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    assert hashlib.sha256(data).hexdigest() == capture["sha256"], path
    assert struct.unpack(">II", data[16:24]) == (capture["width"], capture["height"])


class Gallery(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.path, self.images = path, 0
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "img":
            self.images += 1
            assert attrs.get("alt") and attrs.get("width") and attrs.get("height")
        for key in ("src", "href"):
            if key not in attrs:
                continue
            url = urlsplit(attrs[key])
            if url.scheme:
                assert url.scheme == "https"
            elif url.path:
                assert not url.path.startswith("/")
                destination = (self.path.parent / url.path).resolve()
                assert destination.is_relative_to(SITE)
                # German navigation destinations are supplied by the independent website ticket.
                integration_pages = {'de/index.html', 'de/about.html', 'de/docs/languages.html'}
                if str(destination.relative_to(SITE)) in integration_pages:
                    continue
                assert destination.is_file(), (self.path, attrs[key])


for page in (SITE / "languages/index.html", SITE / "de/languages/index.html"):
    assert Gallery(page).images == 24
    assert "/private/" not in page.read_text() and "/Users/" not in page.read_text()
session = json.loads((SITE / "assets/marketing/mcp-session.json").read_text())
assert session["commit"] == projects["commons-collections"]["commit"]
assert len(session["exchanges"]) == 3
for exchange in session["exchanges"]:
    assert "error" not in exchange["response"]
    assert not exchange["response"]["result"].get("isError")
review = json.loads((SITE / "assets/marketing/petclinic-pr-comment.json").read_text())
assert review["base"] == review["target_source_commit"] == projects["petclinic"]["commit"]
assert review["tracked_source_changes"] == []
assert review["policy"] == projects["petclinic"]["configuration"]
assert hashlib.sha256(review["policy"].encode()).hexdigest() == review["policy_sha256"]
for path in (SITE / "assets/marketing").iterdir():
    if path.suffix in {".json", ".md"}:
        assert "/Users/" not in path.read_text() and "/private/" not in path.read_text(), path
print(f"Verified {len(manifest['captures'])} real PNGs, their provenance, MCP exchange and bilingual gallery.")
