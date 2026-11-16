//! Deterministic sequential Pulse execution for checked blueprint DAGs.

use super::{CheckedNode, CheckedProgram, ResolvedImplementation};
use crate::{
    contracts::{Endpoint, NodeRecord, Outcome, Profile, Receipt, ReceiptSchema, WireDocument},
    diagnostic::{Error, FaultCode, Result},
    runtime::{self, ExecutionContext, RuntimeHost},
    source::Host,
    value::{Identifier, Value},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

/// Execute a checked finite graph under the supplied invocation context.
/// Invalid input is rejected before creating a host or dispatching an effect;
/// runtime denial, approval, and failures are returned as receipt outcomes.
pub fn run(
    checked: &CheckedProgram,
    inputs: BTreeMap<Identifier, Value>,
    invocation_id: String,
    context: &mut ExecutionContext,
    sink: &mut dyn Write,
) -> Result<Receipt> {
    validate_inputs(&checked.contract.parameters, &inputs)?;

    let principal = context.principal().to_owned();
    let mut host = RuntimeHost::new(context.clone(), sink, "");
    let mut nodes = Vec::new();
    let outcome = execute_graph(checked, &inputs, &mut host, "", &mut nodes);
    let (decisions, effects) = host.take_records();

    let mut receipt = Receipt {
        schema: ReceiptSchema::V03,
        profile: Profile::Slice0,
        invocation_id,
        program: checked.digest.clone(),
        principal,
        observed_time: None,
        nodes,
        decisions,
        effects,
        outcome,
    };
    receipt.normalize()?;
    Ok(receipt)
}

fn validate_inputs(
    parameters: &[crate::contracts::Port],
    inputs: &BTreeMap<Identifier, Value>,
) -> Result<()> {
    if parameters.len() != inputs.len() {
        return Err(Error::new(
            "E_INPUT",
            "invocation must provide exactly the declared input fields",
        ));
    }
    for parameter in parameters {
        let value = inputs.get(&parameter.name).ok_or_else(|| {
            Error::new(
                "E_INPUT",
                format!("missing invocation input {}", parameter.name),
            )
        })?;
        if value.value_type() != parameter.r#type {
            return Err(Error::new(
                "E_INPUT",
                format!("wrong value type for invocation input {}", parameter.name),
            ));
        }
    }
    if inputs
        .keys()
        .any(|name| !parameters.iter().any(|parameter| parameter.name == *name))
    {
        return Err(Error::new(
            "E_INPUT",
            "invocation contains an undeclared input field",
        ));
    }
    Ok(())
}

fn execute_graph(
    checked: &CheckedProgram,
    inputs: &BTreeMap<Identifier, Value>,
    host: &mut dyn Host,
    path_prefix: &str,
    receipt_nodes: &mut Vec<NodeRecord>,
) -> Outcome {
    let mut predecessors: BTreeMap<Identifier, BTreeSet<Identifier>> = checked
        .nodes
        .keys()
        .cloned()
        .map(|node| (node, BTreeSet::new()))
        .collect();
    for connection in &checked.spec.connections {
        if let (Endpoint::Node { node: before, .. }, Endpoint::Node { node: after, .. }) =
            (&connection.from, &connection.to)
            && let Some(dependencies) = predecessors.get_mut(after)
        {
            dependencies.insert(before.clone());
        }
    }
    for edge in &checked.spec.order_edges {
        if let Some(dependencies) = predecessors.get_mut(&edge.after) {
            dependencies.insert(edge.before.clone());
        }
    }

    let mut pending: BTreeSet<_> = checked.nodes.keys().cloned().collect();
    let mut completed = BTreeSet::new();
    let mut values = BTreeMap::new();

    while !pending.is_empty() {
        let next = pending
            .iter()
            .find(|node| {
                predecessors
                    .get(*node)
                    .is_some_and(|dependencies| dependencies.is_subset(&completed))
            })
            .cloned();
        let Some(id) = next else {
            let outcome = failed(
                "checked graph has no ready node",
                FaultCode::BindingUnavailable,
            );
            append_skipped(checked, &pending, path_prefix, &outcome, receipt_nodes);
            return outcome;
        };
        pending.remove(&id);

        let Some(node) = checked.nodes.get(&id) else {
            let outcome = failed(
                "checked graph references a missing node",
                FaultCode::BindingUnavailable,
            );
            append_skipped(checked, &pending, path_prefix, &outcome, receipt_nodes);
            return outcome;
        };
        let path = qualify(path_prefix, id.as_str());
        let arguments = match arguments_for(checked, node, inputs, &values) {
            Ok(arguments) => arguments,
            Err(error) => {
                let outcome = failed(&error.diagnostic.message, FaultCode::BindingUnavailable);
                receipt_nodes.push(NodeRecord {
                    path: path.clone(),
                    memory: node.node.memory.clone(),
                    outcome: outcome.clone(),
                });
                append_skipped(checked, &pending, path_prefix, &outcome, receipt_nodes);
                return outcome;
            }
        };

        host.set_node_path(&path);
        let outcome = match &node.implementation {
            ResolvedImplementation::Source(module) => {
                runtime::outcome(module.evaluate_export(&node.node.export, &arguments, host))
            }
            ResolvedImplementation::Program(program) => {
                let nested_inputs = match input_map(&node.contract.parameters, &arguments) {
                    Ok(inputs) => inputs,
                    Err(error) => {
                        let outcome =
                            failed(&error.diagnostic.message, FaultCode::BindingUnavailable);
                        receipt_nodes.push(NodeRecord {
                            path: path.clone(),
                            memory: node.node.memory.clone(),
                            outcome: outcome.clone(),
                        });
                        append_skipped(checked, &pending, path_prefix, &outcome, receipt_nodes);
                        return outcome;
                    }
                };
                execute_graph(program, &nested_inputs, host, &path, receipt_nodes)
            }
        };

        let outcome = match outcome {
            Outcome::Succeeded { value } if value.value_type() != node.contract.result_type => {
                failed(
                    &format!(
                        "node {} returned {:?}, expected {:?}",
                        id,
                        value.value_type(),
                        node.contract.result_type
                    ),
                    FaultCode::BindingUnavailable,
                )
            }
            outcome => outcome,
        };
        receipt_nodes.push(NodeRecord {
            path,
            memory: node.node.memory.clone(),
            outcome: outcome.clone(),
        });

        match outcome {
            Outcome::Succeeded { value } => {
                values.insert(id.clone(), value);
                completed.insert(id);
            }
            stopping => {
                append_skipped(checked, &pending, path_prefix, &stopping, receipt_nodes);
                return stopping;
            }
        }
    }

    match output_value(checked, inputs, &values) {
        Ok(value) if value.value_type() == checked.spec.output.r#type => {
            Outcome::Succeeded { value }
        }
        Ok(value) => failed(
            &format!(
                "program returned {:?}, expected {:?}",
                value.value_type(),
                checked.spec.output.r#type
            ),
            FaultCode::BindingUnavailable,
        ),
        Err(error) => failed(&error.diagnostic.message, FaultCode::BindingUnavailable),
    }
}

fn arguments_for(
    checked: &CheckedProgram,
    node: &CheckedNode,
    inputs: &BTreeMap<Identifier, Value>,
    values: &BTreeMap<Identifier, Value>,
) -> Result<Vec<Value>> {
    let mut arguments = Vec::with_capacity(node.contract.parameters.len());
    for parameter in &node.contract.parameters {
        let connection = checked
            .spec
            .connections
            .iter()
            .find(|connection| {
                matches!(&connection.to, Endpoint::Node { node: id, port } if id == &node.node.id && port == &parameter.name)
            })
            .ok_or_else(|| {
                Error::new(
                    "E_UNBOUND",
                    format!("checked parameter {}.{} is unbound", node.node.id, parameter.name),
                )
            })?;
        let value = match &connection.from {
            Endpoint::Input { port } => inputs.get(port),
            Endpoint::Node { node, .. } => values.get(node),
            Endpoint::Output { .. } => None,
        }
        .cloned()
        .ok_or_else(|| {
            Error::new(
                "E_RUNTIME_BINDING",
                format!(
                    "connection source for {}.{} is unavailable",
                    node.node.id, parameter.name
                ),
            )
        })?;
        if value.value_type() != parameter.r#type {
            return Err(Error::new(
                "E_RUNTIME_BINDING",
                format!(
                    "runtime value type mismatch at {}.{}",
                    node.node.id, parameter.name
                ),
            ));
        }
        arguments.push(value);
    }
    Ok(arguments)
}

fn output_value(
    checked: &CheckedProgram,
    inputs: &BTreeMap<Identifier, Value>,
    values: &BTreeMap<Identifier, Value>,
) -> Result<Value> {
    let connection = checked
        .spec
        .connections
        .iter()
        .find(|connection| {
            matches!(&connection.to, Endpoint::Output { port } if port == &checked.spec.output.name)
        })
        .ok_or_else(|| Error::new("E_OUTPUT", "checked result connection is missing"))?;
    match &connection.from {
        Endpoint::Input { port } => inputs.get(port).cloned(),
        Endpoint::Node { node, .. } => values.get(node).cloned(),
        Endpoint::Output { .. } => None,
    }
    .ok_or_else(|| Error::new("E_RUNTIME_BINDING", "program result source is unavailable"))
}

fn input_map(
    ports: &[crate::contracts::Port],
    values: &[Value],
) -> Result<BTreeMap<Identifier, Value>> {
    if ports.len() != values.len() {
        return Err(Error::new(
            "E_RUNTIME_BINDING",
            "nested program argument count disagrees with its checked contract",
        ));
    }
    let mut inputs = BTreeMap::new();
    for (port, value) in ports.iter().zip(values.iter()) {
        if port.r#type != value.value_type() {
            return Err(Error::new(
                "E_RUNTIME_BINDING",
                format!("nested program argument type mismatch for {}", port.name),
            ));
        }
        inputs.insert(port.name.clone(), value.clone());
    }
    Ok(inputs)
}

fn append_skipped(
    checked: &CheckedProgram,
    pending: &BTreeSet<Identifier>,
    prefix: &str,
    stopping: &Outcome,
    receipt_nodes: &mut Vec<NodeRecord>,
) {
    let reason = match stopping {
        Outcome::Denied { reason }
        | Outcome::AwaitingApproval { reason }
        | Outcome::Skipped { reason }
        | Outcome::Cancelled { reason } => format!("not started after {reason}"),
        Outcome::Failed { message, .. } => format!("not started after {message}"),
        Outcome::Succeeded { .. } => "not started after graph stopped".to_owned(),
    };
    for id in pending {
        if let Some(node) = checked.nodes.get(id) {
            receipt_nodes.push(NodeRecord {
                path: qualify(prefix, id.as_str()),
                memory: node.node.memory.clone(),
                outcome: Outcome::Skipped {
                    reason: reason.clone(),
                },
            });
        }
    }
}

fn qualify(prefix: &str, node: &str) -> String {
    if prefix.is_empty() {
        node.to_owned()
    } else {
        format!("{prefix}/{node}")
    }
}

fn failed(message: &str, fault: FaultCode) -> Outcome {
    Outcome::Failed {
        fault,
        message: message.to_owned(),
    }
}
