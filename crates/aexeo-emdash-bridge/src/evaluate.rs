use seogeo_contracts::Finding;
use seogeo_core::{Config, run_checks_for_site};

use crate::document::EmdashDocument;
use crate::site::build_site_from_documents;

pub fn evaluate_documents(
    documents: &[EmdashDocument],
    config: &Config,
) -> anyhow::Result<Vec<Finding>> {
    let site = build_site_from_documents(documents)?;
    Ok(run_checks_for_site(&site, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portable_text::{
        BlockStyle, PortableTextBlock, PortableTextChild, PortableTextSpan,
    };

    fn rich_document() -> EmdashDocument {
        EmdashDocument {
            route: "/about".to_string(),
            title: "About Acme Robotics".to_string(),
            description: Some("The story behind our company and team.".to_string()),
            canonical: Some("https://example.com/about".to_string()),
            lang: Some("en".to_string()),
            alternates: Vec::new(),
            meta: Default::default(),
            schema: Vec::new(),
            body: vec![PortableTextBlock {
                key: Some("h".to_string()),
                style: BlockStyle::H1,
                list_item: None,
                level: None,
                children: vec![PortableTextChild::Span(PortableTextSpan {
                    key: None,
                    text: "About Acme Robotics".to_string(),
                    marks: Vec::new(),
                })],
                mark_defs: Vec::new(),
            }],
        }
    }

    #[test]
    fn evaluates_documents_without_error_using_default_config() {
        let config = Config::default();
        let findings = evaluate_documents(&[rich_document()], &config).unwrap();
        // Every finding the evaluator returns must carry a stable Aexeo rule id.
        for finding in &findings {
            assert!(
                !finding.rule_id.is_empty(),
                "rule_id must never be empty: {finding:?}"
            );
        }
    }

    #[test]
    fn evaluate_routes_findings_through_the_same_rule_ids_a_static_site_would_see() {
        let mut document = rich_document();
        // Drop the description; seogeo-core SEO002 fires on missing meta description.
        document.description = None;
        let config = Config::default();
        let findings = evaluate_documents(&[document], &config).unwrap();
        assert!(
            findings.iter().any(|finding| finding.rule_id == "SEO002"),
            "missing description should produce SEO002 finding; got: {findings:?}"
        );
    }
}
