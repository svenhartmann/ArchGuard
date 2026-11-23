//! Blanking of comments and literal contents for the language readers of
//! ADR 0021/0022: offsets and lines stay those of the source.

/// `src` with every region `region_at` finds replaced by spaces (line breaks
/// kept), so offsets and lines stay those of the source. `region_at(b, i)`
/// gives (blank from, blank to, continue at) for a comment or literal at `i`.
/// A byte order mark is blanked too: it would glue onto the first identifier.
pub fn blank_regions(
    src: &str,
    region_at: impl Fn(&[u8], usize) -> Option<(usize, usize, usize)>,
) -> String {
    let mut b = src.as_bytes().to_vec();
    if b.starts_with("\u{FEFF}".as_bytes()) {
        b[..3].fill(b' ');
    }
    let mut i = 0;
    while i < b.len() {
        match region_at(&b, i) {
            Some((from, to, next)) => {
                blank(&mut b, from, to);
                i = next.max(i + 1);
            }
            None => i += 1,
        }
    }
    String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Spaces for `b[from..to]`, line breaks kept; bounds are clamped.
pub fn blank(b: &mut [u8], from: usize, to: usize) {
    let to = to.min(b.len());
    for c in &mut b[from.min(to)..to] {
        if *c != b'\n' {
            *c = b' ';
        }
    }
}

/// Offset of `pat` at or after `from`.
pub fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|p| from + p)
}

/// Offset of the line break ending the line of `i`, or the end.
pub fn line_end(b: &[u8], i: usize) -> usize {
    b[i..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |p| i + p)
}
