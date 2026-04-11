use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

use crate::config::Config;

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn source_dir_override(root: &Path, config: &Config) -> Option<PathBuf> {
    if config.source_dir == "." {
        return None;
    }
    let candidate = root.join(&config.source_dir);
    candidate
        .exists()
        .then(|| canonical_or_original(&candidate))
}

fn detect_nextjs_export(root: &Path) -> bool {
    root.join("out/index.html").exists() || root.join(".next/server/app").exists()
}

fn resolve_nextjs_export(root: &Path, config: &Config) -> PathBuf {
    if let Some(path) = source_dir_override(root, config) {
        return path;
    }
    let out_dir = root.join("out");
    canonical_or_original(if out_dir.exists() { &out_dir } else { root })
}

fn detect_astro_dist(root: &Path) -> bool {
    root.join("dist/index.html").exists()
        && (root.join("astro.config.mjs").exists() || root.join("astro.config.ts").exists())
}

fn resolve_astro_dist(root: &Path, config: &Config) -> PathBuf {
    if let Some(path) = source_dir_override(root, config) {
        return path;
    }
    let dist_dir = root.join("dist");
    canonical_or_original(if dist_dir.exists() { &dist_dir } else { root })
}

fn detect_docusaurus_build(root: &Path) -> bool {
    root.join("build/index.html").exists()
        && ["js", "mjs", "cjs", "ts"]
            .iter()
            .any(|ext| root.join(format!("docusaurus.config.{ext}")).exists())
}

fn resolve_docusaurus_build(root: &Path, config: &Config) -> PathBuf {
    if let Some(path) = source_dir_override(root, config) {
        return path;
    }
    let build_dir = root.join("build");
    canonical_or_original(if build_dir.exists() { &build_dir } else { root })
}

fn resolve_generic(root: &Path, config: &Config) -> PathBuf {
    if let Some(path) = source_dir_override(root, config) {
        return path;
    }
    canonical_or_original(root)
}

pub fn resolve_static_adapter_name(root: &Path, config: &Config) -> Result<&'static str> {
    if config.adapter != "auto" {
        return match config.adapter.as_str() {
            "nextjs-export" => Ok("nextjs-export"),
            "astro-dist" => Ok("astro-dist"),
            "docusaurus-build" => Ok("docusaurus-build"),
            "generic" => Ok("generic"),
            other => bail!("unknown adapter '{other}'"),
        };
    }

    if detect_nextjs_export(root) {
        return Ok("nextjs-export");
    }
    if detect_astro_dist(root) {
        return Ok("astro-dist");
    }
    if detect_docusaurus_build(root) {
        return Ok("docusaurus-build");
    }
    Ok("generic")
}

pub fn resolve_static_site_root(root: &Path, config: &Config) -> Result<PathBuf> {
    let adapter = resolve_static_adapter_name(root, config)?;
    Ok(match adapter {
        "nextjs-export" => resolve_nextjs_export(root, config),
        "astro-dist" => resolve_astro_dist(root, config),
        "docusaurus-build" => resolve_docusaurus_build(root, config),
        "generic" => resolve_generic(root, config),
        other => bail!("unknown adapter '{other}'"),
    })
}

#[cfg(test)]
mod tests {
    use super::{resolve_static_adapter_name, resolve_static_site_root};
    use crate::config::Config;
    use std::fs;

    #[test]
    fn auto_detects_astro_dist_output() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("dist/index.html"), "<html></html>").unwrap();
        fs::write(root.join("astro.config.mjs"), "export default {};").unwrap();

        let config = Config::default();
        assert_eq!(
            resolve_static_adapter_name(root, &config).unwrap(),
            "astro-dist"
        );
        assert_eq!(
            resolve_static_site_root(root, &config).unwrap(),
            root.join("dist").canonicalize().unwrap()
        );
    }

    #[test]
    fn explicit_source_dir_still_overrides_adapter_defaults() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("public-build")).unwrap();
        fs::write(root.join("public-build/index.html"), "<html></html>").unwrap();

        let config = Config {
            adapter: "astro-dist".to_string(),
            source_dir: "public-build".to_string(),
            ..Config::default()
        };

        assert_eq!(
            resolve_static_adapter_name(root, &config).unwrap(),
            "astro-dist"
        );
        assert_eq!(
            resolve_static_site_root(root, &config).unwrap(),
            root.join("public-build").canonicalize().unwrap()
        );
    }

    #[test]
    fn rejects_unknown_explicit_adapter() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            adapter: "unknown".to_string(),
            ..Config::default()
        };

        assert!(resolve_static_site_root(temp_dir.path(), &config).is_err());
    }
}
