use nexa::{core1, digest::Digest, memory::Store};
use serde_json::{Value, json};
use std::path::Path;

fn seal(store: &Store, name: &str, imports: &[(&str, &Digest)]) -> Digest {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/native-http")
        .join(format!("{name}.nexa"));
    let text = std::fs::read_to_string(&path).unwrap();
    let imports = imports
        .iter()
        .map(|(alias, digest)| (alias.to_string(), (*digest).clone()))
        .collect();
    core1::seal(store, path.to_str().unwrap(), &text, imports)
        .unwrap_or_else(|error| panic!("seal {name}: {error}"))
}

fn assemble(store: &Store, decimal_name: &str) -> Digest {
    let decimal = seal(store, decimal_name, &[]);
    let headers = seal(store, "headers", &[("decimal", &decimal)]);
    let request = seal(store, "request", &[("decimal", &decimal)]);
    let response = seal(store, "response", &[("headers", &headers)]);
    let stream = seal(store, "response_stream", &[("response", &response)]);
    let result = seal(store, "exchange_result", &[("response", &response)]);
    seal(
        store,
        "exchange",
        &[
            ("request", &request),
            ("stream", &stream),
            ("result", &result),
        ],
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn input(chunks: &[&[u8]], eof: bool) -> Value {
    json!({
        "authority": hex(b"127.0.0.1:8080"),
        "target": hex(b"/message?lang=en"),
        "chunks": chunks.iter().map(|bytes| hex(bytes)).collect::<Vec<_>>(),
        "eof": eof,
    })
}

#[test]
fn exchange_composes_locked_codecs_and_preserves_binary_response_at_every_split() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let digest = assemble(&store, "decimal");
    let records = core1::inspect_closure(&store, &digest).unwrap();
    assert_eq!(records.len(), 7);
    assert_eq!(
        records[&digest].imports.keys().collect::<Vec<_>>(),
        ["request", "result", "stream"]
    );

    let message = b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nX-Test: one\r\nX-Test: two\r\n\r\n\x00\xff\r\nrest";
    let baseline = core1::run(
        &store,
        &digest,
        &input(&[message], true),
        core1::DEFAULT_FUEL,
    )
    .unwrap();
    assert_eq!(baseline["variant"], "complete", "{baseline}");
    let fields = &baseline["fields"];
    assert_eq!(fields["response"]["status"], "404");
    assert_eq!(fields["response"]["body"], "00ff0d0a");
    assert_eq!(fields["remainder"], hex(b"rest"));
    assert_eq!(fields["response"]["headers"].as_array().unwrap().len(), 3);
    assert_eq!(
        fields["request"],
        hex(b"GET /message?lang=en HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nConnection: close\r\n\r\n")
    );

    for split in 0..=message.len() {
        let chunks = [&message[..split], &message[split..]];
        let result = core1::run(&store, &digest, &input(&chunks, true), core1::DEFAULT_FUEL)
            .unwrap_or_else(|error| panic!("split at {split}: {error}"));
        assert_eq!(result, baseline, "split at {split}");
    }

    let replacement = assemble(&store, "decimal-alternate");
    assert_ne!(replacement, digest);
    for subject in [&digest, &replacement] {
        assert_eq!(
            core1::run(
                &store,
                subject,
                &input(&[message], true),
                core1::DEFAULT_FUEL
            )
            .unwrap(),
            baseline
        );
    }
}

#[test]
fn checked_in_exchange_fixtures_match() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let digest = assemble(&store, "decimal");
    let fixtures: Value =
        serde_json::from_str(include_str!("../examples/native-http/fixtures.json")).unwrap();
    for case in fixtures["cases"].as_array().expect("fixture cases") {
        let name = case["name"].as_str().expect("fixture name");
        let result = core1::run(&store, &digest, &case["input"], core1::DEFAULT_FUEL)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(result, case["expected"], "{name}");
    }
}
