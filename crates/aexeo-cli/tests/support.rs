use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_aexeo-cli")
}

pub fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

pub fn parse_json(output: &[u8]) -> Value {
    serde_json::from_slice(output).unwrap()
}

#[allow(dead_code)]
pub fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(path)
}
