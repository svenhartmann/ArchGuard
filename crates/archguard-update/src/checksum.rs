//! SHA-256 digests and `SHA256SUMS` files (`<hex>  <file name>` per line).

use sha2::{Digest, Sha256};

/// Lower-case hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Decodes a downloaded sums file named `sums_name`.
pub fn sums_text(bytes: Vec<u8>, sums_name: &str) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| format!("{sums_name} is not text"))
}

/// The digest `sums` (a file named `sums_name`) lists for the file `name`.
pub fn expected(sums: &str, sums_name: &str, name: &str) -> Result<String, String> {
    sums.lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, file)| *file == name)
        .map(|(digest, _)| digest.to_string())
        .ok_or_else(|| format!("{sums_name} lists no {name}"))
}

#[cfg(test)]
mod tests {
    use super::{expected, sha256_hex};

    #[test]
    fn digest_is_lower_hex_and_sums_lookup_matches_whole_names() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let sums = "aa  a.dmg\nbb  a.dmg.sha256\n";
        assert_eq!(expected(sums, "S", "a.dmg").unwrap(), "aa");
        assert_eq!(
            expected(sums, "S", "b.dmg").unwrap_err(),
            "S lists no b.dmg"
        );
    }
}
