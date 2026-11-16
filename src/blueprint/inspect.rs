//! Bounded, read-only inspection views for checked programs.

use super::{CheckedNode, CheckedProgram, ResolvedImplementation, SourceReference};
use crate::{
    contracts::{CapabilityRequest, Connection, Endpoint, Port, Profile},
    diagnostic::{Error, Result, SourceSpan},
    digest::Digest,
    value::Identifier,
};
use serde::Serialize;

pub const DEFAULT_PAGE_SIZE: usize = 50;
pub const MAX_PAGE_SIZE: usize = 50;

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramInspection {
    pub schema: &'static str,
    pub profile: Profile,
    pub subject: Digest,
    pub module: String,
    pub entry: String,
    pub inputs: Vec<Port>,
    pub output: Port,
    pub node_count: usize,
    pub nodes: Vec<NodeSummary>,
    pub connection_count: usize,
    pub effects: Vec<String>,
    pub required_bindings: Vec<String>,
    pub capability_requirements: Vec<CapabilityRequest>,
    pub dependencies: Vec<Digest>,
    pub unresolved_requirements: Vec<String>,
    pub offset: usize,
    pub limit: usize,
    pub next_offset: Option<usize>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSummary {
    pub path: String,
    pub id: Identifier,
    pub memory: Digest,
    pub export: Identifier,
    pub implementation: &'static str,
    pub contract: crate::contracts::CallableContract,
    pub source_references: Vec<SourceReference>,
    pub source_span: Option<SourceSpan>,
    pub child_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeInspection {
    pub node: NodeSummary,
    pub incoming_connection_count: usize,
    pub incoming_connections: Vec<Connection>,
    pub incoming_truncated: bool,
    pub outgoing_connection_count: usize,
    pub outgoing_connections: Vec<Connection>,
    pub outgoing_truncated: bool,
    pub children: Option<ProgramInspection>,
    pub offset: usize,
    pub limit: usize,
    pub next_offset: Option<usize>,
    pub has_more: bool,
}

pub fn inspect(checked: &CheckedProgram, node_path: Option<&str>) -> Result<serde_json::Value> {
    inspect_page(checked, node_path, 0, DEFAULT_PAGE_SIZE)
}

pub fn inspect_page(
    checked: &CheckedProgram,
    node_path: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<serde_json::Value> {
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(Error::new(
            "E_PAGE_LIMIT",
            format!("inspection limit must be between 1 and {MAX_PAGE_SIZE}"),
        ));
    }

    let value = if let Some(node_path) = node_path {
        let (program, node, local_path) = resolve_path(checked, node_path)?;
        let child_count = match &node.implementation {
            ResolvedImplementation::Source(_) => 0,
            ResolvedImplementation::Program(program) => program.nodes.len(),
        };
        if offset > child_count {
            return Err(Error::new(
                "E_PAGE_OFFSET",
                "inspection page offset exceeds the selected node's child count",
            ));
        }
        let node_view = node_inspection(program, node, &local_path, offset, limit)?;
        serde_json::to_value(node_view)
    } else {
        if offset > checked.nodes.len() {
            return Err(Error::new(
                "E_PAGE_OFFSET",
                "inspection page offset exceeds the program node count",
            ));
        }
        let view = program_inspection(checked, "", offset, limit);
        serde_json::to_value(view)
    };
    value.map_err(|error| Error::new("E_INSPECT", error.to_string()))
}

fn program_inspection(
    checked: &CheckedProgram,
    prefix: &str,
    offset: usize,
    limit: usize,
) -> ProgramInspection {
    let all_nodes: Vec<_> = checked.nodes.values().collect();
    let node_count = all_nodes.len();
    let page: Vec<_> = all_nodes.iter().skip(offset).take(limit).copied().collect();
    let next_offset = (offset + page.len() < node_count).then_some(offset + page.len());
    let nodes = page
        .into_iter()
        .map(|node| node_summary(checked, node, prefix))
        .collect();

    ProgramInspection {
        schema: "nexa.program-inspection/0.3",
        profile: checked.spec.profile,
        subject: checked.digest.clone(),
        module: checked.module_name().to_owned(),
        entry: checked.spec.entry.clone(),
        inputs: checked.spec.inputs.clone(),
        output: checked.spec.output.clone(),
        node_count,
        nodes,
        connection_count: checked.spec.connections.len(),
        effects: checked.spec.derived.effects.clone(),
        required_bindings: checked.spec.derived.required_bindings.clone(),
        capability_requirements: checked.spec.derived.capability_requirements.clone(),
        dependencies: checked.spec.derived.dependencies.clone(),
        unresolved_requirements: Vec::new(),
        offset,
        limit,
        next_offset,
        has_more: next_offset.is_some(),
    }
}

fn node_summary(checked: &CheckedProgram, node: &CheckedNode, prefix: &str) -> NodeSummary {
    let path = if prefix.is_empty() {
        node.node.id.to_string()
    } else {
        format!("{prefix}/{}", node.node.id)
    };
    let (implementation, child_count) = match &node.implementation {
        ResolvedImplementation::Source(_) => ("source", 0),
        ResolvedImplementation::Program(program) => ("program", program.nodes.len()),
    };
    NodeSummary {
        path,
        id: node.node.id.clone(),
        memory: node.node.memory.clone(),
        export: node.node.export.clone(),
        implementation,
        contract: node.contract.clone(),
        source_references: node.source_references.clone(),
        source_span: checked
            .blueprint_source_map
            .as_ref()
            .and_then(|source_map| source_map.nodes.get(&node.node.id).cloned()),
        child_count,
    }
}

fn node_inspection(
    checked: &CheckedProgram,
    node: &CheckedNode,
    path: &str,
    offset: usize,
    limit: usize,
) -> Result<NodeInspection> {
    let incoming: Vec<_> = checked
        .spec
        .connections
        .iter()
        .filter(|connection| {
            matches!(&connection.to, Endpoint::Node { node: id, .. } if id == &node.node.id)
        })
        .cloned()
        .collect();
    let outgoing: Vec<_> = checked
        .spec
        .connections
        .iter()
        .filter(|connection| {
            matches!(&connection.from, Endpoint::Node { node: id, .. } if id == &node.node.id)
        })
        .cloned()
        .collect();
    let children = match &node.implementation {
        ResolvedImplementation::Source(_) => None,
        ResolvedImplementation::Program(program) => {
            Some(program_inspection(program, path, offset, limit))
        }
    };

    let next_offset = children.as_ref().and_then(|children| children.next_offset);
    let incoming_connection_count = incoming.len();
    let outgoing_connection_count = outgoing.len();
    let incoming_truncated = incoming_connection_count > MAX_PAGE_SIZE;
    let outgoing_truncated = outgoing_connection_count > MAX_PAGE_SIZE;
    let incoming_connections = incoming.into_iter().take(MAX_PAGE_SIZE).collect();
    let outgoing_connections = outgoing.into_iter().take(MAX_PAGE_SIZE).collect();
    Ok(NodeInspection {
        node: node_summary(
            checked,
            node,
            path.rsplit_once('/').map_or("", |(prefix, _)| prefix),
        ),
        incoming_connection_count,
        incoming_connections,
        incoming_truncated,
        outgoing_connection_count,
        outgoing_connections,
        outgoing_truncated,
        children,
        offset,
        limit,
        next_offset,
        has_more: next_offset.is_some(),
    })
}

fn resolve_path<'a>(
    root: &'a CheckedProgram,
    path: &str,
) -> Result<(&'a CheckedProgram, &'a CheckedNode, String)> {
    let segments: Vec<_> = path.split('/').collect();
    if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return Err(Error::new("E_NODE_PATH", "node path is empty or malformed"));
    }

    let mut current = root;
    let mut full_path = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let id = Identifier::new((*segment).to_owned()).map_err(|_| {
            Error::new(
                "E_NODE_PATH",
                format!("invalid node path segment: {segment}"),
            )
        })?;
        let node = current
            .nodes
            .get(&id)
            .ok_or_else(|| Error::new("E_NODE_PATH", format!("unknown node path: {path}")))?;
        full_path.push(*segment);
        if index + 1 == segments.len() {
            return Ok((current, node, full_path.join("/")));
        }
        match &node.implementation {
            ResolvedImplementation::Program(program) => current = program,
            ResolvedImplementation::Source(_) => {
                return Err(Error::new(
                    "E_NODE_PATH",
                    format!("node path continues through source node {segment}"),
                ));
            }
        }
    }

    Err(Error::new("E_NODE_PATH", "node path does not resolve"))
}
