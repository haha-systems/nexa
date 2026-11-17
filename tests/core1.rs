use nexa::{canonical, digest::Digest, memory::Store};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn write(root: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn core1(store: &Path, operation: &str, target: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexa"));
    command
        .args(["core1", operation])
        .arg(target)
        .arg("--store")
        .arg(store);
    command
}

fn seal_command(store: &Path, source: &Path, imports: &[(&str, &Digest)]) -> Command {
    let mut command = core1(store, "seal", source);
    for (alias, digest) in imports {
        command.arg("--import").arg(format!("{alias}={digest}"));
    }
    command
}

fn stdout(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn assert_error(output: Output, code: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty(), "{stderr}");
    let diagnostic: Value = serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|error| panic!("expected JSON diagnostic: {error}: {stderr}"));
    assert_eq!(diagnostic["code"], code, "{stderr}");
}

fn seal(store: &Path, source: &Path, imports: &[(&str, &Digest)]) -> Digest {
    let output = stdout(seal_command(store, source, imports).output().unwrap());
    let printed = std::str::from_utf8(&output).unwrap();
    printed.strip_suffix('\n').unwrap().parse().unwrap()
}

fn inspect(store: &Path, digest: &Digest) -> Value {
    json_stdout(core1(store, "inspect", digest.as_str()).output().unwrap())
}

fn run(store: &Path, digest: &Digest, input: &Path, fuel: Option<u64>) -> Output {
    let mut command = core1(store, "run", digest.as_str());
    command.arg("--input-file").arg(input);
    if let Some(fuel) = fuel {
        command.arg("--fuel").arg(fuel.to_string());
    }
    command.output().unwrap()
}

fn json_stdout(output: Output) -> Value {
    serde_json::from_slice(&stdout(output)).unwrap()
}

fn offset_source(offset: i64) -> String {
    format!(
        "nexa 0.3
module acceptance.offset
export fn add(value is Int) -> Int {{
    return value + {offset}
}}
"
    )
}

const IMPORTS_OFFSET: &str = "nexa 0.3
module acceptance.caller
import offset
export fn apply(value is Int) -> Int {
    return offset.add(value)
}
";

#[test]
fn sealed_imports_keep_their_exact_digest_across_reseals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = root.join("store");
    let dependency = write(root, "offset.nexa", offset_source(1));
    let dependency_v1 = seal(&store, &dependency, &[]);
    assert_eq!(seal(&store, &dependency, &[]), dependency_v1);

    let caller = write(root, "caller.nexa", IMPORTS_OFFSET);
    let caller_v1 = seal(&store, &caller, &[("offset", &dependency_v1)]);
    let record = inspect(&store, &caller_v1);
    assert_eq!(record["schema"], "nexa.core1-memory/0.1");
    assert_eq!(record["profile"], "core1");
    assert_eq!(record["module"], "acceptance.caller");
    assert_eq!(record["implementation"], env!("NEXA_IMPLEMENTATION_DIGEST"));
    assert_eq!(
        record["source"],
        Digest::of_bytes(IMPORTS_OFFSET.as_bytes()).as_str()
    );
    assert_eq!(record["imports"], json!({"offset": dependency_v1}));
    assert_eq!(record["signature"]["name"], "apply");
    assert_eq!(record["signature"]["params"][0]["name"], "value");
    assert_eq!(record["signature"]["params"][0]["ty"]["kind"], "Int");
    assert_eq!(record["signature"]["result"]["kind"], "Int");
    assert_error(
        Command::new(env!("CARGO_BIN_EXE_nexa"))
            .args(["memory", "inspect", caller_v1.as_str(), "--json", "--store"])
            .arg(&store)
            .output()
            .unwrap(),
        "E_CONTRACT",
    );

    assert_error(
        seal_command(&store, &caller, &[]).output().unwrap(),
        "E_IMPORT",
    );
    assert_error(
        seal_command(
            &store,
            &caller,
            &[("offset", &dependency_v1), ("extra", &dependency_v1)],
        )
        .output()
        .unwrap(),
        "E_IMPORT",
    );
    assert_error(
        seal_command(
            &store,
            &caller,
            &[("offset", &dependency_v1), ("offset", &dependency_v1)],
        )
        .output()
        .unwrap(),
        "E_IMPORT",
    );

    fs::write(&dependency, offset_source(7)).unwrap();
    let dependency_v2 = seal(&store, &dependency, &[]);
    let caller_v2 = seal(&store, &caller, &[("offset", &dependency_v2)]);
    assert_ne!(dependency_v1, dependency_v2);
    assert_ne!(caller_v1, caller_v2);

    fs::remove_file(dependency).unwrap();
    fs::remove_file(caller).unwrap();
    let input = write(root, "input.json", br#"{"value":"40"}"#);
    assert_eq!(inspect(&store, &caller_v1), record);
    assert_eq!(
        json_stdout(run(&store, &caller_v1, &input, None)),
        json!("41")
    );
    assert_eq!(
        json_stdout(run(&store, &caller_v2, &input, None)),
        json!("47")
    );
}

#[test]
fn same_shaped_types_from_distinct_memories_are_incompatible() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = root.join("store");
    let packet = "nexa 0.3
module acceptance.packets
export type Packet is { payload is Bytes }
export fn unpack(packet is Packet) -> Bytes {
    return packet.payload
}
";
    let v1_source = write(root, "packet-v1.nexa", packet);
    let v2_source = write(
        root,
        "packet-v2.nexa",
        packet.replace(
            "return packet.payload",
            "let payload = packet.payload\n    return payload",
        ),
    );
    let v1 = seal(&store, &v1_source, &[]);
    let v2 = seal(&store, &v2_source, &[]);
    assert_ne!(v1, v2);

    let caller = "nexa 0.3
module acceptance.nominal_caller
import v1
import v2
export fn apply(value is Bytes) -> Bytes {
    let first is v1.Packet = v1.Packet { payload: value }
    let second is v2.Packet = v2.Packet { payload: v1.unpack(first) }
    return v2.unpack(second)
}
";
    let caller_source = write(root, "caller.nexa", caller);
    let imports = [("v1", &v1), ("v2", &v2)];
    let checked = seal(&store, &caller_source, &imports);
    let input = write(root, "input.json", br#"{"value":"00ff80"}"#);
    assert_eq!(
        json_stdout(run(&store, &checked, &input, None)),
        json!("00ff80")
    );

    fs::write(
        &caller_source,
        caller.replace("v2.unpack(second)", "v2.unpack(first)"),
    )
    .unwrap();
    assert_error(
        seal_command(&store, &caller_source, &imports)
            .output()
            .unwrap(),
        "E_CORE1_TYPE",
    );
}

#[test]
fn typed_control_flow_preserves_large_integers_and_binary_values_with_fuel() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = root.join("store");
    let source = write(
        root,
        "counter.nexa",
        r#"nexa 0.3
module acceptance.counter
export type Job is { count is Int, start is Int, payload is Bytes }
export type Input is count_job(job is Job) | skip
export type Report is { counts is List[Int], payload is Bytes }

export fn execute(work is Input) -> Result[Report, String] {
    match work {
        Input.count_job(job: job) => {
            var index = 0
            var total = job.start
            while index < job.count {
                index = index + 1
                total = total + 1
            }
            return Result.ok(value: Report { counts: [job.start, total], payload: job.payload })
        }
        Input.skip => { return Result.err(error: "skipped") }
    }
}
"#,
    );
    let digest = seal(&store, &source, &[]);
    let input = root.join("input.json");
    let job = |count: &str, start: Value| {
        json!({"work": {"variant": "count_job", "fields": {"job": {
            "count": count, "start": start, "payload": "00ff0d0a80"
        }}}})
    };
    let write_input =
        |value: Value| fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();

    write_input(job("2", json!("9007199254740993")));
    assert_eq!(
        json_stdout(run(&store, &digest, &input, None)),
        json!({"variant":"ok", "fields":{"value":{
            "counts":["9007199254740993", "9007199254740995"], "payload":"00ff0d0a80"
        }}})
    );

    write_input(json!({"work":{"variant":"skip", "fields":{}}}));
    assert_eq!(
        json_stdout(run(&store, &digest, &input, Some(64))),
        json!({"variant":"err", "fields":{"error":"skipped"}})
    );
    write_input(job("128", json!("0")));
    assert_error(run(&store, &digest, &input, Some(64)), "E_CORE1_FUEL");
    write_input(job("1", json!("9223372036854775807")));
    assert_error(run(&store, &digest, &input, None), "E_CORE1_OVERFLOW");
    write_input(job("2", json!(9007199254740993_i64)));
    assert_error(run(&store, &digest, &input, Some(1)), "E_CORE1_VALUE");
}

#[test]
fn sealing_rejects_invalid_and_impure_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store_path = root.join("store");
    let invalid_sources = [
        r#"nexa 0.3
module acceptance.wrong_return
export fn apply(value is Int) -> Int {
    if false { return "wrong type" }
    return value
}
"#,
        r#"nexa 0.3
module acceptance.missing_variant
export fn apply(item is Maybe[Int]) -> Int {
    match item {
        Maybe.some(value: n) => { return n }
    }
    return 0
}
"#,
        r#"nexa 0.3
module acceptance.host_action
export fn apply(message is String) -> Unit {
    return std.console.write_line(message)
}
"#,
    ];
    for source in invalid_sources {
        let path = write(root, "invalid.nexa", source);
        assert_error(
            seal_command(&store_path, &path, &[]).output().unwrap(),
            "E_CORE1_TYPE",
        );
        let store = Store::open(&store_path).unwrap();
        assert!(
            !store
                .object_path(&Digest::of_bytes(source.as_bytes()))
                .exists()
        );
    }
}

#[test]
fn inspection_and_execution_verify_signatures_and_imported_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store_path = root.join("store");
    let dependency_source = offset_source(1);
    let dependency = write(root, "offset.nexa", &dependency_source);
    let dependency_digest = seal(&store_path, &dependency, &[]);
    let caller = write(root, "caller.nexa", IMPORTS_OFFSET);
    let caller_digest = seal(&store_path, &caller, &[("offset", &dependency_digest)]);
    let input = write(root, "input.json", br#"{"value":"40"}"#);
    assert_eq!(
        json_stdout(run(&store_path, &caller_digest, &input, None)),
        json!("41")
    );

    let store = Store::open(&store_path).unwrap();
    let mut false_record = inspect(&store_path, &caller_digest);
    false_record["signature"]["result"]["kind"] = json!("Bytes");
    let false_digest = store
        .put_raw(&canonical::encode(&false_record).unwrap())
        .unwrap();
    assert_error(
        core1(&store_path, "inspect", false_digest.as_str())
            .output()
            .unwrap(),
        "E_CONTRACT",
    );
    assert_error(run(&store_path, &false_digest, &input, None), "E_CONTRACT");

    let object = store.object_path(&Digest::of_bytes(dependency_source.as_bytes()));
    fs::write(object, offset_source(100)).unwrap();
    assert_error(
        core1(&store_path, "inspect", caller_digest.as_str())
            .output()
            .unwrap(),
        "E_INTEGRITY",
    );
    assert_error(
        run(&store_path, &caller_digest, &input, None),
        "E_INTEGRITY",
    );
}
