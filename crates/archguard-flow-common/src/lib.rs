//! `archguard-flow-common` — L2: what the UI-flow and domain readers
//! (ADR 0021, 0022) share: the extract context, file discovery, no-follow
//! reads and the text lexers over masked sources.

pub mod extract_ctx;
#[cfg(test)]
mod extract_ctx_tests;
pub mod lang_blank;
pub mod lang_code;
pub mod lang_csharp;
pub mod lang_guard;
pub mod lang_java;
pub mod lang_markup;
pub mod lang_php;
pub mod lang_pyscan;
pub mod lang_python;
pub mod lang_rust;
#[cfg(test)]
mod lang_tests;
pub mod lang_text;
pub mod lang_ts;
pub mod lang_ts_decl;
pub mod lang_ts_http;
pub mod lang_ts_imports;
#[cfg(test)]
mod lang_ts_tests;
pub mod repo_file;
pub mod support;
