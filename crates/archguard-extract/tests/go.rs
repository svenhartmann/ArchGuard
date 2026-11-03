use archguard_extract::{extractor_for, walk};
#[test]
fn go_files_have_an_import_scanner_and_ignore_string_or_comment_lookalikes() {
    let d = tempfile::tempdir().unwrap();
    let source = "package api\n// import \"bad/comment\"\nvar doc = `import \"bad/raw\"`\nimport \"fmt\"\nimport (\n alias \"example.org/shop/store\"\n _ `example.org/shop/init`\n . \"example.org/shop/utils\"\n)\n";
    std::fs::write(d.path().join("api.go"), source).unwrap();
    let files = walk(d.path(), &[]).unwrap();
    assert_eq!(files.len(), 1);
    let data = extractor_for(files[0].lang)
        .unwrap()
        .extract("api.go", source);
    assert_eq!(data.imports.len(), 4);
    assert_eq!(data.imports[0].line, 4);
    assert_eq!(data.imports[1].line, 6);
    assert_eq!(data.declarations.len(), 1);
}
#[test]
fn go_imports_decode_go_hex_octal_and_unicode_escapes() {
    let source = r#"package p
import "example.org/\x61\142\u0063"
"#;
    let data = archguard_extract::GoExtractor;
    let result = archguard_extract::Extractor::extract(&data, "p.go", source);
    assert!(
        matches!(&result.imports[..], [archguard_extract::RawImport {target:archguard_extract::TargetSpec::Symbol {path,..},..}] if path=="example.org/abc")
    );
}
