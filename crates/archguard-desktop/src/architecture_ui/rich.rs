//! The arc42 chapter editor that edits formatted text in place (WYSIWYG).
//! Slint has no input for styled text, so the editor keeps its own model:
//! [`inline`] holds a block's text with its marks, [`doc`] the chapter as
//! blocks that write back to Markdown, [`editor`] the caret, selection and
//! edits, and [`layout`] lays blocks out as Slint would draw them.

pub mod doc;
pub mod editor;
pub mod inline;
pub mod layout;
