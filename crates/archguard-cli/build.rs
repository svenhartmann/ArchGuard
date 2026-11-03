//! Bakes the Rust target triple into the binary: `archguard update` picks the
//! release archive named after it.
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=ARCHGUARD_TARGET={target}");
}
