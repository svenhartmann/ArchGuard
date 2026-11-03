use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DocumentParams {
    pub path: Option<String>,
    /// architecture (default), or arc42.
    pub template: Option<String>,
    /// arc42 only: chapter 1..=12. Omit for the complete document.
    pub chapter: Option<u8>,
    /// arc42 only: Unicode character offset. Follow next_offset.
    pub offset: Option<usize>,
    /// arc42 only: 1..=2000 characters, additionally bounded by 4 KiB JSON.
    pub limit: Option<usize>,
}
impl DocumentParams {
    pub fn validate(&self) -> Result<bool, ErrorData> {
        let arc42 = match self.template.as_deref().unwrap_or("architecture") {
            "architecture" => false,
            "arc42" => true,
            _ => {
                return Err(crate::tools::invalid_params(
                    "template must be architecture or arc42",
                ));
            }
        };
        if !arc42 && (self.chapter.is_some() || self.offset.is_some() || self.limit.is_some()) {
            return Err(crate::tools::invalid_params(
                "chapter/offset/limit require template=arc42",
            ));
        }
        if self.chapter.is_some_and(|c| !(1..=12).contains(&c)) {
            return Err(crate::tools::invalid_params("chapter must be 1..=12"));
        }
        if self.limit.is_some_and(|c| !(1..=2000).contains(&c)) {
            return Err(crate::tools::invalid_params("limit must be 1..=2000"));
        }
        Ok(arc42)
    }
}

pub fn page(markdown: &str, p: &DocumentParams) -> Result<Value, ErrorData> {
    let chars: Vec<_> = markdown.chars().collect();
    let offset = p.offset.unwrap_or(0);
    if offset > chars.len() {
        return Err(crate::tools::invalid_params(
            "offset exceeds the document length",
        ));
    }
    let mut low = 0;
    let mut high = p.limit.unwrap_or(1000).min(chars.len() - offset);
    let value = |count: usize| json!({"template":"arc42","chapter":p.chapter,"offset":offset,"total_chars":chars.len(),"next_offset":(offset+count<chars.len()).then_some(offset+count),"markdown":chars[offset..offset+count].iter().collect::<String>()});
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if value(mid).to_string().len() <= crate::tools::MAX_RESPONSE_BYTES {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    Ok(value(low))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_and_escaped_markdown_round_trip_with_a_byte_bound() {
        let expected = "🦀 Ä 日本語 \\\"\n".repeat(1300);
        let mut params = DocumentParams {
            template: Some("arc42".into()),
            limit: Some(2000),
            ..Default::default()
        };
        let mut actual = String::new();
        loop {
            let row = page(&expected, &params).unwrap();
            assert!(row.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
            actual.push_str(row["markdown"].as_str().unwrap());
            if let Some(next) = row["next_offset"].as_u64() {
                assert!(next as usize > params.offset.unwrap_or(0));
                params.offset = Some(next as usize);
            } else {
                break;
            }
        }
        assert_eq!(actual, expected);
    }
}
