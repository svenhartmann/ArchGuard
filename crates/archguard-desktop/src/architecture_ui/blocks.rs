//! Chapter text for the window: the body the web shows under the chapter
//! heading, and Markdown blocks as `ArcBlock` rows.

use slint::{ModelRc, SharedString, StyledText, VecModel};

use super::markdown::{self, Align, Block};
use crate::ui::ArcBlock;

/// The chapter body after its `## N.` heading, as the web's `update()`;
/// with a derived diagram, chapter 5 drops its Mermaid source.
pub fn chapter_body(markdown: &str, number: u8, derived: bool) -> String {
    let marker = format!("## {number}.");
    let start = line_starts(markdown).find(|&i| {
        markdown[i..].starts_with(&marker)
            && markdown[i + marker.len()..].starts_with(char::is_whitespace)
    });
    let mut body = match start {
        None => markdown.to_string(),
        Some(i) => {
            let rest = &markdown[i..];
            rest.split_once('\n').map_or("", |(_, b)| b).to_string()
        }
    };
    if derived && number == 5 {
        body = drop_mermaid(&body);
    }
    body
}

/// Everything before the first `## N.` chapter heading.
pub fn preamble(markdown: &str) -> &str {
    let end = line_starts(markdown).find(|&i| {
        let rest = markdown[i..].strip_prefix("## ").unwrap_or("");
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && rest[digits..].starts_with('.')
    });
    &markdown[..end.unwrap_or(markdown.len())]
}

fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1))
}

/// `/```mermaid[\s\S]*?```/g` → "".
fn drop_mermaid(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("```mermaid") {
        out.push_str(&rest[..start]);
        match rest[start + 10..].find("```") {
            Some(end) => rest = &rest[start + 10 + end + 3..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn styled(text: &str) -> StyledText {
    StyledText::from_markdown(&markdown::inline_source(text))
        .unwrap_or_else(|_| StyledText::from_plain_text(&markdown::plain(text)))
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn base(kind: i32) -> ArcBlock {
    ArcBlock {
        kind,
        level: 0,
        marker: SharedString::new(),
        text: StyledText::default(),
        plain: SharedString::new(),
        cells: model(Vec::new()),
        heads: model(Vec::new()),
        align: model(Vec::new()),
        widths: model(Vec::new()),
        quote: false,
        open: false,
        last: false,
    }
}

/// Window rows for parsed blocks; tables become one row per table row.
pub fn ui_blocks(blocks: &[Block], muted_note: &str) -> Vec<ArcBlock> {
    let mut out = Vec::new();
    for b in blocks {
        push_block(b, false, muted_note, &mut out);
    }
    out
}

fn push_block(b: &Block, quote: bool, note: &str, out: &mut Vec<ArcBlock>) {
    let start = out.len();
    match b {
        Block::Heading { level, text } => out.push(ArcBlock {
            level: *level as i32,
            plain: markdown::plain(text).into(),
            open: text == "Offen zu ergänzen",
            ..base(0)
        }),
        Block::Paragraph(text) => out.push(ArcBlock {
            text: styled(text),
            ..base(1)
        }),
        Block::Item {
            depth,
            marker,
            text,
        } => out.push(ArcBlock {
            level: *depth as i32,
            marker: marker.into(),
            text: styled(text),
            ..base(2)
        }),
        Block::Table {
            columns,
            align,
            rows,
        } => push_table(columns, align, rows, out),
        Block::Code { lang, text } => {
            if lang == "mermaid" {
                out.push(ArcBlock {
                    text: StyledText::from_plain_text(note),
                    ..base(7)
                });
            }
            out.push(ArcBlock {
                plain: text.into(),
                ..base(5)
            });
        }
        Block::Quote(inner) => {
            for q in inner {
                push_block(q, true, note, out);
            }
        }
        Block::Rule => out.push(base(6)),
    }
    if quote {
        for row in &mut out[start..] {
            row.quote = true;
        }
    }
}

fn push_table(columns: &[String], align: &[Align], rows: &[Vec<String>], out: &mut Vec<ArcBlock>) {
    let align: Vec<i32> = align
        .iter()
        .map(|a| match a {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
        })
        .collect();
    let widths = model(column_widths(columns, rows));
    out.push(ArcBlock {
        heads: model(columns.iter().map(|c| markdown::plain(c).into()).collect()),
        widths: widths.clone(),
        ..base(3)
    });
    if rows.is_empty() {
        out.push(ArcBlock {
            last: true,
            ..base(8)
        });
        return;
    }
    for (i, row) in rows.iter().enumerate() {
        out.push(ArcBlock {
            cells: model(row.iter().map(|c| styled(c)).collect()),
            align: model(align.clone()),
            widths: widths.clone(),
            last: i + 1 == rows.len(),
            ..base(4)
        });
    }
}

/// Share of the table width per column, the same for every row: by the
/// longest text in the column (6 to 48 characters), at least 8 %.
fn column_widths(columns: &[String], rows: &[Vec<String>]) -> Vec<f32> {
    let weight = |i: usize| {
        std::iter::once(&columns[i])
            .chain(rows.iter().filter_map(|r| r.get(i)))
            .map(|c| markdown::plain(c).chars().count())
            .max()
            .unwrap_or(0)
            .clamp(6, 48) as f32
    };
    let weights: Vec<f32> = (0..columns.len()).map(weight).collect();
    let total: f32 = weights.iter().sum();
    let floor = (1.0 / columns.len().max(1) as f32).min(0.08);
    let raised: Vec<f32> = weights.iter().map(|w| (w / total).max(floor)).collect();
    let sum: f32 = raised.iter().sum();
    raised.iter().map(|w| w / sum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    const DOC: &str = "# arc42\n\nIntro **x**.\n\n## 5. Bausteinsicht\n\n### Paketübersicht\n\n```mermaid\nflowchart LR\n```\n\n| a | b |\n| --- | ---: |\n| 1 | 2 |\n";

    #[test]
    fn body_starts_after_the_chapter_heading() {
        let body = chapter_body(DOC, 5, false);
        assert!(body.starts_with("\n### Paketübersicht"));
        assert!(body.contains("```mermaid"));
        assert!(!chapter_body(DOC, 5, true).contains("mermaid"));
        // A chapter without its heading keeps the whole text.
        assert_eq!(chapter_body(DOC, 6, false), DOC);
        // "## 50." is not chapter 5.
        assert_eq!(chapter_body("## 50. x\nbody", 5, false), "## 50. x\nbody");
    }

    #[test]
    fn preamble_ends_before_the_first_chapter() {
        assert_eq!(preamble(DOC), "# arc42\n\nIntro **x**.\n\n");
        assert_eq!(preamble("no chapters"), "no chapters");
    }

    #[test]
    fn blocks_mark_open_headings_tables_and_mermaid_notes() {
        let md = "### Offen zu ergänzen\n\n- item\n\n| a | b |\n| --- | ---: |\n| 1 | 2 |\n| 3 | 4 |\n\n```mermaid\nflowchart LR\n```\n\n> quoted";
        let rows = ui_blocks(&markdown::parse(md), "note");
        let kinds: Vec<i32> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec![0, 2, 3, 4, 4, 7, 5, 1]);
        assert!(rows[0].open);
        assert_eq!(rows[2].heads.row_count(), 2);
        assert_eq!(rows[3].align.row_data(1), Some(2));
        assert!(!rows[3].last && rows[4].last);
        assert!(rows[7].quote);
        let empty = ui_blocks(&markdown::parse("| a | b |\n| --- | --- |\n"), "");
        assert_eq!(empty.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![3, 8]);
    }

    #[test]
    fn all_rows_of_a_table_share_column_widths_by_content() {
        let md = "| Baustein | Dateien | Nutzt |\n| --- | ---: | --- |\n| archguard-engine/src | 80 | archguard-core, archguard-extract/src, archguard-analysis |\n| core | 4 | – |\n";
        let rows = ui_blocks(&markdown::parse(md), "");
        let widths = |r: &ArcBlock| {
            (0..r.widths.row_count())
                .map(|i| r.widths.row_data(i).unwrap())
                .collect::<Vec<f32>>()
        };
        let head = widths(&rows[0]);
        assert_eq!(head.len(), 3);
        assert!(
            rows.iter().all(|r| widths(r) == head),
            "every row uses the header's widths"
        );
        assert!((head.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert!(head[2] > head[0] && head[0] > head[1], "{head:?}");
        assert!(head[1] >= 0.08, "a short column keeps a minimum: {head:?}");
    }
}
