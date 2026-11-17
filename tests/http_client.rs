#![cfg(feature = "http-client")]

use nexa::{canonical, digest::Digest, memory::Store};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
struct LocalServer {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl LocalServer {
    fn start(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            loop {
                match stop_rx.try_recv() {
                    Ok(()) | Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {}
                }

                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_request(&mut stream);
                        let first_line = request.lines().next().unwrap_or_default().to_owned();
                        thread_requests.lock().unwrap().push(first_line);
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            origin,
            requests,
            stop: Some(stop_tx),
            worker: Some(worker),
        }
    }

    fn request_lines(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    fn wait_for_requests(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.requests.lock().unwrap().len() < count && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            self.requests.lock().unwrap().len(),
            count,
            "timed out waiting for {count} HTTP requests"
        );
    }

    fn assert_request_count(&self, count: usize) {
        assert_eq!(self.requests.lock().unwrap().len(), count);
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn example(path: &str) -> PathBuf {
    repo().join("examples/http-client").join(path)
}

fn cli(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nexa"))
        .current_dir(repo())
        .args(args)
        .output()
        .expect("run feature-enabled Nexa CLI subprocess")
}

fn path_arg(args: &mut Vec<String>, path: &Path) {
    args.push(path.to_string_lossy().into_owned());
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn assert_exit(output: &Output, exit: i32) {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "expected exit {exit}; stdout={:?}; stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success(output: &Output) {
    assert_exit(output, 0);
}

fn seal_source(store: &Path, source: &Path) {
    let mut command = args(&["memory", "seal"]);
    path_arg(&mut command, source);
    command.extend(args(&["--store"]));
    path_arg(&mut command, store);
    assert_success(&cli(&command));
}

fn program_digest(path: &Path) -> Digest {
    canonical::digest(&canonical::parse(&fs::read(path).unwrap()).unwrap()).unwrap()
}

fn make_manifest(origin: &str, http_decision: &str, console_decision: &str) -> Value {
    json!({
        "schema":"nexa.application/0.3",
        "principal":"app:http-client",
        "requests":[
            {"capability":"http.get","resource":origin},
            {"capability":"console.write","resource":"stdout"}
        ],
        "policies":[
            {"operation":"std.http.get","resource":origin,"decision":http_decision},
            {"operation":"std.console.write_line","resource":"stdout","decision":console_decision}
        ]
    })
}

fn run_program(
    store: &Path,
    program: &Path,
    url: &str,
    manifest: &Path,
    grants: &[String],
    receipt: &Path,
    input_path: &Path,
) -> Output {
    write_json(input_path, &json!({"url":url}));
    let mut command = args(&["run"]);
    path_arg(&mut command, program);
    command.extend(args(&["--input-file"]));
    path_arg(&mut command, input_path);
    command.extend(args(&["--manifest"]));
    path_arg(&mut command, manifest);
    for grant in grants {
        command.extend(args(&["--allow"]));
        command.push(grant.clone());
    }
    command.extend(args(&["--receipt"]));
    path_arg(&mut command, receipt);
    command.extend(args(&["--store"]));
    path_arg(&mut command, store);
    cli(&command)
}

fn origin_manifest_file(dir: &Path, origin: &str, http: &str, console: &str) -> PathBuf {
    let identity = canonical::digest(&json!(origin)).unwrap();
    let path = dir.join(format!(
        "manifest-{http}-{console}-{}.json",
        &identity.as_str()[7..]
    ));
    write_json(&path, &make_manifest(origin, http, console));
    path
}

fn find_effect(receipt: &Value, operation: &str) -> Value {
    receipt["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["operation"] == operation)
        .unwrap_or_else(|| panic!("missing {operation} effect in receipt: {receipt}"))
        .clone()
}

#[test]
fn feature_http_client_is_origin_scoped_and_works_through_composition_and_reuse() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    fs::create_dir_all(&store).unwrap();
    let body = "héllo from HTTP 世界";
    let server = LocalServer::start(body.as_bytes().to_vec());
    let origin = server.origin.clone();

    // Seal real source implementations, including the existing console writer.
    for source in [
        example("memories/fetch"),
        example("memories/format"),
        repo().join("examples/hello/memories/write"),
    ] {
        seal_source(&store, &source);
    }

    let lock = tmp.path().join("client.lock");
    let mut resolve = args(&["resolve"]);
    path_arg(&mut resolve, &example("client.nexa"));
    resolve.extend(args(&["--store"]));
    path_arg(&mut resolve, &store);
    resolve.extend(args(&["--write-lock"]));
    path_arg(&mut resolve, &lock);
    assert_success(&cli(&resolve));

    let source_program = tmp.path().join("client.source.program.json");
    let mut compose_source = args(&["compose"]);
    path_arg(&mut compose_source, &example("client.nexa"));
    compose_source.extend(args(&["--locked", "--lock"]));
    path_arg(&mut compose_source, &lock);
    compose_source.extend(args(&["--store"]));
    path_arg(&mut compose_source, &store);
    compose_source.extend(args(&["--out"]));
    path_arg(&mut compose_source, &source_program);
    assert_success(&cli(&compose_source));

    let request_program = tmp.path().join("client.request.program.json");
    let mut compose_request = args(&["compose", "--request"]);
    path_arg(&mut compose_request, &example("client.compose.json"));
    compose_request.extend(args(&["--locked", "--lock"]));
    path_arg(&mut compose_request, &lock);
    compose_request.extend(args(&["--store"]));
    path_arg(&mut compose_request, &store);
    compose_request.extend(args(&["--out"]));
    path_arg(&mut compose_request, &request_program);
    assert_success(&cli(&compose_request));
    assert_eq!(
        fs::read(&source_program).unwrap(),
        fs::read(&request_program).unwrap(),
        "source and typed request must produce identical normalized ProgramSpecs"
    );
    let subject = program_digest(&source_program);
    assert_eq!(subject, program_digest(&request_program));

    // Inspection reports the locked HTTP capability as a dynamic URL-origin
    // contract and never contacts the server.
    let mut inspect = args(&["inspect"]);
    path_arg(&mut inspect, &source_program);
    inspect.extend(args(&["--store"]));
    path_arg(&mut inspect, &store);
    inspect.push("--json".into());
    let inspected = cli(&inspect);
    assert_success(&inspected);
    let summary: Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(summary["subject"], subject.as_str());
    assert_eq!(summary["node_count"], 3);
    assert_eq!(summary["connection_count"], 4);
    assert!(
        summary["effects"]
            .as_array()
            .unwrap()
            .contains(&json!("net.http"))
    );
    assert!(
        summary["effects"]
            .as_array()
            .unwrap()
            .contains(&json!("io.stdout"))
    );
    assert_eq!(summary["inputs"][0]["name"], "url");
    assert_eq!(summary["inputs"][0]["type"], "String");
    assert_eq!(summary["output"]["type"], "Unit");
    let nodes = summary["nodes"].as_array().unwrap();
    for node_id in ["fetch", "format", "write"] {
        assert!(nodes.iter().any(|node| node["id"] == node_id));
    }
    let requirements = summary["capability_requirements"].as_array().unwrap();
    assert!(requirements.iter().any(|requirement| {
        requirement["capability"] == "http.get" && requirement["resource"] == "url.origin"
    }));
    assert!(requirements.iter().any(|requirement| {
        requirement["capability"] == "console.write" && requirement["resource"] == "stdout"
    }));
    assert!(server.request_lines().is_empty());

    let manifest = origin_manifest_file(tmp.path(), &origin, "allow", "allow");
    let url = format!("{origin}/test?query=one");
    let input = tmp.path().join("input.json");
    let success_receipt = tmp.path().join("success.receipt.json");
    let success = run_program(
        &store,
        &source_program,
        &url,
        &manifest,
        &["stdout".into(), origin.clone()],
        &success_receipt,
        &input,
    );
    assert_exit(&success, 0);
    assert_eq!(success.stdout, format!("Response: {body}\n").as_bytes());
    assert!(success.stderr.is_empty());
    server.wait_for_requests(1);
    assert_eq!(server.request_lines()[0], "GET /test?query=one HTTP/1.1");

    let receipt = read_json(&success_receipt);
    assert_eq!(receipt["program"], subject.as_str());
    assert_eq!(receipt["outcome"]["kind"], "Succeeded");
    assert_eq!(
        receipt["outcome"]["value"],
        json!({"type":"Unit","value":null})
    );
    assert_eq!(receipt["effects"].as_array().unwrap().len(), 2);
    let http_effect = find_effect(&receipt, "std.http.get");
    assert_eq!(http_effect["resource"], origin);
    assert_eq!(http_effect["dispatched"], true);
    let console_effect = find_effect(&receipt, "std.console.write_line");
    assert_eq!(console_effect["resource"], "stdout");
    assert_eq!(console_effect["dispatched"], true);
    assert!(
        receipt["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| {
                decision["operation"] == "std.http.get"
                    && decision["resource"] == origin
                    && decision["decision"] == "allow"
            })
    );

    // A manifest request and policy cannot replace the launcher’s explicit
    // origin grant. No HTTP request reaches the local server.
    let no_network_receipt = tmp.path().join("no-network.receipt.json");
    let no_network = run_program(
        &store,
        &source_program,
        &url,
        &manifest,
        &["stdout".into()],
        &no_network_receipt,
        &input,
    );
    assert_exit(&no_network, 3);
    assert!(no_network.stdout.is_empty());
    assert_eq!(read_json(&no_network_receipt)["outcome"]["kind"], "Denied");
    assert_eq!(
        find_effect(&read_json(&no_network_receipt), "std.http.get")["dispatched"],
        false
    );
    server.assert_request_count(1);

    // Granting network authority but omitting stdout allows the GET, then
    // denies the later console action with both effects retained in the receipt.
    let no_stdout_receipt = tmp.path().join("no-stdout.receipt.json");
    let no_stdout = run_program(
        &store,
        &source_program,
        &url,
        &manifest,
        std::slice::from_ref(&origin),
        &no_stdout_receipt,
        &input,
    );
    assert_exit(&no_stdout, 3);
    assert!(no_stdout.stdout.is_empty());
    let partial = read_json(&no_stdout_receipt);
    assert_eq!(partial["outcome"]["kind"], "Denied");
    assert_eq!(find_effect(&partial, "std.http.get")["dispatched"], true);
    assert_eq!(
        find_effect(&partial, "std.console.write_line")["dispatched"],
        false
    );
    server.wait_for_requests(2);

    // Deny, Ask, and a different exact origin all stop before socket dispatch.
    for (label, decision, grants, exit) in [
        ("deny", "deny", vec!["stdout".to_owned(), origin.clone()], 3),
        ("ask", "ask", vec!["stdout".to_owned(), origin.clone()], 4),
        (
            "wrong-origin",
            "allow",
            vec!["stdout".to_owned(), "http://127.0.0.1:1".to_owned()],
            3,
        ),
    ] {
        let gate_manifest = origin_manifest_file(tmp.path(), &origin, decision, "allow");
        let receipt_path = tmp.path().join(format!("{label}.receipt.json"));
        let result = run_program(
            &store,
            &source_program,
            &url,
            &gate_manifest,
            &grants,
            &receipt_path,
            &input,
        );
        assert_exit(&result, exit);
        assert!(result.stdout.is_empty());
        let gate_receipt = read_json(&receipt_path);
        assert_eq!(
            gate_receipt["outcome"]["kind"],
            if exit == 4 {
                "AwaitingApproval"
            } else {
                "Denied"
            }
        );
        assert_eq!(
            find_effect(&gate_receipt, "std.http.get")["dispatched"],
            false
        );
        server.assert_request_count(2);
    }

    let wrong_origin = origin_manifest_file(tmp.path(), "http://wrong.invalid", "allow", "allow");
    let wrong_manifest_receipt = tmp.path().join("wrong-manifest-origin.receipt.json");
    let wrong_manifest = run_program(
        &store,
        &source_program,
        &url,
        &wrong_origin,
        &["stdout".into(), origin.clone()],
        &wrong_manifest_receipt,
        &input,
    );
    assert_exit(&wrong_manifest, 3);
    assert!(wrong_manifest.stdout.is_empty());
    assert_eq!(
        read_json(&wrong_manifest_receipt)["outcome"]["kind"],
        "Denied"
    );
    assert_eq!(
        find_effect(&read_json(&wrong_manifest_receipt), "std.http.get")["dispatched"],
        false
    );
    server.assert_request_count(2);

    // Save the composed graph as a memory, then load and execute it through a
    // separate process. Nested memories carry code identity, not authority.
    let mut seal_program = args(&["memory", "seal", "--program"]);
    path_arg(&mut seal_program, &source_program);
    seal_program.extend(args(&[
        "--name",
        "local.http.client",
        "--version",
        "1.0.0",
        "--store",
    ]));
    path_arg(&mut seal_program, &store);
    assert_success(&cli(&seal_program));

    let reuse_source = tmp.path().join("reuse.nexa");
    fs::write(
        &reuse_source,
        "nexa 0.3\nmodule example.http_reuse\n\nexport blueprint fetch_again {\n  input url is String\n  output result is Unit\n\n  node saved = memory(\"local.http.client@1.0.0\", export: \"fetch_page\")\n  connect input.url -> saved.url\n  connect saved.result -> output.result\n}\n",
    )
    .unwrap();
    let reuse_lock = tmp.path().join("reuse.lock");
    let mut resolve_reuse = args(&["resolve"]);
    path_arg(&mut resolve_reuse, &reuse_source);
    resolve_reuse.extend(args(&["--store"]));
    path_arg(&mut resolve_reuse, &store);
    resolve_reuse.extend(args(&["--write-lock"]));
    path_arg(&mut resolve_reuse, &reuse_lock);
    assert_success(&cli(&resolve_reuse));

    let reuse_program = tmp.path().join("reuse.program.json");
    let mut compose_reuse = args(&["compose"]);
    path_arg(&mut compose_reuse, &reuse_source);
    compose_reuse.extend(args(&["--locked", "--lock"]));
    path_arg(&mut compose_reuse, &reuse_lock);
    compose_reuse.extend(args(&["--store"]));
    path_arg(&mut compose_reuse, &store);
    compose_reuse.extend(args(&["--out"]));
    path_arg(&mut compose_reuse, &reuse_program);
    assert_success(&cli(&compose_reuse));

    let nested_url = format!("{origin}/nested?query=two");
    let nested_receipt_path = tmp.path().join("nested.receipt.json");
    let nested_run = run_program(
        &store,
        &reuse_program,
        &nested_url,
        &manifest,
        &["stdout".into(), origin.clone()],
        &nested_receipt_path,
        &input,
    );
    assert_exit(&nested_run, 0);
    assert_eq!(nested_run.stdout, format!("Response: {body}\n").as_bytes());
    server.wait_for_requests(3);
    let nested_receipt = read_json(&nested_receipt_path);
    assert_eq!(
        find_effect(&nested_receipt, "std.http.get")["node_path"],
        "saved/fetch"
    );
    assert_eq!(
        find_effect(&nested_receipt, "std.console.write_line")["node_path"],
        "saved/write"
    );

    let nested_denied_path = tmp.path().join("nested-no-network.receipt.json");
    let nested_denied = run_program(
        &store,
        &reuse_program,
        &nested_url,
        &manifest,
        &["stdout".into()],
        &nested_denied_path,
        &input,
    );
    assert_exit(&nested_denied, 3);
    assert!(nested_denied.stdout.is_empty());
    assert_eq!(read_json(&nested_denied_path)["outcome"]["kind"], "Denied");
    server.assert_request_count(3);

    // The immutable HTTP program’s subject and inspected contract agree with
    // the fresh-process execution identity.
    assert_eq!(program_digest(&source_program), subject);
    let stored_program = Store::open(&store).unwrap();
    let stored_subject = stored_program
        .put_document(&read_program(&source_program))
        .unwrap();
    assert_eq!(stored_subject, subject);
}

fn read_program(path: &Path) -> nexa::contracts::ProgramSpec {
    let value = canonical::parse(&fs::read(path).unwrap()).unwrap();
    nexa::contracts::decode(value).unwrap()
}
