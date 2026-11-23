//! `archguard-flow-clike` — L2: the kernel the Java, Kotlin and C# readers
//! (ADR 0021, 0022) share: one source with comments blanked, class and method
//! declarations with their attributes, raw-text lexing, the requests a handler
//! sends, and the Quarkus build-file check.

pub mod decl;
pub mod handlers;
pub mod lex;
pub mod quarkus_build;
pub mod source;
pub mod supertypes;
