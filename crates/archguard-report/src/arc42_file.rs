//! The text rules of a project's own arc42 Markdown, shared by the desktop
//! app and `archguard doc --arc42`: the `## N.` chapter headings, the
//! generated `<!-- archguard:arc42:NN:start/end -->` blocks and the diagram
//! blocks `<!-- archguard:diagram:ID:start/end -->` "Insert into chapter"
//! puts into a chapter of own text. Text outside both kinds of block is
//! the author's. Finding, reading and writing the files is the engine's
//! (`archguard_engine::arc42_file`).

use std::ops::Range;

use archguard_core::{Diagram, DiagramFormat};

use crate::arc42::Arc42Document;
use crate::arc42_diagrams::section as diagram_section;

/// The start of a generated block's marker.
pub const MARKER: &str = "<!-- archguard:arc42:";
/// The start of a diagram block's marker; `<id>:start -->` / `<id>:end -->` follow.
pub const DIAGRAM_MARKER: &str = "<!-- archguard:diagram:";
/// A UTF-8 byte order mark some editors put before the first line.
pub const BOM: char = '\u{feff}';

/// Chapter `n` (from its heading, LF line ends) as `doc` has it: the
/// generated blocks and diagram blocks refreshed, every shown diagram of
/// the chapter in it (see [`complete_diagrams`]). A broken block marker
/// is an error.
pub fn update_chapter(
    doc: &Arc42Document,
    n: u8,
    text: &str,
    format: DiagramFormat,
) -> Result<String, String> {
    let refreshed = refresh_diagrams(doc, &refresh(doc, text)?);
    Ok(complete_diagrams(doc, n, text, &refreshed, format).0)
}

/// A diagram's Markdown `section` as a diagram block with its markers.
pub fn diagram_block(id: &str, section: &str) -> String {
    format!(
        "{DIAGRAM_MARKER}{id}:start -->\n{}\n{DIAGRAM_MARKER}{id}:end -->",
        section.trim_end()
    )
}

/// Diagram `d` is in `text`: as a diagram block or as its generated
/// section, in either format.
pub fn in_text(text: &str, d: &Diagram) -> bool {
    diagram_ranges(text).iter().any(|(_, id)| *id == d.id)
        || [DiagramFormat::Mermaid, DiagramFormat::Plantuml]
            .into_iter()
            .any(|f| text.contains(&diagram_section(d, f)))
}

/// Each diagram block of `md`: its bytes (start marker to end marker) and
/// its diagram's id; a start without its end is no block.
pub fn diagram_ranges(md: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = md[from..].find(DIAGRAM_MARKER).map(|i| from + i) {
        let rest = &md[at + DIAGRAM_MARKER.len()..];
        from = at + DIAGRAM_MARKER.len();
        let Some(id) = rest.split_once(":start -->").map(|(id, _)| id) else {
            continue;
        };
        if id.contains(char::is_whitespace) {
            continue;
        }
        let end_marker = format!("{DIAGRAM_MARKER}{id}:end -->");
        // Not past the next start of the same diagram.
        let until = md[from..]
            .find(&format!("{DIAGRAM_MARKER}{id}:start -->"))
            .map_or(md.len(), |i| from + i);
        if let Some(end) = md[from..until]
            .find(&end_marker)
            .map(|i| from + i + end_marker.len())
        {
            out.push((at..end, id.to_string()));
            from = end;
        }
    }
    out
}

/// Refresh each diagram block in `text` from `doc`: its diagram's section
/// in the block's format (Mermaid unless it holds PlantUML); a diagram
/// the analysis no longer has stays as written.
pub fn refresh_diagrams(doc: &Arc42Document, text: &str) -> String {
    let mut out = text.to_string();
    for (r, id) in diagram_ranges(text).into_iter().rev() {
        let Some(d) = doc.diagrams.iter().find(|d| d.id == id) else {
            continue;
        };
        let format = if text[r.clone()].contains("```plantuml") {
            DiagramFormat::Plantuml
        } else {
            DiagramFormat::Mermaid
        };
        out.replace_range(r, &diagram_block(&id, &diagram_section(d, format)));
    }
    out
}

/// `text` with the diagrams of chapter `n` as the documentation has them:
/// a shown one that is in neither generated text nor a diagram block goes
/// in at the end as a diagram block, a hidden one's diagram block goes out.
/// A shown one that was in the chapter `before` and is not in `text` was
/// removed or taken over as own text: it stays out and is returned.
pub fn complete_diagrams(
    doc: &Arc42Document,
    n: u8,
    before: &str,
    text: &str,
    format: DiagramFormat,
) -> (String, Vec<String>) {
    let mut out = text.to_string();
    for (r, id) in diagram_ranges(text).into_iter().rev() {
        if doc.diagrams.iter().any(|d| d.id == id && !d.shown) {
            // With the line break and blank line it went in with; at the
            // end, with the blank line before it.
            let blank = out[r.end..].len() - out[r.end..].trim_start_matches('\n').len();
            let start = if r.end + blank == out.len() {
                out[..r.start].trim_end_matches('\n').len() + 1
            } else {
                r.start
            };
            out.replace_range(start.min(r.start)..r.end + blank.min(2), "");
        }
    }
    let mut dropped = Vec::new();
    for d in doc.diagrams.iter().filter(|d| d.chapter == n && d.shown) {
        if in_text(&out, d) {
            continue;
        }
        if in_text(before, d) {
            dropped.push(d.id.clone());
        } else {
            let block = diagram_block(&d.id, &diagram_section(d, format));
            out = format!("{}\n\n{block}\n", out.trim_end());
        }
    }
    (out, dropped)
}

/// Refresh the generated blocks in `text`; text without any block stays
/// as it is.
pub fn refresh(doc: &Arc42Document, text: &str) -> Result<String, String> {
    if !text.contains(MARKER) {
        return Ok(text.to_string());
    }
    doc.merge(text, None)
}

/// The generated chapter `n`, from its heading on.
pub fn generated(doc: &Arc42Document, n: u8) -> String {
    let text = doc.markdown(Some(n));
    match section(&text, n) {
        Some(r) => text[r].trim_end().to_string() + "\n",
        None => text,
    }
}

/// Where `text`'s lines start; the first after a byte order mark, so
/// `\u{feff}## 1.` is chapter 1 and saving it keeps the mark.
pub fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    let first = if text.starts_with(BOM) {
        BOM.len_utf8()
    } else {
        0
    };
    std::iter::once(first).chain(text.match_indices('\n').map(|(i, _)| i + 1))
}

/// The chapter number of a `## N.` heading line.
pub fn heading_number(line: &str) -> Option<u8> {
    let rest = line.strip_prefix("## ")?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    // `## 5.1 Whitebox` is a section of chapter 5, not its heading.
    let after = rest[digits..].strip_prefix('.')?;
    if digits == 0 || !(after.is_empty() || after.starts_with([' ', '\t', '\r', '\n'])) {
        return None;
    }
    rest[..digits].parse().ok()
}

/// Each chapter heading of `text`: where it starts and its number.
pub fn headings(text: &str) -> impl Iterator<Item = (usize, u8)> + '_ {
    line_starts(text).filter_map(|i| heading_number(&text[i..]).map(|n| (i, n)))
}

/// Where the first chapter heading of `text` starts.
pub fn first_chapter(text: &str) -> Option<usize> {
    headings(text).next().map(|(i, _)| i)
}

/// The byte range of chapter `n`: its heading up to the next chapter
/// heading.
pub fn section(text: &str, n: u8) -> Option<Range<usize>> {
    let all: Vec<_> = headings(text).collect();
    let k = all.iter().position(|&(_, m)| m == n)?;
    let end = all.get(k + 1).map_or(text.len(), |&(i, _)| i);
    Some(all[k].0..end)
}

/// `text` with chapter `n` replaced by `chapter`, or inserted before the
/// first higher chapter.
pub fn put_section(text: &str, n: u8, chapter: &str) -> String {
    let mut chapter = chapter.to_string();
    if let Some(r) = section(text, n) {
        // Keep the blank line that separated it from the next chapter.
        if r.end < text.len() && !chapter.ends_with("\n\n") {
            chapter.push('\n');
        }
        let mut out = text.to_string();
        out.replace_range(r, &chapter);
        return out;
    }
    let at = headings(text)
        .find(|&(_, m)| m > n)
        .map_or(text.len(), |(i, _)| i);
    let mut out = text[..at].to_string();
    if !out.trim_start_matches(BOM).is_empty() && !out.ends_with("\n\n") {
        out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
    }
    out.push_str(&chapter);
    if at < text.len() && !chapter.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&text[at..]);
    out
}

/// The chapter a file in an `arc42/` folder holds: the first number in its
/// name after an `arc42` prefix, if it is 1..=12.
pub fn chapter_of(name: &str) -> Option<u8> {
    let stem = name.to_lowercase();
    let stem = stem.strip_suffix(".md")?;
    let stem = stem.strip_prefix("arc42").unwrap_or(stem);
    let digits: String = stem
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|n| (1..=12).contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A start marker without its end is text: the block of the same
    /// diagram further down does not swallow what is between.
    #[test]
    fn an_unended_diagram_start_does_not_swallow_the_text_after_it() {
        let block = diagram_block("calls", "### Aufrufe\n\n```mermaid\nflowchart LR\n```");
        let md = format!("<!-- archguard:diagram:calls:start -->\nEigener Absatz.\n\n{block}\n");
        let ranges = diagram_ranges(&md);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&md[ranges[0].0.clone()], block);
    }

    #[test]
    fn folder_files_are_chapters_by_the_first_number_in_their_name() {
        assert_eq!(chapter_of("05_building_block_view.md"), Some(5));
        assert_eq!(chapter_of("arc42-11.md"), Some(11));
        assert_eq!(chapter_of("chapter12.md"), Some(12));
        assert_eq!(chapter_of("13_extra.md"), None);
        assert_eq!(chapter_of("readme.md"), None);
        assert_eq!(chapter_of("05.adoc"), None);
        assert_eq!(chapter_of("ARC42_03_Kontext.MD"), Some(3));
    }

    #[test]
    fn a_numbered_subsection_is_not_a_chapter_heading() {
        let text = "## 5. Bausteine\n\n## 5.1 Whitebox\n\nw\n\n## 6.\n";
        assert_eq!(heading_number("## 5.1 Whitebox"), None);
        assert_eq!(heading_number("## 5.\n"), Some(5));
        assert_eq!(section(text, 5), Some(0..text.find("## 6.").unwrap()));
        assert_eq!(headings(text).count(), 2);
    }

    /// `## 05.` is chapter 5 as the file names and markers count it, and a
    /// bare `## 5.` closing a file without a newline is still its heading.
    #[test]
    fn a_leading_zero_and_a_heading_at_the_end_name_the_chapter() {
        assert_eq!(heading_number("## 05. Bausteinsicht"), Some(5));
        assert_eq!(heading_number("## 5."), Some(5));
        let text = "## 4. Lösung\n\nx\n\n## 05.";
        assert_eq!(
            section(text, 5),
            Some(text.find("## 05.").unwrap()..text.len())
        );
    }

    #[test]
    fn put_section_inserts_a_missing_chapter_in_order() {
        let text = "# T\n\n## 2. B\n\nb\n\n## 4. D\n\nd\n";
        assert_eq!(
            put_section(text, 3, "## 3. C\n\nc\n"),
            "# T\n\n## 2. B\n\nb\n\n## 3. C\n\nc\n\n## 4. D\n\nd\n"
        );
        assert_eq!(
            put_section(text, 5, "## 5. E\n"),
            "# T\n\n## 2. B\n\nb\n\n## 4. D\n\nd\n\n## 5. E\n"
        );
        assert_eq!(
            put_section(text, 2, "## 2. B\n\nneu\n"),
            "# T\n\n## 2. B\n\nneu\n\n## 4. D\n\nd\n"
        );
    }
}
