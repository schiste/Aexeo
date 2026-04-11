use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_seogeo-cli")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap()
}

#[test]
fn install_script_copies_binary_and_runs_smoke_test() {
    let temp_dir = tempfile::tempdir().unwrap();
    let install_dir = temp_dir.path().join("bin");
    let script = repo_root().join("scripts/install-seogeo.sh");
    let output = Command::new("sh")
        .arg(script)
        .arg("--from-binary")
        .arg(bin())
        .arg("--dest-dir")
        .arg(&install_dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);

    let install_path = install_dir.join("seogeo-cli");
    assert!(install_path.exists());

    let help = Command::new(&install_path).arg("--help").output().unwrap();
    assert!(help.status.success());
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert!(stdout.contains("check"));
    assert!(stdout.contains("config"));
}
