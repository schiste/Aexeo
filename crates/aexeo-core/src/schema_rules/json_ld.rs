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
