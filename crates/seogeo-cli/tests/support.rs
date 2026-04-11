use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_seogeo-cli")
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
