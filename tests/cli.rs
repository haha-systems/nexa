use serde_json::json;
use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nexa"))
}

#[test]
fn independent_processes_produce_the_same_document_identity() {
    let input = format!(
        "{}/examples/hello/hello.compose.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let a = binary().args(["hash", &input]).output().unwrap();
    let b = binary().args(["hash", &input]).output().unwrap();
    assert!(a.status.success());
    assert!(b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert!(a.stderr.is_empty());
    let digest = String::from_utf8(a.stdout).unwrap();
    assert!(digest.trim().parse::<nexa::digest::Digest>().is_ok());
}

#[test]
fn malformed_input_reports_a_structured_error_and_no_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, br#"{"x":1,"x":2}"#).unwrap();
    let output = binary().arg("canonicalize").arg(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "E_JSON");
    assert!(error["message"].as_str().unwrap().contains("duplicate key"));
}

#[test]
fn kernel_is_inspectable_without_invoking_its_console_operation() {
    let output = binary().arg("kernel").output().unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["descriptor"]["profile"], "slice0");
    assert_eq!(
        value["descriptor"]["operations"].as_array().unwrap().len(),
        2
    );
    let expected = nexa::canonical::digest(&value["descriptor"]).unwrap();
    assert_eq!(value["digest"], json!(expected));
    assert!(output.stderr.is_empty());
}

#[test]
fn canonical_output_is_exact_and_raw_hash_keeps_source_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.json");
    std::fs::write(&path, b"{ \"z\":2, \"a\":1 }\n").unwrap();
    let output = binary().arg("canonicalize").arg(&path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, br#"{"a":1,"z":2}"#);
    let output = binary()
        .arg("hash")
        .arg(&path)
        .args(["--kind", "raw"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected = nexa::digest::Digest::of_bytes(&std::fs::read(path).unwrap());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        expected.as_str()
    );
}

#[test]
fn run_requires_explicit_execution_inputs() {
    let output = binary().arg("run").arg("anything").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}
