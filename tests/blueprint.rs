use nexa::{
    blueprint, canonical,
    contracts::{BlueprintDraft, Endpoint, LockRecord, OrderEdge, ProgramSpec},
    memory::{Store, seal_source},
    value::Identifier,
};
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/hello")
        .join(name)
}

fn setup() -> (TempDir, Store, BlueprintDraft, LockRecord) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("store")).unwrap();
    for name in ["trim", "greet", "write"] {
        seal_source(fixture(&format!("memories/{name}")), &store).unwrap();
    }

    let source = std::fs::read_to_string(fixture("hello.nexa")).unwrap();
    let draft = blueprint::parse_source("hello.nexa", &source).unwrap();
    let requests = draft
        .nodes
        .iter()
        .map(|node| (node.selector.clone(), node.export.clone()))
        .collect::<Vec<_>>();
    let lock = store.resolve_requests(&requests).unwrap();
    (temp, store, draft, lock)
}

fn id(name: &str) -> Identifier {
    Identifier::new(name).unwrap()
}

fn spec_digest(spec: &ProgramSpec) -> nexa::digest::Digest {
    canonical::digest(&serde_json::to_value(spec).unwrap()).unwrap()
}

#[test]
fn source_and_request_compose_to_one_spec_and_graph_order_is_canonical() {
    let (_temp, store, draft, lock) = setup();
    let source_spec = blueprint::compose(&draft, &lock, &store).unwrap();
    let request =
        blueprint::decode_request(include_bytes!("../examples/hello/hello.compose.json")).unwrap();
    let request_spec = blueprint::compose(&request, &lock, &store).unwrap();
    assert_eq!(spec_digest(&source_spec), spec_digest(&request_spec));

    let mut reordered = draft;
    reordered.nodes.reverse();
    reordered.connections.reverse();
    reordered.order_edges.reverse();
    let reordered_spec = blueprint::compose(&reordered, &lock, &store).unwrap();
    assert_eq!(spec_digest(&source_spec), spec_digest(&reordered_spec));

    let checked = blueprint::load_checked(&source_spec, &store).unwrap();
    let first: JsonValue = blueprint::inspect_page(&checked, None, 0, 2).unwrap();
    assert_eq!(first["node_count"], 3);
    assert_eq!(first["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(first["next_offset"], 2);
    assert_eq!(first["has_more"], true);
    let second: JsonValue = blueprint::inspect_page(&checked, None, 2, 2).unwrap();
    assert_eq!(second["nodes"][0]["path"], "write");
    assert_eq!(second["has_more"], false);
}

#[test]
fn invalid_ports_types_missing_bindings_and_cycles_are_rejected() {
    let (_temp, store, draft, lock) = setup();

    let mut wrong_type = draft.clone();
    let edge = wrong_type
        .connections
        .iter_mut()
        .find(|edge| matches!(&edge.to, Endpoint::Node { node, .. } if node.as_str() == "greeting"))
        .unwrap();
    edge.from = Endpoint::Node {
        node: id("write"),
        port: id("result"),
    };
    assert_eq!(
        blueprint::compose(&wrong_type, &lock, &store)
            .unwrap_err()
            .diagnostic
            .code,
        "E_PORT_TYPE"
    );

    let mut missing_port = draft.clone();
    let edge = missing_port
        .connections
        .iter_mut()
        .find(|edge| matches!(&edge.to, Endpoint::Node { node, .. } if node.as_str() == "greeting"))
        .unwrap();
    edge.to = Endpoint::Node {
        node: id("greeting"),
        port: id("missing"),
    };
    assert_eq!(
        blueprint::compose(&missing_port, &lock, &store)
            .unwrap_err()
            .diagnostic
            .code,
        "E_PORT"
    );

    let mut unbound = draft.clone();
    unbound.connections.retain(|edge| {
        !matches!(
            (&edge.from, &edge.to),
            (
                Endpoint::Input { port: input },
                Endpoint::Node { node, port }
            ) if input.as_str() == "name" && node.as_str() == "clean" && port.as_str() == "name"
        )
    });
    assert_eq!(
        blueprint::compose(&unbound, &lock, &store)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNBOUND"
    );

    let mut cyclic = draft;
    cyclic.order_edges.push(OrderEdge {
        before: id("greeting"),
        after: id("clean"),
    });
    assert_eq!(
        blueprint::compose(&cyclic, &lock, &store)
            .unwrap_err()
            .diagnostic
            .code,
        "E_CYCLE"
    );
}

#[test]
fn checker_rejects_a_forged_program_effect_summary() {
    let (_temp, store, draft, lock) = setup();
    let mut spec = blueprint::compose(&draft, &lock, &store).unwrap();
    spec.derived.effects.clear();
    let error = blueprint::load_checked(&spec, &store)
        .err()
        .expect("forged derived effect summary is rejected");
    assert_eq!(error.diagnostic.code, "E_DERIVED");
}
