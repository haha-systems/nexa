//! Typed, content-pinned Core1 invocation graphs.
//!
//! Core1 blueprints are separate from Slice 0 blueprints: their wire type
//! strings are derived from checked Core1 signatures, including nominal type
//! identity digests.

mod runner;

use crate::{
    canonical,
    contracts::{EmptyList, Language},
    diagnostic::{Error, Result},
    digest::Digest,
    memory::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use crate::contracts::{Connection, DraftNode, Endpoint, Node, OrderEdge};
pub use crate::value::Identifier;

pub const DRAFT_SCHEMA: &str = "nexa.core1-blueprint-draft/0.1";
pub const PROGRAM_SCHEMA: &str = "nexa.core1-program/0.1";
pub const PROFILE: &str = "core1";
const MAX_NODES: usize = 128;
const MAX_CONNECTIONS: usize = 4096;
const MAX_ORDER_EDGES: usize = 4096;
const MAX_PORTS: usize = 4096;
const MAX_PORT_TYPE_BYTES: usize = 4096;
const MAX_ENTRY_BYTES: usize = 256;

/// A Core1 type is the checked engine's full Display form, including the
/// declaring module and type-identity digest for nominal types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Port {
    pub name: Identifier,
    pub r#type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub schema: String,
    pub language: Language,
    pub profile: String,
    pub entry: String,
    pub inputs: Vec<Port>,
    pub output: Port,
    pub nodes: Vec<DraftNode>,
    pub connections: Vec<Connection>,
    pub order_edges: Vec<OrderEdge>,
    pub settings: EmptyList,
    pub requirements: EmptyList,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub schema: String,
    pub language: Language,
    pub profile: String,
    pub entry: String,
    pub inputs: Vec<Port>,
    pub output: Port,
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
    pub order_edges: Vec<OrderEdge>,
    pub settings: EmptyList,
    pub implementation: Digest,
    pub dependencies: Vec<Digest>,
    pub requirements: EmptyList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallableKind {
    Function,
}

/// The single callable exposed by a Core1 memory, converted from its verified
/// engine signature for dashboard inspection.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Callable {
    pub export_name: Identifier,
    pub kind: CallableKind,
    pub parameters: Vec<Port>,
    pub result_type: String,
    pub effects: Vec<String>,
    pub required_bindings: Vec<String>,
    pub capability_requirements: Vec<crate::contracts::CapabilityRequest>,
}

pub fn callable(memory: &super::Memory) -> Result<Callable> {
    let signature: super::engine::Signature = serde_json::from_value(memory.signature.clone())
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    let (export_name, parameters, result_type) = signature.ports();
    Ok(Callable {
        export_name: Identifier::new(export_name)?,
        kind: CallableKind::Function,
        parameters: parameters
            .into_iter()
            .map(|(name, r#type)| {
                Ok(Port {
                    name: Identifier::new(name)?,
                    r#type,
                })
            })
            .collect::<Result<_>>()?,
        result_type,
        effects: Vec::new(),
        required_bindings: Vec::new(),
        capability_requirements: Vec::new(),
    })
}

/// An immutable checked blueprint. All direct memories and their import
/// closure were loaded and checked before this value was constructed.
pub struct CheckedProgram {
    program: Program,
    subject: Digest,
    nodes: BTreeMap<Identifier, Arc<super::CheckedModule>>,
    order: Vec<Identifier>,
}

impl CheckedProgram {
    pub fn program(&self) -> &Program {
        &self.program
    }

    pub fn subject(&self) -> &Digest {
        &self.subject
    }

    pub fn run(&self, input: &Json, fuel: u64) -> Result<Json> {
        runner::run_verified(self, input, fuel)
    }
}

pub fn compose(store: &Store, draft: Draft) -> Result<CheckedProgram> {
    let draft = normalize_draft(draft)?;
    let implementation: Digest = env!("NEXA_IMPLEMENTATION_DIGEST").parse()?;
    let mut loader = super::Loader::new(store);
    let mut nodes = BTreeMap::new();
    let mut program_nodes = Vec::with_capacity(draft.nodes.len());

    for draft_node in &draft.nodes {
        let memory_digest: Digest = draft_node.selector.parse()?;
        if memory_digest.as_str() != draft_node.selector {
            return Err(Error::new(
                "E_DIGEST",
                "Core1 selectors must be exact digests",
            ));
        }
        let (memory, checked) = loader.load(&memory_digest, 0)?;
        if memory.implementation != implementation {
            return Err(Error::new(
                "E_KERNEL",
                "Core1 memory pins a different implementation",
            ));
        }
        let (export, _, _) = checked.ports();
        if draft_node.export.as_str() != export {
            return Err(Error::new(
                "E_EXPORT",
                format!("Core1 memory exports {export}, not {}", draft_node.export),
            ));
        }
        if nodes.insert(draft_node.id.clone(), checked).is_some() {
            return Err(Error::new("E_NODE", "blueprint node IDs must be unique"));
        }
        program_nodes.push(Node {
            id: draft_node.id.clone(),
            memory: memory_digest,
            export: draft_node.export.clone(),
        });
    }

    let order = check_graph(
        &draft.inputs,
        &draft.output,
        &program_nodes,
        &draft.connections,
        &draft.order_edges,
        &nodes,
    )?;
    let dependencies = loader.loaded.keys().cloned().collect();
    let program = Program {
        schema: PROGRAM_SCHEMA.to_owned(),
        language: draft.language,
        profile: PROFILE.to_owned(),
        entry: draft.entry,
        inputs: draft.inputs,
        output: draft.output,
        nodes: program_nodes,
        connections: draft.connections,
        order_edges: draft.order_edges,
        settings: EmptyList::default(),
        implementation,
        dependencies,
        requirements: EmptyList::default(),
    };
    validate_program_size(&program)?;
    let subject = program_digest(&program)?;
    Ok(CheckedProgram {
        program,
        subject,
        nodes,
        order,
    })
}

pub fn load_checked(store: &Store, program: Program) -> Result<CheckedProgram> {
    validate_program_canonical(&program)?;
    if program.schema != PROGRAM_SCHEMA {
        return Err(Error::new("E_SCHEMA", "unsupported Core1 program schema"));
    }
    if program.profile != PROFILE || program.language != Language::V03 {
        return Err(Error::new(
            "E_PROFILE",
            "Core1 programs require language 0.3 and profile core1",
        ));
    }
    validate_entry(&program.entry)?;
    validate_ports(&program.inputs, &program.output)?;

    let implementation: Digest = env!("NEXA_IMPLEMENTATION_DIGEST").parse()?;
    if program.implementation != implementation {
        return Err(Error::new(
            "E_KERNEL",
            "Core1 program pins a different implementation",
        ));
    }
    let mut loader = super::Loader::new(store);
    let mut nodes = BTreeMap::new();
    for node in &program.nodes {
        let (memory, checked) = loader.load(&node.memory, 0)?;
        if memory.implementation != implementation {
            return Err(Error::new(
                "E_KERNEL",
                "Core1 memory pins a different implementation",
            ));
        }
        let (export, _, _) = checked.ports();
        if node.export.as_str() != export {
            return Err(Error::new(
                "E_EXPORT",
                format!("Core1 memory exports {export}, not {}", node.export),
            ));
        }
        if nodes.insert(node.id.clone(), checked).is_some() {
            return Err(Error::new("E_NODE", "program node IDs must be unique"));
        }
    }
    let order = check_graph(
        &program.inputs,
        &program.output,
        &program.nodes,
        &program.connections,
        &program.order_edges,
        &nodes,
    )?;
    let dependencies: Vec<_> = loader.loaded.keys().cloned().collect();
    if program.dependencies != dependencies {
        return Err(Error::new(
            "E_DEPENDENCY",
            "Core1 program dependency closure does not match its resolved nodes",
        ));
    }
    validate_program_size(&program)?;
    let subject = program_digest(&program)?;
    Ok(CheckedProgram {
        program,
        subject,
        nodes,
        order,
    })
}

fn normalize_draft(mut draft: Draft) -> Result<Draft> {
    if draft.schema != DRAFT_SCHEMA {
        return Err(Error::new(
            "E_SCHEMA",
            "unsupported Core1 blueprint draft schema",
        ));
    }
    if draft.profile != PROFILE || draft.language != Language::V03 {
        return Err(Error::new(
            "E_PROFILE",
            "Core1 drafts require language 0.3 and profile core1",
        ));
    }
    validate_entry(&draft.entry)?;
    validate_port_bounds(&draft.inputs, &draft.output)?;
    if draft.nodes.is_empty() || draft.nodes.len() > MAX_NODES {
        return Err(Error::new(
            "E_LIMIT",
            format!("Core1 blueprint requires 1 to {MAX_NODES} nodes"),
        ));
    }
    if draft.connections.is_empty() || draft.connections.len() > MAX_CONNECTIONS {
        return Err(Error::new(
            "E_LIMIT",
            format!("Core1 blueprint requires 1 to {MAX_CONNECTIONS} connections"),
        ));
    }
    if draft.order_edges.len() > MAX_ORDER_EDGES {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 blueprint has too many order edges",
        ));
    }
    if draft.nodes.iter().any(|node| node.selector.len() != 71) {
        return Err(Error::new(
            "E_DIGEST",
            "Core1 selectors must be exact digest strings",
        ));
    }
    draft
        .inputs
        .sort_by(|left, right| left.name.cmp(&right.name));
    draft.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    draft.connections.sort();
    draft.order_edges.sort();
    ensure_unique(
        draft.inputs.iter().map(|port| &port.name),
        "blueprint input names must be unique",
    )?;
    ensure_unique(
        draft.nodes.iter().map(|node| &node.id),
        "blueprint node IDs must be unique",
    )?;
    ensure_unique(
        draft.connections.iter(),
        "blueprint connections must be unique",
    )?;
    ensure_unique(
        draft.order_edges.iter(),
        "blueprint order edges must be unique",
    )?;
    let encoded =
        serde_json::to_vec(&draft).map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    if encoded.len() > crate::core1::INPUT_LIMIT {
        return Err(Error::new("E_LIMIT", "Core1 blueprint draft exceeds 1 MiB"));
    }
    Ok(draft)
}

fn validate_program_canonical(program: &Program) -> Result<()> {
    if program.nodes.is_empty() || program.nodes.len() > MAX_NODES {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 program node count is outside limits",
        ));
    }
    if program.connections.is_empty() || program.connections.len() > MAX_CONNECTIONS {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 program connection count is outside limits",
        ));
    }
    if program.order_edges.len() > MAX_ORDER_EDGES {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 program has too many order edges",
        ));
    }
    validate_port_bounds(&program.inputs, &program.output)?;
    let mut normalized = program.clone();
    normalized
        .inputs
        .sort_by(|left, right| left.name.cmp(&right.name));
    normalized
        .nodes
        .sort_by(|left, right| left.id.cmp(&right.id));
    normalized.connections.sort();
    normalized.order_edges.sort();
    normalized.dependencies.sort();
    let supplied = serde_json::to_value(program)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    let expected = serde_json::to_value(normalized)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    if supplied != expected {
        return Err(Error::new(
            "E_CANONICAL",
            "Core1 program arrays must use their deterministic sorted order",
        ));
    }
    ensure_unique(
        program.inputs.iter().map(|port| &port.name),
        "Core1 program input names must be unique",
    )?;
    ensure_unique(
        program.nodes.iter().map(|node| &node.id),
        "Core1 program node IDs must be unique",
    )?;
    ensure_unique(
        program.connections.iter(),
        "Core1 program connections must be unique",
    )?;
    ensure_unique(
        program.order_edges.iter(),
        "Core1 program order edges must be unique",
    )?;
    ensure_unique(
        program.dependencies.iter(),
        "Core1 program dependencies must be unique",
    )?;
    Ok(())
}

fn validate_entry(entry: &str) -> Result<()> {
    if entry.is_empty() || entry.len() > MAX_ENTRY_BYTES {
        return Err(Error::new(
            "E_ENTRY",
            "Core1 blueprint entry has an invalid length",
        ));
    }
    for part in entry.split('.') {
        Identifier::new(part.to_owned()).map_err(|_| {
            Error::new(
                "E_ENTRY",
                "Core1 blueprint entry must be a dotted identifier path",
            )
        })?;
    }
    Ok(())
}

fn validate_port_bounds(inputs: &[Port], output: &Port) -> Result<()> {
    if inputs.len() > MAX_PORTS {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 blueprint input count is outside limits",
        ));
    }
    if inputs
        .iter()
        .chain(std::iter::once(output))
        .any(|port| port.r#type.is_empty() || port.r#type.len() > MAX_PORT_TYPE_BYTES)
    {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 port type string exceeds the limit",
        ));
    }
    if output.name.as_str() != "result" {
        return Err(Error::new(
            "E_PORT",
            "Core1 blueprint output port must be named result",
        ));
    }
    ensure_unique(
        inputs.iter().map(|port| &port.name),
        "Core1 blueprint input names must be unique",
    )
}

fn validate_ports(inputs: &[Port], output: &Port) -> Result<()> {
    validate_port_bounds(inputs, output)
}

fn ensure_unique<'a, T: Ord + 'a>(
    mut values: impl Iterator<Item = &'a T>,
    message: &str,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    if values.any(|value| !seen.insert(value)) {
        Err(Error::new("E_DUPLICATE", message))
    } else {
        Ok(())
    }
}

fn check_graph(
    inputs: &[Port],
    output: &Port,
    nodes_spec: &[Node],
    connections: &[Connection],
    order_edges: &[OrderEdge],
    nodes: &BTreeMap<Identifier, Arc<super::CheckedModule>>,
) -> Result<Vec<Identifier>> {
    let input_types: BTreeMap<_, _> = inputs
        .iter()
        .map(|port| (port.name.clone(), port.r#type.clone()))
        .collect();
    let mut signatures = BTreeMap::new();
    for (id, module) in nodes {
        let (_, params, result) = module.ports();
        let param_count = params.len();
        let params: BTreeMap<_, _> = params.into_iter().collect();
        if params.len() != param_count {
            return Err(Error::new(
                "E_CORE1_INVARIANT",
                "Core1 signature has duplicate parameters",
            ));
        }
        signatures.insert(id.clone(), (params, result));
    }
    let mut predecessors: BTreeMap<_, BTreeSet<_>> = nodes
        .keys()
        .cloned()
        .map(|node| (node, BTreeSet::new()))
        .collect();
    let mut bound = BTreeMap::<Identifier, BTreeSet<Identifier>>::new();
    let mut used_inputs = BTreeSet::new();
    let mut output_source = None;

    for connection in connections {
        let source_type = match &connection.from {
            Endpoint::Input { port } => {
                used_inputs.insert(port.clone());
                input_types.get(port).cloned().ok_or_else(|| {
                    Error::new("E_ENDPOINT", format!("unknown blueprint input {port}"))
                })?
            }
            Endpoint::Node { node, port } => {
                let (_, result) = signatures
                    .get(node)
                    .ok_or_else(|| Error::new("E_ENDPOINT", format!("unknown node {node}")))?;
                if port.as_str() != "result" {
                    return Err(Error::new(
                        "E_PORT",
                        format!("unknown result port {node}.{port}"),
                    ));
                }
                result.clone()
            }
            Endpoint::Output { .. } => {
                return Err(Error::new(
                    "E_ENDPOINT",
                    "an output cannot be a connection source",
                ));
            }
        };
        let destination_type = match &connection.to {
            Endpoint::Node { node, port } => {
                let (params, _) = signatures
                    .get(node)
                    .ok_or_else(|| Error::new("E_ENDPOINT", format!("unknown node {node}")))?;
                let parameter = params.get(port.as_str()).ok_or_else(|| {
                    Error::new("E_PORT", format!("unknown parameter {node}.{port}"))
                })?;
                if !bound.entry(node.clone()).or_default().insert(port.clone()) {
                    return Err(Error::new(
                        "E_PRODUCER",
                        format!("parameter {node}.{port} has more than one producer"),
                    ));
                }
                parameter.clone()
            }
            Endpoint::Output { port } => {
                if port.as_str() != "result" {
                    return Err(Error::new("E_ENDPOINT", "invalid Core1 output destination"));
                }
                if !matches!(&connection.from, Endpoint::Node { .. }) {
                    return Err(Error::new(
                        "E_ENDPOINT",
                        "Core1 blueprint output must come from a node result",
                    ));
                }
                if output_source.replace(connection.from.clone()).is_some() {
                    return Err(Error::new(
                        "E_PRODUCER",
                        "blueprint output has more than one producer",
                    ));
                }
                output.r#type.clone()
            }
            Endpoint::Input { .. } => {
                return Err(Error::new(
                    "E_ENDPOINT",
                    "an input cannot be a connection destination",
                ));
            }
        };
        if source_type != destination_type {
            return Err(Error::new(
                "E_PORT_TYPE",
                format!(
                    "connection type mismatch: {} ({source_type}) cannot connect to {} ({destination_type})",
                    endpoint_name(&connection.from),
                    endpoint_name(&connection.to),
                ),
            ));
        }
        if let (Endpoint::Node { node: producer, .. }, Endpoint::Node { node: consumer, .. }) =
            (&connection.from, &connection.to)
        {
            predecessors
                .get_mut(consumer)
                .expect("destination node checked above")
                .insert(producer.clone());
        }
    }

    if output_source.is_none() {
        return Err(Error::new(
            "E_UNBOUND",
            "blueprint output result is unbound",
        ));
    }
    if used_inputs.len() != inputs.len() {
        let missing = inputs
            .iter()
            .find(|port| !used_inputs.contains(&port.name))
            .expect("input counts differ")
            .name
            .to_string();
        return Err(Error::new(
            "E_UNBOUND",
            format!("blueprint input {missing} is not connected"),
        ));
    }

    for (id, (params, _)) in &signatures {
        for param in params.keys() {
            if !bound
                .get(id)
                .is_some_and(|ports| ports.iter().any(|port| port.as_str() == param))
            {
                return Err(Error::new(
                    "E_UNBOUND",
                    format!("required parameter {id}.{param} is not connected"),
                ));
            }
        }
    }
    for order in order_edges {
        if !nodes.contains_key(&order.before) || !nodes.contains_key(&order.after) {
            return Err(Error::new(
                "E_ENDPOINT",
                "order edge references an unknown node",
            ));
        }
        predecessors
            .get_mut(&order.after)
            .expect("node membership checked")
            .insert(order.before.clone());
    }
    if nodes_spec.len() != nodes.len()
        || nodes_spec.iter().any(|node| !nodes.contains_key(&node.id))
    {
        return Err(Error::new(
            "E_NODE",
            "resolved Core1 node set disagrees with program",
        ));
    }

    let mut remaining = predecessors;
    let mut ready: BTreeSet<_> = remaining
        .iter()
        .filter(|(_, deps)| deps.is_empty())
        .map(|(node, _)| node.clone())
        .collect();
    let mut order = Vec::with_capacity(remaining.len());
    while let Some(node) = ready.pop_first() {
        order.push(node.clone());
        for (candidate, deps) in remaining.iter_mut() {
            if deps.remove(&node) && deps.is_empty() {
                ready.insert(candidate.clone());
            }
        }
    }
    if order.len() != remaining.len() {
        return Err(Error::new(
            "E_CYCLE",
            "data and order edges contain a cycle",
        ));
    }
    Ok(order)
}

fn endpoint_name(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Input { port } => format!("input.{port}"),
        Endpoint::Node { node, port } => format!("{node}.{port}"),
        Endpoint::Output { port } => format!("output.{port}"),
    }
}

fn validate_program_size(program: &Program) -> Result<()> {
    let value = serde_json::to_value(program)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    if canonical::encode(&value)?.len() > crate::core1::INPUT_LIMIT {
        return Err(Error::new("E_LIMIT", "Core1 program exceeds 1 MiB"));
    }
    Ok(())
}

fn program_digest(program: &Program) -> Result<Digest> {
    let value = serde_json::to_value(program)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    canonical::digest(&value)
}
