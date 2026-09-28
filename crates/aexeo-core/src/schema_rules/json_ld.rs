use serde_json::Value;

pub(super) type SchemaObject = (Value, usize);

pub fn iter_schema_types(payload: &Value) -> Vec<String> {
    let mut found = Vec::new();
    match payload {
        Value::Object(map) => {
            if let Some(value) = map.get("@type") {
                match value {
                    Value::String(text) => found.push(text.clone()),
                    Value::Array(items) => {
                        for item in items {
                            if let Value::String(text) = item {
                                found.push(text.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                found.extend(iter_schema_types(nested));
            }
        }
        Value::Array(items) => {
            for item in items {
                found.extend(iter_schema_types(item));
            }
        }
        _ => {}
    }
    found
}

pub fn iter_schema_field_values(payload: &Value, field_name: &str) -> Vec<String> {
    let mut found = Vec::new();
    match payload {
        Value::Object(map) => {
            if let Some(value) = map.get(field_name) {
                match value {
                    Value::String(text) => found.push(text.clone()),
                    Value::Array(items) => {
                        for item in items {
                            if let Value::String(text) = item {
                                found.push(text.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                found.extend(iter_schema_field_values(nested, field_name));
            }
        }
        Value::Array(items) => {
            for item in items {
                found.extend(iter_schema_field_values(item, field_name));
            }
        }
        _ => {}
    }
    found
}

pub(super) fn iter_schema_objects(payload: &Value, depth: usize) -> Vec<SchemaObject> {
    let mut found = Vec::new();
    match payload {
        Value::Object(map) => {
            if map.contains_key("@type") {
                found.push((payload.clone(), depth));
            }
            for nested in map.values() {
                found.extend(iter_schema_objects(nested, depth + 1));
            }
        }
        Value::Array(items) => {
            for item in items {
                found.extend(iter_schema_objects(item, depth + 1));
            }
        }
        _ => {}
    }
    found
}

pub(super) fn required_fields_for_type(object_type: &str) -> Option<&'static [&'static str]> {
    match object_type {
        "WebSite" => Some(&["name", "url"]),
        "Organization" => Some(&["name", "url"]),
        "SoftwareApplication" => Some(&["name", "operatingSystem", "applicationCategory"]),
        "Product" => Some(&["name", "description"]),
        "Article" => Some(&["headline", "author"]),
        "TechArticle" => Some(&["headline", "author"]),
        "HowTo" => Some(&["name", "step"]),
        "ItemList" => Some(&["itemListElement"]),
        "SearchAction" => Some(&["target", "query-input"]),
        "Review" => Some(&["reviewRating", "author"]),
        "Offer" => Some(&["price", "priceCurrency"]),
        "VideoObject" => Some(&["name", "thumbnailUrl"]),
        "FAQPage" => Some(&["mainEntity"]),
        "BreadcrumbList" => Some(&["itemListElement"]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        iter_schema_field_values, iter_schema_objects, iter_schema_types, required_fields_for_type,
    };
    use serde_json::json;

    /// The single most common shape: a `@graph` array, which is how most
    /// schema generators emit more than one entity.
    #[test]
    fn collects_types_from_a_graph() {
        let payload = json!({
            "@context": "https://schema.org",
            "@graph": [
                { "@type": "WebSite", "name": "Example" },
                { "@type": "Organization", "name": "Example Inc" }
            ]
        });
        let mut types = iter_schema_types(&payload);
        types.sort();
        assert_eq!(types, vec!["Organization", "WebSite"]);
    }

    /// A single entity's `@type` may be a bare string or an array, and both
    /// appear in the wild.
    #[test]
    fn accepts_string_and_array_type_forms() {
        assert_eq!(
            iter_schema_types(&json!({ "@type": "Article" })),
            vec!["Article"]
        );
        assert_eq!(
            iter_schema_types(&json!({ "@type": ["Article", "BlogPosting"] })),
            vec!["Article", "BlogPosting"]
        );
    }

    /// Non-string `@type` members must be ignored rather than stringified.
    /// A payload like `{"@type": 42}` is malformed, and coercing it would
    /// produce a finding about a type that does not exist.
    #[test]
    fn ignores_non_string_type_members() {
        assert!(iter_schema_types(&json!({ "@type": 42 })).is_empty());
        assert!(iter_schema_types(&json!({ "@type": null })).is_empty());
        assert!(
            iter_schema_types(&json!({ "@type": ["Article", 7, null, "WebSite"] }))
                == vec!["Article", "WebSite"]
        );
    }

    /// Nested graphs: an entity's `mainEntity` that itself contains schema.
    #[test]
    fn recurses_through_nested_objects_and_arrays() {
        let payload = json!({
            "@graph": [{
                "@type": "FAQPage",
                "mainEntity": [{
                    "@type": "Question",
                    "acceptedAnswer": { "@type": "Answer", "text": "42" }
                }]
            }]
        });
        let mut types = iter_schema_types(&payload);
        types.sort();
        assert_eq!(types, vec!["Answer", "FAQPage", "Question"]);
    }

    #[test]
    fn returns_nothing_for_a_payload_with_no_schema() {
        assert!(iter_schema_types(&json!({ "name": "not schema" })).is_empty());
        assert!(iter_schema_types(&json!([])).is_empty());
        assert!(iter_schema_types(&json!("a string")).is_empty());
        assert!(iter_schema_types(&json!(null)).is_empty());
    }

    /// `iter_schema_field_values` powers the title-alignment and
    /// required-property rules, so the same shape handling matters.
    #[test]
    fn collects_named_field_values_in_both_forms() {
        let payload = json!({
            "@graph": [
                { "@type": "WebSite", "name": "Example" },
                { "@type": "Article", "headline": ["A", "B"] }
            ]
        });
        let mut names = iter_schema_field_values(&payload, "name");
        names.sort();
        assert_eq!(names, vec!["Example"]);
        assert_eq!(
            iter_schema_field_values(&payload, "headline"),
            vec!["A", "B"]
        );
    }

    /// A field the payload never mentions yields nothing rather than a
    /// placeholder, so "field absent" is distinguishable from "field empty".
    #[test]
    fn absent_field_yields_no_values() {
        let payload = json!({ "@type": "WebSite", "name": "Example" });
        assert!(
            iter_schema_field_values(&payload, "description").is_empty(),
            "an absent field must not be confused with an empty one"
        );
    }

    /// `iter_schema_objects` returns the object and its nesting depth, which
    /// is how a nested entity is told apart from a top-level one.
    #[test]
    fn reports_objects_with_their_depth() {
        let payload = json!({
            "@graph": [
                { "@type": "WebSite" },
                { "mainEntity": { "@type": "Organization" } }
            ]
        });
        let objects = iter_schema_objects(&payload, 0);
        let types: Vec<&str> = objects
            .iter()
            .map(|(value, _)| value["@type"].as_str().unwrap_or("?"))
            .collect();
        let depths: Vec<usize> = objects.iter().map(|(_, depth)| *depth).collect();
        assert_eq!(types, vec!["WebSite", "Organization"]);
        // `@graph` is an array under the root, so a member of it is two
        // levels down; the entity nested inside that member is three.
        assert_eq!(depths, vec![2, 3], "depths: {depths:?}");
    }

    #[test]
    fn objects_require_a_type_key() {
        // A nested object with no `@type` is not a schema entity.
        let objects = iter_schema_objects(&json!({ "name": "x", "nested": { "a": 1 } }), 0);
        assert!(objects.is_empty());
    }

    /// `required_fields_for_type` is the table behind the SCH required-field
    /// rules. An unknown type must return `None` so callers can skip it,
    /// not return an empty slice that reads as "nothing required".
    #[test]
    fn required_fields_are_declared_for_known_types_only() {
        assert_eq!(
            required_fields_for_type("WebSite"),
            Some(&["name", "url"][..])
        );
        assert_eq!(
            required_fields_for_type("SoftwareApplication"),
            Some(&["name", "operatingSystem", "applicationCategory"][..])
        );
        assert_eq!(required_fields_for_type("NotARealType"), None);
        assert_eq!(required_fields_for_type(""), None);
    }

    /// Every table entry must be non-empty and free of duplicates, or a rule
    /// silently becomes vacuous.
    #[test]
    fn required_field_entries_are_non_empty_and_unique() {
        const KNOWN: &[&str] = &[
            "WebSite",
            "Organization",
            "SoftwareApplication",
            "Product",
            "Article",
            "TechArticle",
            "HowTo",
            "ItemList",
            "SearchAction",
            "Review",
            "Offer",
            "VideoObject",
            "FAQPage",
            "BreadcrumbList",
        ];
        for object_type in KNOWN {
            let fields = required_fields_for_type(object_type)
                .unwrap_or_else(|| panic!("{object_type} should be in the table"));
            assert!(!fields.is_empty(), "{object_type} requires nothing");
            let mut unique = fields.to_vec();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                unique.len(),
                fields.len(),
                "{object_type} lists a field twice"
            );
            for field in fields.iter() {
                assert!(
                    !field.trim().is_empty(),
                    "{object_type} lists a blank field"
                );
            }
        }
    }
}
