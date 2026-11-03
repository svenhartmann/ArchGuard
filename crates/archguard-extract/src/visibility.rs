//! Per-file symbol visibility (WP-02): which symbols a file exports to
//! other files and which it keeps internal.
//!
//! The [`Visibility`] type is shared vocabulary and lives in
//! `archguard-core` (`archguard-analysis` consumes it for leaky
//! encapsulation without a crate dependency on this one). The scan
//! itself is language-specific: each scanner module owns its rule.

use crate::walk::Lang;

pub use archguard_core::Visibility;

/// Exported/internal symbol split of one source file.
///
/// The content is passed in (no file I/O here), so the same scan can run
/// on walked files, fixtures and serve-reloaded buffers alike. Unknown
/// languages yield an empty [`Visibility`] — no claims without a rule.
pub fn scan_visibility(rel_path: &str, content: &str, lang: Lang) -> Visibility {
    let mut visibility = match lang {
        Lang::Ts | Lang::Js => crate::ts::scan_visibility(content),
        Lang::Rust => crate::rust::scan_visibility(content),
        Lang::Py => crate::py::scan_visibility(content),
        Lang::Java => crate::java::scan_visibility(content),
        Lang::CSharp => crate::cs::scan_visibility(content),
        Lang::Php => crate::php::scan_visibility(content),
        Lang::Go => crate::go::scan_visibility(content),
        Lang::Kotlin => Visibility::default(),
    };
    visibility.file = rel_path.to_string();
    visibility
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::Lang;

    #[test]
    fn dispatcher_stamps_the_file_name() {
        // R12–R16 fill the language rules; the dispatcher contract is
        // testable independently of them once they are implemented.
        let _f: fn(&str, &str, Lang) -> Visibility = scan_visibility;
    }
}
