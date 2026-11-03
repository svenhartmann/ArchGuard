//! `archguard-extract` — L1: file discovery and import extraction.
//!
//! [`walk`] discovers the source files of a project tree (T05); the
//! [`Extractor`] trait plus [`registry`] / [`extractor_for`] form the scanner
//! interface the language scanners (T08–T12) plug into, and [`TargetSpec`] /
//! [`RawImport`] / [`Declaration`] are what those scanners report before
//! resolution. [`resolve`] is that resolution (T07): the pure two-stage
//! mechanisms — path specs against the file tree, symbol specs against the
//! [`DeclMap`] of known modules.

pub mod cs;
pub mod debt;
pub mod extract;
pub mod go;
pub mod java;
mod lex;
pub mod php;
pub mod py;
pub mod resolve;
pub mod rust;
pub mod ts;
pub mod visibility;
pub mod walk;

pub use cs::CsExtractor;
pub use debt::{DebtMarker, MarkerKind, scan_debt};
pub use extract::{
    Declaration, Extraction, Extractor, RawImport, TargetSpec, extractor_for, registry,
};
pub use go::GoExtractor;
pub use java::JavaExtractor;
pub use php::{PhpExtractor, parse_psr4};
pub use py::PyExtractor;
pub use resolve::{DeclMap, SymbolTarget, resolve_relative, strip_root};
pub use rust::RustExtractor;
pub use ts::TsExtractor;
pub use visibility::{Visibility, scan_visibility};
pub use walk::{Lang, SourceFile, walk};
