//! "Show here" of the Screens and Domain views: the whole file over the
//! view (`ui/code_overlay.slint`), read as plain text, so every
//! extractor's files show (`.razor`, `.ts`, `.html`, `.cs`, `.java`,
//! `.xaml`, `.vue`, `.slint`). The jumps form a stack: Back returns to the
//! one before, closing forgets them all.

use std::path::Path;

use crate::tab_support::model;
use crate::ui::{CodeLine, CodeView};

/// The files and lines shown, the last one on top.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Jumps(Vec<(String, u32)>);

impl Jumps {
    /// Show `file` at `line` on top of the jumps before.
    pub fn show(&mut self, file: &str, line: u32) {
        if !file.is_empty() && self.0.last() != Some(&(file.to_string(), line)) {
            self.0.push((file.to_string(), line));
        }
    }

    /// Back to the jump before; the last one stays.
    pub fn back(&mut self) {
        if self.0.len() > 1 {
            self.0.pop();
        }
    }

    pub fn close(&mut self) {
        self.0.clear();
    }

    pub fn current(&self) -> Option<(&str, u32)> {
        self.0.last().map(|(f, l)| (f.as_str(), *l))
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

/// The overlay for the top jump; closed when there is none.
pub fn view(root: &Path, jumps: &Jumps) -> CodeView {
    let Some((file, line)) = jumps.current() else {
        return CodeView::default();
    };
    let (lines, hit, columns) = read(root, file, line).unwrap_or_default();
    CodeView {
        open: true,
        title: file.rsplit('/').next().unwrap_or(file).into(),
        location: format!("{file}:{line}").into(),
        file: file.into(),
        line: line as i32,
        lines: model(lines),
        hit: hit as i32,
        columns: columns as i32,
        can_back: jumps.0.len() > 1,
    }
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
    fn jumps_open_go_back_and_close() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("A.cs"), "a\nb\nc\n").expect("file");
        let mut j = Jumps::default();
        assert!(!view(dir.path(), &j).open, "closed at first");
        j.show("A.cs", 2);
        j.show("A.cs", 2);
        let v = view(dir.path(), &j);
        assert!(v.open && !v.can_back, "the same jump twice is one");
        assert_eq!(
            (v.location.as_str(), v.hit, v.lines.row_count()),
            ("A.cs:2", 1, 3)
        );
        j.show("A.cs", 3);
        assert!(view(dir.path(), &j).can_back);
        j.back();
        assert_eq!(j.current(), Some(("A.cs", 2)));
        j.back();
        assert!(j.current().is_some(), "Back keeps the first jump");
        j.close();
        assert!(!view(dir.path(), &j).open);
        j.show("missing.cs", 1);
        let v = view(dir.path(), &j);
        assert!(
            v.open && v.lines.row_count() == 0,
            "unreadable: open, says so"
        );
    }
}
