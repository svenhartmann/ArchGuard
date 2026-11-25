//! Deterministic broken variants of a source text, shared by the no-panic tests of the flow crates.
#![allow(dead_code)]

/// Fixed-seed byte noise (LCG), so a failing variant reproduces.
pub fn noise(n: usize) -> Vec<u8> {
    let mut x: u32 = 0x2545_F491;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

/// Empty, prefixes at a fixed stride (cut mid token and mid UTF-8 sequence), noise,
/// noise after a prefix, unbalanced openers, and invalid UTF-8 in the middle.
pub fn variants(seed: &str) -> Vec<Vec<u8>> {
    let b = seed.as_bytes();
    let stride = (b.len() / 40).max(1);
    let mut out = vec![Vec::new()];
    out.extend((stride..b.len()).step_by(stride).map(|n| b[..n].to_vec()));
    out.push(noise(512));
    out.push([&b[..b.len() / 2], &noise(64)[..]].concat());
    out.push([&b"{{{((([[[\"'`<!--/*"[..], b].concat());
    out.push(
        [
            &b[..b.len() / 2],
            &[0xff, 0xfe, 0, 0xc3][..],
            &b[b.len() / 2..],
        ]
        .concat(),
    );
    out
}

/// The same variants as text, for functions that take `&str`.
pub fn text_variants(seed: &str) -> Vec<String> {
    variants(seed)
        .iter()
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .collect()
}
