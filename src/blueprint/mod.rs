//! Typed blueprint parsing, checking, inspection, and graph execution.

mod checker;
mod inspect;
mod parser;
mod runner;

pub mod nested;

use crate::{
    contracts::{CallableContract, Node, ProgramSpec},
    digest::Digest,
    source::CheckedModule,
    value::Identifier,
};
use std::collections::BTreeMap;

pub use checker::{compose, compose_checked, compose_parsed, compose_parsed_checked, load_checked};
pub use inspect::{NodeInspection, ProgramInspection, inspect, inspect_page};
pub use parser::{
    BlueprintSourceMap, ParsedBlueprint, decode_request, parse_source, parse_source_with_spans,
};
pub use runner::run;

/// A checked graph is immutable and contains only exact, verified memory
/// implementations. Its serialized ProgramSpec has been re-derived against
/// each selected export before this value can be constructed.
pub struct CheckedProgram {
    pub(crate) spec: ProgramSpec,
    pub(crate) digest: Digest,
    pub(crate) contract: CallableContract,
    pub(crate) nodes: BTreeMap<Identifier, CheckedNode>,
    pub(crate) blueprint_source_map: Option<BlueprintSourceMap>,
}

impl CheckedProgram {
    pub fn program(&self) -> &ProgramSpec {
        &self.spec
    }

    pub fn program_digest(&self) -> &Digest {
        &self.digest
    }

    pub fn contract(&self) -> &CallableContract {
        &self.contract
    }

    pub fn export(&self) -> &CallableContract {
        &self.contract
    }

    pub fn module_name(&self) -> &str {
        self.spec
            .entry
            .rsplit_once('.')
            .map_or(self.spec.entry.as_str(), |(module, _)| module)
    }

    pub fn dependency_closure(&self) -> &[Digest] {
        &self.spec.derived.dependencies
    }

    pub fn blueprint_source_map(&self) -> Option<&BlueprintSourceMap> {
        self.blueprint_source_map.as_ref()
    }
}

pub(crate) struct CheckedNode {
    pub node: Node,
    pub contract: CallableContract,
    pub implementation: ResolvedImplementation,
    pub source_references: Vec<SourceReference>,
    pub dependency_closure: Vec<Digest>,
}

pub(crate) enum ResolvedImplementation {
    Source(CheckedModule),
    Program(Box<CheckedProgram>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReference {
    pub memory: Digest,
    pub path: String,
    pub blob: Digest,
    pub module: String,
}
