//! Checks that `tab_support::Texts` keys used in Rust exist in
//! `ui/texts.slint` and have a German translation.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

fn crate_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// The string literals in `text`, unescaped (`\"`, `\\`).
fn literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => s.extend(chars.next()),
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

/// `{` and `}` escaped as Slint's `@tr` format strings need them.
pub fn braced(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}

pub fn unbraced(text: &str) -> String {
    text.replace("{{", "{").replace("}}", "}")
}

/// `(key, @tr text)` of every `ui/texts.slint` entry.
fn template_entries() -> Vec<(String, String)> {
    let slint = std::fs::read_to_string(crate_file("../archguard-desktop-ui/ui/texts.slint")).expect("texts.slint");
    slint
        .lines()
        .filter(|l| l.trim_start().starts_with("{ key:"))
        .map(|l| {
            let mut lit = literals(l).into_iter();
            (
                lit.next().unwrap_or_default(),
                lit.next().unwrap_or_default(),
            )
        })
        .collect()
}

pub fn template_keys() -> Vec<String> {
    template_entries().into_iter().map(|(k, _)| k).collect()
}

/// msgid → msgstr of all German parts, without context entries.
pub fn po_de() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let dir = std::fs::read_dir(crate_file("../archguard-desktop-ui/lang/de")).expect("lang/de");
    for part in dir.flatten() {
        let text = std::fs::read_to_string(part.path()).expect("po part");
        let (mut context, mut id) = (false, None);
        for line in text.lines() {
            if line.starts_with("msgctxt ") {
                context = true;
            } else if let Some(rest) = line.strip_prefix("msgid ") {
                id = literals(rest).pop();
            } else if let Some(rest) = line.strip_prefix("msgstr ") {
                if let (false, Some(id)) = (context, id.take()) {
                    out.insert(id, literals(rest).pop().unwrap_or_default());
                }
                context = false;
            }
        }
    }
    out
}

/// The crate's Rust files, without this one (its scanner names the calls).
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("src dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("texts_check.rs") {
            out.push(p);
        }
    }
}

/// Literal keys passed to `lang.t/f/plural` or `texts.t/f/plural`.
fn used_keys() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_sources(&crate_file("src"), &mut files);
    let mut keys = BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("source");
        for call in ["lang.t(", "lang.f(", "texts.t(", "texts.f("] {
            for (at, _) in text.match_indices(call) {
                let rest = text[at + call.len()..].trim_start();
                if rest.starts_with('"') {
                    keys.extend(literals(rest).into_iter().take(1));
                }
            }
        }
        for call in ["lang.plural(", "texts.plural("] {
            for (at, _) in text.match_indices(call) {
                let rest = &text[at + call.len()..];
                let args = &rest[..rest.find(')').unwrap_or(rest.len())];
                keys.extend(literals(args).into_iter().take(2));
            }
        }
    }
    keys
}

#[test]
fn every_key_used_in_rust_is_a_template() {
    let templates: BTreeSet<String> = template_keys().into_iter().collect();
    let used = used_keys();
    assert!(used.len() > 100, "the scan finds the call sites");
    let missing: Vec<_> = used.difference(&templates).collect();
    assert!(missing.is_empty(), "not in ui/texts.slint: {missing:?}");
}

#[test]
fn every_template_is_its_key_in_english_and_translated() {
    let po = po_de();
    let entries = template_entries();
    let mut seen = BTreeSet::new();
    for (key, text) in &entries {
        assert!(seen.insert(key), "{key} twice");
        assert_eq!(&unbraced(text), key, "@tr text of {key}");
        assert_eq!(text, &braced(key), "braces of {key} escaped");
        assert!(po.contains_key(text), "no German for {text}");
    }
}
