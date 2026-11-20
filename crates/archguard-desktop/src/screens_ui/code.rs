//! "Show here": the source around a file:line in the details panel, read
//! as plain text, so every extractor's files show (`.razor`, `.ts`,
//! `.html`, `.cs`, `.java`, `.xaml`, `.vue`, `.slint`).

use std::path::Path;

use crate::ui::CodeLine;

/// Lines shown before and after the marked one.
pub const CONTEXT: usize = 40;

/// The source around `line` (1-based; 0 reads as 1) of `text`, the line
/// marked; and the marked line's index in the result.
pub fn excerpt(text: &str, line: u32) -> (Vec<CodeLine>, usize) {
    let at = (line.max(1) - 1) as usize;
    let first = at.saturating_sub(CONTEXT);
    let lines = (text.lines().enumerate())
        .skip(first)
        .take(2 * CONTEXT + 1)
        .map(|(i, l)| CodeLine {
            number: i as i32 + 1,
            text: l.replace('\t', "    ").into(),
            hit: i == at,
        })
        .collect();
    (lines, at - first)
}

/// [`excerpt`] of `rel` in the project at `root`; none when the file is
/// outside the project or unreadable.
pub fn read(root: &Path, rel: &str, line: u32) -> Option<(Vec<CodeLine>, usize)> {
    let path = crate::editor::inside(root, rel)?;
    let text = std::fs::read(path).ok()?;
    Some(excerpt(&String::from_utf8_lossy(&text), line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_excerpt_marks_the_line_and_keeps_its_number() {
        let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let (e, hit) = excerpt(&text, 50);
        assert_eq!(e.len(), 2 * CONTEXT + 1);
        assert_eq!((e[0].number, e[0].text.as_str()), (10, "line 10"));
        assert_eq!((hit, e[hit].number, e[hit].hit), (40, 50, true));
        assert_eq!(e.iter().filter(|l| l.hit).count(), 1);
        let (start, hit) = excerpt(&text, 0);
        assert_eq!((start[0].number, hit, start[0].hit), (1, 0, true));
        assert_eq!(excerpt(&text, 99).0.last().map(|l| l.number), Some(100));
    }

    #[test]
    fn files_outside_the_project_are_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("p");
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("Page.razor"), "@page \"/x\"\n<h1/>\n").expect("file");
        std::fs::write(dir.path().join("secret"), "x").expect("file");
        let (lines, hit) = read(&root, "Page.razor", 2).expect("read");
        assert_eq!((lines[hit].text.as_str(), lines.len()), ("<h1/>", 2));
        assert!(read(&root, "../secret", 1).is_none());
    }
}
