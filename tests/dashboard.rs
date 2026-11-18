use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use tempfile::TempDir;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

struct Dashboard {
    _child: ChildGuard,
    base_url: String,
    _temp: TempDir,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct Response {
    status: u16,
    headers: String,
    body: Vec<u8>,
}

impl Dashboard {
    fn start() -> Self {
        let temp = tempfile::tempdir().expect("create isolated dashboard store");
        let mut child = ChildGuard(
            Command::new(env!("CARGO_BIN_EXE_nexa-dashboard"))
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .arg("--port")
                .arg("0")
                .arg("--store")
                .arg(temp.path().join("store"))
                .arg("--seed-examples")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("spawn Nexa dashboard"),
        );

        // Read stdout off-thread so a startup failure remains bounded even if the child
        // stays alive without printing its listening address.
        let stdout = child
            .0
            .stdout
            .take()
            .expect("capture dashboard startup output");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        let base_url = loop {
            if let Some(address) = child.0.try_wait().expect("check dashboard startup") {
                panic!("dashboard exited before startup address was printed: {address}");
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "dashboard did not print its listening URL within {STARTUP_TIMEOUT:?}"
            );
            match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(line) => {
                    if let Some(url) = extract_loopback_url(&line) {
                        break url;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("dashboard closed startup output before printing its listening URL")
                }
            }
        };

        let address = base_url
            .strip_prefix("http://")
            .expect("dashboard startup URL uses HTTP")
            .parse::<SocketAddr>()
            .expect("dashboard startup URL contains a socket address");
        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            match TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
                Ok(stream) => {
                    drop(stream);
                    break;
                }
                Err(_) if std::time::Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => {
                    panic!("dashboard did not accept connections before timeout: {error}")
                }
            }
        }

        let dashboard = Self {
            _child: child,
            base_url,
            _temp: temp,
        };
        let health = dashboard.get("/api/health");
        assert_eq!(health.status, 200, "dashboard health endpoint is ready");
        let health: Value = json_body(&health);
        assert_eq!(health["ok"], true);
        assert_eq!(health["network_enabled"], cfg!(feature = "http-client"));
        dashboard
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        client_header: bool,
        origin: Option<&str>,
    ) -> Response {
        let address = self.base_url.strip_prefix("http://").unwrap();
        let mut stream = TcpStream::connect(address).expect("connect to dashboard");
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .expect("set dashboard read timeout");
        stream
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .expect("set dashboard write timeout");

        let body = body.unwrap_or_default();
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        if method == "POST" {
            request.push_str("Content-Type: application/json\r\n");
        }
        if client_header {
            request.push_str("X-Nexa-Client: dashboard\r\n");
        }
        if let Some(origin) = origin {
            request.push_str(&format!("Origin: {origin}\r\n"));
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .expect("write dashboard HTTP request");
        stream
            .write_all(body)
            .expect("write dashboard request body");
        stream
            .shutdown(Shutdown::Write)
            .expect("finish dashboard HTTP request");

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .expect("read dashboard HTTP response");
        let header_end = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("HTTP response has a header terminator");
        let header_bytes = &raw[..header_end];
        let headers = String::from_utf8_lossy(header_bytes).into_owned();
        let status = headers
            .lines()
            .next()
            .expect("HTTP status line")
            .split_whitespace()
            .nth(1)
            .expect("HTTP status code")
            .parse()
            .expect("numeric HTTP status code");
        let mut response_body = raw[header_end + 4..].to_vec();
        if headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("Transfer-Encoding: chunked"))
        {
            response_body = decode_chunked(&response_body);
        }
        Response {
            status,
            headers,
            body: response_body,
        }
    }

    fn get(&self, path: &str) -> Response {
        self.request("GET", path, None, false, None)
    }

    fn post(&self, path: &str, body: &Value) -> Response {
        self.request(
            "POST",
            path,
            Some(&serde_json::to_vec(body).expect("serialize JSON request")),
            true,
            Some(&self.base_url),
        )
    }

    fn json(&self, path: &str, body: &Value) -> (Response, Value) {
        let response = self.post(path, body);
        let value = serde_json::from_slice(&response.body).unwrap_or_else(|error| {
            panic!(
                "expected JSON from {path}, got HTTP {} with body {:?}: {error}",
                response.status,
                String::from_utf8_lossy(&response.body)
            )
        });
        (response, value)
    }
}

fn extract_loopback_url(line: &str) -> Option<String> {
    let start = line.find("http://127.0.0.1:")?;
    let candidate = line[start..]
        .split_whitespace()
        .next()?
        .trim_end_matches([')', ']', ',', ';']);
    let authority = candidate.strip_prefix("http://")?;
    let port = authority.rsplit_once(':')?.1.parse::<u16>().ok()?;
    (port != 0).then(|| candidate.to_owned())
}

fn decode_chunked(mut bytes: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    loop {
        let line_end = bytes
            .windows(2)
            .position(|window| window == b"\r\n")
            .expect("chunk size line terminator");
        let size = usize::from_str_radix(
            std::str::from_utf8(&bytes[..line_end])
                .expect("chunk size text")
                .split(';')
                .next()
                .expect("chunk size"),
            16,
        )
        .expect("hex chunk size");
        bytes = &bytes[line_end + 2..];
        if size == 0 {
            return decoded;
        }
        assert!(bytes.len() >= size + 2, "complete HTTP response chunk");
        decoded.extend_from_slice(&bytes[..size]);
        assert_eq!(&bytes[size..size + 2], b"\r\n", "chunk terminator");
        bytes = &bytes[size + 2..];
    }
}

fn json_body(response: &Response) -> Value {
    serde_json::from_slice(&response.body).unwrap_or_else(|error| {
        panic!(
            "expected JSON response, got HTTP {} and {:?}: {error}",
            response.status,
            String::from_utf8_lossy(&response.body)
        )
    })
}

#[test]
fn dashboard_builds_and_runs_core1_as_a_typed_blueprint() {
    let dashboard = Dashboard::start();
    let response = dashboard.get("/api/workspace");
    assert_eq!(response.status, 200);
    let workspace = json_body(&response);
    let proof = workspace["programs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|program| program["id"] == "core1-http")
        .expect("the proof is listed alongside the other blueprints");
    let draft = &proof["draft"];
    assert_eq!(draft["profile"], "core1");
    let nodes = draft["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    let memories = workspace["memories"].as_array().unwrap();
    for node in nodes {
        let memory = memories
            .iter()
            .find(|memory| memory["digest"] == node["selector"])
            .unwrap();
        assert_eq!(memory["profile"], "core1");
        assert!(
            memory["source"][0]["text"]
                .as_str()
                .unwrap()
                .contains("export fn")
        );
        assert_eq!(memory["exports"][0]["export_name"], node["export"]);
    }

    let (response, built) = dashboard.json("/api/build", &json!({"draft": draft}));
    assert_eq!(response.status, 200, "{built}");
    assert_eq!(built["inspection"]["node_count"], 3);
    assert_eq!(built["program"]["profile"], "core1");
    assert_eq!(
        built["program"]["nodes"].as_array().unwrap().len(),
        nodes.len()
    );
    let mut reordered = draft.clone();
    reordered["nodes"].as_array_mut().unwrap().reverse();
    reordered["connections"].as_array_mut().unwrap().reverse();
    let (response, rebuilt) = dashboard.json("/api/build", &json!({"draft": reordered}));
    assert_eq!(response.status, 200, "{rebuilt}");
    assert_eq!(built["subject"], rebuilt["subject"]);

    let fixtures: Value =
        serde_json::from_str(include_str!("../examples/native-http/fixtures.json")).unwrap();
    for fixture in fixtures["cases"].as_array().unwrap() {
        let request = json!({
            "program": built["program"],
            "input": fixture["input"],
            "manifest": proof["manifest"],
            "allow": []
        });
        let (response, payload) = dashboard.json("/api/run", &request);
        assert_eq!(response.status, 200, "{}: {payload}", fixture["name"]);
        assert_eq!(payload["receipt"]["program"], built["subject"]);
        assert_eq!(payload["receipt"]["outcome"]["kind"], "Succeeded");
        assert_eq!(payload["receipt"]["outcome"]["value"], fixture["expected"]);
        assert_eq!(payload["receipt"]["effects"], json!([]));
    }

    // Both ports accept Bytes. Swapping them builds a different program
    // and causes the original target bytes to fail authority validation.
    let mut swapped = draft.clone();
    for wire in swapped["connections"].as_array_mut().unwrap() {
        if wire["from"]["kind"] == "input" {
            match wire["from"]["port"].as_str() {
                Some("authority") => wire["from"]["port"] = json!("target"),
                Some("target") => wire["from"]["port"] = json!("authority"),
                _ => {}
            }
        }
    }
    let (response, swapped) = dashboard.json("/api/build", &json!({"draft": swapped}));
    assert_eq!(response.status, 200, "{swapped}");
    assert_ne!(swapped["subject"], built["subject"]);
    let (response, payload) = dashboard.json(
        "/api/run",
        &json!({
            "program": swapped["program"],
            "input": proof["inputs"],
            "manifest": proof["manifest"],
            "allow": []
        }),
    );
    assert_eq!(response.status, 200, "{payload}");
    assert_eq!(
        payload["receipt"]["outcome"]["value"],
        json!({"variant": "request_error", "fields": {"error": "invalid_authority"}})
    );

    let mut rewired = draft.clone();
    let wires = rewired["connections"].as_array_mut().unwrap();
    let from_authority = wires
        .iter_mut()
        .find(|wire| wire["from"]["kind"] == "input" && wire["from"]["port"] == "authority")
        .unwrap();
    from_authority["from"]["port"] = json!("chunks");
    let (response, diagnostic) = dashboard.json("/api/build", &json!({"draft": rewired}));
    assert_eq!(response.status, 400, "{diagnostic}");

    let mut tampered = built["program"].clone();
    tampered["connections"].as_array_mut().unwrap().pop();
    let (response, diagnostic) = dashboard.json(
        "/api/run",
        &json!({
            "program": tampered,
            "input": proof["inputs"],
            "manifest": proof["manifest"],
            "allow": []
        }),
    );
    assert_eq!(response.status, 400, "{diagnostic}");
}

#[test]
fn dashboard_seeds_builds_runs_seals_and_serves_static_assets() {
    let dashboard = Dashboard::start();

    let workspace = dashboard.get("/api/workspace");
    assert_eq!(workspace.status, 200);
    let workspace: Value = json_body(&workspace);
    assert!(workspace["kernel"]["descriptor"].is_object());
    assert!(workspace["kernel"]["digest"].as_str().is_some());
    assert_eq!(workspace["network_enabled"], cfg!(feature = "http-client"));
    let memories = workspace["memories"].as_array().expect("memory list");
    assert!(memories.len() >= 3, "seeded examples are present");
    for (selector, source_fragment) in [
        ("local.text.trim@1.0.0", "std.text.trim_ascii"),
        ("local.hello.greet@1.0.0", "Hello, "),
        ("local.console.line@1.0.0", "std.console.write_line"),
    ] {
        let memory = memories
            .iter()
            .find(|memory| memory["selector"] == selector)
            .unwrap_or_else(|| panic!("seeded memory {selector} is available"));
        assert!(memory["digest"].as_str().is_some());
        assert!(memory["exports"].is_array());
        let sources = memory["source"].as_array().expect("memory source files");
        assert!(
            sources.iter().any(|source| {
                source["path"].as_str().is_some()
                    && source["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(source_fragment))
            }),
            "{selector} contains its seeded source"
        );
    }

    let draft: Value = serde_json::from_str(include_str!("../examples/hello/hello.compose.json"))
        .expect("example hello draft is valid JSON");
    let hello_example = workspace["programs"]
        .as_array()
        .expect("program examples")
        .iter()
        .find(|example| example["id"] == "hello")
        .expect("hello example is available");
    assert_eq!(hello_example["draft"]["connections"], draft["connections"]);
    assert_eq!(hello_example["inputs"]["name"], "  Ada  ");
    assert!(hello_example["manifest"].is_object());
    let (built, built_body) = dashboard.json("/api/build", &json!({ "draft": draft }));
    assert_eq!(built.status, 200, "build response: {built_body}");
    let program = built_body["program"].clone();
    assert!(program.is_object(), "build returns a composed program");
    assert_eq!(program["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(program["connections"], draft["connections"]);
    assert_eq!(program["connections"].as_array().unwrap().len(), 4);
    assert!(built_body["subject"].as_str().is_some());
    assert!(built_body["inspection"].is_object());

    let manifest: Value = serde_json::from_str(include_str!("../examples/hello/hello.app.json"))
        .expect("example hello manifest is valid JSON");
    let run_request = |allow: Vec<&str>| {
        json!({
            "program": program,
            "input": { "name": "  Ada  " },
            "manifest": manifest,
            "allow": allow,
        })
    };
    let (run, run_body) = dashboard.json("/api/run", &run_request(vec!["stdout"]));
    assert_eq!(run.status, 200, "run response: {run_body}");
    assert_eq!(run_body["exit_code"], 0);
    assert_eq!(run_body["stdout"], "Hello, Ada!\n");
    assert!(run_body["receipt"].is_object());
    assert_eq!(run_body["receipt"]["outcome"]["kind"], "Succeeded");

    let (denied, denied_body) = dashboard.json("/api/run", &run_request(vec![]));
    assert_eq!(denied.status, 200, "denied run response: {denied_body}");
    assert_eq!(denied_body["exit_code"], 3);
    assert_eq!(denied_body["stdout"], "");
    assert!(denied_body["receipt"].is_object());
    assert_eq!(denied_body["receipt"]["outcome"]["kind"], "Denied");
    assert!(
        denied_body["receipt"]["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|effect| {
                effect["operation"] == "std.console.write_line" && effect["dispatched"] == false
            })
    );

    let bad_wire = {
        let mut draft = draft.clone();
        draft["connections"][0]["to"]["port"] = json!("missing");
        draft
    };
    let (bad_build, bad_build_body) = dashboard.json("/api/build", &json!({ "draft": bad_wire }));
    assert!(
        !(200..300).contains(&bad_build.status),
        "bad typed connection should fail construction: {bad_build_body}"
    );
    assert!(bad_build_body["diagnostic"].is_object());

    let (sealed, sealed_body) = dashboard.json(
        "/api/seal",
        &json!({
            "program": program,
            "name": "local.hello.webcomposed",
            "version": "1.0.0",
        }),
    );
    assert_eq!(sealed.status, 200, "seal response: {sealed_body}");
    assert!(sealed_body["digest"].as_str().is_some());
    let after_seal = json_body(&dashboard.get("/api/workspace"));
    assert!(
        after_seal["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|memory| { memory["selector"] == "local.hello.webcomposed@1.0.0" })
    );

    let index = dashboard.get("/");
    assert_eq!(index.status, 200);
    assert!(
        index
            .headers
            .to_ascii_lowercase()
            .contains("content-type: text/html")
    );
    assert!(String::from_utf8_lossy(&index.body).contains("Nexa"));

    for (path, mime) in [
        ("/styles.css", "text/css"),
        ("/graph.css", "text/css"),
        ("/app.js", "text/javascript"),
        ("/graph.js", "text/javascript"),
    ] {
        let asset = dashboard.get(path);
        assert_eq!(asset.status, 200, "static asset {path} is served");
        assert!(
            asset
                .headers
                .to_ascii_lowercase()
                .contains(&format!("content-type: {mime}")),
            "static asset {path} has {mime} content type: {}",
            asset.headers
        );
    }
}

#[test]
fn dashboard_rejects_bad_json_cross_site_origins_and_unmarked_clients() {
    let dashboard = Dashboard::start();

    let malformed = dashboard.request(
        "POST",
        "/api/build",
        Some(br#"{"draft": "#),
        true,
        Some(&dashboard.base_url),
    );
    assert_eq!(malformed.status, 400);
    assert!(json_body(&malformed)["diagnostic"].is_object());

    let draft: Value = serde_json::from_str(include_str!("../examples/hello/hello.compose.json"))
        .expect("example hello draft is valid JSON");
    let valid_body = serde_json::to_vec(&json!({ "draft": draft })).unwrap();
    let cross_site = dashboard.request(
        "POST",
        "/api/build",
        Some(&valid_body),
        true,
        Some("http://untrusted.example"),
    );
    assert!(
        !(200..300).contains(&cross_site.status),
        "cross-site Origin is rejected"
    );

    let absent_origin = dashboard.request("POST", "/api/build", Some(&valid_body), true, None);
    assert_eq!(
        absent_origin.status, 200,
        "CLI POST without Origin is allowed"
    );
    assert!(json_body(&absent_origin)["program"].is_object());

    let unmarked_client = dashboard.request(
        "POST",
        "/api/build",
        Some(&valid_body),
        false,
        Some(&dashboard.base_url),
    );
    assert!(
        !(200..300).contains(&unmarked_client.status),
        "POST without the dashboard client header is rejected"
    );

    let draft_json = serde_json::to_string(&draft).unwrap();
    let duplicate_fields = format!("{{\"draft\":{draft_json},\"draft\":{draft_json}}}");
    let duplicate_json = dashboard.request(
        "POST",
        "/api/build",
        Some(duplicate_fields.as_bytes()),
        true,
        Some(&dashboard.base_url),
    );
    assert_eq!(duplicate_json.status, 400);
    assert!(json_body(&duplicate_json)["diagnostic"].is_object());
}
