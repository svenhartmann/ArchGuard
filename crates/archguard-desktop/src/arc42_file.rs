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
use std::path::{Path, PathBuf};

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
const MARKER: &str = "<!-- archguard:arc42:";

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

/// The arc42 text as read from disk. A target that does not exist yet
/// (just chosen by the user) reads as empty.
#[derive(Debug, Clone)]
pub struct Arc42Files {
    pub target: Target,
    /// The whole document of a file target, as on disk.
    whole: Option<String>,
    /// The chapter files of a folder target, as on disk.
    chapters: BTreeMap<u8, (PathBuf, String)>,
}

impl Arc42Files {
    pub fn load(target: Target) -> Result<Self, String> {
        let mut files = Arc42Files {
            target,
            whole: None,
            chapters: BTreeMap::new(),
        };
        match &files.target {
            Target::File(path) => files.whole = read(path)?,
            Target::Folder(dir) => {
                let mut entries: Vec<_> = std::fs::read_dir(dir)
                    .map_err(|e| format!("{}: {e}", dir.display()))?
                    .flatten()
                    .collect();
                entries.sort_by_key(|e| e.file_name());
                for e in entries {
                    let Some(n) = chapter_of(&e.file_name().to_string_lossy()) else {
                        continue;
                    };
                    if files.chapters.contains_key(&n) || !e.path().is_file() {
                        continue;
                    }
                    if let Some(text) = read(&e.path())? {
                        files.chapters.insert(n, (e.path(), text));
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
        match saved {
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
    /// do not survive; a broken block marker refuses the save.
    pub fn save_chapter(
        &mut self,
        doc: &Arc42Document,
        n: u8,
        edited: &str,
    ) -> Result<PathBuf, String> {
        let edited = edited.replace("\r\n", "\n");
        let mut edited = refresh(doc, &edited)?;
        if !edited.ends_with('\n') {
            edited.push('\n');
        }
        let (path, old) = self.file_of(n);
        let text = self.compose(doc, n, old.as_deref(), edited);
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
        Ok(path)
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

    /// The file that holds chapter `n`, with its text if it exists.
    fn file_of(&self, n: u8) -> (PathBuf, Option<String>) {
        let text = match &self.target {
            Target::File(_) => self.whole.clone(),
            Target::Folder(_) => self.chapters.get(&n).map(|(_, t)| t.clone()),
        };
        (self.path_of(n), text)
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

fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1))
}

/// The chapter number of a `## N.` heading line.
fn heading_number(line: &str) -> Option<u8> {
    let rest = line.strip_prefix("## ")?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !rest[digits..].starts_with('.') {
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
mod tests {
    use super::*;
    use archguard_engine::report::Arc42Chapter;

    fn doc(evidence: &str) -> Arc42Document {
        Arc42Document {
            schema_version: 1,
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
        assert_eq!(files.save_chapter(&new, 5, &edited), Ok(path.clone()));
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
        assert!(files.save_chapter(&new, 5, &edited).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            saved,
            "no file was changed"
        );
        // Removing both markers makes the chapter free text.
        let free = "## 5. Title\n\nNur eigener Text.\n";
        assert!(files.save_chapter(&new, 5, free).is_ok());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Nur eigener Text.")
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
        files.save_chapter(&new, 3, &edited).unwrap();
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
            files.save_chapter(&new, 5, "## 5. Bausteine\n\nNeu."),
            Ok(folder.join("05_bausteine.md"))
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("05_bausteine.md")).unwrap(),
            "## 5. Bausteine\n\nNeu.\n"
        );
        let nine = files.chapter(&new, 9) + "Entscheidung A.\n";
        assert_eq!(
            files.save_chapter(&new, 9, &nine),
            Ok(folder.join("arc42-09.md"))
        );
        let written = std::fs::read_to_string(folder.join("arc42-09.md")).unwrap();
        assert!(written.starts_with("## 9. Title") && written.contains("New 9"));
        assert!(written.ends_with("Entscheidung A.\n"));
        assert!(!written.contains("## 10."));
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
