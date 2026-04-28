use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockStyle {
    #[default]
    Normal,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    Blockquote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListItem {
    Bullet,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableTextSpan {
    #[serde(rename = "_key", default)]
    pub key: Option<String>,
    pub text: String,
    #[serde(default)]
    pub marks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkDef {
    #[serde(rename = "_key")]
    pub key: String,
    #[serde(rename = "_type")]
    pub mark_type: String,
    #[serde(flatten)]
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "_type", rename_all = "lowercase")]
pub enum PortableTextChild {
    Span(PortableTextSpan),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableTextBlock {
    #[serde(rename = "_key", default)]
    pub key: Option<String>,
    #[serde(default)]
    pub style: BlockStyle,
    #[serde(default, rename = "listItem", skip_serializing_if = "Option::is_none")]
    pub list_item: Option<ListItem>,
    #[serde(default)]
    pub level: Option<u32>,
    #[serde(default)]
    pub children: Vec<PortableTextChild>,
    #[serde(default, rename = "markDefs")]
    pub mark_defs: Vec<MarkDef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_canonical_block_with_link_markdef() {
        let raw = r#"{
            "_type": "block",
            "_key": "b1",
            "style": "h2",
            "markDefs": [{"_key": "m1", "_type": "link", "href": "https://example.com"}],
            "children": [
                {"_type": "span", "_key": "s1", "text": "Read ", "marks": []},
                {"_type": "span", "_key": "s2", "text": "the docs", "marks": ["m1", "strong"]}
            ]
        }"#;
        let block: PortableTextBlock = serde_json::from_str(raw).unwrap();
        assert_eq!(block.style, BlockStyle::H2);
        assert_eq!(block.children.len(), 2);
        let PortableTextChild::Span(second) = &block.children[1];
        assert_eq!(second.text, "the docs");
        assert_eq!(second.marks, vec!["m1".to_string(), "strong".to_string()]);
        assert_eq!(block.mark_defs.len(), 1);
        assert_eq!(block.mark_defs[0].mark_type, "link");
        assert_eq!(
            block.mark_defs[0].data.get("href").and_then(|v| v.as_str()),
            Some("https://example.com")
        );
    }
}
