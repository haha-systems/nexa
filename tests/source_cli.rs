use serde_json::{Value, json};
use std::{path::Path, process::Command};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/hello")
        .join(name)
}

#[test]
fn source_check_and_pure_eval_use_real_implementations() {
    let checked = Command::new(env!("CARGO_BIN_EXE_nexa"))
        .arg("check")
        .arg(fixture("memories/trim/src/main.nexa"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let checked: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(checked["status"], "Checked");
    assert_eq!(checked["exports"][0]["export_name"], "trim");
    let evaluated = Command::new(env!("CARGO_BIN_EXE_nexa"))
        .arg("eval")
        .arg(fixture("memories/trim/src/main.nexa"))
        .args(["--entry", "trim", "--input-file"])
        .arg(fixture("input.json"))
        .output()
        .unwrap();
    assert!(
        evaluated.status.success(),
        "{}",
        String::from_utf8_lossy(&evaluated.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&evaluated.stdout).unwrap(),
        json!({"type":"String","value":"Ada"})
    );
}

#[test]
fn pure_eval_does_not_dispatch_an_action_export() {
    let input = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(input.path(), br#"{"message":"must not be printed"}"#).unwrap();
    let evaluated = Command::new(env!("CARGO_BIN_EXE_nexa"))
        .arg("eval")
        .arg(fixture("memories/write/src/main.nexa"))
        .args(["--entry", "write", "--input-file"])
        .arg(input.path())
        .output()
        .unwrap();
    assert_eq!(evaluated.status.code(), Some(2));
    assert!(evaluated.stdout.is_empty());
    assert!(String::from_utf8_lossy(&evaluated.stderr).contains("E_CALL_KIND"));
}
