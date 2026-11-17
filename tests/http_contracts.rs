#![cfg(feature = "http-client")]

use nexa::{
    contracts::{
        ApplicationManifest, ApplicationSchema, CapabilityRequest, Expectation, ExpectedStatus,
        FixtureCase, FixtureSuite, FixturesSchema, PolicyDecision, PolicyRule, WireDocument,
    },
    kernel::KernelDescriptor,
    source,
    value::{Value, ValueType},
};
use std::collections::BTreeMap;

#[test]
fn bundled_http_extension_registers_an_effectful_get_action() {
    let descriptor = KernelDescriptor::bundled().unwrap();
    assert_eq!(descriptor.profile, nexa::contracts::Profile::Slice0);
    assert_eq!(descriptor.extensions, vec!["http-client/0.1"]);
    assert_eq!(descriptor.operations.len(), 3);

    let operation = descriptor
        .operations
        .iter()
        .find(|operation| operation.name == "std.http.get")
        .unwrap();
    assert_eq!(
        operation.contract.kind,
        nexa::contracts::CallableKind::Action
    );
    assert_eq!(operation.contract.export_name.as_str(), "get");
    assert_eq!(operation.contract.parameters.len(), 1);
    assert_eq!(operation.contract.parameters[0].name.as_str(), "url");
    assert_eq!(operation.contract.parameters[0].r#type, ValueType::String);
    assert_eq!(operation.contract.result_type, ValueType::String);
    assert_eq!(operation.contract.effects, vec!["net.http"]);
    assert_eq!(operation.contract.required_bindings, vec!["std.http.get"]);
    assert_eq!(
        operation.contract.capability_requirements,
        vec![CapabilityRequest {
            capability: "http.get".into(),
            resource: "url.origin".into(),
        }]
    );

    let mut descriptor = descriptor;
    descriptor
        .extensions
        .extend(["zeta/1".into(), "alpha/1".into()]);
    descriptor.normalize().unwrap();
    assert_eq!(
        descriptor.extensions,
        vec!["alpha/1", "http-client/0.1", "zeta/1"]
    );
    descriptor.extensions.push("http-client/0.1".into());
    assert_eq!(
        descriptor.normalize().unwrap_err().diagnostic.code,
        "E_DUPLICATE"
    );
}

#[test]
fn http_get_source_call_is_effectful_and_rejects_a_pure_caller() {
    let action = r#"nexa 0.3
module example.fetch
export action fetch(url is String) -> String effects {net.http} {
  return std.http.get(url)
}
"#;
    let checked = source::check("fetch.nexa", action).unwrap();
    let contract = checked.contract("fetch").unwrap();
    assert_eq!(contract.kind, nexa::contracts::CallableKind::Action);
    assert_eq!(contract.effects, vec!["net.http"]);
    assert_eq!(contract.required_bindings, vec!["std.http.get"]);
    assert_eq!(contract.capability_requirements[0].resource, "url.origin");

    let pure = r#"nexa 0.3
module example.pure_fetch
export fn fetch(url is String) -> String { return std.http.get(url) }
"#;
    assert_eq!(
        source::check("pure_fetch.nexa", pure)
            .unwrap_err()
            .diagnostic
            .code,
        "E_EFFECT_IN_FN"
    );
}

#[test]
fn application_and_fixture_origins_are_canonicalized_and_scoped() {
    let mut manifest = ApplicationManifest {
        schema: ApplicationSchema::V03,
        principal: "app:test".into(),
        requests: vec![CapabilityRequest {
            capability: "http.get".into(),
            resource: "HTTPS://Example.COM:443/".into(),
        }],
        policies: vec![PolicyRule {
            operation: "std.http.get".into(),
            resource: "http://EXAMPLE.com:80/".into(),
            decision: PolicyDecision::Allow,
        }],
    };
    manifest.normalize().unwrap();
    assert_eq!(manifest.requests[0].resource, "https://example.com");
    assert_eq!(manifest.policies[0].resource, "http://example.com");

    let mut fixture = FixtureSuite {
        schema: FixturesSchema::V03,
        cases: vec![FixtureCase {
            name: "fetch-origin".into(),
            input: BTreeMap::new(),
            allow: vec!["HTTPS://Example.COM:443/".into(), "stdout".into()],
            expect: Expectation {
                stdout: String::new(),
                result: Value::String(String::new()),
                status: ExpectedStatus::Succeeded,
                exit_code: 0,
            },
        }],
    };
    fixture.normalize().unwrap();
    assert_eq!(
        fixture.cases[0].allow,
        vec!["https://example.com", "stdout"]
    );
}

#[test]
fn invalid_and_duplicate_http_origins_are_rejected_after_canonicalization() {
    let mut invalid = ApplicationManifest {
        schema: ApplicationSchema::V03,
        principal: "app:test".into(),
        requests: vec![CapabilityRequest {
            capability: "http.get".into(),
            resource: "https://example.com/private".into(),
        }],
        policies: vec![],
    };
    assert_eq!(
        invalid.normalize().unwrap_err().diagnostic.code,
        "E_HTTP_URL"
    );

    let mut unsupported = ApplicationManifest {
        schema: ApplicationSchema::V03,
        principal: "app:test".into(),
        requests: vec![CapabilityRequest {
            capability: "http.post".into(),
            resource: "https://example.com".into(),
        }],
        policies: vec![],
    };
    assert_eq!(
        unsupported.normalize().unwrap_err().diagnostic.code,
        "E_UNSUPPORTED"
    );

    let mut duplicate = ApplicationManifest {
        schema: ApplicationSchema::V03,
        principal: "app:test".into(),
        requests: vec![
            CapabilityRequest {
                capability: "http.get".into(),
                resource: "https://example.com/".into(),
            },
            CapabilityRequest {
                capability: "http.get".into(),
                resource: "HTTPS://EXAMPLE.COM:443".into(),
            },
        ],
        policies: vec![],
    };
    assert_eq!(
        duplicate.normalize().unwrap_err().diagnostic.code,
        "E_DUPLICATE"
    );

    let mut duplicate_fixture = FixtureSuite {
        schema: FixturesSchema::V03,
        cases: vec![FixtureCase {
            name: "duplicate-origin".into(),
            input: BTreeMap::new(),
            allow: vec![
                "https://example.com/".into(),
                "HTTPS://EXAMPLE.COM:443".into(),
            ],
            expect: Expectation {
                stdout: String::new(),
                result: Value::Unit(()),
                status: ExpectedStatus::Succeeded,
                exit_code: 0,
            },
        }],
    };
    assert_eq!(
        duplicate_fixture.normalize().unwrap_err().diagnostic.code,
        "E_DUPLICATE"
    );
}
