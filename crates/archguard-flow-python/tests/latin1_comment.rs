//! One Latin-1 byte in a comment must not cost a file its routes: the text
//! is read lossily, identifiers are ASCII and survive.

use archguard_flow_common::extract_ctx::with_root_config;
use archguard_flow_python::ui_flow_flask;

fn endpoints(app: &[u8]) -> usize {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();
    std::fs::write(dir.path().join("app.py"), app).unwrap();
    with_root_config(dir.path(), ui_flow_flask::extract_with)
        .endpoints
        .len()
}

const CODE: &[u8] = b"from flask import Flask\napp = Flask(__name__)\n@app.route(\"/p\")\ndef p():\n    return \"x\"\n";

#[test]
fn utf8_comment_keeps_the_route() {
    assert_eq!(endpoints(&[b"# caf\xC3\xA9\n", CODE].concat()), 1);
}

#[test]
fn latin1_byte_in_a_comment_keeps_the_route() {
    assert_eq!(endpoints(&[b"# caf\xE9\n", CODE].concat()), 1);
}
