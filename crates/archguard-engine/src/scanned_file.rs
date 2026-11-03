//! The per-file record the pipeline and its Rust resolver share.
use archguard_extract::Extraction;

/// One scanned file with everything the pipeline derived from it.
pub(super) struct ScannedFile {
    /// Project-root-relative walked path (matches PSR-4 directories).
    pub(super) rel_path: String,
    /// Source-root-relative path (node naming, refs, DeclMap keys).
    pub(super) stripped: String,
    /// Name of the graph node representing this file.
    pub(super) node_name: String,
    /// Language id recorded on the file node.
    pub(super) lang: &'static str,
    /// What the scanner found in it.
    pub(super) extraction: Extraction,
}
