//! "Show here" of the Screens and Domain views: the whole file over the
//! view (`ui/code_overlay.slint`), read as plain text, so every
//! extractor's files show (`.razor`, `.ts`, `.html`, `.cs`, `.java`,
//! `.xaml`, `.vue`, `.slint`). The file is read once when it opens, not on
//! every rebuild of the view.

use std::path::Path;

use crate::tab_support::model;
use crate::ui::{CodeLine, CodeView};

/// The open file and line with its lines as read; none when closed.
#[derive(Debug, Clone, Default)]
pub struct Code(Option<Shown>);

#[derive(Debug, Clone)]
struct Shown {
    file: String,
    line: u32,
    /// [`read`]; none when the file could not be read.
    read: Option<(Vec<CodeLine>, usize, usize)>,
}

impl Code {
    /// Open `file` (project-relative under `root`) at `line`.
    pub fn show(&mut self, root: &Path, file: &str, line: u32) {
        if !file.is_empty() {
            let read = read(root, file, line);
            self.0 = Some(Shown {
                file: file.into(),
                line,
                read,
            });
        }
    }

    pub fn close(&mut self) {
        self.0 = None;
    }

    #[cfg(test)]
    pub fn current(&self) -> Option<(&str, u32)> {
        self.0.as_ref().map(|s| (s.file.as_str(), s.line))
    }

    /// The overlay; closed when nothing is shown.
    pub fn view(&self) -> CodeView {
        let Some(s) = &self.0 else {
            return CodeView::default();
        };
        let (lines, hit, columns) = s.read.clone().unwrap_or_default();
        CodeView {
            open: true,
            title: s.file.rsplit('/').next().unwrap_or(&s.file).into(),
            location: format!("{}:{}", s.file, s.line).into(),
            file: s.file.as_str().into(),
            line: s.line as i32,
            lines: model(lines),
            hit: hit as i32,
            columns: columns as i32,
        }
    }
}

/// Every line of `text`, `line` (1-based; 0 reads as 1) marked; the marked
/// line's index and the longest line's length in characters.
pub fn whole(text: &str, line: u32) -> (Vec<CodeLine>, usize, usize) {
    let at = (line.max(1) - 1) as usize;
    let mut columns = 0;
    let lines: Vec<CodeLine> = (text.lines().enumerate())
        .map(|(i, l)| {
            let l = l.replace('\t', "    ");
            columns = columns.max(l.chars().count());
            CodeLine {
                number: i as i32 + 1,
                text: l.into(),
                hit: i == at,
            }
        })
        .collect();
    let hit = at.min(lines.len().saturating_sub(1));
    (lines, hit, columns)
}

/// [`whole`] of `rel` in the project at `root`; none when the file is
/// outside the project or unreadable.
pub fn read(root: &Path, rel: &str, line: u32) -> Option<(Vec<CodeLine>, usize, usize)> {
    let path = crate::editor::inside(root, rel)?;
    let text = std::fs::read(path).ok()?;
    Some(whole(&String::from_utf8_lossy(&text), line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn the_whole_file_shows_with_the_line_marked() {
        let text: String = (1..=500).map(|i| format!("line {i}\n")).collect();
        let (e, hit, columns) = whole(&text, 250);
        assert_eq!(e.len(), 500, "no cut-off at ±40 lines");
        assert_eq!((hit, e[hit].number, e[hit].hit), (249, 250, true));
        assert_eq!(e.iter().filter(|l| l.hit).count(), 1);
        assert_eq!(columns, "line 500".len());
        let (start, hit, _) = whole(&text, 0);
        assert_eq!((start[0].number, hit, start[0].hit), (1, 0, true));
        assert_eq!(whole("a\n", 9).1, 0, "a line past the end marks the last");
    }

    #[test]
    fn files_outside_the_project_are_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("p");
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("Page.razor"), "@page \"/x\"\n<h1/>\n").expect("file");
        std::fs::write(dir.path().join("secret"), "x").expect("file");
        let (lines, hit, _) = read(&root, "Page.razor", 2).expect("read");
        assert_eq!((lines[hit].text.as_str(), lines.len()), ("<h1/>", 2));
        assert!(read(&root, "../secret", 1).is_none());
    }

    #[test]
    fn the_overlay_opens_reads_once_and_closes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("A.cs");
        std::fs::write(&file, "a\nb\nc\n").expect("file");
        let mut c = Code::default();
        assert!(!c.view().open, "closed at first");
        c.show(dir.path(), "A.cs", 2);
        std::fs::remove_file(&file).expect("removed");
        let v = c.view();
        assert!(v.open);
        assert_eq!(
            (v.location.as_str(), v.hit, v.lines.row_count()),
            ("A.cs:2", 1, 3),
            "the lines read on opening, not on each rebuild"
        );
        c.close();
        assert!(!c.view().open && c.current().is_none());
        c.show(dir.path(), "missing.cs", 1);
        let v = c.view();
        assert!(
            v.open && v.lines.row_count() == 0,
            "unreadable: open, says so"
        );
    }
}
