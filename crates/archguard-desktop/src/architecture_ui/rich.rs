//! The arc42 chapter editor that edits formatted text in place (WYSIWYG).
//! Slint has no input for styled text, so the editor keeps its own model:
//! [`inline`] holds a block's text with its marks, [`doc`] the chapter as
//! blocks that write back to Markdown.

pub mod doc;
pub mod editor;
pub mod inline;
