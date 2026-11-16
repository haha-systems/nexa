//! Locked composition and semantic graph checking.

use super::{
    BlueprintSourceMap, CheckedNode, CheckedProgram, ResolvedImplementation, SourceReference,
    nested,
};
use crate::{
    canonical,
    contracts::{
        BlueprintDraft, CallableContract, CallableKind, Connection, DraftNode, EmptyList, Endpoint,
        LockRecord, MemoryRecord, Node, OrderEdge, Port, ProgramContract, ProgramSchema,
        ProgramSpec, WireDocument,
    },
    diagnostic::{Error, Result},
    digest::Digest,
    memory::Store,
    value::Identifier,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn compose(draft: &BlueprintDraft, lock: &LockRecord, store: &Store) -> Result<ProgramSpec> {
    compose_with_source_map(draft, None, lock, store)
}

pub fn compose_parsed(
    parsed: &super::ParsedBlueprint,
    lock: &LockRecord,
    store: &Store,
) -> Result<ProgramSpec> {
    compose_with_source_map(&parsed.draft, Some(&parsed.source_map), lock, store)
}

fn compose_with_source_map(
    draft: &BlueprintDraft,
    source_map: Option<&BlueprintSourceMap>,
    lock: &LockRecord,
    store: &Store,
) -> Result<ProgramSpec> {
    Ok(compose_checked_with_source_map(draft, lock, store, source_map)?.spec)
}

pub fn compose_checked(
    draft: &BlueprintDraft,
    lock: &LockRecord,
    store: &Store,
) -> Result<CheckedProgram> {
    compose_checked_with_source_map(draft, lock, store, None)
}

pub fn compose_parsed_checked(
    parsed: &super::ParsedBlueprint,
    lock: &LockRecord,
    store: &Store,
) -> Result<CheckedProgram> {
    compose_checked_with_source_map(&parsed.draft, lock, store, Some(&parsed.source_map))
}

fn compose_checked_with_source_map(
    draft: &BlueprintDraft,
    lock: &LockRecord,
    store: &Store,
    source_map: Option<&BlueprintSourceMap>,
) -> Result<CheckedProgram> {
    let mut draft = draft.clone();
    super::parser::validate_node_result_endpoints(
        &draft.connections,
        &draft.nodes.iter().map(|node| node.id.clone()).collect(),
    )?;
    draft.normalize()?;
    store.validate_lock(lock)?;
    if draft.language != lock.language || draft.profile != lock.profile {
        return Err(Error::new(
            "E_LOCK_PROFILE",
            "blueprint language/profile does not match the lock",
        ));
    }
    let nodes = resolve_nodes(&draft.nodes, lock, store, &mut BTreeSet::new())?;
    check_graph(
        &draft.inputs,
        &draft.output,
        &draft.nodes,
        &draft.connections,
        &draft.order_edges,
        &nodes,
    )?;
    let node_specs = draft
        .nodes
        .iter()
        .map(|draft_node| {
            let resolved = nodes
                .get(&draft_node.id)
                .expect("every draft node has a resolved implementation");
            Node {
                id: draft_node.id.clone(),
                memory: resolved.node.memory.clone(),
                export: draft_node.export.clone(),
            }
        })
        .collect();
    let derived = derive_program_contract(&draft.inputs, &draft.output, &nodes);
    let mut spec = ProgramSpec {
        schema: ProgramSchema::V03,
        language: draft.language,
        profile: draft.profile,
        entry: draft.entry,
        inputs: draft.inputs,
        output: draft.output,
        nodes: node_specs,
        connections: draft.connections,
        order_edges: draft.order_edges,
        settings: EmptyList::default(),
        kernel: lock.kernel.clone(),
        requirements: EmptyList::default(),
        derived,
    };
    spec.normalize()?;
    let mut checked = load_checked_with_ancestors(&spec, store, &mut BTreeSet::new())?;
    checked.blueprint_source_map = source_map.cloned();
    Ok(checked)
}

pub fn load_checked(spec: &ProgramSpec, store: &Store) -> Result<CheckedProgram> {
    load_checked_with_ancestors(spec, store, &mut BTreeSet::new())
}

pub(crate) fn load_checked_with_ancestors(
    spec: &ProgramSpec,
    store: &Store,
    visiting: &mut BTreeSet<Digest>,
) -> Result<CheckedProgram> {
    let mut spec = spec.clone();
    super::parser::validate_node_result_endpoints(
        &spec.connections,
        &spec.nodes.iter().map(|node| node.id.clone()).collect(),
    )?;
    spec.normalize()?;

    if spec.profile != crate::contracts::Profile::Slice0
        || spec.language != crate::contracts::Language::V03
    {
        return Err(Error::new(
            "E_UNSUPPORTED",
            "only language 0.3 profile slice0 programs are supported",
        ));
    }
    if &spec.kernel != store.kernel_digest() {
        return Err(Error::new(
            "E_KERNEL",
            "program kernel digest does not match the installed kernel",
        ));
    }

    let nodes = load_nodes(&spec.nodes, &spec.kernel, store, visiting)?;
    check_graph(
        &spec.inputs,
        &spec.output,
        &spec
            .nodes
            .iter()
            .map(|node| DraftNode {
                id: node.id.clone(),
                selector: node.memory.to_string(),
                export: node.export.clone(),
            })
            .collect::<Vec<_>>(),
        &spec.connections,
        &spec.order_edges,
        &nodes,
    )?;

    let expected_derived = derive_program_contract(&spec.inputs, &spec.output, &nodes);
    let supplied = serde_json::to_value(&spec.derived)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    let expected = serde_json::to_value(&expected_derived)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    if supplied != expected {
        return Err(Error::new(
            "E_DERIVED",
            "serialized program contract does not match loaded node contracts",
        ));
    }
    spec.derived = expected_derived;
    spec.normalize()?;

    let contract = program_callable(&spec)?;
    let program_json =
        serde_json::to_value(&spec).map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    let program_digest = canonical::digest(&program_json)?;

    Ok(CheckedProgram {
        spec,
        digest: program_digest,
        contract,
        nodes,
        blueprint_source_map: None,
    })
}

fn resolve_nodes(
    drafts: &[DraftNode],
    lock: &LockRecord,
    store: &Store,
    visiting: &mut BTreeSet<Digest>,
) -> Result<BTreeMap<Identifier, CheckedNode>> {
    let mut nodes = BTreeMap::new();
    for draft in drafts {
        let digest = store.resolve_locked(lock, &draft.selector, &draft.export)?;
        if !lock.dependencies.contains(&digest) {
            return Err(Error::new(
                "E_DEPENDENCY",
                "locked selector is missing from the lock dependency closure",
            ));
        }
        let record = store.load_memory_local(&digest)?;
        verify_record_context(&record, lock.language, lock.profile, &lock.kernel)?;
        let resolved = checked_node(
            draft.id.clone(),
            draft.export.clone(),
            digest,
            record,
            store,
            visiting,
        )?;
        nodes.insert(draft.id.clone(), resolved);
    }
    Ok(nodes)
}

fn load_nodes(
    node_specs: &[Node],
    kernel: &Digest,
    store: &Store,
    visiting: &mut BTreeSet<Digest>,
) -> Result<BTreeMap<Identifier, CheckedNode>> {
    let mut nodes = BTreeMap::new();
    for node in node_specs {
        let record = store.load_memory_local(&node.memory)?;
        verify_record_context(
            &record,
            crate::contracts::Language::V03,
            crate::contracts::Profile::Slice0,
            kernel,
        )?;
        let resolved = checked_node(
            node.id.clone(),
            node.export.clone(),
            node.memory.clone(),
            record,
            store,
            visiting,
        )?;
        nodes.insert(node.id.clone(), resolved);
    }
    Ok(nodes)
}

fn verify_record_context(
    record: &MemoryRecord,
    language: crate::contracts::Language,
    profile: crate::contracts::Profile,
    kernel: &Digest,
) -> Result<()> {
    if record.language != language || record.profile != profile {
        return Err(Error::new(
            "E_MEMORY_PROFILE",
            "memory language/profile does not match the locked program",
        ));
    }
    if &record.kernel != kernel {
        return Err(Error::new(
            "E_KERNEL",
            "memory kernel digest does not match the locked program",
        ));
    }
    Ok(())
}

fn checked_node(
    id: Identifier,
    export: Identifier,
    digest: Digest,
    record: MemoryRecord,
    store: &Store,
    visiting: &mut BTreeSet<Digest>,
) -> Result<CheckedNode> {
    let stored_contract = record
        .exports
        .iter()
        .find(|contract| contract.export_name == export)
        .cloned()
        .ok_or_else(|| {
            Error::new(
                "E_EXPORT",
                format!("memory {digest} does not export {export}"),
            )
        })?;

    let dependency_closure = record.dependencies.clone();
    let (contract, implementation) = match &record.implementation {
        crate::contracts::Implementation::Source { entry } => {
            if entry != &export {
                return Err(Error::new(
                    "E_EXPORT",
                    "selected source export disagrees with its memory entry",
                ));
            }
            let module = store.load_source_module(&digest)?;
            let checked_contract = module.contract(export.as_str())?;
            if !same_callable(&stored_contract, &checked_contract) {
                return Err(Error::new(
                    "E_CONTRACT",
                    "source memory contract disagrees with its checked implementation",
                ));
            }
            (checked_contract, ResolvedImplementation::Source(module))
        }
        crate::contracts::Implementation::Program { .. } => {
            let program = nested::load_nested_program(store, &digest, visiting)?;
            let checked_contract = program.contract().clone();
            if !same_callable(&stored_contract, &checked_contract) {
                return Err(Error::new(
                    "E_CONTRACT",
                    "program memory contract disagrees with its checked graph",
                ));
            }
            (
                checked_contract,
                ResolvedImplementation::Program(Box::new(program)),
            )
        }
    };

    let mut contract = contract;
    contract.normalize()?;
    let source_references = record
        .source_map
        .iter()
        .map(|source| SourceReference {
            memory: digest.clone(),
            path: source.path.clone(),
            blob: source.blob.clone(),
            module: match &implementation {
                ResolvedImplementation::Source(module) => module.module_name().to_owned(),
                ResolvedImplementation::Program(_) => String::new(),
            },
        })
        .collect();

    Ok(CheckedNode {
        node: Node {
            id,
            memory: digest,
            export,
        },
        contract,
        implementation,
        source_references,
        dependency_closure,
    })
}

fn same_callable(left: &CallableContract, right: &CallableContract) -> bool {
    left.export_name == right.export_name
        && left.kind == right.kind
        && left.parameters == right.parameters
        && left.result_type == right.result_type
        && left.effects == right.effects
        && left.required_bindings == right.required_bindings
        && left.capability_requirements == right.capability_requirements
}

fn derive_program_contract(
    inputs: &[Port],
    output: &Port,
    nodes: &BTreeMap<Identifier, CheckedNode>,
) -> ProgramContract {
    let mut dependencies = BTreeSet::new();
    let mut effects = BTreeSet::new();
    let mut required_bindings = BTreeSet::new();
    let mut capability_requirements = BTreeSet::new();

    for node in nodes.values() {
        dependencies.insert(node.node.memory.clone());
        dependencies.extend(node.dependency_closure.iter().cloned());
        match &node.implementation {
            ResolvedImplementation::Source(_) => {}
            ResolvedImplementation::Program(_) => {}
        }
        effects.extend(node.contract.effects.iter().cloned());
        required_bindings.extend(node.contract.required_bindings.iter().cloned());
        capability_requirements.extend(node.contract.capability_requirements.iter().cloned());
    }

    ProgramContract {
        inputs: inputs.to_vec(),
        result_type: output.r#type,
        dependencies: dependencies.into_iter().collect(),
        effects: effects.into_iter().collect(),
        required_bindings: required_bindings.into_iter().collect(),
        capability_requirements: capability_requirements.into_iter().collect(),
        unresolved_requirements: EmptyList::default(),
    }
}

fn program_callable(spec: &ProgramSpec) -> Result<CallableContract> {
    let name = spec
        .entry
        .rsplit('.')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error::new("E_NAME", "program entry has no local name"))?;
    let mut contract = CallableContract {
        export_name: Identifier::new(name.to_owned())?,
        kind: if spec.derived.effects.is_empty() {
            CallableKind::Function
        } else {
            CallableKind::Action
        },
        parameters: spec.inputs.clone(),
        result_type: spec.output.r#type,
        effects: spec.derived.effects.clone(),
        required_bindings: spec.derived.required_bindings.clone(),
        capability_requirements: spec.derived.capability_requirements.clone(),
    };
    contract.normalize()?;
    Ok(contract)
}

fn check_graph(
    inputs: &[Port],
    output: &Port,
    draft_nodes: &[DraftNode],
    connections: &[Connection],
    order_edges: &[OrderEdge],
    nodes: &BTreeMap<Identifier, CheckedNode>,
) -> Result<()> {
    let input_types: BTreeMap<_, _> = inputs
        .iter()
        .map(|port| (port.name.clone(), port.r#type))
        .collect();
    let mut predecessors: BTreeMap<Identifier, BTreeSet<Identifier>> = nodes
        .keys()
        .cloned()
        .map(|node| (node, BTreeSet::new()))
        .collect();
    let mut bound: BTreeMap<Identifier, BTreeSet<Identifier>> = BTreeMap::new();

    for connection in connections {
        let source_type = match &connection.from {
            Endpoint::Input { port } => *input_types.get(port).ok_or_else(|| {
                Error::new("E_ENDPOINT", format!("unknown blueprint input {port}"))
            })?,
            Endpoint::Node { node, port } => {
                let producer = nodes
                    .get(node)
                    .ok_or_else(|| Error::new("E_ENDPOINT", format!("unknown node {node}")))?;
                if port.as_str() != "result" {
                    return Err(Error::new(
                        "E_PORT",
                        format!("unknown result port {node}.{port}"),
                    ));
                }
                producer.contract.result_type
            }
            Endpoint::Output { .. } => {
                return Err(Error::new(
                    "E_ENDPOINT",
                    "an output port cannot be a connection source",
                ));
            }
        };

        let destination_type = match &connection.to {
            Endpoint::Node { node, port } => {
                let consumer = nodes
                    .get(node)
                    .ok_or_else(|| Error::new("E_ENDPOINT", format!("unknown node {node}")))?;
                let parameter = consumer
                    .contract
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *port)
                    .ok_or_else(|| {
                        Error::new("E_PORT", format!("unknown parameter {node}.{port}"))
                    })?;
                if !bound.entry(node.clone()).or_default().insert(port.clone()) {
                    return Err(Error::new(
                        "E_PRODUCER",
                        format!("parameter {node}.{port} has more than one producer"),
                    ));
                }
                parameter.r#type
            }
            Endpoint::Output { port } => {
                if port.as_str() != "result" || output.name.as_str() != "result" {
                    return Err(Error::new("E_ENDPOINT", "invalid output destination"));
                }
                output.r#type
            }
            Endpoint::Input { .. } => {
                return Err(Error::new(
                    "E_ENDPOINT",
                    "an input port cannot be a connection destination",
                ));
            }
        };

        if source_type != destination_type {
            return Err(Error::new(
                "E_PORT_TYPE",
                format!(
                    "connection type mismatch: {} ({:?}) cannot connect to {} ({:?})",
                    endpoint_name(&connection.from),
                    source_type,
                    endpoint_name(&connection.to),
                    destination_type,
                ),
            ));
        }

        if let (Endpoint::Node { node: producer, .. }, Endpoint::Node { node: consumer, .. }) =
            (&connection.from, &connection.to)
        {
            predecessors
                .get_mut(consumer)
                .expect("destination validation follows")
                .insert(producer.clone());
        }
    }

    for (id, node) in nodes {
        let node_bound = bound.get(id);
        for parameter in &node.contract.parameters {
            if !node_bound.is_some_and(|ports| ports.contains(&parameter.name)) {
                return Err(Error::new(
                    "E_UNBOUND",
                    format!(
                        "required parameter {id}.{} is not connected",
                        parameter.name
                    ),
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

    // Kahn's algorithm both detects data/order cycles and confirms all nodes
    // participate in the finite invocation DAG.
    let mut remaining: BTreeMap<_, _> = predecessors
        .iter()
        .map(|(node, deps)| (node.clone(), deps.clone()))
        .collect();
    let mut ready: BTreeSet<_> = remaining
        .iter()
        .filter(|(_, deps)| deps.is_empty())
        .map(|(node, _)| node.clone())
        .collect();
    let mut visited = 0usize;
    while let Some(node) = ready.pop_first() {
        visited += 1;
        for (candidate, deps) in remaining.iter_mut() {
            if deps.remove(&node) && deps.is_empty() {
                ready.insert(candidate.clone());
            }
        }
    }
    if visited != remaining.len() {
        return Err(Error::new(
            "E_CYCLE",
            "data and order edges contain a cycle",
        ));
    }

    // `draft_nodes` carries no additional semantics here, but checking that its
    // IDs correspond to the resolved node map catches internal caller mistakes.
    if draft_nodes.len() != nodes.len()
        || draft_nodes
            .iter()
            .any(|draft| !nodes.contains_key(&draft.id))
    {
        return Err(Error::new(
            "E_NODE",
            "resolved node set disagrees with blueprint",
        ));
    }

    Ok(())
}

fn endpoint_name(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Input { port } => format!("input.{port}"),
        Endpoint::Node { node, port } => format!("{node}.{port}"),
        Endpoint::Output { port } => format!("output.{port}"),
    }
}
