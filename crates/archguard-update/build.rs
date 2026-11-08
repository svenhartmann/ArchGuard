//! Bakes the Rust target triple into the crate: the desktop app picks the
//! release package named after it (`release::TARGET`).
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=ARCHGUARD_TARGET={target}");
}
