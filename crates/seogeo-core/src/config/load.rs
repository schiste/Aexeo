use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::Config;

pub fn load_config(root: &Path, explicit_path: Option<&Path>) -> Result<Config> {
    let config_path = explicit_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("seogeo.toml"));
    if !config_path.exists() {
        return Ok(Config::default());
    }
    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read config at {}", config_path.display()))?;
    let config = toml::from_str::<Config>(&text)
        .with_context(|| format!("failed to parse TOML config at {}", config_path.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::load_config;
    use crate::config::default_rule_switches;
    use std::fs;

    #[test]
    fn loads_defaults_when_config_is_missing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.adapter, "auto");
        assert_eq!(config.audit_log_limit, 5);
        assert!(
            default_rule_switches()
                .get("html")
                .copied()
                .unwrap_or(false)
        );
        assert_eq!(config.default_twitter_card, "summary");
    }

    #[test]
    fn loads_simple_toml_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(
            temp_dir.path().join("seogeo.toml"),
            r#"
site_url = "https://example.com"
profile = "chau7"
audit_log_limit = 9
[severity_overrides]
SEO001 = "warning"
[checks]
html = true
links = false
"#,
        )
        .unwrap();
        let config = load_config(temp_dir.path(), None).unwrap();
        assert_eq!(config.site_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.profile, "chau7");
        assert_eq!(config.audit_log_limit, 9);
        assert_eq!(
            config.severity_overrides.get("SEO001").map(String::as_str),
            Some("warning")
        );
        assert_eq!(config.checks.get("links").copied(), Some(false));
    }
}
