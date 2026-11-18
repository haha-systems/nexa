//! Local-only dashboard API for composing, checking, running, and sealing Nexa
//! programs. This binary deliberately uses a fixed route and file allowlist;
//! it does not accept filesystem paths or shell commands from HTTP requests.

use clap::Parser;
use nexa::{
    blueprint, canonical,
    contracts::{
        self, ApplicationManifest, BlueprintDraft, CallableContract, Implementation, ProgramSpec,
        WireDocument,
    },
    core1,
    diagnostic::{Error, Result},
    digest::Digest,
    kernel::KernelDescriptor,
    memory::{self, Store},
    runtime::ExecutionContext,
    value::{Identifier, Value, ValueType},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_PORT: u16 = 8787;
const MAX_REQUEST_BODY: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_ACTIVE_CONNECTIONS: usize = 16;
const MAX_MEMORIES: usize = 200;
const MAX_SOURCE_BYTES_PER_MEMORY: usize = 64 * 1024;
const MAX_WORKSPACE_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Parser)]
#[command(name = "nexa-dashboard", about = "Run the local Nexa dashboard")]
struct Args {
    /// Local TCP port. Use 0 to ask the operating system to select a port.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Immutable memory store used by this dashboard instance.
    #[arg(long, default_value = ".nexa/dashboard-store")]
    store: PathBuf,

    /// Seal the greeting and Core1 proof, plus the HTTP adapter example when enabled.
    #[arg(long)]
    seed_examples: bool,
}

#[derive(Clone)]
struct Service {
    store: Store,
    port: u16,
    static_root: PathBuf,
    core1_proof: Option<Digest>,
}

#[derive(Debug)]
struct HttpFailure {
    status: u16,
    code: &'static str,
    message: String,
}

impl HttpFailure {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildRequest {
    draft: BlueprintDraft,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    program: ProgramSpec,
    input: Json,
    manifest: ApplicationManifest,
    allow: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SealRequest {
    program: ProgramSpec,
    name: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Core1RunRequest {
    input: Json,
}

#[derive(Serialize)]
struct WorkspaceResponse {
    kernel: KernelView,
    memories: Vec<MemoryView>,
    programs: Vec<ProgramExample>,
    network_enabled: bool,
    memories_limited: bool,
}

#[derive(Serialize)]
struct KernelView {
    descriptor: KernelDescriptor,
    digest: Digest,
}

#[derive(Serialize)]
struct MemoryView {
    digest: Digest,
    selector: String,
    name: String,
    version: String,
    kind: &'static str,
    exports: Vec<CallableContract>,
    source: Vec<SourceView>,
    program: Option<ProgramSpec>,
    evidence: Vec<Digest>,
    program_evidence: Vec<Digest>,
}

#[derive(Serialize)]
struct SourceView {
    path: String,
    text: String,
}

#[derive(Serialize)]
struct ProgramExample {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    draft: BlueprintDraft,
    /// Concrete input values from the checked-in example input JSON.
    inputs: Json,
    /// Typed input ports from the actual compose draft.
    input_ports: Vec<nexa::contracts::Port>,
    manifest: ApplicationManifest,
}

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn main() {
    let args = Args::parse();
    if let Err(error) = start(args) {
        eprintln!("nexa-dashboard: {error}");
        process::exit(1);
    }
}

fn start(args: Args) -> std::result::Result<(), String> {
    let store = Store::open(&args.store).map_err(|error| error.to_string())?;
    let core1_proof = if args.seed_examples {
        seed_examples(&store).map_err(|error| error.to_string())?;
        Some(seed_core1_proof(&store).map_err(|error| error.to_string())?)
    } else {
        None
    };

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, args.port))
        .map_err(|error| format!("bind 127.0.0.1:{}: {error}", args.port))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("read local listener address: {error}"))?
        .port();
    let service = Arc::new(Service {
        store,
        port,
        static_root: Path::new(env!("CARGO_MANIFEST_DIR")).join("dashboard"),
        core1_proof,
    });
    let active = Arc::new(AtomicUsize::new(0));

    println!("Listening on http://127.0.0.1:{port}");
    let _ = std::io::stdout().flush();

    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("nexa-dashboard: accept failed: {error}");
                continue;
            }
        };

        if !try_acquire_connection(&active) {
            let _ = stream.set_write_timeout(Some(SOCKET_TIMEOUT));
            let _ = write_response(
                stream,
                json_response(
                    503,
                    &json!({"diagnostic":{"code":"E_BUSY","message":"dashboard is at its connection limit"}}),
                ),
            );
            continue;
        }

        let service = Arc::clone(&service);
        let worker_active = Arc::clone(&active);
        if let Err(error) = thread::Builder::new()
            .name("nexa-dashboard-client".into())
            .spawn(move || {
                let _guard = ActiveConnection(worker_active);
                handle_connection(stream, &service);
            })
        {
            active.fetch_sub(1, Ordering::SeqCst);
            eprintln!("nexa-dashboard: could not start request worker: {error}");
        }
    }
    Ok(())
}

fn try_acquire_connection(active: &AtomicUsize) -> bool {
    let mut current = active.load(Ordering::SeqCst);
    loop {
        if current >= MAX_ACTIVE_CONNECTIONS {
            return false;
        }
        match active.compare_exchange_weak(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => return true,
            Err(updated) => current = updated,
        }
    }
}

fn handle_connection(mut stream: TcpStream, service: &Service) {
    if let Err(error) = stream
        .set_read_timeout(Some(SOCKET_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(SOCKET_TIMEOUT)))
    {
        let _ = write_response(
            stream,
            json_response(
                500,
                &json!({"diagnostic":{"code":"E_SOCKET","message":error.to_string()}}),
            ),
        );
        return;
    }

    let response = match read_request(&mut stream) {
        Ok(request) => dispatch(request, service),
        Err(error) => json_response(
            error.status,
            &json!({"diagnostic":{"code":error.code,"message":error.message}}),
        ),
    };
    let _ = write_response(stream, response);
}

fn read_request(stream: &mut TcpStream) -> std::result::Result<Request, HttpFailure> {
    let mut bytes = Vec::with_capacity(8192);
    let header_end = loop {
        if let Some(position) = find_header_end(&bytes) {
            if position > MAX_HEADER_BYTES {
                return Err(HttpFailure::new(
                    431,
                    "E_HEADERS_TOO_LARGE",
                    "HTTP request headers exceed the configured limit",
                ));
            }
            break position;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(HttpFailure::new(
                431,
                "E_HEADERS_TOO_LARGE",
                "HTTP request headers exceed the configured limit",
            ));
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).map_err(|error| {
            HttpFailure::new(
                400,
                "E_HTTP_READ",
                format!("could not read request: {error}"),
            )
        })?;
        if read == 0 {
            return Err(HttpFailure::new(
                400,
                "E_HTTP_READ",
                "connection closed before the HTTP headers were complete",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    };

    let header_text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| {
        HttpFailure::new(
            400,
            "E_HTTP_HEADERS",
            "HTTP headers must be valid ASCII text",
        )
    })?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if method.is_empty()
        || target.is_empty()
        || version != "HTTP/1.1"
        || parts.next().is_some()
        || !target.starts_with('/')
        || target.contains(['?', '#', '\\'])
    {
        return Err(HttpFailure::new(
            400,
            "E_HTTP_REQUEST",
            "expected an HTTP/1.1 request with an origin-form path and no query",
        ));
    }

    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpFailure::new(
                400,
                "E_HTTP_HEADERS",
                "malformed HTTP header line",
            ));
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(HttpFailure::new(
                400,
                "E_HTTP_HEADERS",
                "invalid HTTP header name",
            ));
        }
        let key = name.to_ascii_lowercase();
        if headers
            .insert(key, value.trim_matches([' ', '\t']).to_owned())
            .is_some()
        {
            return Err(HttpFailure::new(
                400,
                "E_HTTP_HEADERS",
                "duplicate HTTP headers are not supported",
            ));
        }
    }

    let body_start = header_end + 4;
    let initial_body = bytes.len().saturating_sub(body_start);
    let content_length = match headers.get("content-length") {
        Some(value) => value.parse::<usize>().map_err(|_| {
            HttpFailure::new(400, "E_CONTENT_LENGTH", "invalid Content-Length header")
        })?,
        None => 0,
    };
    if headers.contains_key("transfer-encoding") {
        return Err(HttpFailure::new(
            400,
            "E_TRANSFER_ENCODING",
            "transfer-encoded request bodies are not supported",
        ));
    }
    if content_length > MAX_REQUEST_BODY {
        return Err(HttpFailure::new(
            413,
            "E_BODY_TOO_LARGE",
            "request body exceeds the 1 MiB limit",
        ));
    }
    if initial_body > content_length {
        return Err(HttpFailure::new(
            400,
            "E_HTTP_BODY",
            "request contains bytes beyond its declared Content-Length",
        ));
    }
    let mut body = bytes[body_start..].to_vec();
    body.resize(content_length, 0);
    if initial_body < content_length {
        stream
            .read_exact(&mut body[initial_body..])
            .map_err(|error| {
                HttpFailure::new(
                    400,
                    "E_HTTP_BODY",
                    format!("incomplete request body: {error}"),
                )
            })?;
    }

    Ok(Request {
        method: method.to_owned(),
        target: target.to_owned(),
        headers,
        body,
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn dispatch(request: Request, service: &Service) -> HttpResponse {
    let host = match request.headers.get("host") {
        Some(host) => host,
        None => {
            return json_response(
                400,
                &json!({"diagnostic":{"code":"E_HOST","message":"Host header is required"}}),
            );
        }
    };
    if !valid_loopback_host(host, service.port) {
        return json_response(
            421,
            &json!({"diagnostic":{"code":"E_HOST","message":"Host must identify this loopback dashboard listener"}}),
        );
    }

    if request.method == "POST" {
        if !valid_same_origin_post(&request, host, service.port) {
            return json_response(
                403,
                &json!({"diagnostic":{"code":"E_ORIGIN","message":"POST requires X-Nexa-Client: dashboard and, when supplied, a matching loopback Origin"}}),
            );
        }
        if !is_json_content_type(request.headers.get("content-type")) {
            return json_response(
                415,
                &json!({"diagnostic":{"code":"E_CONTENT_TYPE","message":"POST requests must use application/json"}}),
            );
        }
    } else if request.method != "GET" {
        return json_response(
            405,
            &json!({"diagnostic":{"code":"E_METHOD","message":"only GET and POST are supported"}}),
        );
    }

    match (request.method.as_str(), request.target.as_str()) {
        ("GET", "/api/health") => json_response(
            200,
            &json!({"ok":true,"network_enabled":cfg!(feature = "http-client")}),
        ),
        ("GET", "/api/workspace") => match workspace(service) {
            Ok(value) => bounded_json_response(200, &value),
            Err(error) => workspace_error_response(error),
        },
        ("GET", "/api/core1/proof") => match core1_proof(service) {
            Ok(value) => bounded_json_response(200, &value),
            Err(error) => workspace_error_response(error),
        },
        ("POST", "/api/core1/run") => match run_core1_proof(&request.body, service) {
            Ok(value) => bounded_json_response(200, &value),
            Err(error) => diagnostic_response(error),
        },
        ("POST", "/api/build") => match build(&request.body, service) {
            Ok(value) => bounded_json_response(200, &value),
            Err(error) => diagnostic_response(error),
        },
        ("POST", "/api/run") => match run_program(&request.body, service) {
            Ok(value) => bounded_json_response(200, &value),
            Err(error) => diagnostic_response(error),
        },
        ("POST", "/api/seal") => match seal_program(&request.body, service) {
            Ok(value) => bounded_json_response(200, &value),
            Err(error) => diagnostic_response(error),
        },
        ("GET", path) => match static_file(path, &service.static_root) {
            Some((bytes, mime)) => HttpResponse::new(200, mime, bytes),
            None => json_response(
                404,
                &json!({"diagnostic":{"code":"E_NOT_FOUND","message":"route or static asset not found"}}),
            ),
        },
        _ => json_response(
            404,
            &json!({"diagnostic":{"code":"E_NOT_FOUND","message":"route not found"}}),
        ),
    }
}

fn valid_loopback_host(value: &str, port: u16) -> bool {
    let value = value.to_ascii_lowercase();
    let expected_port = port.to_string();
    ["localhost", "127.0.0.1"].iter().any(|host| {
        value
            .strip_prefix(&format!("{host}:"))
            .is_some_and(|candidate| candidate == expected_port)
    })
}

fn valid_same_origin_post(request: &Request, host: &str, port: u16) -> bool {
    if request.headers.get("x-nexa-client").map(String::as_str) != Some("dashboard") {
        return false;
    }
    let Some(origin) = request.headers.get("origin") else {
        return true;
    };
    let host = host.to_ascii_lowercase();
    let expected_localhost = format!("http://localhost:{port}");
    let expected_ip = format!("http://127.0.0.1:{port}");
    (origin == &expected_localhost && host == format!("localhost:{port}"))
        || (origin == &expected_ip && host == format!("127.0.0.1:{port}"))
}

fn is_json_content_type(value: Option<&String>) -> bool {
    value.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
    })
}

fn workspace(service: &Service) -> Result<Json> {
    let descriptor = KernelDescriptor::bundled()?;
    let descriptor_json = serde_json::to_value(&descriptor)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    let kernel = KernelView {
        digest: canonical::digest(&descriptor_json)?,
        descriptor,
    };

    let aliases = service.store.catalog_aliases()?;
    let memories_limited = aliases.len() > MAX_MEMORIES;
    let mut memories = Vec::with_capacity(aliases.len().min(MAX_MEMORIES));
    let mut source_bytes_total = 0usize;
    for (selector, digest) in aliases.into_iter().take(MAX_MEMORIES) {
        let record = service.store.load_memory(&digest)?;
        let (kind, program, program_evidence) = match &record.implementation {
            Implementation::Source { .. } => ("source", None, Vec::new()),
            Implementation::Program { program } => (
                "program",
                Some(service.store.load_document(program)?),
                service.store.evidence_for(program)?,
            ),
        };
        let mut source = Vec::new();
        let source_blobs = if kind == "source" {
            service.store.source_bytes(&digest)?
        } else {
            Vec::new()
        };
        for (path, bytes) in source_blobs {
            if bytes.len() > MAX_SOURCE_BYTES_PER_MEMORY {
                return Err(Error::new(
                    "E_LIMIT",
                    format!("memory {selector} source exceeds the 64 KiB dashboard limit"),
                )
                .at(path));
            }
            source_bytes_total = source_bytes_total.saturating_add(bytes.len());
            if source_bytes_total > MAX_WORKSPACE_SOURCE_BYTES {
                return Err(Error::new(
                    "E_LIMIT",
                    "workspace memory source exceeds the 4 MiB dashboard limit",
                ));
            }
            let text = String::from_utf8(bytes)
                .map_err(|error| Error::new("E_UTF8", error.to_string()).at(path.clone()))?;
            source.push(SourceView { path, text });
        }
        memories.push(MemoryView {
            digest: digest.clone(),
            selector,
            name: record.metadata.name,
            version: record.metadata.version,
            kind,
            exports: record.exports,
            source,
            program,
            evidence: service.store.evidence_for(&digest)?,
            program_evidence,
        });
    }

    let hello = ProgramExample {
        id: "hello",
        title: "Hello",
        description: "Trim a name, compose a greeting, and write it to stdout.",
        draft: read_example_document("examples/hello/hello.compose.json")?,
        inputs: read_example_json("examples/hello/input.json")?,
        input_ports: read_example_document::<BlueprintDraft>("examples/hello/hello.compose.json")?
            .inputs,
        manifest: read_example_document("examples/hello/hello.app.json")?,
    };
    #[cfg(feature = "http-client")]
    let programs = vec![
        hello,
        ProgramExample {
            id: "http-client",
            title: "HTTP fetch",
            description: "Fetch an explicitly granted HTTP origin, format the body, and write it to stdout.",
            draft: read_example_document("examples/http-client/client.compose.json")?,
            inputs: read_example_json("examples/http-client/input.json")?,
            input_ports: read_example_document::<BlueprintDraft>(
                "examples/http-client/client.compose.json",
            )?
            .inputs,
            manifest: read_example_document("examples/http-client/client.app.json")?,
        },
    ];
    #[cfg(not(feature = "http-client"))]
    let programs = vec![hello];

    serde_json::to_value(WorkspaceResponse {
        kernel,
        memories,
        programs,
        network_enabled: cfg!(feature = "http-client"),
        memories_limited,
    })
    .map_err(|error| Error::new("E_CONTRACT", error.to_string()))
}

fn read_example_document<T: WireDocument>(relative: &str) -> Result<T> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let bytes = fs::read(&path)
        .map_err(|error| Error::new("E_IO", error.to_string()).at(path.display().to_string()))?;
    let value = canonical::parse(&bytes)
        .map_err(|error| Error::new("E_JSON", error.diagnostic.message).at(relative.to_owned()))?;
    contracts::decode(value)
}

fn read_example_json(relative: &str) -> Result<Json> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let bytes = fs::read(&path)
        .map_err(|error| Error::new("E_IO", error.to_string()).at(path.display().to_string()))?;
    canonical::parse(&bytes)
        .map_err(|error| Error::new("E_JSON", error.diagnostic.message).at(relative.to_owned()))
}

fn build(body: &[u8], service: &Service) -> Result<Json> {
    let request: BuildRequest = decode_request(body)?;
    let requests = request
        .draft
        .nodes
        .iter()
        .map(|node| (node.selector.clone(), node.export.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let lock = service.store.resolve_requests(&requests)?;
    let checked = blueprint::compose_checked(&request.draft, &lock, &service.store)?;
    Ok(json!({
        "program": checked.program(),
        "subject": checked.program_digest(),
        "inspection": blueprint::inspect(&checked, None)?,
    }))
}

fn run_program(body: &[u8], service: &Service) -> Result<Json> {
    let request: RunRequest = decode_request(body)?;
    if request.allow.len() > 32 || request.allow.iter().any(|resource| resource.len() > 2048) {
        return Err(Error::new(
            "E_LIMIT",
            "allow list exceeds its 32-resource or 2 KiB per-resource limit",
        ));
    }
    let checked = blueprint::load_checked(&request.program, &service.store)?;
    let arguments = json_inputs(request.input, &request.program.inputs)?;
    let principal = request.manifest.principal.clone();
    let mut context = ExecutionContext::from_manifest_with_resources(
        request.manifest,
        principal,
        &request.allow,
    )?;
    let mut stdout = Vec::new();
    let receipt = blueprint::run(
        &checked,
        arguments,
        invocation_id(),
        &mut context,
        &mut stdout,
    )?;
    let stdout =
        String::from_utf8(stdout).map_err(|error| Error::new("E_UTF8", error.to_string()))?;
    let exit_code = receipt_code(&receipt);
    Ok(json!({
        "receipt": receipt,
        "stdout": stdout,
        "exit_code": exit_code,
    }))
}

fn receipt_code(receipt: &nexa::contracts::Receipt) -> u8 {
    receipt.outcome.exit_code() as u8
}

fn json_inputs(
    value: Json,
    ports: &[nexa::contracts::Port],
) -> Result<BTreeMap<Identifier, Value>> {
    let object = value.as_object().ok_or_else(|| {
        Error::new(
            "E_INPUT",
            "invocation input must be an object with exactly the declared fields",
        )
    })?;
    if object.len() != ports.len() {
        return Err(Error::new(
            "E_INPUT",
            "missing or extra invocation input fields",
        ));
    }
    let mut inputs = BTreeMap::new();
    for port in ports {
        let raw = object
            .get(port.name.as_str())
            .ok_or_else(|| Error::new("E_INPUT", format!("missing input {}", port.name)))?;
        let value = match (port.r#type, raw) {
            (ValueType::String, Json::String(text)) => Value::String(text.clone()),
            (ValueType::Unit, Json::Null) => Value::Unit(()),
            _ => {
                return Err(Error::new(
                    "E_INPUT",
                    format!(
                        "wrong type for input {}: expected {:?}",
                        port.name, port.r#type
                    ),
                ));
            }
        };
        inputs.insert(port.name.clone(), value);
    }
    Ok(inputs)
}

fn seal_program(body: &[u8], service: &Service) -> Result<Json> {
    let request: SealRequest = decode_request(body)?;
    if request.name.len() > 128 || request.version.len() > 128 {
        return Err(Error::new(
            "E_LIMIT",
            "name and version must be at most 128 bytes",
        ));
    }
    let digest = memory::seal_program(
        &request.program,
        &service.store,
        &request.name,
        &request.version,
    )?;
    Ok(json!({"digest":digest}))
}

fn decode_request<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T> {
    let value = canonical::parse(body)?;
    serde_json::from_value(value).map_err(|error| Error::new("E_JSON", error.to_string()))
}

fn invocation_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let host_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "dashboard-{}-{host_time}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn seed_examples(store: &Store) -> Result<()> {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    for name in ["trim", "greet", "write"] {
        memory::seal_source(examples.join("hello/memories").join(name), store)?;
    }
    #[cfg(feature = "http-client")]
    for name in ["fetch", "format"] {
        memory::seal_source(examples.join("http-client/memories").join(name), store)?;
    }
    Ok(())
}

fn seed_core1_proof(store: &Store) -> Result<Digest> {
    let seal = |name, source, imports: &[(&str, &Digest)]| {
        let imports = imports
            .iter()
            .map(|(alias, digest)| (alias.to_string(), (*digest).clone()))
            .collect();
        core1::seal(store, name, source, imports)
    };
    let decimal = seal(
        "decimal.nexa",
        include_str!("../../examples/native-http/decimal.nexa"),
        &[],
    )?;
    let headers = seal(
        "headers.nexa",
        include_str!("../../examples/native-http/headers.nexa"),
        &[("decimal", &decimal)],
    )?;
    let request = seal(
        "request.nexa",
        include_str!("../../examples/native-http/request.nexa"),
        &[("decimal", &decimal)],
    )?;
    let response = seal(
        "response.nexa",
        include_str!("../../examples/native-http/response.nexa"),
        &[("headers", &headers)],
    )?;
    seal(
        "exchange.nexa",
        include_str!("../../examples/native-http/exchange.nexa"),
        &[("request", &request), ("response", &response)],
    )
}

fn core1_subject(service: &Service) -> Result<&Digest> {
    service.core1_proof.as_ref().ok_or_else(|| {
        Error::new(
            "E_NOT_FOUND",
            "Start the local dashboard with --seed-examples to load the Core1 proof.",
        )
    })
}

fn core1_proof(service: &Service) -> Result<Json> {
    let subject = core1_subject(service)?;
    let memories = core1::inspect_closure(&service.store, subject)?;
    let sources = memories
        .iter()
        .map(|(digest, memory)| {
            let bytes = core1::read_bounded(
                &service.store.object_path(&memory.source),
                core1::SOURCE_LIMIT,
            )?;
            memory.source.verify(&bytes)?;
            let text = String::from_utf8(bytes)
                .map_err(|error| Error::new("E_UTF8", error.to_string()))?;
            Ok((digest.clone(), text))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let input = canonical::parse(include_bytes!("../../examples/native-http/input.json"))?;
    Ok(json!({"subject": subject, "memories": memories, "sources": sources, "input": input}))
}

fn run_core1_proof(body: &[u8], service: &Service) -> Result<Json> {
    let request: Core1RunRequest = decode_request(body)?;
    let subject = core1_subject(service)?;
    let value = core1::run(&service.store, subject, &request.input, core1::DEFAULT_FUEL)?;
    Ok(json!({"subject": subject, "value": value}))
}

fn static_file(path: &str, root: &Path) -> Option<(Vec<u8>, &'static str)> {
    let (name, content_type) = match path {
        "/" | "/index.html" | "/dashboard/" | "/dashboard/index.html" => {
            ("index.html", "text/html; charset=utf-8")
        }
        "/styles.css" | "/dashboard/styles.css" => ("styles.css", "text/css; charset=utf-8"),
        "/graph.css" | "/dashboard/graph.css" => ("graph.css", "text/css; charset=utf-8"),
        "/app.js" | "/dashboard/app.js" => ("app.js", "text/javascript; charset=utf-8"),
        "/graph.js" | "/dashboard/graph.js" => ("graph.js", "text/javascript; charset=utf-8"),
        "/core1-proof.js" | "/dashboard/core1-proof.js" => {
            ("core1-proof.js", "text/javascript; charset=utf-8")
        }
        _ => return None,
    };
    let path = root.join(name);
    let bytes = fs::read(path).ok()?;
    (bytes.len() <= 2 * 1024 * 1024).then_some((bytes, content_type))
}

fn diagnostic_response(error: Error) -> HttpResponse {
    json_response(400, &json!({"diagnostic":error.diagnostic}))
}

fn workspace_error_response(error: Error) -> HttpResponse {
    let status = match error.diagnostic.code.as_str() {
        "E_NOT_FOUND" => 404,
        "E_KERNEL" => 409,
        "E_LIMIT" => 413,
        _ => 500,
    };
    json_response(status, &json!({"diagnostic":error.diagnostic}))
}

struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    fn new(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }
}

fn json_response(status: u16, value: &Json) -> HttpResponse {
    match serde_json::to_vec(value) {
        Ok(body) => HttpResponse::new(status, "application/json; charset=utf-8", body),
        Err(_) => HttpResponse::new(
            500,
            "application/json; charset=utf-8",
            b"{\"diagnostic\":{\"code\":\"E_JSON\",\"message\":\"could not encode response\"}}"
                .to_vec(),
        ),
    }
}

fn bounded_json_response(status: u16, value: &Json) -> HttpResponse {
    match serde_json::to_vec(value) {
        Ok(body) if body.len() <= MAX_RESPONSE_BYTES => {
            HttpResponse::new(status, "application/json; charset=utf-8", body)
        }
        _ => json_response(
            413,
            &json!({"diagnostic":{"code":"E_LIMIT","message":"response exceeds the 8 MiB dashboard limit"}}),
        ),
    }
}

fn write_response(mut stream: TcpStream, response: HttpResponse) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\nCache-Control: no-store\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}
