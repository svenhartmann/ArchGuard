//! The project's own arc42 Markdown on disk, for the desktop app and
//! `archguard doc --arc42` alike: found in the project, read per chapter,
//! refreshed and written back. The text rules (chapters, generated
//! blocks, diagram blocks) are [`archguard_report::arc42_file`].
//!
//! A project keeps its arc42 text either in one `arc42.md` (all chapters
//! under `## N.` headings) or in an `arc42/` folder with one file per
//! chapter, named by its number (`05_building_blocks.md`, `arc42-05.md`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use archguard_core::DiagramFormat;
use archguard_report::Arc42Document;
use archguard_report::arc42_file::{
    BOM, MARKER, chapter_of, first_chapter, generated, headings, put_section, refresh,
    refresh_diagrams, section, update_chapter,
};

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
/// The largest arc42 file read; a bigger one is no documentation.
const LIMIT: u64 = 8 * 1024 * 1024;

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

/// One file a refresh changes: its text on disk (`None`: new) and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub old: Option<String>,
    pub new: String,
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
    /// The project's arc42 text as [`find`] locates it, else the
    /// `arc42.md` in `root` a first update creates.
    pub fn open(root: &Path, format: DiagramFormat) -> Result<Self, String> {
        let target = find(root).unwrap_or_else(|| Target::File(root.join("arc42.md")));
        Ok(Arc42Files {
            format,
            ..Self::load(target)?
        })
    }

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

    /// Whether the project has any of this text on disk.
    pub fn exists(&self) -> bool {
        self.whole.is_some() || !self.chapters.is_empty()
    }

    /// The project's own text of chapter `n`, LF, from its heading.
    fn own(&self, n: u8) -> Option<String> {
        let own = match &self.target {
            Target::File(_) => self
                .whole
                .as_deref()
                .and_then(|w| section(w, n).map(|r| w[r].to_string())),
            Target::Folder(_) => self
                .chapters
                .get(&n)
                .map(|(_, t)| t.trim_start_matches(BOM).to_string()),
        };
        // LF, as the diagram sections the text is compared with.
        own.map(|t| t.replace("\r\n", "\n"))
    }

    /// Chapter `n` as Markdown from its `## N.` heading, the generated
    /// blocks refreshed; the generated chapter if the project has no text
    /// for it.
    pub fn chapter(&self, doc: &Arc42Document, n: u8) -> String {
        match self.own(n) {
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

    /// The files `archguard doc --arc42` changes (chapter `only`, or all):
    /// every chapter refreshed from `doc` as the desktop saves it
    /// ([`update_chapter`]). Text that is not there yet is created from
    /// `doc`: the whole document, or a folder's chapter files.
    pub fn update(&self, doc: &Arc42Document, only: Option<u8>) -> Result<Vec<Change>, String> {
        let pick = |n: &u8| only.is_none_or(|o| o == *n);
        let mut changes = Vec::new();
        match &self.target {
            Target::File(path) => {
                let new = match &self.whole {
                    Some(whole) => self.update_whole(doc, whole, only)?,
                    None => doc.markdown(only),
                };
                if self.whole.as_ref() != Some(&new) {
                    changes.push(self.change(path.clone(), new));
                }
            }
            Target::Folder(_) if self.chapters.is_empty() => {
                for n in (1..=12).filter(pick) {
                    changes.push(self.change(self.path_of(n), generated(doc, n)));
                }
            }
            Target::Folder(_) => {
                for (&n, (path, old)) in self.chapters.iter().filter(|(n, _)| pick(n)) {
                    let own = self.own(n).unwrap_or_default();
                    let mut new = update_chapter(doc, n, &own, self.format)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    if new != own {
                        if !new.ends_with('\n') {
                            new.push('\n');
                        }
                        let new = self.compose(doc, n, Some(old), new);
                        changes.push(self.change(path.clone(), new));
                    }
                }
            }
        }
        Ok(changes)
    }

    fn change(&self, path: PathBuf, new: String) -> Change {
        let old = match &self.target {
            Target::File(_) => self.whole.clone(),
            Target::Folder(_) => self
                .chapters
                .values()
                .find(|(p, _)| *p == path)
                .map(|(_, t)| t.clone()),
        };
        Change { path, old, new }
    }

    /// A whole document refreshed: its generated blocks as
    /// [`Arc42Document::merge`] does, then each chapter's diagrams.
    fn update_whole(
        &self,
        doc: &Arc42Document,
        whole: &str,
        only: Option<u8>,
    ) -> Result<String, String> {
        let text = whole.replace("\r\n", "\n");
        if !text.contains(MARKER) && first_chapter(&text).is_none() {
            return Err("arc42 document has neither a generated block nor a `## N.` chapter heading; no file was changed. Use --stdout to obtain blocks for an existing document.".into());
        }
        // A selected chapter of own text has no block to merge.
        let own = only.is_some_and(|n| !text.contains(&format!("{MARKER}{n:02}:start -->")));
        let mut out = if text.contains(MARKER) && !own {
            doc.merge(&text, only)?
        } else {
            text.clone()
        };
        if only.is_none() {
            out = refresh_diagrams(doc, &out);
        }
        let chapters: Vec<u8> = headings(&out).map(|(_, n)| n).collect();
        for n in chapters
            .into_iter()
            .filter(|&n| only.is_none_or(|o| o == n))
        {
            let Some(r) = section(&out, n) else { continue };
            let new = update_chapter(doc, n, &out[r.clone()], self.format)?;
            if new != out[r] {
                out = put_section(&out, n, &new);
            }
        }
        if out == text {
            return Ok(whole.to_string());
        }
        Ok(if whole.contains("\r\n") {
            out.replace('\n', "\r\n")
        } else {
            out
        })
    }

    /// The chapters whose text a refresh from `doc` changes.
    pub fn stale(&self, doc: &Arc42Document, only: Option<u8>) -> Result<Vec<u8>, String> {
        let mut stale = Vec::new();
        for n in (1..=12).filter(|&n| only.is_none_or(|o| o == n)) {
            if !self.update(doc, Some(n))?.is_empty() {
                stale.push(n);
            }
        }
        Ok(stale)
    }

    /// The whole document as one Markdown, the project's text with its
    /// generated blocks and diagrams as current as `doc` (chapter `only`
    /// after the document's preamble): what an export converts.
    pub fn markdown(&self, doc: &Arc42Document, only: Option<u8>) -> Result<String, String> {
        let whole = match (&self.target, &self.whole) {
            (Target::File(_), Some(whole)) => Some(self.update_whole(doc, whole, None)?),
            (Target::File(_), None) => return Ok(doc.markdown(only)),
            (Target::Folder(_), _) => None,
        };
        let whole = whole.map(|w| w.replace("\r\n", "\n"));
        let mut out = match &whole {
            Some(w) if only.is_none() => return Ok(w.clone()),
            Some(w) => w[..first_chapter(w).unwrap_or(w.len())].to_string(),
            None => self.preamble(doc),
        };
        for n in (1..=12).filter(|&n| only.is_none_or(|o| o == n)) {
            let chapter = match (&whole, self.own(n)) {
                (Some(w), _) => section(w, n).map(|r| w[r].to_string()),
                (None, Some(own)) => Some(update_chapter(doc, n, &own, self.format)?),
                (None, None) => None,
            };
            let chapter = chapter.unwrap_or_else(|| generated(doc, n));
            out = put_section(&out, n, &chapter);
        }
        Ok(out)
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
        let (mut edited, dropped) =
            archguard_report::arc42_file::complete_diagrams(doc, n, &before, &edited, self.format);
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
        write(&path, on_disk.as_deref(), &text)?;
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
            (Target::Folder(_), Some(old)) if old.starts_with(BOM) => format!("{BOM}{chapter}"),
            (Target::Folder(_), _) => chapter,
        };
        if old.is_some_and(|o| o.contains("\r\n")) {
            text.replace('\n', "\r\n")
        } else {
            text
        }
    }
}

/// A file's text; `None` if it does not exist. Never a symlink or another
/// non-regular file, and none over 8 MiB.
fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
        Ok(m) if !m.is_file() => {
            return Err(format!(
                "{} must be a regular file, not a symlink",
                path.display()
            ));
        }
        Ok(m) if m.len() > LIMIT => return Err(format!("{} exceeds 8 MiB", path.display())),
        Ok(_) => {}
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Replace `path` with `text` in one step, its folder created; refused
/// (nothing written) if the file no longer holds `old`, the text the new
/// one was made from, so a change made meanwhile is never lost.
pub fn write(path: &Path, old: Option<&str>, text: &str) -> Result<(), String> {
    use std::io::Write;
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let dir = path
        .parent()
        .ok_or_else(|| fail(std::io::ErrorKind::InvalidInput.into()))?;
    std::fs::create_dir_all(dir).map_err(fail)?;
    let temp = path.with_file_name(format!(".archguard-arc42-{}.tmp", std::process::id()));
    let result = (|| {
        // create_new: the temporary name can never be a planted symlink.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(fail)?;
        if let Ok(m) = std::fs::metadata(path) {
            file.set_permissions(m.permissions()).map_err(fail)?;
        }
        file.write_all(text.as_bytes()).map_err(fail)?;
        file.sync_all().map_err(fail)?;
        if read(path)?.as_deref() != old {
            return Err(format!(
                "{} changed on disk meanwhile; nothing was written, retry without losing the edits",
                path.display()
            ));
        }
        // rename replaces the entry; it never writes through a link.
        std::fs::rename(&temp, path).map_err(fail)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
