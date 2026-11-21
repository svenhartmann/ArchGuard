//! The result rows narrowed by a search text and by collapsed packages.
//! A group is a package row with the file rows that follow it.

use std::collections::BTreeSet;

use super::diff::Row;

/// Indices into `rows` to show. A group shows when its package or one of its
/// files contains `query` (case-insensitive); the package's own match keeps
/// all its files, otherwise only the matching ones. Files of a collapsed
/// package stay hidden.
pub fn visible(rows: &[Row], query: &str, collapsed: &BTreeSet<String>) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let hit = |r: &Row| q.is_empty() || r.name.to_lowercase().contains(&q);
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let end = (i + 1..rows.len())
            .find(|&j| !rows[j].file)
            .unwrap_or(rows.len());
        let package = hit(&rows[i]);
        let files: Vec<usize> = (i + 1..end)
            .filter(|&j| !collapsed.contains(&rows[i].name) && (package || hit(&rows[j])))
            .collect();
        let file_match = (i + 1..end).any(|j| hit(&rows[j]));
        if package || file_match {
            out.push(i);
            out.extend(files);
        }
        i = end;
    }
    out
}

/// The packages that have files, i.e. the ones that can collapse.
pub fn groups(rows: &[Row]) -> BTreeSet<String> {
    rows.windows(2)
        .enumerate()
        .filter(|(_, w)| !w[0].file && w[1].file)
        .map(|(i, _)| rows[i].name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::diff::{Cycle, Status};

    fn row(name: &str, file: bool) -> Row {
        Row {
            name: name.into(),
            file,
            status: Status::Changed,
            cycle: Cycle::None,
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            row("core", false),
            row("core/a.ts", true),
            row("core/b.ts", true),
            row("ui", false),
            row("ui/v.ts", true),
            row("util", false),
        ]
    }

    #[test]
    fn empty_query_shows_everything() {
        let none = BTreeSet::new();
        assert_eq!(visible(&rows(), "  ", &none), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn package_match_keeps_its_files_file_match_keeps_only_itself() {
        let none = BTreeSet::new();
        assert_eq!(visible(&rows(), "CORE", &none), vec![0, 1, 2]);
        assert_eq!(visible(&rows(), "b.ts", &none), vec![0, 2]);
        assert_eq!(visible(&rows(), "zzz", &none), Vec::<usize>::new());
    }

    #[test]
    fn collapsed_package_hides_its_files() {
        let collapsed = BTreeSet::from(["core".to_string()]);
        assert_eq!(visible(&rows(), "", &collapsed), vec![0, 3, 4, 5]);
        assert_eq!(
            groups(&rows()),
            BTreeSet::from(["core".into(), "ui".into()])
        );
    }
}
