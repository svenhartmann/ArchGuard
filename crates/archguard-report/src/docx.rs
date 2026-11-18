//! Markdown → Word (.docx, ADR 0013) for the arc42 document, so it can be
//! imported into Confluence ("Import Word document") or edited in Word.
//! Headings use the built-in styles Heading 1–4 (Confluence builds the
//! page tree and the table of contents from them), tables are Word
//! tables, lists are Word lists. The document model has no rendered
//! diagram images: Mermaid and PlantUML stay as source in a code block,
//! labelled with their language. HTML comments are dropped.

use std::fmt::Write as _;
use std::io::{Cursor, Write as _};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// The .docx package of the CommonMark+tables document `md`.
#[tracing::instrument(skip_all)]
pub fn markdown_to_docx(md: &str) -> Vec<u8> {
    let mut w = Writer::default();
    // The parser splits text at escapes; one run per text keeps it whole.
    let mut text = String::new();
    for event in Parser::new_ext(md, Options::ENABLE_TABLES) {
        if let Event::Text(t) = &event {
            text.push_str(t);
            continue;
        }
        if !text.is_empty() {
            w.event(Event::Text(std::mem::take(&mut text).into()));
        }
        w.event(event);
    }
    w.close();
    package(&w)
}

#[derive(Default)]
struct Writer {
    body: String,
    /// The table being written; paragraphs go into `cell` meanwhile.
    table: Option<String>,
    cell: Option<String>,
    header: bool,
    /// The open paragraph: properties and runs.
    para: Option<(String, String)>,
    /// Style of the next paragraph (heading or code).
    style: Option<&'static str>,
    bold: usize,
    italic: usize,
    /// Open lists: numbering instance id.
    lists: Vec<usize>,
    /// Numbering instances: `true` for ordered (each restarts at 1).
    nums: Vec<bool>,
    /// External link targets; relationship ids are `link{index}`.
    links: Vec<String>,
    link: Option<usize>,
    code: Option<String>,
    /// The lines of an open HTML block.
    html: Option<String>,
}

impl Writer {
    fn target(&mut self) -> &mut String {
        match (&mut self.cell, &mut self.table) {
            (Some(cell), _) => cell,
            (None, Some(table)) => table,
            (None, None) => &mut self.body,
        }
    }

    fn open(&mut self) {
        if self.para.is_some() {
            return;
        }
        let mut props = String::new();
        if let Some(style) = self.style {
            let _ = write!(props, "<w:pStyle w:val=\"{style}\"/>");
        } else if let Some(&num) = self.lists.last() {
            let _ = write!(
                props,
                "<w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{num}\"/></w:numPr>",
                self.lists.len().min(9) - 1
            );
        }
        self.para = Some((props, String::new()));
    }

    fn close(&mut self) {
        if let Some((props, runs)) = self.para.take() {
            let p = format!("<w:p><w:pPr>{props}</w:pPr>{runs}</w:p>");
            self.target().push_str(&p);
        }
    }

    fn run(&mut self, text: &str, code: bool) {
        self.open();
        let mut props = String::new();
        if self.bold > 0 || self.header {
            props.push_str("<w:b/>");
        }
        if self.italic > 0 {
            props.push_str("<w:i/>");
        }
        if code {
            props.push_str(
                "<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/>",
            );
        }
        if self.link.is_some() {
            props.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        }
        let run = format!(
            "<w:r><w:rPr>{props}</w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>",
            esc(text)
        );
        let (_, runs) = self.para.as_mut().expect("open paragraph");
        match self.link {
            Some(i) => {
                let _ = write!(runs, "<w:hyperlink r:id=\"link{i}\">{run}</w:hyperlink>");
            }
            None => runs.push_str(&run),
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => match &mut self.code {
                Some(code) => code.push_str(&t),
                None => self.run(&t, false),
            },
            Event::Code(c) => self.run(&c, true),
            Event::SoftBreak => self.run(" ", false),
            Event::HardBreak => {
                self.open();
                self.para
                    .as_mut()
                    .expect("open")
                    .1
                    .push_str("<w:r><w:br/></w:r>");
            }
            Event::Rule => {
                self.close();
                self.body.push_str("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"auto\"/></w:pBdr></w:pPr></w:p>");
            }
            // Comments are editing hints of the Markdown file; other HTML
            // (e.g. a type like `List<T>` in prose) is kept as text.
            Event::Html(h) => self.html.get_or_insert_default().push_str(&h),
            Event::InlineHtml(h) if !h.starts_with("<!--") => self.run(&h, false),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph | Tag::Item => self.close(),
            Tag::Heading { level, .. } => {
                self.close();
                const STYLES: [&str; 4] = ["Heading1", "Heading2", "Heading3", "Heading4"];
                self.style = Some(STYLES[(level as usize).min(4) - 1]);
            }
            Tag::CodeBlock(kind) => self.code_start(kind),
            Tag::List(start) => {
                self.close();
                self.nums.push(start.is_some());
                self.lists.push(self.nums.len());
            }
            Tag::Table(aligns) => {
                self.close();
                let grid = "<w:gridCol/>".repeat(aligns.len());
                self.table = Some(format!(
                    "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>"
                ));
            }
            Tag::TableHead => {
                self.header = true;
                self.target()
                    .push_str("<w:tr><w:trPr><w:tblHeader/></w:trPr>");
            }
            Tag::TableRow => self.target().push_str("<w:tr>"),
            Tag::TableCell => self.cell = Some(String::new()),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Link { dest_url, .. } => {
                self.links.push(dest_url.to_string());
                self.link = Some(self.links.len() - 1);
            }
            _ => {}
        }
    }

    /// An HTML block other than a comment is kept as text.
    fn html_end(&mut self) {
        let html = self.html.take().unwrap_or_default();
        if !html.trim_start().starts_with("<!--") {
            self.run(html.trim_end(), false);
            self.close();
        }
    }

    fn code_start(&mut self, kind: CodeBlockKind<'_>) {
        self.close();
        if let CodeBlockKind::Fenced(lang) = kind
            && matches!(lang.as_ref(), "mermaid" | "plantuml")
        {
            // No rendered image exists; say what the source is.
            self.style = Some("Caption");
            self.run(&format!("{lang} diagram source"), false);
            self.close();
        }
        self.code = Some(String::new());
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Item => self.close(),
            TagEnd::HtmlBlock => self.html_end(),
            TagEnd::Heading(_) => {
                self.close();
                self.style = None;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                self.style = Some("Code");
                for line in code.strip_suffix('\n').unwrap_or(&code).split('\n') {
                    self.run(line, true);
                    self.close();
                }
                self.style = None;
            }
            TagEnd::List(_) => {
                self.close();
                self.lists.pop();
            }
            TagEnd::TableCell => {
                self.close();
                let mut cell = self.cell.take().unwrap_or_default();
                if cell.is_empty() {
                    cell.push_str("<w:p/>");
                }
                let tc = format!(
                    "<w:tc><w:tcPr><w:tcW w:w=\"0\" w:type=\"auto\"/></w:tcPr>{cell}</w:tc>"
                );
                self.target().push_str(&tc);
            }
            TagEnd::TableHead => {
                self.header = false;
                self.target().push_str("</w:tr>");
            }
            TagEnd::TableRow => self.target().push_str("</w:tr>"),
            TagEnd::Table => {
                let table = self.table.take().unwrap_or_default();
                self.body.push_str(&table);
                self.body.push_str("</w:tbl><w:p/>");
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Link => self.link = None,
            _ => {}
        }
    }
}

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

/// Paragraph style with an outline level, for the headings.
fn heading(n: u8, size: u8) -> String {
    format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"80\"/><w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"{size}\"/></w:rPr></w:style>",
        n - 1
    )
}

fn styles() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles {W}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>{}{}{}{}<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"40\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"0\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F2F2\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/><w:sz w:val=\"18\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Caption\"><w:name w:val=\"caption\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"40\"/></w:pPr><w:rPr><w:i/><w:sz w:val=\"18\"/></w:rPr></w:style><w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style><w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/></w:tblBorders></w:tblPr></w:style></w:styles>",
        heading(1, 36),
        heading(2, 30),
        heading(3, 26),
        heading(4, 22)
    )
}

/// Abstract numbering 1 is a bullet list, 2 a decimal list; every list
/// gets its own instance so numbered lists restart at 1.
fn numbering(nums: &[bool]) -> String {
    let level = |i: usize, ordered: bool| {
        let (fmt, text) = if ordered {
            ("decimal", format!("%{}.", i + 1))
        } else {
            ("bullet", "•".to_owned())
        };
        format!(
            "<w:lvl w:ilvl=\"{i}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>",
            720 * (i + 1)
        )
    };
    let abstracts: String = [false, true]
        .iter()
        .enumerate()
        .map(|(a, &ordered)| {
            let levels: String = (0..9).map(|i| level(i, ordered)).collect();
            format!(
                "<w:abstractNum w:abstractNumId=\"{}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{levels}</w:abstractNum>",
                a + 1
            )
        })
        .collect();
    let instances: String = nums
        .iter()
        .enumerate()
        .map(|(i, &ordered)| {
            format!(
                "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
                i + 1,
                if ordered { 2 } else { 1 }
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering {W}>{abstracts}{instances}</w:numbering>"
    )
}

fn package(w: &Writer) -> Vec<u8> {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document {W}><w:body>{}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1417\" w:right=\"1417\" w:bottom=\"1134\" w:left=\"1417\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
        w.body
    );
    let links: String = w
        .links
        .iter()
        .enumerate()
        .map(|(i, url)| {
            format!(
                "<Relationship Id=\"link{i}\" Type=\"{REL}/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
                esc(url)
            )
        })
        .collect();
    let rels = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"styles\" Type=\"{REL}/styles\" Target=\"styles.xml\"/><Relationship Id=\"numbering\" Type=\"{REL}/numbering\" Target=\"numbering.xml\"/>{links}</Relationships>"
    );
    let parts = [
        ("[Content_Types].xml", CONTENT_TYPES.to_owned()),
        ("_rels/.rels", ROOT_RELS.to_owned()),
        ("word/_rels/document.xml.rels", rels),
        ("word/document.xml", document),
        ("word/styles.xml", styles()),
        ("word/numbering.xml", numbering(&w.nums)),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    // A fixed time keeps equal documents byte-identical.
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, text) in parts {
        zip.start_file(name, options)
            .and_then(|()| zip.write_all(text.as_bytes()).map_err(Into::into))
            .expect("writing to memory cannot fail");
    }
    zip.finish()
        .expect("writing to memory cannot fail")
        .into_inner()
}

#[cfg(test)]
mod tests {
    use super::markdown_to_docx;
    use std::io::{Cursor, Read};

    fn part(docx: &[u8], name: &str) -> String {
        let mut zip = zip::ZipArchive::new(Cursor::new(docx)).expect("zip");
        let mut text = String::new();
        zip.by_name(name)
            .expect(name)
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    const MD: &str = "# arc42\n\nScope **cs** & <x>.\n\n## 1. Intro\n\n<!-- archguard:arc42:01:start -->\n### Goals\n\n<!-- multi\nline note -->\n\n- one\n- [two](https://arc42.org/)\n\n| Source | Files |\n| --- | ---: |\n| a\\|b | 3 |\n\n```mermaid\nflowchart LR\n  e0[\"a\"]\n```\n\n1. first\n";

    #[test]
    fn headings_use_word_heading_styles() {
        let doc = part(&markdown_to_docx(MD), "word/document.xml");
        assert!(doc.contains("<w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:rPr></w:rPr><w:t xml:space=\"preserve\">arc42</w:t>"), "{doc}");
        assert!(doc.contains("<w:pStyle w:val=\"Heading2\"/>"), "{doc}");
        assert!(doc.contains("<w:pStyle w:val=\"Heading3\"/>"), "{doc}");
        let styles = part(&markdown_to_docx(MD), "word/styles.xml");
        assert!(styles.contains("<w:name w:val=\"heading 1\"/>"), "{styles}");
        assert!(styles.contains("<w:outlineLvl w:val=\"2\"/>"), "{styles}");
    }

    #[test]
    fn tables_lists_links_and_text_are_word_structures() {
        let docx = markdown_to_docx(MD);
        let doc = part(&docx, "word/document.xml");
        assert_eq!(doc.matches("<w:tbl>").count(), 1, "{doc}");
        assert_eq!(doc.matches("<w:tc>").count(), 4, "{doc}");
        assert!(doc.contains("<w:tblHeader/>"), "{doc}");
        assert!(doc.contains(">a|b</w:t>"), "{doc}");
        assert!(doc.contains("Scope </w:t>"), "{doc}");
        assert!(
            doc.contains("<w:b/></w:rPr><w:t xml:space=\"preserve\">cs</w:t>"),
            "{doc}"
        );
        assert!(
            doc.contains(">&lt;x&gt;</w:t>"),
            "inline HTML stays text: {doc}"
        );
        assert!(
            !doc.contains("archguard:arc42"),
            "comments are dropped: {doc}"
        );
        assert!(
            !doc.contains("line note"),
            "multi-line comments are dropped: {doc}"
        );
        assert_eq!(doc.matches("<w:numId w:val=\"1\"/>").count(), 2, "{doc}");
        assert_eq!(doc.matches("<w:numId w:val=\"2\"/>").count(), 1, "{doc}");
        assert!(doc.contains("<w:hyperlink r:id=\"link0\">"), "{doc}");
        let rels = part(&docx, "word/_rels/document.xml.rels");
        assert!(
            rels.contains("Target=\"https://arc42.org/\" TargetMode=\"External\""),
            "{rels}"
        );
        let numbering = part(&docx, "word/numbering.xml");
        assert!(
            numbering.contains("<w:num w:numId=\"2\"><w:abstractNumId w:val=\"2\"/>"),
            "{numbering}"
        );
    }

    #[test]
    fn diagrams_stay_source_code_with_a_label() {
        let doc = part(&markdown_to_docx(MD), "word/document.xml");
        assert!(doc.contains(">mermaid diagram source</w:t>"), "{doc}");
        assert!(doc.contains("<w:pStyle w:val=\"Code\"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/></w:rPr><w:t xml:space=\"preserve\">  e0[&quot;a&quot;]</w:t>"), "{doc}");
    }

    #[test]
    fn equal_input_gives_equal_bytes() {
        assert_eq!(markdown_to_docx(MD), markdown_to_docx(MD));
    }
}
