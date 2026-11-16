use nexa::{
    canonical,
    contracts::{self, WireDocument},
    digest::Digest,
    kernel::KernelDescriptor,
    schemas,
    value::{Identifier, Value, ValueType},
};
use serde_json::json;

fn draft() -> serde_json::Value {
    canonical::parse(include_bytes!("../examples/hello/hello.compose.json")).unwrap()
}

fn normalized(value: serde_json::Value) -> nexa::diagnostic::Result<serde_json::Value> {
    contracts::normalized_json(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn canonical_bytes_have_sorted_keys_exact_escaping_and_no_newline() {
    let value =
        canonical::parse(br#" { "z": [1, -2, true, null], "a": "Zo\u00eb\n\t\u0001\\\"" } "#)
            .unwrap();
    assert_eq!(
        canonical::encode(&value).unwrap(),
        "{\"a\":\"Zoë\\n\\t\\u0001\\\\\\\"\",\"z\":[1,-2,true,null]}".as_bytes()
    );
}

#[test]
fn json_key_order_whitespace_and_unicode_escapes_do_not_change_identity() {
    let a = canonical::parse(br#"{"word":"Zo\u00eb", "n":1}"#).unwrap();
    let b = canonical::parse("{ \"n\": 1, \"word\": \"Zoë\" }".as_bytes()).unwrap();
    assert_eq!(
        canonical::digest(&a).unwrap(),
        canonical::digest(&b).unwrap()
    );
}

#[test]
fn unicode_is_not_normalized_and_generic_array_order_is_preserved() {
    assert_ne!(
        canonical::digest(&json!({"v":"é"})).unwrap(),
        canonical::digest(&json!({"v":"e\u{301}"})).unwrap()
    );
    assert_ne!(
        canonical::digest(&json!([1, 2])).unwrap(),
        canonical::digest(&json!([2, 1])).unwrap()
    );
}

#[test]
fn duplicate_keys_are_rejected_even_when_nested_or_escaped() {
    for bytes in [
        br#"{"a":1,"a":2}"#.as_slice(),
        br#"{"n":[{"a":1,"\u0061":2}]}"#,
    ] {
        let error = canonical::parse(bytes).unwrap_err();
        assert!(error.to_string().contains("duplicate key"));
    }
}

#[test]
fn invalid_numbers_and_non_json_input_fail() {
    for bytes in [
        "-0",
        "0.0",
        "1e0",
        "9223372036854775808",
        "-9223372036854775809",
        "01",
        "NaN",
        "true false",
        "{",
        "\"\\uD800\"",
    ] {
        assert!(
            canonical::parse(bytes.as_bytes()).is_err(),
            "accepted {bytes}"
        );
    }
    for bytes in ["0", "-9223372036854775808", "9223372036854775807"] {
        assert!(
            canonical::parse(bytes.as_bytes()).is_ok(),
            "rejected {bytes}"
        );
    }
    assert!(canonical::parse(&[b'"', 0xff, b'"']).is_err());
}

#[test]
fn in_memory_numbers_and_keys_obey_the_same_rules() {
    assert!(canonical::encode(&json!({"v": 1.25})).is_err());
    assert!(canonical::encode(&json!({"v": u64::MAX})).is_err());
    assert!(canonical::parse("{\"é\":1}".as_bytes()).is_err());
    assert!(canonical::parse(br#"{"bad-key":1}"#).is_err());
}

#[test]
fn raw_digest_matches_sha256_known_vector_and_detects_tampering() {
    let digest = Digest::of_bytes(b"abc");
    assert_eq!(
        digest.as_str(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    digest.verify(b"abc").unwrap();
    assert!(digest.verify(b"abd").is_err());
    assert_ne!(Digest::of_bytes(b"abc"), Digest::of_bytes(b"abc\n"));
}

#[test]
fn malformed_digest_never_enters_a_typed_contract() {
    for value in ["sha256:abc", "sha256:ZZZZ", "SHA256:abc", "sha256:é", ""] {
        assert!(value.parse::<Digest>().is_err());
        assert!(serde_json::from_value::<Digest>(json!(value)).is_err());
    }
}

#[test]
fn typed_unit_is_explicit_and_different_from_an_empty_string() {
    let unit = serde_json::to_value(Value::Unit(())).unwrap();
    assert_eq!(unit, json!({"type":"Unit", "value":null}));
    assert_eq!(Value::Unit(()).value_type(), ValueType::Unit);
    assert_eq!(Value::String(String::new()).value_type(), ValueType::String);
    assert!(serde_json::from_value::<Value>(json!({"type":"Unit","value":""})).is_err());
    assert!(serde_json::from_value::<Value>(json!({"type":"Unit"})).is_err());
    assert!(serde_json::from_value::<Value>(json!({"type":"String","value":null})).is_err());
    assert!(Identifier::new("input").is_err());
    assert!(Identifier::new("name").is_ok());
}

#[test]
fn blueprint_array_order_is_nonsemantic_but_connections_are_semantic() {
    let original = draft();
    let mut reordered = original.clone();
    reordered["nodes"].as_array_mut().unwrap().reverse();
    reordered["connections"].as_array_mut().unwrap().reverse();
    let a = normalized(original).unwrap();
    let b = normalized(reordered).unwrap();
    assert_eq!(
        canonical::digest(&a).unwrap(),
        canonical::digest(&b).unwrap()
    );
    let mut changed = draft();
    changed["nodes"][0]["selector"] = json!("local.text.trim@2.0.0");
    assert_ne!(
        canonical::digest(&a).unwrap(),
        canonical::digest(&normalized(changed).unwrap()).unwrap()
    );
}

#[test]
fn duplicate_node_ids_are_rejected_without_silent_deduplication() {
    let mut value = draft();
    value["nodes"][1]["id"] = value["nodes"][0]["id"].clone();
    assert_eq!(
        normalized(value).unwrap_err().diagnostic.code,
        "E_DUPLICATE"
    );
}

#[test]
fn duplicates_or_multiple_producers_are_rejected() {
    let mut value = draft();
    let duplicate = value["connections"][0].clone();
    value["connections"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(
        normalized(value).unwrap_err().diagnostic.code,
        "E_DUPLICATE"
    );
    let mut value = draft();
    value["connections"][0]["to"] = value["connections"][1]["to"].clone();
    assert_eq!(normalized(value).unwrap_err().diagnostic.code, "E_PRODUCER");
}

#[test]
fn unknown_fields_versions_types_and_unsupported_work_fail_closed() {
    let mut value = draft();
    value["ignored"] = json!(true);
    assert!(normalized(value).is_err());
    let mut value = draft();
    value["profile"] = json!("ai-agent");
    assert!(normalized(value).is_err());
    let mut value = draft();
    value["language"] = json!("0.2");
    assert!(normalized(value).is_err());
    let mut value = draft();
    value["inputs"][0]["type"] = json!("Any");
    assert!(normalized(value).is_err());
    let mut value = draft();
    value["requirements"] = json!([{"id":"unimplemented"}]);
    assert!(normalized(value).is_err());
}

#[test]
fn known_shape_errors_are_reported_but_resolution_is_not_fabricated() {
    let mut value = draft();
    value["connections"][0]["from"]["port"] = json!("missing");
    assert_eq!(normalized(value).unwrap_err().diagnostic.code, "E_ENDPOINT");
    let mut value = draft();
    value["connections"].as_array_mut().unwrap().pop();
    assert_eq!(normalized(value).unwrap_err().diagnostic.code, "E_OUTPUT");
    let mut value = draft();
    value["connections"][0]["to"]["port"] = json!("unresolved_parameter");
    // This stage has no memory resolver, so it cannot establish callable ports.
    assert!(normalized(value).is_ok());
}

#[test]
fn manifest_paths_cannot_escape_or_alias_the_source_directory() {
    let value = canonical::parse(include_bytes!(
        "../examples/hello/memories/trim/memory.json"
    ))
    .unwrap();
    for path in [
        "../outside.nexa",
        "/tmp/x.nexa",
        "src/../x.nexa",
        "src//x.nexa",
        "src/./x.nexa",
        "C:\\x.nexa",
    ] {
        let mut changed = value.clone();
        changed["sources"] = json!([path]);
        assert_eq!(normalized(changed).unwrap_err().diagnostic.code, "E_PATH");
    }
}

#[test]
fn application_manifest_cannot_request_unsupported_resources() {
    let mut value = canonical::parse(include_bytes!("../examples/hello/hello.app.json")).unwrap();
    value["requests"][0]["resource"] = json!("filesystem");
    assert_eq!(
        normalized(value).unwrap_err().diagnostic.code,
        "E_UNSUPPORTED"
    );
}

#[test]
fn agreed_json_fixtures_fit_the_wire_contracts() {
    for bytes in [
        include_bytes!("../examples/hello/hello.compose.json").as_slice(),
        include_bytes!("../examples/hello/hello.app.json").as_slice(),
        include_bytes!("../examples/hello/fixtures.json").as_slice(),
        include_bytes!("../examples/hello/memories/trim/memory.json").as_slice(),
    ] {
        contracts::normalized_json(bytes).unwrap();
    }
}

#[test]
fn kernel_identity_tracks_abi_and_implementation() {
    let original = KernelDescriptor::bundled().unwrap();
    let hash = |descriptor| canonical::digest(&serde_json::to_value(descriptor).unwrap()).unwrap();
    let baseline = hash(&original);
    let mut changed = original.clone();
    changed.abi.push_str("-changed");
    assert_ne!(baseline, hash(&changed));
    let mut changed = original.clone();
    changed.implementation = Digest::of_bytes(b"other implementation");
    assert_ne!(baseline, hash(&changed));
    let mut changed = original.clone();
    changed.operations.reverse();
    changed.normalize().unwrap();
    assert_eq!(baseline, hash(&changed));
    let mut invalid = original;
    invalid.operations[0].contract.kind = contracts::CallableKind::Function;
    assert!(invalid.normalize().is_err());
}

#[test]
fn generated_schemas_match_the_committed_contracts() {
    for name in schemas::NAMES {
        let path = format!(
            "{}/schemas/{name}-0.3.schema.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).expect("generate schemas before testing"))
                .unwrap();
        assert_eq!(saved, schemas::named(name).unwrap(), "schema drift: {name}");
        assert_eq!(
            saved["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }
}

#[test]
fn outcomes_have_the_specified_exit_codes() {
    use contracts::Outcome;
    let cases = [
        (
            Outcome::Succeeded {
                value: Value::Unit(()),
            },
            0,
        ),
        (
            Outcome::Failed {
                fault: nexa::diagnostic::FaultCode::Io,
                message: "broken output".into(),
            },
            1,
        ),
        (
            Outcome::Denied {
                reason: "missing permission".into(),
            },
            3,
        ),
        (
            Outcome::AwaitingApproval {
                reason: "approval needed".into(),
            },
            4,
        ),
        (
            Outcome::Skipped {
                reason: "guard false".into(),
            },
            5,
        ),
        (
            Outcome::Cancelled {
                reason: "cancelled".into(),
            },
            5,
        ),
    ];
    for (outcome, expected) in cases {
        assert_eq!(outcome.exit_code() as u8, expected);
    }
}
