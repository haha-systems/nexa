use std::io::{self, Write};

use nexa::{
    contracts::{
        ApplicationManifest, ApplicationSchema, CapabilityRequest, Outcome, PolicyDecision,
        PolicyRule,
    },
    diagnostic::{FaultCode, SourceSpan},
    runtime::{
        E_AUTH_DENIED, E_AWAITING_APPROVAL, E_HOST_ARGUMENT, E_HOST_IO, ExecutionContext,
        RuntimeHost, outcome,
    },
    source::{self, Host},
    value::Value,
};

const OPERATION: &str = "std.console.write_line";

fn rule(decision: PolicyDecision) -> PolicyRule {
    PolicyRule {
        operation: OPERATION.into(),
        resource: "stdout".into(),
        decision,
    }
}

fn manifest(request_stdout: bool, policies: Vec<PolicyRule>) -> ApplicationManifest {
    ApplicationManifest {
        schema: ApplicationSchema::V03,
        principal: "local.hello".into(),
        requests: if request_stdout {
            vec![CapabilityRequest {
                capability: "console.write".into(),
                resource: "stdout".into(),
            }]
        } else {
            vec![]
        },
        policies,
    }
}

fn context(request_stdout: bool, granted: bool, policies: Vec<PolicyRule>) -> ExecutionContext {
    ExecutionContext::from_manifest(
        manifest(request_stdout, policies),
        "local.hello".into(),
        granted,
    )
    .unwrap()
}

fn span() -> SourceSpan {
    SourceSpan {
        source: "write.nexa".into(),
        start: 20,
        end: 52,
    }
}

#[test]
fn allow_requires_manifest_request_host_grant_and_policy() {
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Allow)]),
        &mut bytes,
        "hello.write",
    );

    let result = host.invoke(OPERATION, &[Value::String("Hello, Ada!".into())], &span());

    assert!(matches!(result, Ok(Value::Unit(()))));
    assert_eq!(host.context().principal(), "local.hello");
    assert_eq!(host.node_path(), "hello.write");
    assert_eq!(host.effects().len(), 1);
    assert!(host.effects()[0].dispatched);
    assert_eq!(host.effects()[0].node_path, "hello.write");
    assert!(
        host.decisions()
            .iter()
            .any(|d| d.decision == PolicyDecision::Allow)
    );
    let (decisions, effects) = host.take_records();
    assert_eq!(decisions.len(), 2);
    assert_eq!(effects.len(), 1);
    assert!(host.decisions().is_empty());
    assert!(host.effects().is_empty());
    assert_eq!(bytes, b"Hello, Ada!\n");
}

#[test]
fn policy_composition_is_deny_dominant_and_records_every_rule() {
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(
            true,
            true,
            vec![
                rule(PolicyDecision::Allow),
                rule(PolicyDecision::Ask),
                rule(PolicyDecision::Deny),
            ],
        ),
        &mut bytes,
        "write",
    );

    let result = host.invoke(OPERATION, &[Value::String("blocked".into())], &span());

    let error = result.unwrap_err();
    assert_eq!(error.diagnostic.code, E_AUTH_DENIED);
    assert!(matches!(outcome(Err(error)), Outcome::Denied { .. }));
    assert_eq!(host.decisions().len(), 4); // three rules and the effective result
    assert_eq!(
        host.decisions()
            .iter()
            .map(|d| d.decision)
            .collect::<Vec<_>>(),
        vec![
            PolicyDecision::Allow,
            PolicyDecision::Ask,
            PolicyDecision::Deny,
            PolicyDecision::Deny
        ]
    );
    assert_eq!(host.effects().len(), 1);
    assert!(!host.effects()[0].dispatched);
    assert!(bytes.is_empty());
}

#[test]
fn empty_and_all_abstain_policy_fail_closed() {
    for policies in [vec![], vec![rule(PolicyDecision::Abstain)]] {
        let mut bytes = Vec::new();
        let mut host = RuntimeHost::new(context(true, true, policies), &mut bytes, "write");

        let result = host.invoke(OPERATION, &[Value::String("no".into())], &span());

        assert_eq!(result.unwrap_err().diagnostic.code, E_AUTH_DENIED);
        assert_eq!(
            host.decisions().last().unwrap().decision,
            PolicyDecision::Deny
        );
        assert!(!host.effects()[0].dispatched);
        assert!(bytes.is_empty());
    }
}

#[test]
fn ask_is_recorded_as_awaiting_approval_without_dispatch() {
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Ask)]),
        &mut bytes,
        "write",
    );

    let result = host.invoke(OPERATION, &[Value::String("pending".into())], &span());

    let error = result.unwrap_err();
    assert_eq!(error.diagnostic.code, E_AWAITING_APPROVAL);
    assert!(matches!(
        outcome(Err(error)),
        Outcome::AwaitingApproval { .. }
    ));
    assert_eq!(
        host.decisions().last().unwrap().decision,
        PolicyDecision::Ask
    );
    assert!(!host.effects()[0].dispatched);
    assert!(bytes.is_empty());
}

#[test]
fn missing_host_grant_or_manifest_request_denies_even_when_policy_allows() {
    for (request, grant) in [(true, false), (false, true)] {
        let mut bytes = Vec::new();
        let mut host = RuntimeHost::new(
            context(request, grant, vec![rule(PolicyDecision::Allow)]),
            &mut bytes,
            "write",
        );

        let result = host.invoke(OPERATION, &[Value::String("blocked".into())], &span());

        assert_eq!(result.unwrap_err().diagnostic.code, E_AUTH_DENIED);
        assert_eq!(
            host.decisions().last().unwrap().decision,
            PolicyDecision::Deny
        );
        assert!(!host.effects()[0].dispatched);
        assert!(bytes.is_empty());
    }
}

#[test]
fn malformed_arguments_fail_before_authorization_or_write() {
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Allow)]),
        &mut bytes,
        "write",
    );

    let result = host.invoke(OPERATION, &[Value::Unit(())], &span());

    assert_eq!(result.unwrap_err().diagnostic.code, E_HOST_ARGUMENT);
    assert!(host.decisions().is_empty());
    assert!(host.effects().is_empty());
    assert!(bytes.is_empty());
}

struct BrokenWriter {
    calls: usize,
}

impl Write for BrokenWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        Err(io::Error::other("broken pipe"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PartialThenBrokenWriter {
    calls: usize,
    written: Vec<u8>,
}

impl Write for PartialThenBrokenWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            let count = bytes.len().min(3);
            self.written.extend_from_slice(&bytes[..count]);
            Ok(count)
        } else {
            Err(io::Error::other("broken after partial write"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn writer_failure_is_recorded_as_io_fault_without_retry() {
    let mut writer = BrokenWriter { calls: 0 };
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Allow)]),
        &mut writer,
        "write",
    );

    let result = host.invoke(OPERATION, &[Value::String("partial".into())], &span());

    let error = result.unwrap_err();
    assert_eq!(error.diagnostic.code, E_HOST_IO);
    assert!(matches!(
        outcome(Err(error)),
        Outcome::Failed {
            fault: FaultCode::Io,
            ..
        }
    ));
    assert_eq!(host.effects().len(), 1);
    assert!(host.effects()[0].dispatched);
    assert_eq!(writer.calls, 1);
}

#[test]
fn partial_writer_failure_is_recorded_as_io_fault() {
    let mut writer = PartialThenBrokenWriter {
        calls: 0,
        written: Vec::new(),
    };
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Allow)]),
        &mut writer,
        "write",
    );

    let result = host.invoke(OPERATION, &[Value::String("partial".into())], &span());

    assert!(matches!(
        outcome(result),
        Outcome::Failed {
            fault: FaultCode::Io,
            ..
        }
    ));
    assert_eq!(
        host.decisions().last().unwrap().decision,
        PolicyDecision::Allow
    );
    assert_eq!(host.effects().len(), 1);
    assert!(host.effects()[0].dispatched);
    assert_eq!(writer.written, b"par");
    assert_eq!(writer.calls, 2); // write_all continues only to finish the same byte stream
}

#[test]
fn checked_helper_action_cannot_bypass_or_duplicate_host_authorization() {
    let source_text = r#"
nexa 0.3
module test.runtime

action helper(message is String) -> Unit effects {io.stdout} {
  return std.console.write_line(message)
}

export action send(message is String) -> Unit effects {io.stdout} {
  return helper(message)
}
"#;
    let checked = source::check("helper.nexa", source_text).unwrap();
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Allow)]),
        &mut bytes,
        "hello.write",
    );

    let result = checked.evaluate_with(
        "send",
        &[Value::String("Hello from helper".into())],
        &mut host,
    );

    assert!(matches!(result, Ok(Value::Unit(()))));
    assert_eq!(host.decisions().len(), 2); // rule result plus effective authorization
    assert_eq!(host.effects().len(), 1);
    assert_eq!(host.effects()[0].node_path, "hello.write");
    assert!(host.effects()[0].dispatched);
    assert_eq!(bytes, b"Hello from helper\n");
}

#[test]
fn checked_helper_action_is_denied_before_stdout_when_grant_is_missing() {
    let source_text = r#"
nexa 0.3
module test.runtime

action helper(message is String) -> Unit effects {io.stdout} {
  return std.console.write_line(message)
}

export action send(message is String) -> Unit effects {io.stdout} {
  return helper(message)
}
"#;
    let checked = source::check("helper.nexa", source_text).unwrap();
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(true, false, vec![rule(PolicyDecision::Allow)]),
        &mut bytes,
        "hello.write",
    );

    let result = checked.evaluate_with(
        "send",
        &[Value::String("must not appear".into())],
        &mut host,
    );

    assert_eq!(result.unwrap_err().diagnostic.code, E_AUTH_DENIED);
    assert_eq!(host.effects().len(), 1);
    assert!(!host.effects()[0].dispatched);
    assert_eq!(
        host.decisions().last().unwrap().decision,
        PolicyDecision::Deny
    );
    assert!(bytes.is_empty());
}

#[test]
fn checked_action_rejects_invalid_input_before_host_dispatch() {
    let source_text = r#"
nexa 0.3
module test.runtime

export action send(message is String) -> Unit effects {io.stdout} {
  return std.console.write_line(message)
}
"#;
    let checked = source::check("invalid-input.nexa", source_text).unwrap();
    let mut bytes = Vec::new();
    let mut host = RuntimeHost::new(
        context(true, true, vec![rule(PolicyDecision::Allow)]),
        &mut bytes,
        "hello.write",
    );

    let result = checked.evaluate_with("send", &[Value::Unit(())], &mut host);

    assert_eq!(result.unwrap_err().diagnostic.code, "E_ARGUMENT_TYPE");
    assert!(host.decisions().is_empty());
    assert!(host.effects().is_empty());
    assert!(bytes.is_empty());
}

#[test]
fn context_rejects_manifest_identity_mismatch_and_unsupported_bindings() {
    let mismatch = ExecutionContext::from_manifest(
        manifest(true, vec![]),
        "different.launcher.principal".into(),
        true,
    );
    assert_eq!(mismatch.unwrap_err().diagnostic.code, "E_PRINCIPAL_BINDING");

    let mut unsupported = manifest(false, vec![]);
    unsupported.requests.push(CapabilityRequest {
        capability: "filesystem.read".into(),
        resource: "root".into(),
    });
    let result = ExecutionContext::from_manifest(unsupported, "local.hello".into(), true);
    assert_eq!(result.unwrap_err().diagnostic.code, "E_UNSUPPORTED");
}
