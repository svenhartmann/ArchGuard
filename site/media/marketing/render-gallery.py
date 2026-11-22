#!/usr/bin/env python3
"""Render the bilingual gallery from captured files and pinned provenance."""
import hashlib
from html import escape
import json
from pathlib import Path
import struct


HERE = Path(__file__).resolve().parent
SITE = HERE.parents[1]
LANGUAGES = [
    ("kafka", "Java", "Java repository overview with a clients source configuration; external references remain partial.", "Java-Repository-Übersicht mit einer Client-Quellkonfiguration; externe Referenzen bleiben teilweise offen."),
    ("eshop", "C#", "C# repository overview. Packages were not restored for these captures; compiler evidence can be incomplete.", "C#-Repository-Übersicht. Pakete wurden für diese Aufnahmen nicht wiederhergestellt; Compilerbelege können unvollständig sein."),
    ("nest", "TypeScript / JavaScript", "Workspace overview, including discovered sample projects, without installing the project's compiler. Grey values are unavailable measurements.", "Workspace-Übersicht einschließlich erkannter Beispielprojekte, ohne Installation des Projekt-Compilers. Graue Werte sind fehlende Messungen."),
    ("prometheus", "Go", "Imports are available. This capture environment has no Go SDK, so compiler metrics stay unknown.", "Imports sind verfügbar. In der Aufnahmeumgebung fehlt das Go-SDK; Compiler-Metriken bleiben unbekannt."),
    ("ripgrep", "Rust", "Workspace crates, syntax metrics. No rust-analyzer or project-compiler opt-in.", "Workspace-Crates und Syntaxmetriken. Kein rust-analyzer und keine Freigabe des Projekt-Compilers."),
    ("django", "Python", "Django package sources, parsed locally without importing the project.", "Django-Paketquellen, lokal geparst, ohne das Projekt zu importieren."),
    ("symfony", "PHP", "Symfony repository overview. Statically bound calls and syntax metrics; dynamic receivers remain unresolved.", "Symfony-Repository-Übersicht. Statisch gebundene Aufrufe und Syntaxmetriken; dynamische Empfänger bleiben offen."),
    ("okhttp", "Kotlin", "Kotlin workspace overview, including discovered build and sample modules. Gradle is not run; dynamic calls remain gaps.", "Kotlin-Workspace-Übersicht einschließlich erkannter Build- und Beispielmodule. Gradle wird nicht ausgeführt; dynamische Aufrufe bleiben Lücken."),
    ("leveldb", "C / C++", "Source and include relationships. Missing include paths remain unresolved; no preprocessor or compiler runs.", "Quellen und Include-Beziehungen. Fehlende Include-Pfade bleiben offen; kein Präprozessor oder Compiler wird ausgeführt."),
    ("swift-nio", "Swift", "Swift Package Manager repository overview, including discovered integration and benchmark targets. Xcode projects are outside this analysis.", "Swift-Package-Manager-Repository-Übersicht einschließlich erkannter Integrations- und Benchmark-Targets. Xcode-Projekte gehören nicht zu dieser Analyse."),
    ("mastodon", "Ruby", "Mastodon repository overview, including its Ruby app and lib plus discovered frontend scopes. Dynamic Ruby dispatch is not complete runtime coverage.", "Mastodon-Repository-Übersicht mit Ruby-Quellen aus app und lib sowie erkannten Frontend-Bereichen. Dynamischer Ruby-Dispatch wird nicht vollständig als Laufzeitverhalten erfasst."),
    ("pekko", "Scala", "Scala repository overview with an actor source configuration and additional discovered scopes; implicit and cross-language calls remain unresolved.", "Scala-Repository-Übersicht mit einer actor-Quellkonfiguration und weiteren erkannten Bereichen; implizite und sprachübergreifende Aufrufe bleiben offen."),
]


def main():
    manifest = json.loads((HERE / "sources.json").read_text())
    projects = {p["id"]: p for p in manifest["projects"]}
    definitions = json.loads((HERE / "captures.json").read_text())
    recorded = []
    for definition in definitions:
        target = SITE / "assets" / "marketing" / (definition["id"] + ".png")
        data = target.read_bytes()
        if data[:8] != b"\x89PNG\r\n\x1a\n":
            raise ValueError(f"Not a real PNG: {target}")
        width, height = struct.unpack(">II", data[16:24])
        recorded.append({**definition, "asset": "assets/marketing/" + target.name,
                         "width": width, "height": height,
                         "sha256": hashlib.sha256(data).hexdigest()})
    manifest["captures"] = recorded
    (SITE / "assets" / "marketing" / "provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
    images = {c["id"]: c for c in recorded}
    for lang in ("en", "de"):
        de = lang == "de"
        base = "../../" if de else "../"
        home = "../index.html"
        title = "Sprachen mit echten Belegen" if de else "Languages with real evidence"
        intro = ("World View und Matrix aus frisch geklonten Open-Source-Projekten. Jede Aufnahme nennt ihren Quellstand und ihre Grenzen."
                 if de else "World View and matrix captures from freshly cloned open-source projects. Every capture names its source revision and its limits.")
        counterpart = "../../languages/index.html" if de else "../de/languages/index.html"
        parts = [f'''<!doctype html>
<html lang="{lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="description" content="{intro}"><title>{title} — ArchGuard</title><link rel="icon" href="{base}assets/favicon.svg" type="image/svg+xml"><link rel="stylesheet" href="{base}styles.css"><link rel="stylesheet" href="{base}languages/gallery.css"><link rel="alternate" hreflang="{'en' if de else 'de'}" href="{counterpart}"></head><body>
<a class="skip" href="#main">{'Zum Inhalt' if de else 'Skip to content'}</a><header class="header wrap"><a class="brand" href="{home}"><span class="mark" aria-hidden="true">A<span>G</span></span> ArchGuard</a><nav aria-label="{'Hauptnavigation' if de else 'Main navigation'}"><a href="../docs/languages.html">{'Sprachumfang' if de else 'Language scope'}</a><a class="language-switch" href="{counterpart}" lang="{'en' if de else 'de'}" hreflang="{'en' if de else 'de'}">{'English' if de else 'Deutsch'}</a><a class="nav-cta" href="{home}#start">{'Loslegen' if de else 'Get started'}</a></nav></header>
<main id="main" class="wrap language-gallery"><div class="eyebrow">{'QUELLEN · UMFANG · GRENZEN' if de else 'SOURCES · SCOPE · LIMITS'}</div><h1>{title}</h1><p class="gallery-intro">{intro}</p>
<div class="evidence-note"><p><strong>{'Fehlende Evidenz ist keine Null.' if de else 'Missing evidence is not a zero.'}</strong> {'Ein Sprachbeispiel belegt keinen vollständigen Compiler- oder Laufzeitsupport. Graue Flächen bleiben unbekannt; die Matrix zeigt statische Abhängigkeiten.' if de else 'A language example does not establish complete compiler or runtime support. Grey surfaces stay unknown; the matrix shows static dependencies.'}</p></div>
<p class="capture-source">{'Die Desktop-Übersicht kann weitere erkannte Projektbereiche einbeziehen; angeforderte Quellfilter stehen in der Provenienz. Paketzyklen in der World View und Dateizyklen in der Matrix haben unterschiedliche Aggregation. Vergleiche nur denselben Umfang und dieselbe Ebene.' if de else 'The desktop overview can include additional discovered project scopes; requested source filters are recorded in the provenance. World View package cycles and matrix file cycles use different aggregation. Compare the same scope and granularity.'}</p>
<nav class="language-jump" aria-label="{'Sprachen' if de else 'Languages'}">''']
        parts.append(''.join(f'<a href="#{slug}">{escape(label)}</a>' for slug, label, _, _ in LANGUAGES))
        parts.append('</nav>')
        for slug, label, note_en, note_de in LANGUAGES:
            p = projects[slug]
            source = p['repository'] + '/tree/' + p['commit']
            license_url = p['repository'] + '/blob/' + p['commit'] + '/' + p['license_path']
            parts.append(f'<section id="{slug}" class="language-example"><div class="language-title"><div class="eyebrow">{escape(label)}</div><h2>{escape(p["name"])}</h2></div><p>{escape(note_de if de else note_en)}</p><p class="capture-source"><a href="{source}">{"Quellstand" if de else "Revision"} <code>{p["commit"][:12]}</code></a> · <a href="{license_url}">{escape(p["license"])}</a> · {manifest["capture_date"]}</p><div class="capture-pair">')
            for view in ('world', 'matrix'):
                image = images[slug + '-' + view]
                caption = 'World View · Complexity' if view == 'world' else ('Abhängigkeitsmatrix' if de else 'Dependency matrix')
                alt = f'{p["name"]}: {caption}; ' + ('echte ArchGuard-Desktop-Aufnahme' if de else 'actual ArchGuard desktop capture')
                parts.append(f'<figure><a href="{base}{image["asset"]}"><img src="{base}{image["asset"]}" width="{image["width"]}" height="{image["height"]}" loading="lazy" alt="{escape(alt)}"></a><figcaption>{caption} · {"Für volle Auflösung öffnen" if de else "Open at full resolution"}</figcaption></figure>')
            parts.append('</div></section>')
        parts.append(f'''<section class="capture-method"><h2>{'Aufnahmen nachvollziehen' if de else 'Reproduce the captures'}</h2><p>{'Die Rohbilder sind unveränderte Offscreen-Renderings der Desktop-App. Der unten verlinkte Nachweis enthält vollständige Commit-IDs, Lizenz-Prüfsummen, Quellfilter und Aufnahmeparameter. Die Quellen wurden nicht gebaut und es wurden keine Abhängigkeiten installiert.' if de else 'The raw images are unchanged offscreen renders of the desktop app. The provenance below records full commit IDs, license checksums, source filters and capture parameters. The projects were not built and their dependencies were not installed.'}</p><p><a href="{base}assets/marketing/provenance.json">{'Provenienz und Aufnahmeparameter (JSON)' if de else 'Provenance and capture parameters (JSON)'}</a> · <a href="../about.html">{'Über die Beispiele' if de else 'About these examples'}</a></p></section></main><footer class="wrap footer"><a class="brand" href="{home}">ArchGuard</a><p>{'Nichtkommerzielles Open-Source-Projekt.' if de else 'Non-commercial open-source project.'}<br>MIT OR Apache-2.0</p><a href="#main">{'Nach oben' if de else 'Back to top'}</a></footer></body></html>''')
        destination = SITE / ('de/languages' if de else 'languages')
        destination.mkdir(parents=True, exist_ok=True)
        (destination / 'index.html').write_text('\n'.join(parts) + '\n')


if __name__ == '__main__':
    main()
