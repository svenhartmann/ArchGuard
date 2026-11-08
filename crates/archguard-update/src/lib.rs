//! `archguard-update` — self-update from the latest published GitHub release,
//! shared by the `archguard` CLI and the desktop app.
//!
//! [`release`] looks up the latest release and its assets, [`version`]
//! compares versions, [`http`] downloads with a size limit and [`checksum`]
//! checks bytes against a `SHA256SUMS` file. [`binary`] is the CLI's archive
//! install.
//! Drafts and pre-releases are invisible to `releases/latest`.

pub mod binary;
pub mod checksum;
pub mod http;
pub mod release;
pub mod version;
