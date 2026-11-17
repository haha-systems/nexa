//! The Slice 0 host boundary for checked source actions.
//!
//! A manifest can request authority and constrain the operation policy, but
//! only the launcher supplies scoped resource grants. Every protected dispatch
//! is checked here, including calls reached through source helper actions.

use std::{collections::BTreeSet, io::Write};

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
#[cfg(feature = "http-client")]
pub const HTTP_GET: &str = "std.http.get";
#[cfg(feature = "http-client")]
pub const HTTP_GET_CAPABILITY: &str = "http.get";

pub const E_AUTH_DENIED: &str = "E_AUTH_DENIED";
pub const E_AWAITING_APPROVAL: &str = "E_AWAITING_APPROVAL";
pub const E_HOST_IO: &str = "E_HOST_IO";
pub const E_HOST_BINDING: &str = "E_HOST_BINDING";
pub const E_HOST_ARGUMENT: &str = "E_HOST_ARGUMENT";
pub const E_HOST_RESULT_TYPE: &str = "E_HOST_RESULT_TYPE";
pub const E_EVALUATION: &str = "E_EVALUATION";
pub const E_PRINCIPAL_BINDING: &str = "E_PRINCIPAL_BINDING";
#[cfg(feature = "http-client")]
pub const E_HTTP_URL: &str = "E_HTTP_URL";
#[cfg(feature = "http-client")]
pub const E_HTTP_STATUS: &str = "E_HTTP_STATUS";
#[cfg(feature = "http-client")]
pub const E_HTTP_TIMEOUT: &str = "E_HTTP_TIMEOUT";
#[cfg(feature = "http-client")]
pub const E_HTTP_BODY_LIMIT: &str = "E_HTTP_BODY_LIMIT";
#[cfg(feature = "http-client")]
pub const E_HTTP_UTF8: &str = "E_HTTP_UTF8";
#[cfg(feature = "http-client")]
pub const E_HTTP_TRANSPORT: &str = "E_HTTP_TRANSPORT";

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
    host_resources: BTreeSet<String>,
    #[cfg(feature = "http-client")]
    http_binding: TrustedBinding,
}

impl ExecutionContext {
    /// Validate a decoded manifest and bind it to launch-time authority.
    pub fn from_manifest(
        manifest: ApplicationManifest,
        bound_principal: String,
        host_stdout_granted: bool,
    ) -> Result<Self> {
        let contains_http_rules = manifest
            .requests
            .iter()
            .any(|request| request.capability == "http.get")
            || manifest
                .policies
                .iter()
                .any(|rule| rule.operation == "std.http.get");
        let resources = if host_stdout_granted {
            vec![STDOUT.to_owned()]
        } else {
            Vec::new()
        };
        let context = Self::from_manifest_with_resources(manifest, bound_principal, &resources)?;

        if contains_http_rules {
            return Err(Error::new(
                "E_UNSUPPORTED",
                "the default execution context does not grant network resources",
            ));
        }

        Ok(context)
    }

    /// Validate a manifest and bind exactly the launcher's scoped resources.
    /// `stdout` is the only non-network resource in this profile. With the
    /// `http-client` feature, other accepted resources are canonical origins.
    pub fn from_manifest_with_resources(
        mut manifest: ApplicationManifest,
        bound_principal: String,
        resources: &[String],
    ) -> Result<Self> {
        manifest.normalize()?;

        if manifest.principal != bound_principal {
            return Err(Error::new(
                E_PRINCIPAL_BINDING,
                "manifest principal does not match the launcher-bound principal",
            ));
        }

        let binding = trusted_console_binding()?;
        #[cfg(feature = "http-client")]
        let http_binding = trusted_http_binding()?;

        let mut host_resources = BTreeSet::new();
        for resource in resources {
            let canonical = canonical_host_resource(resource)?;
            if !host_resources.insert(canonical) {
                return Err(Error::new(
                    "E_DUPLICATE",
                    format!("duplicate host resource grant: {resource}"),
                ));
            }
        }

        Ok(Self {
            principal: bound_principal,
            requests: manifest.requests,
            policies: manifest.policies,
            binding,
            host_resources,
            #[cfg(feature = "http-client")]
            http_binding,
        })
    }

    pub fn principal(&self) -> &str {
        &self.principal
    }

    pub fn host_stdout_granted(&self) -> bool {
        self.host_resources.contains(STDOUT)
    }

    fn host_resource_granted(&self, resource: &str) -> bool {
        self.host_resources.contains(resource)
    }

    fn requests(&self, capability: &str, resource: &str) -> bool {
        self.requests
            .iter()
            .any(|request| request.capability == capability && request.resource == resource)
    }
}

fn canonical_host_resource(resource: &str) -> Result<String> {
    if resource == STDOUT {
        return Ok(STDOUT.to_owned());
    }

    #[cfg(feature = "http-client")]
    {
        crate::http_client::canonical_origin_grant(resource).map_err(|error| {
            Error::new(
                "E_UNSUPPORTED",
                format!(
                    "unsupported host resource grant {resource}: {}",
                    error.diagnostic.message
                ),
            )
        })
    }

    #[cfg(not(feature = "http-client"))]
    {
        Err(Error::new(
            "E_UNSUPPORTED",
            format!("unsupported host resource grant: {resource}"),
        ))
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

#[cfg(feature = "http-client")]
fn trusted_http_binding() -> Result<TrustedBinding> {
    let descriptor = KernelDescriptor::bundled()?;
    let operation = descriptor
        .operations
        .iter()
        .find(|operation| operation.name == HTTP_GET)
        .ok_or_else(|| Error::new(E_HOST_BINDING, "bundled kernel has no HTTP GET operation"))?;
    let contract = &operation.contract;
    let valid_effects = contract.effects.len() == 1 && contract.effects[0].as_str() == "net.http";
    let valid_binding =
        contract.required_bindings.len() == 1 && contract.required_bindings[0].as_str() == HTTP_GET;
    let [requirement] = contract.capability_requirements.as_slice() else {
        return Err(Error::new(
            E_HOST_BINDING,
            "HTTP GET binding must declare exactly one capability requirement",
        ));
    };

    if contract.kind != CallableKind::Action
        || !valid_effects
        || !valid_binding
        || requirement.capability != HTTP_GET_CAPABILITY
        || requirement.resource != "url.origin"
    {
        return Err(Error::new(
            E_HOST_BINDING,
            "bundled HTTP GET metadata is unsupported by the HTTP client runtime",
        ));
    }

    Ok(TrustedBinding {
        operation: operation.name.clone(),
        capability: requirement.capability.clone(),
        resource: requirement.resource.clone(),
    })
}

/// A source host that authorizes protected actions and routes console output
/// into a caller-provided sink.
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

    fn authorize(
        &mut self,
        operation: &str,
        resource: &str,
        capability: &str,
    ) -> (PolicyDecision, String) {
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
        let requested = self.context.requests(capability, resource);
        let granted = self.context.host_resource_granted(resource);

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
                format!("application did not request {capability} on {resource}"),
            )
        } else if !granted {
            (
                PolicyDecision::Deny,
                format!("launcher did not grant {capability} on {resource}"),
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

    fn invoke_console(&mut self, args: &[Value], call_site: &SourceSpan) -> Result<Value> {
        let [Value::String(message)] = args else {
            return Err(error_at(
                E_HOST_ARGUMENT,
                "std.console.write_line expects exactly one String argument",
                call_site,
            ));
        };

        let operation = self.context.binding.operation.clone();
        let resource = self.context.binding.resource.clone();
        let capability = self.context.binding.capability.clone();
        let (decision, reason) = self.authorize(&operation, &resource, &capability);
        self.effects.push(EffectRecord {
            node_path: self.node_path.clone(),
            operation,
            resource,
            dispatched: decision == PolicyDecision::Allow,
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

    #[cfg(feature = "http-client")]
    fn invoke_http_get(&mut self, args: &[Value], call_site: &SourceSpan) -> Result<Value> {
        let [Value::String(url)] = args else {
            return Err(error_at(
                E_HOST_ARGUMENT,
                "std.http.get expects exactly one String URL argument",
                call_site,
            ));
        };

        // Parsing validates URL structure without resolving names or opening a
        // connection. The concrete origin is the resource used for all checks.
        let request = crate::http_client::GetRequest::parse(url)
            .map_err(|error| with_call_site(error, call_site))?;
        let origin = request.origin().to_owned();
        let operation = self.context.http_binding.operation.clone();
        let capability = self.context.http_binding.capability.clone();
        let (decision, reason) = self.authorize(&operation, &origin, &capability);
        self.effects.push(EffectRecord {
            node_path: self.node_path.clone(),
            operation,
            resource: origin,
            dispatched: decision == PolicyDecision::Allow,
        });

        match decision {
            PolicyDecision::Allow => request
                .execute(crate::http_client::HttpLimits::default())
                .map(Value::String)
                .map_err(|error| with_call_site(error, call_site)),
            PolicyDecision::Ask => Err(error_at(E_AWAITING_APPROVAL, reason, call_site)),
            PolicyDecision::Deny | PolicyDecision::Abstain => {
                Err(error_at(E_AUTH_DENIED, reason, call_site))
            }
        }
    }
}

impl Host for RuntimeHost<'_> {
    fn set_node_path(&mut self, node_path: &str) {
        RuntimeHost::set_node_path(self, node_path);
    }

    fn invoke(&mut self, operation: &str, args: &[Value], call_site: &SourceSpan) -> Result<Value> {
        if operation == self.context.binding.operation.as_str() {
            return self.invoke_console(args, call_site);
        }

        #[cfg(feature = "http-client")]
        if operation == self.context.http_binding.operation.as_str() {
            return self.invoke_http_get(args, call_site);
        }

        Err(error_at(
            E_HOST_BINDING,
            format!("host binding is unavailable for operation {operation}"),
            call_site,
        ))
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

#[cfg(feature = "http-client")]
fn with_call_site(mut error: Error, span: &SourceSpan) -> Error {
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
                #[cfg(feature = "http-client")]
                E_HTTP_URL => Outcome::Failed {
                    fault: FaultCode::HttpRequest,
                    message,
                },
                #[cfg(feature = "http-client")]
                E_HTTP_STATUS => Outcome::Failed {
                    fault: FaultCode::HttpStatus,
                    message,
                },
                #[cfg(feature = "http-client")]
                E_HTTP_TIMEOUT => Outcome::Failed {
                    fault: FaultCode::HttpTimeout,
                    message,
                },
                #[cfg(feature = "http-client")]
                E_HTTP_BODY_LIMIT => Outcome::Failed {
                    fault: FaultCode::HttpBodyLimit,
                    message,
                },
                #[cfg(feature = "http-client")]
                E_HTTP_UTF8 => Outcome::Failed {
                    fault: FaultCode::HttpUtf8,
                    message,
                },
                #[cfg(feature = "http-client")]
                E_HTTP_TRANSPORT => Outcome::Failed {
                    fault: FaultCode::HttpTransport,
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
