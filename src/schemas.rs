//! JSON Schemas are generated from the same Rust wire types used by the CLI.
//! Cross-field and resolution-dependent rules still need semantic validation.

use crate::{
    contracts::*,
    diagnostic::{Error, Result},
};
use serde_json::Value;

pub const NAMES: &[&str] = &[
    "memory-source",
    "memory",
    "blueprint-draft",
    "program",
    "lock",
    "application",
    "receipt",
    "evidence",
    "fixtures",
    "kernel",
    "diagnostic",
    "value",
];

pub fn named(name: &str) -> Result<Value> {
    macro_rules! schema {
        ($t:ty) => {
            serde_json::to_value(schemars::schema_for!($t))
                .map_err(|e| Error::new("E_SCHEMA", e.to_string()))?
        };
    }
    let mut value = match name {
        "memory-source" => schema!(SourceManifest),
        "memory" => schema!(MemoryRecord),
        "blueprint-draft" => schema!(BlueprintDraft),
        "program" => schema!(ProgramSpec),
        "lock" => schema!(LockRecord),
        "application" => schema!(ApplicationManifest),
        "receipt" => schema!(Receipt),
        "evidence" => schema!(EvidenceReport),
        "fixtures" => schema!(FixtureSuite),
        "kernel" => schema!(crate::kernel::KernelDescriptor),
        "diagnostic" => schema!(crate::diagnostic::Diagnostic),
        "value" => schema!(crate::value::Value),
        _ => return Err(Error::new("E_SCHEMA", format!("unknown schema: {name}"))),
    };
    value["$id"] = format!("https://nexa.invalid/schemas/{name}-0.3.schema.json").into();
    Ok(value)
}
