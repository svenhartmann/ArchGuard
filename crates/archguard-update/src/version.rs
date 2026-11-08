//! Release versions: `x.y.z`, compared numerically.

/// `x.y.z` with an optional `-pre` suffix ignored; `latest` never names a pre-release.
pub fn parse(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('-').next()?.split('.').map(|p| p.parse().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn versions_compare_numerically_and_reject_malformed_tags() {
        assert!(parse("0.10.0") > parse("0.9.9"));
        assert_eq!(parse("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(parse("1.2"), None);
        assert_eq!(parse("1.2.3.4"), None);
        assert_eq!(parse("latest"), None);
    }
}
