//! The Slice 0 host boundary for checked source actions.
//!
//! A manifest can request authority and constrain the operation policy, but
//! only the launcher supplies the scoped stdout grant. Every console dispatch
//! is checked here, including calls reached through source helper actions.

use std::io::Write;

use crate::{
    contracts::{
        ApplicationManifest, CallableKind, CapabilityRequest, DecisionRecord, EffectRecord,
        Outcome, PolicyDecision, PolicyRule, WireDocument,
    },
    diagnostic::{Error, FaultCode, Result, SourceSpan},
    kernel::KernelDescriptor,
    source::Host,
    value::Value,
};

pub const CONSOLE_WRITE_LINE: &str = "std.console.write_line";
pub const CONSOLE_WRITE: &str = "console.write";
pub const STDOUT: &str = "stdout";

pub const E_AUTH_DENIED: &str = "E_AUTH_DENIED";
pub const E_AWAITING_APPROVAL: &str = "E_AWAITING_APPROVAL";
pub const E_HOST_IO: &str = "E_HOST_IO";
pub const E_HOST_BINDING: &str = "E_HOST_BINDING";
pub const E_HOST_ARGUMENT: &str = "E_HOST_ARGUMENT";
pub const E_HOST_RESULT_TYPE: &str = "E_HOST_RESULT_TYPE";
pub const E_EVALUATION: &str = "E_EVALUATION";
pub const E_PRINCIPAL_BINDING: &str = "E_PRINCIPAL_BINDING";

#[derive(Debug, Clone)]
struct TrustedBinding {
    operation: String,
    capability: String,
    resource: String,
}

/// Immutable authority supplied to one invocation.
///
/// The launcher is responsible for binding `bound_principal` to the current
/// process. The manifest's principal field is a label and must match that
/// binding; it cannot authenticate itself.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    principal: String,
    requests: Vec<CapabilityRequest>,
    policies: Vec<PolicyRule>,
    binding: TrustedBinding,
    host_stdout_granted: bool,
}

impl ExecutionContext {
    /// Validate a decoded manifest and bind it to launch-time authority.
    pub fn from_manifest(
        mut manifest: ApplicationManifest,
        bound_principal: String,
        host_stdout_granted: bool,
    ) -> Result<Self> {
        manifest.normalize()?;
        let binding = trusted_console_binding()?;

        if manifest.principal != bound_principal {
            return Err(Error::new(
                E_PRINCIPAL_BINDING,
                "manifest principal does not match the launcher-bound principal",
            ));
        }

        Ok(Self {
            principal: bound_principal,
            requests: manifest.requests,
            policies: manifest.policies,
            binding,
            host_stdout_granted,
        })
    }

    pub fn principal(&self) -> &str {
        &self.principal
    }

    pub fn host_stdout_granted(&self) -> bool {
        self.host_stdout_granted
    }

    fn requests_stdout_write(&self) -> bool {
        self.requests.iter().any(|request| {
            request.capability == self.binding.capability
                && request.resource == self.binding.resource
        })
    }
}

/// Read the capability/resource mapping from the installed descriptor instead
/// of treating an application manifest as the source of trusted metadata.
fn trusted_console_binding() -> Result<TrustedBinding> {
    let descriptor = KernelDescriptor::bundled()?;
    let operation = descriptor
        .operations
        .iter()
        .find(|operation| operation.name == CONSOLE_WRITE_LINE)
        .ok_or_else(|| {
            Error::new(
                E_HOST_BINDING,
                "bundled kernel does not provide std.console.write_line",
            )
        })?;
    let contract = &operation.contract;
    let valid_effects = contract.effects.len() == 1 && contract.effects[0].as_str() == "io.stdout";
    let valid_binding = contract.required_bindings.len() == 1
        && contract.required_bindings[0].as_str() == CONSOLE_WRITE_LINE;
    let [requirement] = contract.capability_requirements.as_slice() else {
        return Err(Error::new(
            E_HOST_BINDING,
            "console write binding must declare exactly one capability requirement",
        ));
    };

    if contract.kind != CallableKind::Action
        || !valid_effects
        || !valid_binding
        || requirement.capability != CONSOLE_WRITE
        || requirement.resource != STDOUT
    {
        return Err(Error::new(
            E_HOST_BINDING,
            "bundled console write metadata is unsupported by slice0 runtime",
        ));
    }

    Ok(TrustedBinding {
        operation: operation.name.clone(),
        capability: requirement.capability.clone(),
        resource: requirement.resource.clone(),
    })
}

/// A source host that routes the checked console action through authorization
/// and into a caller-provided output sink.
pub struct RuntimeHost<'a> {
    context: ExecutionContext,
    sink: &'a mut dyn Write,
    node_path: String,
    decisions: Vec<DecisionRecord>,
    effects: Vec<EffectRecord>,
}

impl<'a> RuntimeHost<'a> {
    pub fn new(
        context: ExecutionContext,
        sink: &'a mut dyn Write,
        node_path: impl Into<String>,
    ) -> Self {
        Self {
            context,
            sink,
            node_path: node_path.into(),
            decisions: Vec::new(),
            effects: Vec::new(),
        }
    }

    pub fn context(&self) -> &ExecutionContext {
        &self.context
    }

    pub fn node_path(&self) -> &str {
        &self.node_path
    }

    pub fn set_node_path(&mut self, node_path: impl Into<String>) {
        self.node_path = node_path.into();
    }

    pub fn decisions(&self) -> &[DecisionRecord] {
        &self.decisions
    }

    pub fn effects(&self) -> &[EffectRecord] {
        &self.effects
    }

    /// Transfer records to a receipt builder while preserving their historical
    /// order. A graph runner can call this once for each node host.
    pub fn take_records(&mut self) -> (Vec<DecisionRecord>, Vec<EffectRecord>) {
        (
            std::mem::take(&mut self.decisions),
            std::mem::take(&mut self.effects),
        )
    }

    fn authorize_console_write(&mut self) -> (PolicyDecision, String) {
        let operation = self.context.binding.operation.as_str();
        let resource = self.context.binding.resource.as_str();
        let mut policy_decisions = Vec::new();
        for (index, rule) in self.context.policies.iter().enumerate() {
            if rule.operation == operation && rule.resource == resource {
                policy_decisions.push(rule.decision);
                self.decisions.push(DecisionRecord {
                    operation: operation.into(),
                    resource: resource.into(),
                    decision: rule.decision,
                    reason: format!("manifest policy rule {}", index + 1),
                });
            }
        }

        let policy_decision = combine_policy_decisions(&policy_decisions);
        let requested = self.context.requests_stdout_write();
        let granted = self.context.host_stdout_granted;

        // Missing authority is itself a denial. It is combined with the
        // policy result so it dominates Ask and Allow.
        let (decision, reason) = if policy_decision == PolicyDecision::Deny {
            (
                PolicyDecision::Deny,
                "policy denied the operation".to_owned(),
            )
        } else if !requested {
            (
                PolicyDecision::Deny,
                format!(
                    "application did not request {} on {}",
                    self.context.binding.capability, resource
                ),
            )
        } else if !granted {
            (
                PolicyDecision::Deny,
                format!(
                    "launcher did not grant {} on {}",
                    self.context.binding.capability, resource
                ),
            )
        } else {
            match policy_decision {
                PolicyDecision::Allow => (
                    PolicyDecision::Allow,
                    "policy allowed the operation".to_owned(),
                ),
                PolicyDecision::Ask => (
                    PolicyDecision::Ask,
                    "policy requires approval before the operation".to_owned(),
                ),
                PolicyDecision::Abstain | PolicyDecision::Deny => (
                    PolicyDecision::Deny,
                    "no policy allowed the operation".to_owned(),
                ),
            }
        };

        self.decisions.push(DecisionRecord {
            operation: operation.into(),
            resource: resource.into(),
            decision,
            reason: reason.clone(),
        });
        (decision, reason)
    }
}

impl Host for RuntimeHost<'_> {
    fn set_node_path(&mut self, node_path: &str) {
        RuntimeHost::set_node_path(self, node_path);
    }

    fn invoke(&mut self, operation: &str, args: &[Value], call_site: &SourceSpan) -> Result<Value> {
        if operation != self.context.binding.operation.as_str() {
            return Err(error_at(
                E_HOST_BINDING,
                format!("host binding is unavailable for operation {operation}"),
                call_site,
            ));
        }

        let [Value::String(message)] = args else {
            return Err(error_at(
                E_HOST_ARGUMENT,
                "std.console.write_line expects exactly one String argument",
                call_site,
            ));
        };

        let (decision, reason) = self.authorize_console_write();
        let dispatched = decision == PolicyDecision::Allow;
        self.effects.push(EffectRecord {
            node_path: self.node_path.clone(),
            operation: self.context.binding.operation.clone(),
            resource: self.context.binding.resource.clone(),
            dispatched,
        });

        match decision {
            PolicyDecision::Allow => {
                // One host dispatch only. A writer may have emitted a prefix
                // before returning an error; the operation is never retried.
                let mut line = Vec::with_capacity(message.len() + 1);
                line.extend_from_slice(message.as_bytes());
                line.push(b'\n');
                self.sink.write_all(&line).map_err(|error| {
                    error_at(
                        E_HOST_IO,
                        format!("stdout write failed: {error}"),
                        call_site,
                    )
                })?;
                Ok(Value::Unit(()))
            }
            PolicyDecision::Ask => Err(error_at(E_AWAITING_APPROVAL, reason, call_site)),
            PolicyDecision::Deny | PolicyDecision::Abstain => {
                Err(error_at(E_AUTH_DENIED, reason, call_site))
            }
        }
    }
}

fn combine_policy_decisions(decisions: &[PolicyDecision]) -> PolicyDecision {
    if decisions.contains(&PolicyDecision::Deny) {
        PolicyDecision::Deny
    } else if decisions.contains(&PolicyDecision::Ask) {
        PolicyDecision::Ask
    } else if decisions.contains(&PolicyDecision::Allow) {
        PolicyDecision::Allow
    } else {
        PolicyDecision::Abstain
    }
}

fn error_at(code: &str, message: impl Into<String>, span: &SourceSpan) -> Error {
    let mut error = Error::new(code, message);
    error.diagnostic.span = Some(span.clone());
    error
}

/// Convert runtime host errors to the structured outcome used by receipts.
/// Construction/checking errors should be handled before calling this helper.
pub fn outcome(result: Result<Value>) -> Outcome {
    match result {
        Ok(value) => Outcome::Succeeded { value },
        Err(error) => {
            let message = error.diagnostic.message.clone();
            match error.diagnostic.code.as_str() {
                E_AUTH_DENIED => Outcome::Denied { reason: message },
                E_AWAITING_APPROVAL => Outcome::AwaitingApproval { reason: message },
                E_HOST_IO => Outcome::Failed {
                    fault: FaultCode::Io,
                    message,
                },
                E_HOST_BINDING | E_HOST_ARGUMENT | E_HOST_RESULT_TYPE => Outcome::Failed {
                    fault: FaultCode::BindingUnavailable,
                    message,
                },
                E_EVALUATION => Outcome::Failed {
                    fault: FaultCode::IntegrityMismatch,
                    message,
                },
                _ => Outcome::Failed {
                    fault: FaultCode::BindingUnavailable,
                    message,
                },
            }
        }
    }
}
