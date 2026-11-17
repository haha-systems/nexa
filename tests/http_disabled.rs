#![cfg(not(feature = "http-client"))]

use nexa::{contracts, kernel::KernelDescriptor, source};

#[test]
fn default_build_has_no_http_binding_effect_or_authority() {
    let kernel = KernelDescriptor::bundled().unwrap();
    assert!(kernel.extensions.is_empty());
    assert_eq!(kernel.operations.len(), 2);
    let source = include_str!("../examples/http-client/memories/fetch/src/main.nexa");
    assert_eq!(
        source::check("fetch.nexa", source)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNKNOWN_EFFECT"
    );
    let manifest = include_bytes!("../examples/http-client/client.app.json");
    assert_eq!(
        contracts::normalized_json(manifest)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNSUPPORTED"
    );
}
