use nexa::{
    blueprint, canonical,
    contracts::{self, BlueprintDraft},
    memory::{self, Store},
    value::Identifier,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn program(directory: &Path, count: usize) -> (Store, std::path::PathBuf) {
    let store = Store::open(directory.join("store")).unwrap();
    memory::seal_source(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello/memories/greet"),
        &store,
    )
    .unwrap();
    let mut nodes = Vec::new();
    let mut connections = Vec::new();
    for index in 0..count {
        let id = format!("n{index:03}");
        nodes.push(json!({"id":id,"selector":"local.hello.greet@1.0.0","export":"greet"}));
        connections.push(json!({"from":{"kind":"input","port":"name"},"to":{"kind":"node","node":id,"port":"name"}}));
    }
    connections.push(json!({"from":{"kind":"node","node":format!("n{:03}",count-1),"port":"result"},"to":{"kind":"output","port":"result"}}));
    let draft: BlueprintDraft = contracts::decode(json!({
        "schema":"nexa.blueprint-draft/0.3","language":"0.3","profile":"slice0",
        "entry":"example.inspect.many","inputs":[{"name":"name","type":"String"}],
        "output":{"name":"result","type":"String"},"nodes":nodes,"connections":connections,
        "order_edges":[],"settings":[],"requirements":[]
    }))
    .unwrap();
    let lock = store
        .resolve_requests(&[(
            "local.hello.greet@1.0.0".into(),
            Identifier::new("greet").unwrap(),
        )])
        .unwrap();
    let spec = blueprint::compose(&draft, &lock, &store).unwrap();
    let path = directory.join("program.json");
    fs::write(
        &path,
        canonical::encode(&serde_json::to_value(&spec).unwrap()).unwrap(),
    )
    .unwrap();
    (store, path)
}

fn inspect(path: &Path, store: &Store, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nexa"))
        .arg("inspect")
        .arg(path)
        .args(["--store"])
        .arg(store.root())
        .arg("--json")
        .args(extra)
        .output()
        .unwrap()
}

fn body(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn inspection_pages_are_bounded_and_cursors_are_bound_to_revision_and_query() {
    let directory = tempfile::tempdir().unwrap();
    let (store, path) = program(directory.path(), 60);
    let first = body(&inspect(&path, &store, &[]));
    assert_eq!(first["node_count"], 60);
    assert_eq!(first["nodes"].as_array().unwrap().len(), 50);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = body(&inspect(&path, &store, &["--cursor", cursor]));
    assert_eq!(second["nodes"].as_array().unwrap().len(), 10);
    assert!(second["next_cursor"].is_null());
    assert_ne!(first["nodes"][0]["id"], second["nodes"][0]["id"]);
    let wrong_query = inspect(&path, &store, &["--node", "n000", "--cursor", cursor]);
    assert_eq!(wrong_query.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_query.stderr).contains("E_CURSOR"));
    let mut changed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    changed["entry"] = json!("example.inspect.changed");
    let changed_path = directory.path().join("changed.json");
    fs::write(&changed_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let stale = inspect(&changed_path, &store, &["--cursor", cursor]);
    assert_eq!(stale.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("E_CURSOR"));
}

#[test]
fn inspection_projection_retains_revision_and_reports_omitted_fields() {
    let directory = tempfile::tempdir().unwrap();
    let (store, path) = program(directory.path(), 2);
    let projected = body(&inspect(&path, &store, &["--fields", "nodes"]));
    assert!(projected["subject"].is_string());
    assert!(projected["nodes"].is_array());
    assert!(
        projected["omitted_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "connection_count")
    );
    assert!(projected.get("connection_count").is_none());
    let invalid = inspect(&path, &store, &["--fields", "made_up_field"]);
    assert_eq!(invalid.status.code(), Some(2));
    let unbounded = inspect(&path, &store, &["--limit", "1000"]);
    assert_eq!(unbounded.status.code(), Some(2));
}
