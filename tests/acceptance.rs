use nexa::{
    canonical,
    contracts::{EvidenceReport, FixtureSuite, Outcome},
    diagnostic::{Error, ExitCode},
    digest::Digest,
    evidence::{self, FixtureInputs},
    memory::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn suite_case_count() -> usize {
    let suite: FixtureSuite =
        serde_json::from_slice(&fs::read(fixture("fixtures.json")).unwrap()).unwrap();
    suite.cases.len()
}

fn fixture(path: &str) -> PathBuf {
    repo().join("examples/hello").join(path)
}

fn arguments(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn push_path(args: &mut Vec<String>, path: &Path) {
    args.push(path.to_string_lossy().into_owned());
}

fn invoke(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nexa"))
        .current_dir(repo())
        .args(args)
        .output()
        .expect("run Nexa CLI subprocess")
}

fn invoke_stdout(args: &[String], stdout: Stdio) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nexa"))
        .current_dir(repo())
        .args(args)
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Nexa CLI subprocess")
        .wait_with_output()
        .expect("wait for Nexa CLI subprocess")
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "expected exit {code}; stdout={:?}; stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success(output: &Output) {
    assert_exit(output, 0);
}

fn assert_construction_error(output: &Output, needles: &[&str]) {
    assert_exit(output, ExitCode::ConstructionError as i32);
    assert!(
        output.stdout.is_empty(),
        "construction errors must not write application stdout"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized_stderr = stderr.to_lowercase();
    for needle in needles {
        assert!(
            normalized_stderr.contains(&needle.to_lowercase()),
            "expected {needle:?} in {stderr:?}"
        );
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn seal_source(store: &Path, source: &Path) -> String {
    let mut args = arguments(&["memory", "seal"]);
    push_path(&mut args, source);
    args.extend(arguments(&["--store"]));
    push_path(&mut args, store);
    let output = invoke(&args);
    assert_success(&output);
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn seal_standard(store: &Path) {
    for memory in ["trim", "greet", "write"] {
        seal_source(store, &fixture(&format!("memories/{memory}")));
    }
}

fn resolve_source(store: &Path, source: &Path, lock: &Path) {
    let mut args = arguments(&["resolve"]);
    push_path(&mut args, source);
    args.extend(arguments(&["--store"]));
    push_path(&mut args, store);
    args.extend(arguments(&["--write-lock"]));
    push_path(&mut args, lock);
    assert_success(&invoke(&args));
}

fn compose_source(store: &Path, source: &Path, lock: &Path, out: &Path) -> Output {
    let mut args = arguments(&["compose"]);
    push_path(&mut args, source);
    args.extend(arguments(&["--locked", "--lock"]));
    push_path(&mut args, lock);
    args.extend(arguments(&["--store"]));
    push_path(&mut args, store);
    args.extend(arguments(&["--out"]));
    push_path(&mut args, out);
    invoke(&args)
}

fn compose_request(store: &Path, request: &Path, lock: &Path, out: &Path) -> Output {
    let mut args = arguments(&["compose", "--request"]);
    push_path(&mut args, request);
    args.extend(arguments(&["--locked", "--lock"]));
    push_path(&mut args, lock);
    args.extend(arguments(&["--store"]));
    push_path(&mut args, store);
    args.extend(arguments(&["--out"]));
    push_path(&mut args, out);
    invoke(&args)
}

fn run_program(
    store: &Path,
    program: &Path,
    input: &Path,
    manifest: &Path,
    allow_stdout: bool,
    receipt: &Path,
) -> Output {
    let mut args = arguments(&["run"]);
    push_path(&mut args, program);
    args.extend(arguments(&["--input-file"]));
    push_path(&mut args, input);
    args.extend(arguments(&["--manifest"]));
    push_path(&mut args, manifest);
    if allow_stdout {
        args.extend(arguments(&["--allow", "stdout"]));
    }
    args.extend(arguments(&["--receipt"]));
    push_path(&mut args, receipt);
    args.extend(arguments(&["--store"]));
    push_path(&mut args, store);
    invoke(&args)
}

fn program_digest(path: &Path) -> Digest {
    let value = canonical::parse(&fs::read(path).unwrap()).unwrap();
    canonical::digest(&value).unwrap()
}

fn object_path(store: &Path, digest: &str) -> PathBuf {
    let digest: Digest = digest.parse().expect("well-formed content digest");
    Store::open(store).unwrap().object_path(&digest)
}

fn source_request_and_execution(store: &Path, tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
    seal_standard(store);
    let lock = tmp.path().join("hello.lock");
    resolve_source(store, &fixture("hello.nexa"), &lock);
    let source_program = tmp.path().join("hello.source.program.json");
    let output = compose_source(store, &fixture("hello.nexa"), &lock, &source_program);
    assert_success(&output);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Hello, Ada!"));
    let request_program = tmp.path().join("hello.request.program.json");
    let output = compose_request(
        store,
        &fixture("hello.compose.json"),
        &lock,
        &request_program,
    );
    assert_success(&output);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Hello, Ada!"));
    (source_program, request_program, lock)
}

#[test]
fn cli_acceptance_identity_runtime_policy_and_pre_effect_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    fs::create_dir_all(&store).unwrap();
    let (source_program, request_program, lock) = source_request_and_execution(&store, &tmp);

    let source_bytes = fs::read(&source_program).unwrap();
    let request_bytes = fs::read(&request_program).unwrap();
    assert_eq!(
        source_bytes, request_bytes,
        "equivalent constructions normalize identically"
    );
    assert_eq!(
        program_digest(&source_program),
        program_digest(&request_program)
    );

    let before = program_digest(&source_program);
    let mut check_args = arguments(&["check"]);
    push_path(&mut check_args, &source_program);
    check_args.extend(arguments(&["--profile", "slice0", "--store"]));
    push_path(&mut check_args, &store);
    check_args.push("--json".into());
    let check = invoke(&check_args);
    assert_success(&check);
    assert!(!String::from_utf8_lossy(&check.stdout).contains("Hello, Ada!"));

    let mut inspect_args = arguments(&["inspect"]);
    push_path(&mut inspect_args, &source_program);
    inspect_args.extend(arguments(&["--store"]));
    push_path(&mut inspect_args, &store);
    inspect_args.push("--json".into());
    let inspect = invoke(&inspect_args);
    assert_success(&inspect);
    let summary: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(summary["subject"], json!(before.as_str()));
    assert_eq!(summary["node_count"], 3);
    assert_eq!(summary["connection_count"], 4);
    assert_eq!(program_digest(&source_program), before);

    let input = tmp.path().join("input.json");
    write_json(&input, &json!({"name": "  Ada  "}));
    let success_receipt = tmp.path().join("success.receipt.json");
    let success = run_program(
        &store,
        &source_program,
        &input,
        &fixture("hello.app.json"),
        true,
        &success_receipt,
    );
    assert_exit(&success, 0);
    assert_eq!(success.stdout, b"Hello, Ada!\n");
    assert!(success.stderr.is_empty());
    let receipt = read_json(&success_receipt);
    assert_eq!(receipt["schema"], "nexa.receipt/0.3");
    assert_eq!(receipt["program"], before.as_str());
    assert_eq!(receipt["outcome"]["kind"], "Succeeded");
    assert_eq!(
        receipt["outcome"]["value"],
        json!({"type": "Unit", "value": null})
    );
    assert_eq!(receipt["effects"].as_array().unwrap().len(), 1);
    assert_eq!(receipt["effects"][0]["dispatched"], true);
    assert_eq!(receipt["effects"][0]["operation"], "std.console.write_line");

    for (name, expected) in [
        ("\tAda\r\n", "Hello, Ada!\n"),
        ("", "Hello, !\n"),
        (" Zoë ", "Hello, Zoë!\n"),
        ("\u{00a0}Ada\u{00a0}", "Hello, \u{00a0}Ada\u{00a0}!\n"),
    ] {
        write_json(&input, &json!({"name": name}));
        let receipt_path = tmp.path().join("positive.receipt.json");
        let run = run_program(
            &store,
            &source_program,
            &input,
            &fixture("hello.app.json"),
            true,
            &receipt_path,
        );
        assert_exit(&run, 0);
        assert_eq!(run.stdout, expected.as_bytes());
        assert_eq!(read_json(&receipt_path)["outcome"]["kind"], "Succeeded");
    }

    write_json(&input, &json!({"name": "Ada"}));
    let denied_receipt = tmp.path().join("denied.receipt.json");
    let denied = run_program(
        &store,
        &source_program,
        &input,
        &fixture("hello.app.json"),
        false,
        &denied_receipt,
    );
    assert_exit(&denied, 3);
    assert!(denied.stdout.is_empty());
    assert_eq!(read_json(&denied_receipt)["outcome"]["kind"], "Denied");

    let mut deny_manifest = read_json(&fixture("hello.app.json"));
    deny_manifest["policies"] = json!([
        {"operation":"std.console.write_line","resource":"stdout","decision":"allow"},
        {"operation":"std.console.write_line","resource":"stdout","decision":"deny"}
    ]);
    let deny_path = tmp.path().join("deny.app.json");
    write_json(&deny_path, &deny_manifest);
    let deny_receipt = tmp.path().join("deny.receipt.json");
    let deny = run_program(
        &store,
        &source_program,
        &input,
        &deny_path,
        true,
        &deny_receipt,
    );
    assert_exit(&deny, 3);
    assert!(deny.stdout.is_empty());
    assert_eq!(read_json(&deny_receipt)["outcome"]["kind"], "Denied");

    let mut abstain_manifest = read_json(&fixture("hello.app.json"));
    abstain_manifest["policies"] = json!([
        {"operation":"std.console.write_line","resource":"stdout","decision":"abstain"}
    ]);
    let abstain_path = tmp.path().join("abstain.app.json");
    write_json(&abstain_path, &abstain_manifest);
    let abstain_receipt = tmp.path().join("abstain.receipt.json");
    let abstain = run_program(
        &store,
        &source_program,
        &input,
        &abstain_path,
        true,
        &abstain_receipt,
    );
    assert_exit(&abstain, 3);
    assert!(abstain.stdout.is_empty());
    assert_eq!(read_json(&abstain_receipt)["outcome"]["kind"], "Denied");

    let mut ask_manifest = read_json(&fixture("hello.app.json"));
    ask_manifest["policies"] = json!([
        {"operation":"std.console.write_line","resource":"stdout","decision":"ask"}
    ]);
    let ask_path = tmp.path().join("ask.app.json");
    write_json(&ask_path, &ask_manifest);
    let ask_receipt = tmp.path().join("ask.receipt.json");
    let ask = run_program(
        &store,
        &source_program,
        &input,
        &ask_path,
        true,
        &ask_receipt,
    );
    assert_exit(&ask, 4);
    assert!(ask.stdout.is_empty());
    assert_eq!(
        read_json(&ask_receipt)["outcome"]["kind"],
        "AwaitingApproval"
    );

    let bad_input = tmp.path().join("bad-input.json");
    write_json(&bad_input, &json!({"name": 9}));
    let bad_receipt = tmp.path().join("bad-input.receipt.json");
    let bad = run_program(
        &store,
        &source_program,
        &bad_input,
        &fixture("hello.app.json"),
        true,
        &bad_receipt,
    );
    assert_construction_error(&bad, &["input", "name"]);

    let draft: Value =
        serde_json::from_slice(&fs::read(fixture("hello.compose.json")).unwrap()).unwrap();
    let wrong_wire = json!({
        "schema":"nexa.blueprint-draft/0.3",
        "language":"0.3",
        "profile":"slice0",
        "entry":"example.invalid.unit_to_string",
        "inputs":[{"name":"name","type":"String"}],
        "output":{"name":"result","type":"String"},
        "nodes":[{"id":"write","selector":"local.console.line@1.0.0","export":"write"}],
        "connections":[
            {"from":{"kind":"input","port":"name"},"to":{"kind":"node","node":"write","port":"message"}},
            {"from":{"kind":"node","node":"write","port":"result"},"to":{"kind":"output","port":"result"}}
        ],
        "order_edges":[],"settings":[],"requirements":[]
    });
    let wrong_path = tmp.path().join("wrong-type.request.json");
    write_json(&wrong_path, &wrong_wire);
    let wrong_program = tmp.path().join("wrong-type.program.json");
    let wrong = compose_request(&store, &wrong_path, &lock, &wrong_program);
    assert_construction_error(&wrong, &["write.result", "output.result"]);
    assert!(!wrong_program.exists());

    let mut cycle = draft;
    cycle["order_edges"] = json!([
        {"before":"clean","after":"greeting"},
        {"before":"greeting","after":"clean"}
    ]);
    let cycle_path = tmp.path().join("cycle.request.json");
    write_json(&cycle_path, &cycle);
    let cycle_program = tmp.path().join("cycle.program.json");
    let cycle_result = compose_request(&store, &cycle_path, &lock, &cycle_program);
    assert_construction_error(&cycle_result, &["cycle"]);
    assert!(!cycle_program.exists());

    let mut forged: Value = serde_json::from_slice(&fs::read(&source_program).unwrap()).unwrap();
    forged["derived"]["effects"] = json!([]);
    let forged_path = tmp.path().join("forged.program.json");
    write_json(&forged_path, &forged);
    let mut forged_check = arguments(&["check"]);
    push_path(&mut forged_check, &forged_path);
    forged_check.extend(arguments(&["--profile", "slice0", "--store"]));
    push_path(&mut forged_check, &store);
    forged_check.push("--json".into());
    assert_construction_error(&invoke(&forged_check), &["E_DERIVED"]);

    // Slice 0 has no command that manufactures Skipped/Cancelled outcomes,
    // but their public exit mapping is part of the same CLI contract.
    assert_eq!(
        Outcome::Skipped {
            reason: "fixture".into()
        }
        .exit_code() as i32,
        5
    );
    assert_eq!(
        Outcome::Cancelled {
            reason: "fixture".into()
        }
        .exit_code() as i32,
        5
    );
}

#[test]
fn cli_acceptance_locks_tamper_generic_sources_reuse_and_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    fs::create_dir_all(&store).unwrap();
    let (source_program, _, lock) = source_request_and_execution(&store, &tmp);
    let program_id = program_digest(&source_program);

    // Stored source bytes are verified against their address on every load.
    let trim_record_id = seal_source(&store, &fixture("memories/trim"));
    let record_path = object_path(&store, &trim_record_id);
    let record = read_json(&record_path);
    let blob_id = record["source_map"][0]["blob"].as_str().unwrap();
    let blob_path = object_path(&store, blob_id);
    let original_blob = fs::read(&blob_path).unwrap();
    let mut altered = original_blob.clone();
    altered[0] ^= 1;
    fs::write(&blob_path, altered).unwrap();
    let tampered_program = tmp.path().join("tampered.program.json");
    let tamper_result = compose_source(&store, &fixture("hello.nexa"), &lock, &tampered_program);
    assert_construction_error(&tamper_result, &["integrity"]);
    assert!(!tampered_program.exists());
    fs::write(&blob_path, original_blob).unwrap();

    // A later alias update changes fresh resolution while the old lock still
    // composes and runs the original source memory.
    let alternate = tmp.path().join("alternate-greet");
    fs::create_dir_all(alternate.join("src")).unwrap();
    fs::copy(
        fixture("memories/greet/memory.json"),
        alternate.join("memory.json"),
    )
    .unwrap();
    let original_source = fs::read_to_string(fixture("memories/greet/src/main.nexa")).unwrap();
    fs::write(
        alternate.join("src/main.nexa"),
        original_source.replace("Hello, ", "Welcome, "),
    )
    .unwrap();
    let new_memory_id = seal_source(&store, &alternate);
    assert_ne!(
        new_memory_id,
        seal_source(&store, &fixture("memories/greet"))
    );

    // Re-sealing the old memory above restores the alias. Seal the changed
    // implementation once more so the alias points at it, then compare locks.
    let new_memory_id = seal_source(&store, &alternate);
    let old_locked_program = tmp.path().join("old-locked.program.json");
    let old_locked = compose_source(&store, &fixture("hello.nexa"), &lock, &old_locked_program);
    assert_success(&old_locked);
    assert_eq!(program_digest(&old_locked_program), program_id);
    let input = tmp.path().join("input.json");
    write_json(&input, &json!({"name":"Ada"}));
    let old_receipt = tmp.path().join("old.receipt.json");
    let old_run = run_program(
        &store,
        &old_locked_program,
        &input,
        &fixture("hello.app.json"),
        true,
        &old_receipt,
    );
    assert_exit(&old_run, 0);
    assert_eq!(old_run.stdout, b"Hello, Ada!\n");

    let new_lock = tmp.path().join("new.lock");
    resolve_source(&store, &fixture("hello.nexa"), &new_lock);
    let new_program = tmp.path().join("new.program.json");
    assert_success(&compose_source(
        &store,
        &fixture("hello.nexa"),
        &new_lock,
        &new_program,
    ));
    assert_ne!(program_digest(&new_program), program_id);
    let new_receipt = tmp.path().join("new.receipt.json");
    let new_run = run_program(
        &store,
        &new_program,
        &input,
        &fixture("hello.app.json"),
        true,
        &new_receipt,
    );
    assert_exit(&new_run, 0);
    assert_eq!(new_run.stdout, b"Welcome, Ada!\n");
    assert!(!new_memory_id.is_empty());

    // A new generic source with nested pure calls and concatenation evaluates
    // through the ordinary source evaluator, without a fixture-specific branch.
    let bracket = tmp.path().join("bracket-memory");
    fs::create_dir_all(bracket.join("src")).unwrap();
    write_json(
        &bracket.join("memory.json"),
        &json!({
            "schema":"nexa.memory-source/0.3",
            "name":"local.text.bracket",
            "version":"1.0.0",
            "module":"local.text.bracket",
            "sources":["src/main.nexa"],
            "entry":"bracket",
            "kernel":"std.slice0@0.3.0",
            "dependencies":[]
        }),
    );
    fs::write(
        bracket.join("src/main.nexa"),
        "nexa 0.3\nmodule local.text.bracket\n\nexport fn bracket(name is String) -> String {\n  return \"[\" + std.text.trim_ascii(name) + \"]\"\n}\n",
    )
    .unwrap();
    seal_source(&store, &bracket);
    let request = tmp.path().join("bracket.request.json");
    write_json(
        &request,
        &json!({
            "schema":"nexa.blueprint-draft/0.3",
            "language":"0.3",
            "profile":"slice0",
            "entry":"example.bracket.start",
            "inputs":[{"name":"name","type":"String"}],
            "output":{"name":"result","type":"String"},
            "nodes":[{"id":"wrap","selector":"local.text.bracket@1.0.0","export":"bracket"}],
            "connections":[
                {"from":{"kind":"input","port":"name"},"to":{"kind":"node","node":"wrap","port":"name"}},
                {"from":{"kind":"node","node":"wrap","port":"result"},"to":{"kind":"output","port":"result"}}
            ],
            "order_edges":[],"settings":[],"requirements":[]
        }),
    );
    let bracket_lock = tmp.path().join("bracket.lock");
    let mut resolve_args = arguments(&["resolve", "--request"]);
    push_path(&mut resolve_args, &request);
    resolve_args.extend(arguments(&["--store"]));
    push_path(&mut resolve_args, &store);
    resolve_args.extend(arguments(&["--write-lock"]));
    push_path(&mut resolve_args, &bracket_lock);
    assert_success(&invoke(&resolve_args));
    let bracket_program = tmp.path().join("bracket.program.json");
    assert_success(&compose_request(
        &store,
        &request,
        &bracket_lock,
        &bracket_program,
    ));
    write_json(&input, &json!({"name":" Ada "}));
    let pure_manifest = tmp.path().join("pure.app.json");
    write_json(
        &pure_manifest,
        &json!({
            "schema":"nexa.application/0.3",
            "principal":"app:bracket",
            "requests":[],
            "policies":[]
        }),
    );
    let bracket_receipt = tmp.path().join("bracket.receipt.json");
    let bracket_run = run_program(
        &store,
        &bracket_program,
        &input,
        &pure_manifest,
        false,
        &bracket_receipt,
    );
    assert_exit(&bracket_run, 0);
    assert!(bracket_run.stdout.is_empty());
    assert_eq!(
        read_json(&bracket_receipt)["outcome"]["value"],
        json!({"type":"String","value":"[Ada]"})
    );

    // Save and invoke the composed program as a memory from a separate CLI
    // process. The inner console action still needs fresh host authority.
    let mut seal_program_args = arguments(&["memory", "seal", "--program"]);
    push_path(&mut seal_program_args, &source_program);
    seal_program_args.extend(arguments(&[
        "--name",
        "local.hello",
        "--version",
        "1.0.0",
        "--store",
    ]));
    push_path(&mut seal_program_args, &store);
    assert_success(&invoke(&seal_program_args));
    let reuse_lock = tmp.path().join("reuse.lock");
    resolve_source(&store, &fixture("reuse.nexa"), &reuse_lock);
    let reuse_program = tmp.path().join("reuse.program.json");
    assert_success(&compose_source(
        &store,
        &fixture("reuse.nexa"),
        &reuse_lock,
        &reuse_program,
    ));
    let reuse_receipt = tmp.path().join("reuse.receipt.json");
    let reuse_run = run_program(
        &store,
        &reuse_program,
        &input,
        &fixture("hello.app.json"),
        true,
        &reuse_receipt,
    );
    assert_exit(&reuse_run, 0);
    assert_eq!(reuse_run.stdout, b"Hello, Ada!\n");
    let nested_receipt = read_json(&reuse_receipt);
    assert_eq!(nested_receipt["effects"].as_array().unwrap().len(), 1);
    assert_eq!(nested_receipt["effects"][0]["node_path"], "saved/write");

    let mut nested_inspect_args = arguments(&["inspect"]);
    push_path(&mut nested_inspect_args, &reuse_program);
    nested_inspect_args.extend(arguments(&["--node", "saved", "--store"]));
    push_path(&mut nested_inspect_args, &store);
    nested_inspect_args.push("--json".into());
    let nested_inspection = invoke(&nested_inspect_args);
    assert_success(&nested_inspection);
    let nested_view: Value = serde_json::from_slice(&nested_inspection.stdout).unwrap();
    assert_eq!(nested_view["node"]["path"], "saved");
    assert_eq!(nested_view["node"]["implementation"], "program");
    assert_eq!(nested_view["node"]["child_count"], 3);
    assert_eq!(nested_view["children"]["node_count"], 3);
    let child_paths: Vec<_> = nested_view["children"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["path"].as_str().unwrap())
        .collect();
    assert!(child_paths.contains(&"saved/clean"));
    assert!(child_paths.contains(&"saved/greeting"));
    assert!(child_paths.contains(&"saved/write"));

    let nested_denied_path = tmp.path().join("reuse.denied.receipt.json");
    let nested_denied = run_program(
        &store,
        &reuse_program,
        &input,
        &fixture("hello.app.json"),
        false,
        &nested_denied_path,
    );
    assert_exit(&nested_denied, 3);
    assert!(nested_denied.stdout.is_empty());
    let nested_denied_receipt = read_json(&nested_denied_path);
    assert_eq!(nested_denied_receipt["outcome"]["kind"], "Denied");
    assert!(
        nested_denied_receipt["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| { decision["decision"] == "deny" && decision["resource"] == "stdout" })
    );

    // `test` stores truthful pass and failure reports separately from the
    // executable subject, and reports actual stdout for a mismatched case.
    let original_program_bytes = fs::read(&source_program).unwrap();
    let report_path = tmp.path().join("hello.evidence.json");
    let mut test_args = arguments(&["test"]);
    push_path(&mut test_args, &source_program);
    test_args.extend(arguments(&["--fixtures"]));
    push_path(&mut test_args, &fixture("fixtures.json"));
    test_args.extend(arguments(&["--manifest"]));
    push_path(&mut test_args, &fixture("hello.app.json"));
    test_args.extend(arguments(&["--store"]));
    push_path(&mut test_args, &store);
    test_args.extend(arguments(&["--report"]));
    push_path(&mut test_args, &report_path);
    let tested = invoke(&test_args);
    assert_success(&tested);
    assert_eq!(fs::read(&source_program).unwrap(), original_program_bytes);
    let summary: Value = serde_json::from_slice(&tested.stdout).unwrap();
    assert_eq!(summary["passed"], true);
    let report: EvidenceReport = serde_json::from_value(read_json(&report_path)).unwrap();
    assert_eq!(report.subject, program_id);
    assert_eq!(report.cases.len(), suite_case_count());
    assert!(report.cases.iter().all(|case| case.passed));
    assert_eq!(
        report.cases[0].actual.as_ref().unwrap().stdout,
        b"Hello, Ada!\n"
    );
    let report_id = evidence::report_digest(&report).unwrap();
    assert!(object_path(&store, report_id.as_str()).is_file());
    assert_eq!(summary["report"], report_id.as_str());
    let stored = Store::open(&store).unwrap();
    assert!(
        stored
            .evidence_for(&program_id)
            .unwrap()
            .contains(&report_id)
    );

    let mut wrong_suite: Value =
        serde_json::from_slice(&fs::read(fixture("fixtures.json")).unwrap()).unwrap();
    wrong_suite["cases"][0]["expect"]["stdout"] = "wrong output\n".into();
    let wrong_fixtures = tmp.path().join("wrong-fixtures.json");
    write_json(&wrong_fixtures, &wrong_suite);
    let failed_report_path = tmp.path().join("failed.evidence.json");
    let mut failed_args = arguments(&["test"]);
    push_path(&mut failed_args, &source_program);
    failed_args.extend(arguments(&["--fixtures"]));
    push_path(&mut failed_args, &wrong_fixtures);
    failed_args.extend(arguments(&["--manifest"]));
    push_path(&mut failed_args, &fixture("hello.app.json"));
    failed_args.extend(arguments(&["--store"]));
    push_path(&mut failed_args, &store);
    failed_args.extend(arguments(&["--report"]));
    push_path(&mut failed_args, &failed_report_path);
    let failed_test = invoke(&failed_args);
    assert_exit(&failed_test, 1);
    let failed_report: EvidenceReport =
        serde_json::from_value(read_json(&failed_report_path)).unwrap();
    assert!(!failed_report.cases[0].passed);
    let failed_case = &failed_report.cases[0];
    assert!(failed_case.detail.contains("stdout expected"));
    assert_eq!(
        failed_case.actual.as_ref().unwrap().stdout,
        b"Hello, Ada!\n"
    );
    let failed_report_id = evidence::report_digest(&failed_report).unwrap();
    assert_ne!(failed_report_id, report_id);
    let evidence = Store::open(&store)
        .unwrap()
        .evidence_for(&program_id)
        .unwrap();
    assert!(evidence.contains(&report_id));
    assert!(evidence.contains(&failed_report_id));
    assert_eq!(program_digest(&source_program), program_id);
    assert_eq!(fs::read(&source_program).unwrap(), original_program_bytes);
}

#[test]
fn fixture_runner_preserves_pre_execution_errors_and_reports_content_identity() {
    let fixture_bytes = fs::read(fixture("fixtures.json")).unwrap();
    let suite: FixtureSuite = serde_json::from_slice(&fixture_bytes).unwrap();
    let inputs = FixtureInputs {
        subject: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .parse()
            .unwrap(),
        kernel: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .parse()
            .unwrap(),
        fixtures: Digest::of_bytes(
            &canonical::encode(&canonical::parse(&fixture_bytes).unwrap()).unwrap(),
        ),
        manifest: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            .parse()
            .unwrap(),
        tool_version: "acceptance-test".into(),
        environment: "test".into(),
    };
    let report = evidence::run_fixtures(inputs, &suite, |_| {
        Err(Error::new("E_SETUP", "test setup failed before evaluation"))
    });
    assert_eq!(report.cases.len(), suite_case_count());
    assert!(!report.cases[0].passed);
    assert!(report.cases[0].actual.is_none());
    assert_eq!(report.cases[0].diagnostic.as_ref().unwrap().code, "E_SETUP");
    assert!(report.cases[0].detail.contains("before an outcome"));
    assert!(evidence::report_digest(&report).is_ok());
}

#[test]
fn runtime_failure_has_exit_one_and_receipt_when_stdout_sink_fails() {
    #[cfg(unix)]
    {
        let full = Path::new("/dev/full");
        if !full.exists() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        fs::create_dir_all(&store).unwrap();
        let (program, _, _) = source_request_and_execution(&store, &tmp);
        let input = tmp.path().join("input.json");
        write_json(&input, &json!({"name":"Ada"}));
        let receipt = tmp.path().join("failed.receipt.json");
        let mut args = arguments(&["run"]);
        push_path(&mut args, &program);
        args.extend(arguments(&["--input-file"]));
        push_path(&mut args, &input);
        args.extend(arguments(&["--manifest"]));
        push_path(&mut args, &fixture("hello.app.json"));
        args.extend(arguments(&["--allow", "stdout", "--receipt"]));
        push_path(&mut args, &receipt);
        args.extend(arguments(&["--store"]));
        push_path(&mut args, &store);
        let full_sink = fs::OpenOptions::new().write(true).open(full).unwrap();
        let output = invoke_stdout(&args, Stdio::from(full_sink));
        assert_exit(&output, 1);
        let receipt = read_json(&receipt);
        assert_eq!(receipt["outcome"]["kind"], "Failed");
        assert_eq!(receipt["outcome"]["fault"], "IO");
        assert_eq!(receipt["effects"].as_array().unwrap().len(), 1);
    }
}
