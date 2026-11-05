//! Compiles the Slint UI with bundled translations.
//!
//! Translations live in parts, one per tab (`lang/<lang>/*.po`, entries
//! only, no header), so tabs can be built independently. They are merged
//! into the single file per language that Slint expects
//! (`<out>/lang/<lang>/LC_MESSAGES/archguard-desktop.po`). A msgid must
//! appear in one part only.

use std::fs;
use std::path::{Path, PathBuf};

fn merge_translations(src: &Path, out: &Path) {
    println!("cargo:rerun-if-changed={}", src.display());
    for lang in fs::read_dir(src).expect("lang/ is readable").flatten() {
        if !lang.path().is_dir() {
            continue;
        }
        let code = lang.file_name().to_string_lossy().into_owned();
        let mut parts: Vec<PathBuf> = fs::read_dir(lang.path())
            .expect("language dir is readable")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "po"))
            .collect();
        parts.sort();
        let mut merged = format!(
            "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Language: {code}\\n\"\n\n"
        );
        for part in parts {
            println!("cargo:rerun-if-changed={}", part.display());
            merged.push_str(&fs::read_to_string(&part).expect("po part is readable"));
            merged.push('\n');
        }
        let dir = out.join(&code).join("LC_MESSAGES");
        fs::create_dir_all(&dir).expect("out dir is writable");
        fs::write(dir.join("archguard-desktop.po"), merged).expect("po is writable");
    }
}

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("lang");
    merge_translations(Path::new("lang"), &out);
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations(out)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("ui/app.slint compiles");
}
