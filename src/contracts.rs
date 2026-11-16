//! Closed Slice 0 wire contracts. Normalization checks local invariants only;
//! resolving exports and checking graph port types are later compiler stages.

use crate::{
    canonical,
    diagnostic::{Error, Result},
    digest::Digest,
    value::{Identifier, Value, ValueType, identifier_shape},
};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Component, Path},
};

macro_rules! schema_tag {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            #[serde(rename = $wire)]
            V03,
        }
    };
}

schema_tag!(SourceSchema, "nexa.memory-source/0.3");
schema_tag!(MemorySchema, "nexa.memory/0.3");
schema_tag!(DraftSchema, "nexa.blueprint-draft/0.3");
schema_tag!(ProgramSchema, "nexa.program/0.3");
schema_tag!(LockSchema, "nexa.lock/0.3");
schema_tag!(ApplicationSchema, "nexa.application/0.3");
schema_tag!(ReceiptSchema, "nexa.receipt/0.3");
schema_tag!(EvidenceSchema, "nexa.evidence/0.3");
schema_tag!(FixturesSchema, "nexa.fixtures/0.3");
schema_tag!(KernelSchema, "nexa.kernel/0.3");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Language {
    #[serde(rename = "0.3")]
    V03,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Profile {
    #[serde(rename = "slice0")]
    Slice0,
}

/// Empty arrays are explicit: Slice 0 cannot silently retain unsupported work.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(try_from = "Vec<serde_json::Value>")]
pub struct EmptyList(Vec<serde_json::Value>);

impl TryFrom<Vec<serde_json::Value>> for EmptyList {
    type Error = Error;
    fn try_from(values: Vec<serde_json::Value>) -> Result<Self> {
        if values.is_empty() {
            Ok(Self(values))
        } else {
            Err(Error::new(
                "E_UNSUPPORTED",
                "this collection must be empty in slice0",
            ))
        }
    }
}

impl JsonSchema for EmptyList {
    fn schema_name() -> Cow<'static, str> {
        "EmptyList".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "array", "maxItems": 0})
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Port {
    pub name: Identifier,
    pub r#type: ValueType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependencyRequest {
    pub alias: Identifier,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceManifest {
    pub schema: SourceSchema,
    pub name: String,
    pub version: String,
    pub module: String,
    pub sources: Vec<String>,
    pub entry: Identifier,
    pub kernel: String,
    pub dependencies: Vec<DependencyRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallableKind {
    Function,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequest {
    pub capability: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallableContract {
    pub export_name: Identifier,
    pub kind: CallableKind,
    pub parameters: Vec<Port>,
    pub result_type: ValueType,
    pub effects: Vec<String>,
    pub required_bindings: Vec<String>,
    pub capability_requirements: Vec<CapabilityRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Implementation {
    Source { entry: Identifier },
    Program { program: Digest },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceBlob {
    pub path: String,
    pub blob: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub provenance: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    pub schema: MemorySchema,
    pub language: Language,
    pub profile: Profile,
    pub implementation: Implementation,
    pub exports: Vec<CallableContract>,
    pub dependencies: Vec<Digest>,
    pub kernel: Digest,
    pub source_map: Vec<SourceBlob>,
    pub metadata: MemoryMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DraftNode {
    pub id: Identifier,
    pub selector: String,
    pub export: Identifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: Identifier,
    pub memory: Digest,
    pub export: Identifier,
}

// Variant order is lexical kind order, matching the canonical endpoint tuple.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Endpoint {
    Input { port: Identifier },
    Node { node: Identifier, port: Identifier },
    Output { port: Identifier },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub from: Endpoint,
    pub to: Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrderEdge {
    pub before: Identifier,
    pub after: Identifier,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlueprintDraft {
    pub schema: DraftSchema,
    pub language: Language,
    pub profile: Profile,
    pub entry: String,
    pub inputs: Vec<Port>,
    pub output: Port,
    pub nodes: Vec<DraftNode>,
    pub connections: Vec<Connection>,
    pub order_edges: Vec<OrderEdge>,
    pub settings: EmptyList,
    pub requirements: EmptyList,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramContract {
    pub inputs: Vec<Port>,
    pub result_type: ValueType,
    pub dependencies: Vec<Digest>,
    pub effects: Vec<String>,
    pub required_bindings: Vec<String>,
    pub capability_requirements: Vec<CapabilityRequest>,
    pub unresolved_requirements: EmptyList,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramSpec {
    pub schema: ProgramSchema,
    pub language: Language,
    pub profile: Profile,
    pub entry: String,
    pub inputs: Vec<Port>,
    pub output: Port,
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
    pub order_edges: Vec<OrderEdge>,
    pub settings: EmptyList,
    pub kernel: Digest,
    pub requirements: EmptyList,
    pub derived: ProgramContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LockBinding {
    pub selector: String,
    pub export: Identifier,
    pub memory: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LockRecord {
    pub schema: LockSchema,
    pub language: Language,
    pub profile: Profile,
    pub kernel: Digest,
    pub bindings: Vec<LockBinding>,
    pub dependencies: Vec<Digest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PolicyDecision {
    Allow,
    Deny,
    Ask,
    Abstain,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyRule {
    pub operation: String,
    pub resource: String,
    pub decision: PolicyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationManifest {
    pub schema: ApplicationSchema,
    pub principal: String,
    pub requests: Vec<CapabilityRequest>,
    pub policies: Vec<PolicyRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Outcome {
    Succeeded {
        value: Value,
    },
    Skipped {
        reason: String,
    },
    Denied {
        reason: String,
    },
    AwaitingApproval {
        reason: String,
    },
    Failed {
        fault: crate::diagnostic::FaultCode,
        message: String,
    },
    Cancelled {
        reason: String,
    },
}

impl Outcome {
    pub fn exit_code(&self) -> crate::diagnostic::ExitCode {
        use crate::diagnostic::ExitCode;
        match self {
            Self::Succeeded { .. } => ExitCode::Success,
            Self::Failed { .. } => ExitCode::RuntimeFailure,
            Self::Denied { .. } => ExitCode::Denied,
            Self::AwaitingApproval { .. } => ExitCode::AwaitingApproval,
            Self::Skipped { .. } | Self::Cancelled { .. } => ExitCode::SkippedOrCancelled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionRecord {
    pub operation: String,
    pub resource: String,
    pub decision: PolicyDecision,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeRecord {
    pub path: String,
    pub memory: Digest,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectRecord {
    pub node_path: String,
    pub operation: String,
    pub resource: String,
    pub dispatched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: ReceiptSchema,
    pub profile: Profile,
    pub invocation_id: String,
    pub program: Digest,
    pub principal: String,
    pub observed_time: Option<String>,
    pub nodes: Vec<NodeRecord>,
    pub decisions: Vec<DecisionRecord>,
    pub effects: Vec<EffectRecord>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReport {
    pub schema: EvidenceSchema,
    pub profile: Profile,
    pub subject: Digest,
    pub tool_version: String,
    pub kernel: Digest,
    pub fixtures: Digest,
    pub manifest: Digest,
    pub environment: String,
    pub allow: Vec<String>,
    pub cases: Vec<CaseResult>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum ExpectedStatus {
    Succeeded,
    Skipped,
    Denied,
    AwaitingApproval,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub stdout: String,
    pub result: Value,
    pub status: ExpectedStatus,
    pub exit_code: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureCase {
    pub name: String,
    pub input: std::collections::BTreeMap<Identifier, String>,
    pub allow: Vec<String>,
    pub expect: Expectation,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureSuite {
    pub schema: FixturesSchema,
    pub cases: Vec<FixtureCase>,
}

/// Consumers must not mistake a normalized document for a checked executable.
pub trait WireDocument: Serialize + DeserializeOwned {
    fn normalize(&mut self) -> Result<()>;
}

pub fn decode<T: WireDocument>(value: serde_json::Value) -> Result<T> {
    let mut document: T =
        serde_json::from_value(value).map_err(|e| Error::new("E_CONTRACT", e.to_string()))?;
    document.normalize()?;
    Ok(document)
}

pub fn normalized_json(bytes: &[u8]) -> Result<serde_json::Value> {
    let value = canonical::parse(bytes)?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::new("E_SCHEMA", "a versioned document needs a schema field"))?;
    macro_rules! decoded {
        ($t:ty) => {
            serde_json::to_value(decode::<$t>(value)?)
                .map_err(|e| Error::new("E_CONTRACT", e.to_string()))
        };
    }
    match schema {
        "nexa.memory-source/0.3" => decoded!(SourceManifest),
        "nexa.memory/0.3" => decoded!(MemoryRecord),
        "nexa.blueprint-draft/0.3" => decoded!(BlueprintDraft),
        "nexa.program/0.3" => decoded!(ProgramSpec),
        "nexa.lock/0.3" => decoded!(LockRecord),
        "nexa.application/0.3" => decoded!(ApplicationManifest),
        "nexa.receipt/0.3" => decoded!(Receipt),
        "nexa.evidence/0.3" => decoded!(EvidenceReport),
        "nexa.fixtures/0.3" => decoded!(FixtureSuite),
        "nexa.kernel/0.3" => decoded!(crate::kernel::KernelDescriptor),
        _ => Err(Error::new(
            "E_SCHEMA",
            format!("unsupported schema: {schema}"),
        )),
    }
}

fn unique<T: Ord + Clone>(values: impl IntoIterator<Item = T>, path: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(Error::new("E_DUPLICATE", "duplicate entry").at(path));
        }
    }
    Ok(())
}

fn set<T: Ord + Clone>(values: &mut [T], path: &str) -> Result<()> {
    unique(values.iter().cloned(), path)?;
    values.sort();
    Ok(())
}

fn ports(values: &[Port], path: &str) -> Result<()> {
    unique(values.iter().map(|port| port.name.clone()), path)
}

fn path_name(value: &str) -> Result<()> {
    if value.split('.').all(identifier_shape) {
        Ok(())
    } else {
        Err(Error::new(
            "E_NAME",
            format!("invalid qualified name: {value}"),
        ))
    }
}

fn selector(value: &str) -> Result<()> {
    if value.starts_with("sha256:") {
        value.parse::<Digest>()?;
    } else if let Some((name, version)) = value.split_once('@') {
        path_name(name)?;
        if version.is_empty() || version.contains(['@', '/', '\\', '\0']) {
            return Err(Error::new(
                "E_SELECTOR",
                "memory selector needs a nonempty version label",
            ));
        }
    } else {
        return Err(Error::new(
            "E_SELECTOR",
            "expected name@version or an immutable digest",
        ));
    }
    Ok(())
}

fn relative_path(value: &str) -> Result<()> {
    if !value.is_empty()
        && !value.contains(['\\', ':', '\0'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        Ok(())
    } else {
        Err(Error::new("E_PATH", "source path must be a confined relative path").at(value))
    }
}

fn graph_common(
    inputs: &[Port],
    output: &Port,
    node_ids: Vec<Identifier>,
    connections: &mut [Connection],
    orders: &mut [OrderEdge],
) -> Result<()> {
    ports(inputs, "inputs")?;
    if output.name.as_str() != "result" {
        return Err(Error::new(
            "E_OUTPUT",
            "the output port must be named result",
        ));
    }
    unique(node_ids.clone(), "nodes")?;
    let nodes: BTreeSet<_> = node_ids.into_iter().collect();
    let input_names: BTreeSet<_> = inputs.iter().map(|p| p.name.clone()).collect();
    set(connections, "connections")?;
    let mut destinations = BTreeSet::new();
    for edge in connections.iter() {
        match &edge.from {
            Endpoint::Input { port } if input_names.contains(port) => {}
            Endpoint::Node { node, port } if nodes.contains(node) && port.as_str() == "result" => {}
            _ => return Err(Error::new("E_ENDPOINT", "invalid source endpoint")),
        }
        match &edge.to {
            Endpoint::Output { port } if port.as_str() == "result" => {}
            Endpoint::Node { node, .. } if nodes.contains(node) => {}
            _ => return Err(Error::new("E_ENDPOINT", "invalid destination endpoint")),
        }
        if !destinations.insert(edge.to.clone()) {
            return Err(Error::new(
                "E_PRODUCER",
                "each destination may have only one producer",
            ));
        }
    }
    if !destinations.contains(&Endpoint::Output {
        port: output.name.clone(),
    }) {
        return Err(Error::new(
            "E_OUTPUT",
            "the result port must be connected exactly once",
        ));
    }
    set(orders, "order_edges")?;
    for edge in orders {
        if !nodes.contains(&edge.before) || !nodes.contains(&edge.after) {
            return Err(Error::new(
                "E_ENDPOINT",
                "order edge references an unknown node",
            ));
        }
    }
    // Cycle detection and callable-dependent parameter/type checks belong to M4.
    Ok(())
}

impl WireDocument for SourceManifest {
    fn normalize(&mut self) -> Result<()> {
        path_name(&self.name)?;
        path_name(&self.module)?;
        if self.version.is_empty() {
            return Err(Error::new("E_VERSION", "version label cannot be empty"));
        }
        selector(&self.kernel)?;
        if self.sources.is_empty() {
            return Err(Error::new(
                "E_SOURCE",
                "at least one source file is required",
            ));
        }
        for source in &self.sources {
            relative_path(source)?;
        }
        set(&mut self.sources, "sources")?;
        unique(
            self.dependencies.iter().map(|d| d.alias.clone()),
            "dependencies",
        )?;
        for dependency in &self.dependencies {
            selector(&dependency.selector)?;
        }
        self.dependencies.sort_by(|a, b| a.alias.cmp(&b.alias));
        Ok(())
    }
}

impl CallableContract {
    pub fn normalize(&mut self) -> Result<()> {
        ports(&self.parameters, "parameters")?;
        set(&mut self.effects, "effects")?;
        for effect in &self.effects {
            path_name(effect)?;
        }
        set(&mut self.required_bindings, "required_bindings")?;
        set(&mut self.capability_requirements, "capability_requirements")?;
        if self.kind == CallableKind::Function
            && (!self.effects.is_empty()
                || !self.capability_requirements.is_empty()
                || !self.required_bindings.is_empty())
        {
            return Err(Error::new(
                "E_PURITY",
                "a pure callable contract cannot require effects or bindings",
            ));
        }
        Ok(())
    }
}

impl WireDocument for MemoryRecord {
    fn normalize(&mut self) -> Result<()> {
        if self.exports.len() != 1 {
            return Err(Error::new(
                "E_EXPORT",
                "slice0 memory exports exactly one callable",
            ));
        }
        self.exports[0].normalize()?;
        set(&mut self.dependencies, "dependencies")?;
        unique(self.source_map.iter().map(|s| s.path.clone()), "source_map")?;
        self.source_map.sort_by(|a, b| a.path.cmp(&b.path));
        for source in &self.source_map {
            relative_path(&source.path)?;
        }
        if let Implementation::Source { entry } = &self.implementation
            && (self.source_map.is_empty() || entry != &self.exports[0].export_name)
        {
            return Err(Error::new(
                "E_SOURCE",
                "source memory needs source blobs and a matching entry export",
            ));
        }
        Ok(())
    }
}

impl WireDocument for BlueprintDraft {
    fn normalize(&mut self) -> Result<()> {
        path_name(&self.entry)?;
        for node in &self.nodes {
            selector(&node.selector)?;
        }
        graph_common(
            &self.inputs,
            &self.output,
            self.nodes.iter().map(|n| n.id.clone()).collect(),
            &mut self.connections,
            &mut self.order_edges,
        )?;
        self.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }
}

impl WireDocument for ProgramSpec {
    fn normalize(&mut self) -> Result<()> {
        path_name(&self.entry)?;
        graph_common(
            &self.inputs,
            &self.output,
            self.nodes.iter().map(|n| n.id.clone()).collect(),
            &mut self.connections,
            &mut self.order_edges,
        )?;
        self.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        ports(&self.derived.inputs, "derived.inputs")?;
        if self.derived.inputs != self.inputs || self.derived.result_type != self.output.r#type {
            return Err(Error::new(
                "E_DERIVED",
                "derived input/result contract disagrees with graph interface",
            ));
        }
        set(&mut self.derived.dependencies, "derived.dependencies")?;
        set(&mut self.derived.effects, "derived.effects")?;
        set(
            &mut self.derived.required_bindings,
            "derived.required_bindings",
        )?;
        set(
            &mut self.derived.capability_requirements,
            "derived.capability_requirements",
        )?;
        // Effect/dependency summaries cannot be trusted until M3/M4 load exports.
        Ok(())
    }
}

impl WireDocument for LockRecord {
    fn normalize(&mut self) -> Result<()> {
        for binding in &self.bindings {
            selector(&binding.selector)?;
        }
        unique(
            self.bindings
                .iter()
                .map(|b| (b.selector.clone(), b.export.clone())),
            "bindings",
        )?;
        self.bindings
            .sort_by(|a, b| (&a.selector, &a.export).cmp(&(&b.selector, &b.export)));
        set(&mut self.dependencies, "dependencies")
    }
}

impl WireDocument for ApplicationManifest {
    fn normalize(&mut self) -> Result<()> {
        if self.principal.is_empty() {
            return Err(Error::new("E_PRINCIPAL", "principal label cannot be empty"));
        }
        set(&mut self.requests, "requests")?;
        for request in &self.requests {
            if request.capability != "console.write" || request.resource != "stdout" {
                return Err(Error::new(
                    "E_UNSUPPORTED",
                    "slice0 supports only console.write on stdout",
                ));
            }
        }
        for rule in &self.policies {
            if rule.operation != "std.console.write_line" || rule.resource != "stdout" {
                return Err(Error::new(
                    "E_UNSUPPORTED",
                    "slice0 policy targets only std.console.write_line on stdout",
                ));
            }
        }
        Ok(())
    }
}

impl WireDocument for Receipt {
    fn normalize(&mut self) -> Result<()> {
        unique(self.nodes.iter().map(|n| n.path.clone()), "nodes")
        // Receipt order is historical evidence and MUST NOT be sorted.
    }
}

impl WireDocument for EvidenceReport {
    fn normalize(&mut self) -> Result<()> {
        unique(self.cases.iter().map(|c| c.name.clone()), "cases")?;
        set(&mut self.allow, "allow")
    }
}

impl WireDocument for FixtureSuite {
    fn normalize(&mut self) -> Result<()> {
        unique(self.cases.iter().map(|c| c.name.clone()), "cases")?;
        for case in &mut self.cases {
            set(&mut case.allow, "allow")?;
            if case.allow.iter().any(|a| a != "stdout") {
                return Err(Error::new(
                    "E_UNSUPPORTED",
                    "slice0 fixture may allow only stdout",
                ));
            }
            if case.expect.exit_code > 5 {
                return Err(Error::new(
                    "E_EXIT",
                    "fixture exit code must be between 0 and 5",
                ));
            }
        }
        Ok(())
    }
}
