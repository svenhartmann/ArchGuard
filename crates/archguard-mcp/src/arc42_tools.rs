use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DocumentParams {
    pub path: Option<String>,
    /// architecture (default), arc42, or structurizr (C4 workspace DSL).
    pub template: Option<String>,
    /// arc42 only: chapter 1..=12. Omit for the complete document.
    pub chapter: Option<u8>,
    /// arc42 only: markdown (default), asciidoc, or catalogue (the diagram
    /// catalogue as JSON text: id, chapter, title, kind, shown, chosen,
    /// in_text, elements, relations).
    pub format: Option<String>,
    /// structurizr only: service id or unique name whose packages become components.
    pub service: Option<String>,
    /// arc42/structurizr: Unicode character offset. Follow next_offset.
    pub offset: Option<usize>,
    /// arc42/structurizr: 1..=2000 characters, additionally bounded by 4 KiB JSON.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Architecture,
    Arc42,
    Structurizr,
}

impl DocumentParams {
    pub fn validate(&self) -> Result<Template, ErrorData> {
        let template = match self.template.as_deref().unwrap_or("architecture") {
            "architecture" => Template::Architecture,
            "arc42" => Template::Arc42,
            "structurizr" => Template::Structurizr,
            _ => {
                return Err(crate::tools::invalid_params(
                    "template must be architecture, arc42 or structurizr",
                ));
            }
        };
        let paged = template != Template::Architecture;
        if !paged && (self.offset.is_some() || self.limit.is_some()) {
            return Err(crate::tools::invalid_params(
                "offset/limit require template=arc42 or structurizr",
            ));
        }
        if template != Template::Arc42 && (self.chapter.is_some() || self.format.is_some()) {
            return Err(crate::tools::invalid_params(
                "chapter/format require template=arc42",
            ));
        }
        if !matches!(
            self.format.as_deref(),
            None | Some("markdown" | "asciidoc" | "catalogue")
        ) {
            return Err(crate::tools::invalid_params(
                "format must be markdown, asciidoc or catalogue",
            ));
        }
        if template != Template::Structurizr && self.service.is_some() {
            return Err(crate::tools::invalid_params(
                "service requires template=structurizr",
            ));
        }
        if self.chapter.is_some_and(|c| !(1..=12).contains(&c)) {
            return Err(crate::tools::invalid_params("chapter must be 1..=12"));
        }
        if self.limit.is_some_and(|c| !(1..=2000).contains(&c)) {
            return Err(crate::tools::invalid_params("limit must be 1..=2000"));
        }
        Ok(template)
    }

    /// The JSON key of the paged text.
    fn key(&self) -> &'static str {
        match (self.template.as_deref(), self.format.as_deref()) {
            (Some("structurizr"), _) => "dsl",
            (_, Some("asciidoc")) => "asciidoc",
            (_, Some("catalogue")) => "catalogue",
            _ => "markdown",
        }
    }
}

/// The arc42 document in the requested format; the catalogue as JSON
/// text against the project's arc42 text, as `doc --arc42 --format json`.
pub fn arc42_text(
    run: &crate::tools::Run,
    doc: &archguard_engine::report::Arc42Document,
    p: &DocumentParams,
) -> Result<String, ErrorData> {
    let markdown = doc.markdown(p.chapter);
    Ok(match p.format.as_deref() {
        Some("asciidoc") => archguard_engine::report::markdown_to_asciidoc(&markdown),
        Some("catalogue") => {
            let format = run.loaded.config.arc42.diagram_format;
            let files = archguard_engine::arc42_file::Arc42Files::open(&run.root, format)
                .map_err(crate::tools::invalid_params)?;
            let all = archguard_engine::arc42_data::catalogue(&files, doc, p.chapter);
            Value::from(all).to_string()
        }
        _ => markdown,
    })
}

/// The Structurizr workspace of `run`, with `service`'s components.
pub fn structurizr(run: &crate::tools::Run, p: &DocumentParams) -> Result<String, ErrorData> {
    let invalid = |e: String| crate::tools::invalid_params(e);
    let topology =
        archguard_engine::analyze_services(&run.root).map_err(|e| invalid(e.to_string()))?;
    let service = match p.service.as_deref() {
        Some(name) => Some(archguard_engine::c4_service(&topology, name).map_err(invalid)?),
        None => None,
    };
    archguard_engine::structurizr_workspace(
        &run.root,
        &run.loaded.config,
        &run.file_graph,
        &topology,
        service,
    )
    .map_err(|e| invalid(e.to_string()))
}

pub fn page(text: &str, p: &DocumentParams) -> Result<Value, ErrorData> {
    let chars: Vec<_> = text.chars().collect();
    let offset = p.offset.unwrap_or(0);
    if offset > chars.len() {
        return Err(crate::tools::invalid_params(
            "offset exceeds the document length",
        ));
    }
    let mut low = 0;
    let mut high = p.limit.unwrap_or(1000).min(chars.len() - offset);
    let template = p.template.as_deref().unwrap_or("arc42");
    let value = |count: usize| {
        let mut v = json!({"template":template,"chapter":p.chapter,"offset":offset,"total_chars":chars.len(),"next_offset":(offset+count<chars.len()).then_some(offset+count)});
        v[p.key()] = chars[offset..offset + count]
            .iter()
            .collect::<String>()
            .into();
        v
    };
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
