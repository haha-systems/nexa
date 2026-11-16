use serde_json::Value as JsonValue;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn cli(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nexa"))
        .args(args)
        .output()
        .expect("run nexa subprocess")
}

fn arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn require_success(output: Output, command: &str) -> Output {
    assert!(
        output.status.success(),
        "{command} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run(args: &[String], command: &str) -> Output {
    require_success(cli(args), command)
}

fn common() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/hello")
}

fn seal_source(root: &Path, memory: &str, store: &Path) {
    run(
        &[
            "memory".into(),
            "seal".into(),
            arg(&root.join("memories").join(memory)),
            "--store".into(),
            arg(store),
        ],
        "seal source memory",
    );
}

fn resolve(root: &Path, source: &str, lock: &Path, store: &Path) {
    run(
        &[
            "resolve".into(),
            arg(&root.join(source)),
            "--write-lock".into(),
            arg(lock),
            "--store".into(),
            arg(store),
        ],
        "resolve blueprint",
    );
}

fn compose(root: &Path, source: &str, lock: &Path, output: &Path, store: &Path) {
    run(
        &[
            "compose".into(),
            arg(&root.join(source)),
            "--locked".into(),
            "--lock".into(),
            arg(lock),
            "--out".into(),
            arg(output),
            "--store".into(),
            arg(store),
        ],
        "compose blueprint",
    );
}

fn seal_program(program: &Path, store: &Path) -> Output {
    run(
        &[
            "memory".into(),
            "seal".into(),
            "--program".into(),
            arg(program),
            "--name".into(),
            "local.hello".into(),
            "--version".into(),
            "1.0.0".into(),
            "--store".into(),
            arg(store),
        ],
        "seal composed program",
    )
}

#[test]
fn composed_memory_reuses_the_whole_graph_across_processes_and_keeps_leaf_authority() {
    let root = common();
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let hello_lock = temp.path().join("hello.lock.json");
    let hello_program = temp.path().join("hello.program.json");
    let reuse_lock = temp.path().join("reuse.lock.json");
    let reuse_program = temp.path().join("reuse.program.json");
    let input = root.join("input.json");
    let manifest = root.join("hello.app.json");

    for memory in ["trim", "greet", "write"] {
        seal_source(&root, memory, &store);
    }
    resolve(&root, "hello.nexa", &hello_lock, &store);
    compose(&root, "hello.nexa", &hello_lock, &hello_program, &store);
    let original_memory = String::from_utf8(seal_program(&hello_program, &store).stdout)
        .unwrap()
        .trim()
        .to_owned();

    let inspection = run(
        &[
            "memory".into(),
            "inspect".into(),
            original_memory.clone(),
            "--json".into(),
            "--store".into(),
            arg(&store),
        ],
        "inspect sealed program memory",
    );
    let inspection: JsonValue = serde_json::from_slice(&inspection.stdout).unwrap();
    let export = &inspection["memory"]["exports"][0];
    assert_eq!(export["export_name"], "hello");
    assert_eq!(export["kind"], "action");
    assert_eq!(export["parameters"][0]["name"], "name");
    assert_eq!(export["parameters"][0]["type"], "String");
    assert_eq!(export["result_type"], "Unit");
    assert_eq!(export["effects"], serde_json::json!(["io.stdout"]));
    assert_eq!(
        export["required_bindings"],
        serde_json::json!(["std.console.write_line"])
    );
    assert_eq!(
        export["capability_requirements"],
        serde_json::json!([{"capability":"console.write","resource":"stdout"}])
    );

    let mut forged: JsonValue =
        serde_json::from_slice(&std::fs::read(&hello_program).unwrap()).unwrap();
    forged["derived"]["effects"] = serde_json::json!([]);
    let forged_path = temp.path().join("forged.program.json");
    std::fs::write(&forged_path, serde_json::to_vec_pretty(&forged).unwrap()).unwrap();
    let rejected = cli(&[
        "memory".into(),
        "seal".into(),
        "--program".into(),
        arg(&forged_path),
        "--name".into(),
        "local.forged".into(),
        "--version".into(),
        "1.0.0".into(),
        "--store".into(),
        arg(&store),
    ]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    let error: JsonValue = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(error["code"], "E_DERIVED");

    resolve(&root, "reuse.nexa", &reuse_lock, &store);
    compose(&root, "reuse.nexa", &reuse_lock, &reuse_program, &store);

    // Every CLI call above and below is a separate process. The second graph
    // must be executable using only objects already written to the store.
    let receipt_path = temp.path().join("reuse.receipt.json");
    let success = run(
        &[
            "run".into(),
            arg(&reuse_program),
            "--input-file".into(),
            arg(&input),
            "--manifest".into(),
            arg(&manifest),
            "--allow".into(),
            "stdout".into(),
            "--receipt".into(),
            arg(&receipt_path),
            "--store".into(),
            arg(&store),
        ],
        "run reused composition",
    );
    assert_eq!(success.stdout, b"Hello, Ada!\n");

    let receipt: JsonValue =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    let paths: BTreeSet<_> = receipt["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["path"].as_str())
        .collect();
    for path in ["saved/clean", "saved/greeting", "saved/write"] {
        assert!(
            paths.contains(path),
            "missing nested receipt path {path}: {paths:?}"
        );
    }
    assert_eq!(receipt["effects"][0]["node_path"], "saved/write");
    assert_eq!(receipt["effects"][0]["dispatched"], true);

    let denied_receipt_path = temp.path().join("denied.receipt.json");
    let denied = cli(&[
        "run".into(),
        arg(&reuse_program),
        "--input-file".into(),
        arg(&input),
        "--manifest".into(),
        arg(&manifest),
        "--receipt".into(),
        arg(&denied_receipt_path),
        "--store".into(),
        arg(&store),
    ]);
    assert_eq!(denied.status.code(), Some(3));
    assert!(denied.stdout.is_empty(), "denied leaf wrote to stdout");

    let denied_receipt: JsonValue =
        serde_json::from_slice(&std::fs::read(&denied_receipt_path).unwrap()).unwrap();
    let denied_write = denied_receipt["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["path"] == "saved/write")
        .expect("inner write node is present in denied receipt");
    assert_eq!(denied_write["outcome"]["kind"], "Denied");
    assert_eq!(denied_receipt["effects"][0]["node_path"], "saved/write");
    assert_eq!(denied_receipt["effects"][0]["dispatched"], false);

    // A second store binds the same selector to a changed, type-compatible
    // source implementation. With the same enclosing name/version, sealing
    // the resulting graph must still produce a new immutable memory identity.
    let changed_store = temp.path().join("changed-store");
    for memory in ["trim", "write"] {
        seal_source(&root, memory, &changed_store);
    }
    let changed_greet = temp.path().join("changed-greet");
    std::fs::create_dir_all(changed_greet.join("src")).unwrap();
    std::fs::copy(
        root.join("memories/greet/memory.json"),
        changed_greet.join("memory.json"),
    )
    .unwrap();
    let greet_source = std::fs::read_to_string(root.join("memories/greet/src/main.nexa"))
        .unwrap()
        .replace("Hello, ", "Hi, ");
    std::fs::write(changed_greet.join("src/main.nexa"), greet_source).unwrap();
    run(
        &[
            "memory".into(),
            "seal".into(),
            arg(&changed_greet),
            "--store".into(),
            arg(&changed_store),
        ],
        "seal changed greeting memory",
    );
    let changed_lock = temp.path().join("changed.lock.json");
    let changed_program = temp.path().join("changed.program.json");
    resolve(&root, "hello.nexa", &changed_lock, &changed_store);
    compose(
        &root,
        "hello.nexa",
        &changed_lock,
        &changed_program,
        &changed_store,
    );
    let changed_memory = String::from_utf8(seal_program(&changed_program, &changed_store).stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert_ne!(original_memory, changed_memory);

    let changed_receipt = temp.path().join("changed.receipt.json");
    let changed_run = run(
        &[
            "run".into(),
            arg(&changed_program),
            "--input-file".into(),
            arg(&input),
            "--manifest".into(),
            arg(&manifest),
            "--allow".into(),
            "stdout".into(),
            "--receipt".into(),
            arg(&changed_receipt),
            "--store".into(),
            arg(&changed_store),
        ],
        "run changed dependency graph",
    );
    assert_eq!(changed_run.stdout, b"Hi, Ada!\n");
}
