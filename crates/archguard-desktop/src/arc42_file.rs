//! The project's own arc42 Markdown: found in the project, read per
//! chapter and written back. Text outside the generated
//! `<!-- archguard:arc42:NN:start/end -->` blocks is the author's; the
//! blocks are refreshed from the analysis with [`Arc42Document::merge`],
//! as `archguard doc --arc42` does.
//!
//! A project keeps its arc42 text either in one `arc42.md` (all chapters
//! under `## N.` headings) or in an `arc42/` folder with one file per
//! chapter, named by its number (`05_building_blocks.md`, `arc42-05.md`).

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use archguard_engine::core::DiagramFormat;
use archguard_engine::report::Arc42Document;

/// Folders the search never enters.
const SKIP: &[&str] = &[
    "target",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "bin",
    "obj",
];
/// How many folder levels below the project root are searched.
const DEPTH: usize = 4;
pub(crate) const MARKER: &str = "<!-- archguard:arc42:";

/// Where the project's arc42 text lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File(PathBuf),
    Folder(PathBuf),
}

/// The shallowest `arc42.md` or `arc42/` folder in the project (a file
/// before a folder on the same level); `None` if there is neither.
pub fn find(root: &Path) -> Option<Target> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..=DEPTH {
        let found = scan(&level);
        if let Some(f) = found.file {
            return Some(Target::File(f));
        }
        if let Some(d) = found.folder {
            // A folder with a whole-document file inside is that file.
            return Some(match single_file(&d) {
                Some(f) => Target::File(f),
                None => Target::Folder(d),
            });
        }
        level = found.next;
    }
    None
}

/// Whether [`find`] returns `path` once it exists: an `arc42.md` in a
/// searched folder, and no other arc42 text in the project.
pub fn finds(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let named = rel
        .file_name()
        .is_some_and(|n| n.to_string_lossy().to_lowercase() == "arc42.md");
    let dirs: Vec<String> = rel
        .parent()
        .into_iter()
        .flat_map(Path::iter)
        .map(|d| d.to_string_lossy().to_lowercase())
        .collect();
    // An `arc42/` folder is found as itself, unless it is the file's own.
    let searched = dirs.iter().enumerate().all(|(i, d)| {
        !d.starts_with('.') && !SKIP.contains(&d.as_str()) && (d != "arc42" || i + 1 == dirs.len())
    });
    named && searched && dirs.len() <= DEPTH && find(root).is_none()
}

/// One folder level of the search.
#[derive(Default)]
struct Level {
    file: Option<PathBuf>,
    folder: Option<PathBuf>,
    /// The folders to search on the next level.
    next: Vec<PathBuf>,
}

fn scan(dirs: &[PathBuf]) -> Level {
    let mut found = Level::default();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_file() && name == "arc42.md" {
                found.file.get_or_insert(e.path());
            } else if kind.is_dir() && name == "arc42" {
                found.folder.get_or_insert(e.path());
            } else if kind.is_dir() && !name.starts_with('.') && !SKIP.contains(&&*name) {
                found.next.push(e.path());
            }
        }
    }
    found
}

fn single_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().to_lowercase();
        (name == "arc42.md" && e.path().is_file()).then(|| e.path())
    })
}

/// The chapter a file in an `arc42/` folder holds: the first number in its
/// name after an `arc42` prefix, if it is 1..=12.
fn chapter_of(name: &str) -> Option<u8> {
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

/// The file of each chapter in an `arc42/` folder: the first by name.
fn chapter_paths(dir: &Path) -> Result<BTreeMap<u8, PathBuf>, String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let mut paths = BTreeMap::new();
    for e in entries {
        if let Some(n) = chapter_of(&e.file_name().to_string_lossy())
            && e.path().is_file()
        {
            paths.entry(n).or_insert_with(|| e.path());
        }
    }
    Ok(paths)
}

/// The arc42 text as read from disk. A target that does not exist yet
/// (just chosen by the user) reads as empty.
#[derive(Debug, Clone)]
pub struct Arc42Files {
    pub target: Target,
    /// The whole document of a file target, as on disk.
    whole: Option<String>,
    /// The chapter files of a folder target, as on disk.
    chapters: BTreeMap<u8, (PathBuf, String)>,
    /// The format a diagram the chapter lacks goes in with (`[arc42]`).
    pub format: DiagramFormat,
    /// The shown diagrams the last save took out of their chapter.
    pub dropped: Vec<String>,
}

impl Arc42Files {
    pub fn load(target: Target) -> Result<Self, String> {
        let mut files = Arc42Files {
            target,
            whole: None,
            chapters: BTreeMap::new(),
            format: DiagramFormat::Mermaid,
            dropped: Vec::new(),
        };
        match &files.target {
            Target::File(path) => files.whole = read(path)?,
            Target::Folder(dir) => {
                for (n, path) in chapter_paths(dir)? {
                    if let Some(text) = read(&path)? {
                        files.chapters.insert(n, (path, text));
                    }
                }
            }
        }
        Ok(files)
    }

    /// Chapter `n` as Markdown from its `## N.` heading, the generated
    /// blocks refreshed; the generated chapter if the project has no text
    /// for it.
    pub fn chapter(&self, doc: &Arc42Document, n: u8) -> String {
        let saved = match &self.target {
            Target::File(_) => self
                .whole
                .as_deref()
                .and_then(|w| section(w, n).map(|r| w[r].to_string())),
            Target::Folder(_) => self.chapters.get(&n).map(|(_, t)| t.clone()),
        };
        // LF, as the diagram sections the text is compared with.
        match saved.map(|t| t.replace("\r\n", "\n")) {
            Some(text) => refresh(doc, &text).unwrap_or(text),
            None => generated(doc, n),
        }
    }

    /// The text before the first chapter of a whole document.
    pub fn preamble(&self, doc: &Arc42Document) -> String {
        let generated = doc.markdown(None);
        let text = self.whole.as_deref().unwrap_or(&generated);
        let end = first_chapter(text).unwrap_or(text.len());
        text[..end].to_string()
    }

    /// Write chapter `n` with the edited Markdown; returns the file
    /// written. The generated blocks are refreshed, so edits inside them
    /// do not survive; a broken block marker refuses the save. `base` is
    /// the file's text the edit started from ([`Self::saved`]): a file
    /// that no longer has it refuses the save too, and `base` becomes its
    /// new text.
    pub fn save_chapter(
        &mut self,
        doc: &Arc42Document,
        n: u8,
        edited: &str,
        base: &mut Option<String>,
    ) -> Result<PathBuf, String> {
        let edited = edited.replace("\r\n", "\n");
        let edited = refresh_diagrams(doc, &refresh(doc, &edited)?);
        let before = self.chapter(doc, n);
        let (mut edited, dropped) = complete_diagrams(doc, n, &before, &edited, self.format);
        if !edited.ends_with('\n') {
            edited.push('\n');
        }
        // One file finds chapters by heading: without its own, the text
        // would join the chapter before and chapter N be written twice.
        if let Target::File(_) = self.target {
            let own = matches!(headings(&edited).collect::<Vec<_>>()[..],
                [(i, m)] if m == n && edited[..i].trim().is_empty());
            if !own {
                return Err(format!(
                    "The chapter must start with its heading \"## {n}. …\" and contain no other chapter heading; it was not saved"
                ));
            }
        }
        // The file the next load reads chapter `n` from: a pull may have
        // added one under another name.
        let path = match &self.target {
            Target::Folder(dir) => chapter_paths(dir)?.remove(&n),
            Target::File(_) => None,
        }
        .unwrap_or_else(|| self.path_of(n));
        let on_disk = read(&path)?;
        if on_disk != *base {
            return Err(self.reload(n, &path, on_disk, base));
        }
        let text = self.compose(doc, n, on_disk.as_deref(), edited);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::export::write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
        match &self.target {
            Target::File(_) => self.whole = Some(text),
            Target::Folder(_) => {
                self.chapters.insert(n, (path.clone(), text));
            }
        }
        self.dropped = dropped;
        Ok(path)
    }

    /// The file at `path` changed on disk: load its new text, so saving
    /// again writes the chapter into it, and say why the save was refused.
    /// If chapter `n` itself changed, saving again replaces that change.
    fn reload(
        &mut self,
        n: u8,
        path: &Path,
        on_disk: Option<String>,
        base: &mut Option<String>,
    ) -> String {
        let own = |file: Option<&str>| match self.target {
            Target::File(_) => file.and_then(|w| section(w, n).map(|r| w[r].to_string())),
            Target::Folder(_) => file.map(str::to_string),
        };
        let itself = own(base.as_deref()) != own(on_disk.as_deref());
        base.clone_from(&on_disk);
        match (&self.target, on_disk) {
            (Target::File(_), text) => self.whole = text,
            (Target::Folder(_), Some(text)) => {
                self.chapters.insert(n, (path.to_path_buf(), text));
            }
            (Target::Folder(_), None) => {
                self.chapters.remove(&n);
            }
        }
        if itself {
            format!(
                "Chapter {n} itself was changed in {} while you edited it; saving again replaces that change. Copy your text, Discard, and reopen to see the new version.",
                path.display()
            )
        } else {
            format!(
                "{} changed on disk while this chapter was edited; its new text is loaded, save again to write this chapter into it",
                path.display()
            )
        }
    }

    /// Whether a file holds chapter `n`.
    pub fn has_chapter(&self, n: u8) -> bool {
        match &self.target {
            Target::File(_) => self
                .whole
                .as_deref()
                .is_some_and(|w| section(w, n).is_some()),
            Target::Folder(_) => self.chapters.contains_key(&n),
        }
    }

    /// The file that holds chapter `n`, or that saving it creates.
    pub fn path_of(&self, n: u8) -> PathBuf {
        match &self.target {
            Target::File(path) => path.clone(),
            Target::Folder(dir) => match self.chapters.get(&n) {
                Some((p, _)) => p.clone(),
                None => dir.join(format!("arc42-{n:02}.md")),
            },
        }
    }

    /// The text of the file that holds chapter `n` as read, if it exists.
    pub fn saved(&self, n: u8) -> Option<String> {
        match &self.target {
            Target::File(_) => self.whole.clone(),
            Target::Folder(_) => self.chapters.get(&n).map(|(_, t)| t.clone()),
        }
    }

    /// The new text of that file, in its old line endings.
    fn compose(&self, doc: &Arc42Document, n: u8, old: Option<&str>, chapter: String) -> String {
        let text = match (&self.target, old) {
            (Target::File(_), Some(whole)) => {
                put_section(&whole.replace("\r\n", "\n"), n, &chapter)
            }
            (Target::File(_), None) => put_section(&doc.markdown(None), n, &chapter),
            (Target::Folder(_), _) => chapter,
        };
        if old.is_some_and(|o| o.contains("\r\n")) {
            text.replace('\n', "\r\n")
        } else {
            text
        }
    }
}

/// A file's text; `None` if it does not exist. Like the CLI, never a
/// symlink or another non-regular file.
fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
        Ok(m) if !m.is_file() => {
            return Err(format!("{} is not a regular file", path.display()));
        }
        Ok(_) => {}
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The start of a diagram block's marker; `<id>:start -->` / `<id>:end -->` follow.
pub const DIAGRAM_MARKER: &str = "<!-- archguard:diagram:";

/// A diagram's Markdown `section` as a diagram block with its markers.
pub fn diagram_block(id: &str, section: &str) -> String {
    format!(
        "{DIAGRAM_MARKER}{id}:start -->\n{}\n{DIAGRAM_MARKER}{id}:end -->",
        section.trim_end()
    )
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
fn refresh_diagrams(doc: &Arc42Document, text: &str) -> String {
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
        let section = archguard_engine::report::diagram_section(d, format);
        out.replace_range(r, &diagram_block(&id, &section));
    }
    out
}

/// `text` with the diagrams of chapter `n` as the documentation has them:
/// a shown one that is in neither generated text nor a diagram block goes
/// in at the end as a diagram block, a hidden one's diagram block goes out.
/// A shown one that was in the chapter `before` and is not in `text` was
/// removed or taken over as own text: it stays out and is returned.
fn complete_diagrams(
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
    let has = |text: &str, d: &archguard_engine::core::Diagram| {
        diagram_ranges(text).iter().any(|(_, id)| *id == d.id)
            || [DiagramFormat::Mermaid, DiagramFormat::Plantuml]
                .into_iter()
                .any(|f| text.contains(&archguard_engine::report::diagram_section(d, f)))
    };
    let mut dropped = Vec::new();
    for d in doc.diagrams.iter().filter(|d| d.chapter == n && d.shown) {
        if has(&out, d) {
            continue;
        }
        if has(before, d) {
            dropped.push(d.id.clone());
        } else {
            let block = diagram_block(&d.id, &archguard_engine::report::diagram_section(d, format));
            out = format!("{}\n\n{block}\n", out.trim_end());
        }
    }
    (out, dropped)
}

/// Refresh the generated blocks in `text`; text without any block stays
/// as it is.
fn refresh(doc: &Arc42Document, text: &str) -> Result<String, String> {
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

pub(crate) fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1))
}

/// The chapter number of a `## N.` heading line.
pub(crate) fn heading_number(line: &str) -> Option<u8> {
    let rest = line.strip_prefix("## ")?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    // `## 5.1 Whitebox` is a section of chapter 5, not its heading.
    let after = rest[digits..].strip_prefix('.')?;
    if digits == 0 || !(after.is_empty() || after.starts_with([' ', '\t', '\r', '\n'])) {
        return None;
    }
    rest[..digits].parse().ok()
}

fn headings(text: &str) -> impl Iterator<Item = (usize, u8)> + '_ {
    line_starts(text).filter_map(|i| heading_number(&text[i..]).map(|n| (i, n)))
}

fn first_chapter(text: &str) -> Option<usize> {
    headings(text).next().map(|(i, _)| i)
}

/// The byte range of chapter `n`: its heading up to the next chapter
/// heading.
fn section(text: &str, n: u8) -> Option<std::ops::Range<usize>> {
    let all: Vec<_> = headings(text).collect();
    let k = all.iter().position(|&(_, m)| m == n)?;
    let end = all.get(k + 1).map_or(text.len(), |&(i, _)| i);
    Some(all[k].0..end)
}

/// `text` with chapter `n` replaced by `chapter`, or inserted before the
/// first higher chapter.
fn put_section(text: &str, n: u8, chapter: &str) -> String {
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
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
    }
    out.push_str(&chapter);
    if at < text.len() && !chapter.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use archguard_engine::report::Arc42Chapter;

    /// A diagram block in own text is written with the analysis' current
    /// section; one the analysis no longer has stays as written.
    #[test]
    fn saving_puts_in_shown_diagrams_and_takes_out_hidden_ones() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("arc42.md");
        let mut new = doc("New");
        let diagram = |id: &str, shown| archguard_engine::core::Diagram {
            id: id.into(),
            chapter: 5,
            title: id.into(),
            note: String::new(),
            kind: archguard_engine::core::DiagramKind::Packages,
            unit: String::new(),
            elements: Vec::new(),
            relations: Vec::new(),
            shown,
        };
        new.diagrams = vec![diagram("shown", true), diagram("hidden", false)];
        let section = |d| archguard_engine::report::diagram_section(d, DiagramFormat::Plantuml);
        let hidden = diagram_block("hidden", &section(&new.diagrams[1]));
        std::fs::write(
            &path,
            format!("## 5. Bausteine\n\nEigener Text.\n\n{hidden}\n\nDanach.\n"),
        )
        .unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        files.format = DiagramFormat::Plantuml;
        let chapter = files.chapter(&new, 5);
        files
            .save_chapter(&new, 5, &chapter, &mut files.saved(5))
            .unwrap();
        let shown = diagram_block("shown", &section(&new.diagrams[0]));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("## 5. Bausteine\n\nEigener Text.\n\nDanach.\n\n{shown}\n"),
        );
    }

    /// A chapter of a file with Windows line ends reads with LF, as the
    /// diagram sections it is compared with.
    #[test]
    fn a_chapter_reads_with_lf_line_ends() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("arc42.md");
        std::fs::write(&path, "## 5. Bausteine\r\n\r\nText.\r\n").unwrap();
        let files = Arc42Files::load(Target::File(path)).unwrap();
        assert_eq!(files.chapter(&doc("New"), 5), "## 5. Bausteine\n\nText.\n");
    }

    fn shown_diagram(id: &str) -> archguard_engine::core::Diagram {
        archguard_engine::core::Diagram {
            id: id.into(),
            chapter: 5,
            title: id.into(),
            note: String::new(),
            kind: archguard_engine::core::DiagramKind::Packages,
            unit: String::new(),
            elements: Vec::new(),
            relations: Vec::new(),
            shown: true,
        }
    }

    /// A shown diagram the user removed from the chapter, or turned into
    /// own text, stays out; saving names it, so the view can hide it.
    #[test]
    fn saving_keeps_a_removed_or_detached_diagram_out() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("arc42.md");
        let mut new = doc("New");
        new.diagrams = vec![shown_diagram("gone"), shown_diagram("own")];
        let section = |d| archguard_engine::report::diagram_section(d, DiagramFormat::Mermaid);
        let gone = diagram_block("gone", &section(&new.diagrams[0]));
        let own = diagram_block("own", &section(&new.diagrams[1]));
        std::fs::write(
            &path,
            format!("## 5. Bausteine\n\nText.\n\n{gone}\n\n{own}\n"),
        )
        .unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        let edited = "## 5. Bausteine\n\nText.\n\n```mermaid\nflowchart LR\n  a --> b\n```\n";
        files
            .save_chapter(&new, 5, edited, &mut files.saved(5))
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        assert_eq!(files.dropped, ["gone", "own"]);
    }

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
    fn saving_refreshes_diagram_blocks_from_the_analysis() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("arc42.md");
        let mut new = doc("New");
        let diagram = archguard_engine::core::Diagram {
            id: "calls".into(),
            chapter: 6,
            title: "Aufrufe".into(),
            note: "Neu.".into(),
            kind: archguard_engine::core::DiagramKind::Packages,
            unit: String::new(),
            elements: Vec::new(),
            relations: Vec::new(),
            shown: true,
        };
        new.diagrams.push(diagram.clone());
        let old = diagram_block(
            "calls",
            "### Aufrufe\n\nAlt.\n\n```mermaid\nflowchart LR\n```",
        );
        let gone = diagram_block("gone", "### Weg\n\n```mermaid\nflowchart LR\n```");
        std::fs::write(&path, format!("## 6. Laufzeit\n\n{old}\n\n{gone}\n")).unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        let chapter = files.chapter(&new, 6);
        files
            .save_chapter(&new, 6, &chapter, &mut files.saved(6))
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let fresh = diagram_block(
            "calls",
            &archguard_engine::report::diagram_section(&diagram, DiagramFormat::Mermaid),
        );
        assert!(text.contains(&fresh) && !text.contains("Alt."), "{text}");
        assert!(text.contains(&gone), "{text}");
    }

    pub(crate) fn doc(evidence: &str) -> Arc42Document {
        Arc42Document {
            schema_version: 1,
            lang: archguard_engine::core::DocLang::De,
            scope: "shop".into(),
            notice: "Notice.\n",
            chapters: (1..=12)
                .map(|n| Arc42Chapter {
                    number: n,
                    title: "Title",
                    markdown: format!("{evidence} {n}"),
                    questions: vec![],
                })
                .collect(),
            diagrams: vec![],
        }
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn find_takes_the_shallowest_file_or_folder_and_skips_build_folders() {
        let d = dir();
        let root = d.path();
        assert_eq!(find(root), None, "nothing to find asks the user");
        std::fs::create_dir_all(root.join("target/arc42")).unwrap();
        std::fs::create_dir_all(root.join(".git/arc42")).unwrap();
        assert_eq!(
            find(root),
            None,
            "build and hidden folders are not searched"
        );
        std::fs::create_dir_all(root.join("docs/deep/arc42")).unwrap();
        assert_eq!(
            find(root),
            Some(Target::Folder(root.join("docs/deep/arc42")))
        );
        std::fs::write(root.join("docs/ARC42.md"), "").unwrap();
        assert_eq!(find(root), Some(Target::File(root.join("docs/ARC42.md"))));
        std::fs::create_dir(root.join("arc42")).unwrap();
        assert_eq!(find(root), Some(Target::Folder(root.join("arc42"))));
        std::fs::write(root.join("arc42/arc42.md"), "").unwrap();
        assert_eq!(
            find(root),
            Some(Target::File(root.join("arc42/arc42.md"))),
            "a folder that holds the whole document is that file"
        );
        std::fs::write(root.join("arc42.md"), "").unwrap();
        assert_eq!(find(root), Some(Target::File(root.join("arc42.md"))));
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
    fn a_file_shows_own_text_with_fresh_blocks_and_falls_back_per_chapter() {
        let d = dir();
        let path = d.path().join("arc42.md");
        let saved = doc("Old").markdown(None).replace(
            "<!-- Eigene Erläuterungen zu Kapitel 5 hier ergänzen. -->",
            "Unser Shop trennt Bestellung und Lager.",
        );
        let saved = saved.replace("\n## 7.", "\n## 7-removed");
        std::fs::write(&path, &saved).unwrap();
        let files = Arc42Files::load(Target::File(path)).unwrap();
        let new = doc("New");
        let five = files.chapter(&new, 5);
        assert!(five.starts_with("## 5. Title\n"), "{five}");
        assert!(five.contains("Unser Shop trennt Bestellung und Lager."));
        assert!(five.contains("New 5") && !five.contains("Old 5"), "{five}");
        assert!(!five.contains("## 6."), "one chapter only: {five}");
        assert!(files.preamble(&new).starts_with("# arc42"));
        // Chapter 7's heading is gone from the file: the generated one shows.
        assert!(
            files.chapter(&new, 6).contains("7-removed"),
            "6 runs to the next heading"
        );
        assert!(files.chapter(&new, 7).starts_with("## 7. Title"));
    }

    #[test]
    fn saving_a_chapter_keeps_other_chapters_and_refreshes_blocks() {
        let d = dir();
        let path = d.path().join("arc42.md");
        let saved = doc("Old").markdown(None) + "\nAnhang des Teams.\n";
        std::fs::write(&path, saved.replace('\n', "\r\n")).unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        let new = doc("New");
        let edited = files
            .chapter(&new, 5)
            .replace("New 5", "edited inside the block")
            .replace(
                "<!-- Eigene Erläuterungen zu Kapitel 5 hier ergänzen. -->",
                "Eigene Notiz zu Bausteinen.",
            );
        assert_eq!(
            files.save_chapter(&new, 5, &edited, &mut files.saved(5)),
            Ok(path.clone())
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.replace("\r\n", "").contains('\n'), "CRLF stays");
        let text = text.replace("\r\n", "\n");
        assert!(text.contains("Eigene Notiz zu Bausteinen."));
        assert!(text.contains("New 5") && !text.contains("edited inside the block"));
        assert!(
            text.contains("Old 4") && text.contains("Old 6"),
            "other chapters untouched"
        );
        assert!(
            text.contains("\n\n## 6. Title"),
            "blank line before the next chapter"
        );
        assert!(text.ends_with("Anhang des Teams.\n"));
        assert!(
            files
                .chapter(&new, 5)
                .contains("Eigene Notiz zu Bausteinen.")
        );
    }

    #[test]
    fn a_broken_block_marker_refuses_the_save() {
        let d = dir();
        let path = d.path().join("arc42.md");
        let saved = doc("Old").markdown(None);
        std::fs::write(&path, &saved).unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        let new = doc("New");
        let edited = files
            .chapter(&new, 5)
            .replace("<!-- archguard:arc42:05:end -->", "");
        assert!(
            files
                .save_chapter(&new, 5, &edited, &mut files.saved(5))
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            saved,
            "no file was changed"
        );
        // Removing both markers makes the chapter free text.
        let free = "## 5. Title\n\nNur eigener Text.\n";
        assert!(
            files
                .save_chapter(&new, 5, free, &mut files.saved(5))
                .is_ok()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Nur eigener Text.")
        );
    }

    /// In one file a chapter is found by its `## N.` heading: an edit
    /// without it would land in the chapter before and duplicate chapter N
    /// on the next save. The save is refused; the file and its base stay.
    #[test]
    fn a_file_refuses_a_chapter_without_its_own_heading() {
        let d = dir();
        let path = d.path().join("arc42.md");
        let saved = doc("Old").markdown(None);
        std::fs::write(&path, &saved).unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        let new = doc("New");
        let five = files.chapter(&new, 5) + "Eigener Text.\n";
        let cases = [
            ("deleted", five.replacen("## 5. Title\n", "", 1)),
            ("dotless", five.replacen("## 5. Title", "## 5 Title", 1)),
            ("renumbered", five.replacen("## 5. Title", "## 6. Title", 1)),
            ("text before", format!("Vorab.\n\n{five}")),
            ("two chapters", five.clone() + "\n## 6. Title\n"),
        ];
        for (case, edited) in cases {
            let mut base = files.saved(5);
            let err = files
                .save_chapter(&new, 5, &edited, &mut base)
                .expect_err(case);
            assert!(err.contains("## 5."), "{case}: {err}");
            assert_eq!(base, files.saved(5), "{case}: base kept");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), saved, "{case}");
        }
        assert!(
            files
                .save_chapter(&new, 5, &five, &mut files.saved(5))
                .is_ok(),
            "its own heading saves"
        );
    }

    #[test]
    fn a_new_file_starts_from_the_generated_document() {
        let d = dir();
        let path = d.path().join("docs/arc42.md");
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        assert_eq!(files.path_of(3), path, "one file holds every chapter");
        let new = doc("New");
        let edited = files.chapter(&new, 3) + "Kontext vom Team.\n";
        files
            .save_chapter(&new, 3, &edited, &mut files.saved(3))
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# arc42"));
        assert!(text.contains("Kontext vom Team.\n\n## 4. Title"), "{text}");
        assert_eq!(text.matches("## 3. Title").count(), 1);
        assert_eq!(text.matches("## ").count(), 12);
        assert_eq!(find(d.path()), Some(Target::File(path)), "found next time");
    }

    #[test]
    fn a_folder_writes_the_chapter_file_or_creates_one() {
        let d = dir();
        let folder = d.path().join("arc42");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("05_bausteine.md"), "## 5. Bausteine\n\nAlt.\n").unwrap();
        std::fs::write(folder.join("readme.md"), "not a chapter").unwrap();
        let mut files = Arc42Files::load(Target::Folder(folder.clone())).unwrap();
        let new = doc("New");
        assert_eq!(files.chapter(&new, 5), "## 5. Bausteine\n\nAlt.\n");
        assert!(files.chapter(&new, 9).starts_with("## 9. Title"));
        assert_eq!(files.path_of(5), folder.join("05_bausteine.md"));
        assert_eq!(
            files.path_of(9),
            folder.join("arc42-09.md"),
            "the file a save creates"
        );
        assert_eq!(
            files.save_chapter(&new, 5, "## 5. Bausteine\n\nNeu.", &mut files.saved(5)),
            Ok(folder.join("05_bausteine.md"))
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("05_bausteine.md")).unwrap(),
            "## 5. Bausteine\n\nNeu.\n"
        );
        let nine = files.chapter(&new, 9) + "Entscheidung A.\n";
        assert_eq!(
            files.save_chapter(&new, 9, &nine, &mut files.saved(9)),
            Ok(folder.join("arc42-09.md"))
        );
        let written = std::fs::read_to_string(folder.join("arc42-09.md")).unwrap();
        assert!(written.starts_with("## 9. Title") && written.contains("New 9"));
        assert!(written.ends_with("Entscheidung A.\n"));
        assert!(!written.contains("## 10."));
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
        assert_eq!(section(text, 5), Some(text.find("## 05.").unwrap()..text.len()));
    }

    #[test]
    fn a_file_changed_on_disk_refuses_the_save_and_keeps_the_change() {
        let d = dir();
        let path = d.path().join("arc42.md");
        std::fs::write(&path, doc("Old").markdown(None)).unwrap();
        let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
        let new = doc("New");
        let edited = files.chapter(&new, 5) + "Mein Kapitel 5.\n";
        let mut base = files.saved(5);
        // Someone else edits chapter 9 meanwhile (git pull, an editor).
        let theirs = std::fs::read_to_string(&path)
            .unwrap()
            .replace("Old 9", "Old 9 und Text von außen");
        std::fs::write(&path, &theirs).unwrap();
        assert!(files.save_chapter(&new, 5, &edited, &mut base).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            theirs,
            "not written"
        );
        // The new text is loaded: saving again keeps the other change.
        files.save_chapter(&new, 5, &edited, &mut base).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Text von außen") && text.contains("Mein Kapitel 5."));
    }

    #[test]
    fn a_chapter_file_changed_on_disk_refuses_the_save() {
        let d = dir();
        let folder = d.path().join("arc42");
        std::fs::create_dir(&folder).unwrap();
        let mut files = Arc42Files::load(Target::Folder(folder.clone())).unwrap();
        let new = doc("New");
        let mut base = files.saved(9);
        std::fs::write(folder.join("arc42-09.md"), "## 9. Von außen\n").unwrap();
        assert!(
            files
                .save_chapter(&new, 9, "## 9. Meins\n", &mut base)
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("arc42-09.md")).unwrap(),
            "## 9. Von außen\n"
        );
    }

    /// A pull adds `09_entscheidungen.md` while chapter 9 (no file yet) is
    /// edited: the save must not write `arc42-09.md`, which the next load
    /// ignores, but refuse and load the pulled file.
    #[test]
    fn a_chapter_file_added_under_another_name_refuses_the_save() {
        let d = dir();
        let folder = d.path().join("arc42");
        std::fs::create_dir(&folder).unwrap();
        let mut files = Arc42Files::load(Target::Folder(folder.clone())).unwrap();
        let new = doc("New");
        let mut base = files.saved(9);
        let pulled = folder.join("09_entscheidungen.md");
        std::fs::write(&pulled, "## 9. Von außen\n").unwrap();
        let err = files
            .save_chapter(&new, 9, "## 9. Meins\n", &mut base)
            .expect_err("refused");
        assert!(err.contains("Chapter 9 itself was changed"), "{err}");
        assert!(!folder.join("arc42-09.md").exists(), "no ignored file");
        assert_eq!(files.path_of(9), pulled, "the pulled file is loaded");
        // Saving again writes the file the next load reads.
        assert_eq!(files.save_chapter(&new, 9, "## 9. Meins\n", &mut base), Ok(pulled.clone()));
        let next = Arc42Files::load(Target::Folder(folder)).unwrap();
        assert_eq!(next.chapter(&new, 9), "## 9. Meins\n");
    }

    #[test]
    fn finds_tells_whether_a_chosen_file_is_found_next_time() {
        let d = dir();
        let root = d.path();
        assert!(finds(root, &root.join("arc42.md")));
        assert!(finds(root, &root.join("docs/ARC42.md")));
        assert!(finds(root, &root.join("docs/arc42/arc42.md")));
        assert!(
            finds(root, &root.join("a/b/c/d/arc42.md")),
            "four levels deep"
        );
        assert!(!finds(root, &root.join("a/b/c/d/e/arc42.md")), "too deep");
        assert!(
            !finds(root, &root.join("docs/architecture.md")),
            "other name"
        );
        assert!(
            !finds(root, &root.join(".github/arc42.md")),
            "hidden folder"
        );
        assert!(!finds(root, &root.join("build/arc42.md")), "build folder");
        assert!(
            !finds(root, &root.join("arc42/sub/arc42.md")),
            "the folder wins"
        );
        assert!(!finds(root, Path::new("/elsewhere/arc42.md")), "outside");
        std::fs::create_dir_all(root.join("docs/arc42")).unwrap();
        assert!(
            !finds(root, &root.join("arc42.md")),
            "other arc42 text is found"
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
