//! Tests of the shared language helpers other than TypeScript
//! (`lang_ts_tests.rs`): C#, Java/Kotlin, markup and blanked-text navigation.
use crate::{lang_csharp as cs, lang_java, lang_markup as markup, lang_text as text};

#[test]
fn csharp_mask_blanks_comments_preprocessor_and_literal_contents() {
    let src = "\u{FEFF}namespace A;\n#region x\nvar s = @\"a\"\"b\"; // c\nvar r = \"\"\"\n{x}\n\"\"\";\n\
               var i = $@\"{a}\\\"; char c = '\\''; char d = 'ä'; /* \"x\" */ var t = \"it's\";\n";
    let m = cs::mask(src);
    assert_eq!(m.len(), src.len());
    assert_eq!(m.lines().count(), src.lines().count());
    assert!(m.starts_with("   namespace A;\n         \n"), "{m:?}");
    // Verbatim strings double their quotes, `$@` and `@$` alike; a backslash ends nothing.
    assert!(m.contains("var s = @\"    \";"), "{m:?}");
    assert!(m.contains("var i = $@\"    \";"), "{m:?}");
    // Raw strings keep their three quotes; their lines stay.
    assert!(m.contains("var r = \"\"\"\n   \n\"\"\";"), "{m:?}");
    assert!(m.contains("char c = '  '; char d = '  ';"), "{m:?}");
    assert!(!m.contains("it's") && !m.contains("\"x\"") && !m.contains("// c"));
}

#[test]
fn csharp_code_keeps_literals_and_apostrophes_in_text_are_no_char_literal() {
    let src = "var u = \"http://x\"; // gone\n<p>Don't</p> @(\"/*\") /* gone */ x";
    assert_eq!(
        cs::code(src),
        "var u = \"http://x\";        \n<p>Don't</p> @(\"/*\")            x"
    );
    let (code, mask) = cs::clean("f(\"a\") // b");
    assert_eq!(
        (code.as_str(), mask.as_str()),
        ("f(\"a\")     ", "f(\" \")     ")
    );
}

#[test]
fn csharp_literals_and_arguments() {
    assert_eq!(cs::literal(" \"/orders\" ").as_deref(), Some("/orders"));
    assert_eq!(cs::literal("@\"a\"\"b\"").as_deref(), Some("a\"\"b"));
    assert_eq!(cs::literal("\"a\" + b"), None);
    assert_eq!(cs::literal("$\"/x/{id}\""), None);
    assert_eq!(cs::literal("\"\"\"raw\"\"\""), None);
    assert_eq!(cs::literal_end(b"'\\u0041' x", 0), Some(8));
    assert_eq!(cs::literal_end(b"'ab' x", 0), None);
    // Brackets and commas inside strings and char literals do not count.
    assert_eq!(
        cs::split_args("\"a,)\", ')', f(b, c), new[] { 1, 2 }"),
        ["\"a,)\"", "')'", "f(b, c)", "new[] { 1, 2 }"]
    );
    assert_eq!(cs::balanced("x(\")\")) + 1", ')'), "x(\")\")");
    assert_eq!(cs::ident_start("await Nav.NavigateTo"), 10);
    assert!(cs::is_ident('ä') && !cs::is_ident('$'));
}

#[test]
fn java_mask_reads_text_blocks_and_one_line_literals() {
    let src = "\u{FEFF}package a;\nString s = \"\"\"\n  x \"y\"\n\"\"\"; char c = '\"'; // c\n";
    let m = lang_java::mask(src);
    assert_eq!(m.len(), src.len());
    assert!(
        m.starts_with("   package a;\nString s = \"\"\"\n       \n\"\"\"; char c = ' ';     \n"),
        "{m:?}"
    );
}

#[test]
fn markup_tags_blocks_and_blanking() {
    let xaml = "<?xml?><Grid x:Name=\"g\"><Button Text=\"a > b\" Clicked=\"On\" />\
                <DataTemplate><Label/></DataTemplate></Grid>";
    let tags: Vec<&str> = markup::tags(xaml).map(|t| t.name).collect();
    assert_eq!(tags, ["Grid", "Button", "DataTemplate", "Label"]);
    let button = markup::tags(xaml).nth(1).unwrap();
    assert!(button.self_closing);
    assert_eq!(
        markup::attributes(button.attrs),
        [("Text", "a > b"), ("Clicked", "On")]
    );
    assert_eq!(markup::attr(button.attrs, "Clicked"), Some("On"));
    let blocks = markup::blocks(xaml, &["DataTemplate"]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(&xaml[blocks[0].0..blocks[0].0 + 13], "<DataTemplate");
    assert_eq!(markup::tag_end(" href=\"a>b\">x"), Some(11));
    assert_eq!(
        markup::blank("a<!--b\nc-->d@*e*@", "<!--", "-->"),
        "a     \n    d@*e*@"
    );
    assert_eq!(markup::strip_tags("<b>Save</b> <i>all</i>"), "Save all");
}

#[test]
fn blanked_text_navigation() {
    let s = "Map<String, List<Integer>> f = (a) -> a + 1, g";
    assert_eq!(text::split_top(s, ","), [&s[..43], " g"]);
    assert_eq!(text::first_assign(s), Some(29));
    assert_eq!(text::first_assign("x => y == z"), None);
    assert_eq!(text::find_top("f(a, b) + c(d)", "("), Some(1));
    assert_eq!(text::find_top("{ x(1) } y(2)", "y"), Some(9));
    assert_eq!(text::matching(b"(a[b](c))", 0), Some(8));
    assert_eq!(text::matching_back(b"(a[b](c))", 8), Some(0));
    assert_eq!(text::unparen(" ((a + b)) "), "a + b");
    assert_eq!(text::prev_word("return  (x", 8), "return");
    for (t, simple) in [
        ("a.b.Order<T>?", "Order"),
        ("() => Base", "Base"),
        ("Base()", "Base"),
        ("Item[]", "Item"),
        ("Item...", "Item"),
        ("(int, string)", "(int, string)"),
    ] {
        assert_eq!(text::simple(t), simple, "{t}");
    }
    let idents: Vec<&str> = text::idents("a1 $b 2c ä", 0, 11)
        .into_iter()
        .map(|x| x.1)
        .collect();
    assert_eq!(idents, ["a1", "$b", "ä"]);
    assert_eq!(text::line("a\nb\n", 99), 3);
    assert_eq!(text::collapse(" a \n b "), "a b");
}
