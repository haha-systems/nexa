//! Sealing checked blueprint compositions as reusable memory records.

use super::Store;
use crate::{
    blueprint,
    contracts::{
        CallableContract, Implementation, MemoryMetadata, MemoryRecord, MemorySchema, ProgramSpec,
    },
    diagnostic::{Error, Result},
    digest::Digest,
    value::Identifier,
};

/// Store a validated ProgramSpec as an immutable memory and register its
/// name/version selector. The callable interface and dependency closure come
/// from the checked graph, never from its serialized summary alone.
pub fn seal_program(
    spec: &ProgramSpec,
    store: &Store,
    name: &str,
    version: &str,
) -> Result<Digest> {
    let checked = blueprint::load_checked(spec, store)?;

    let program = store.put_document(spec)?;
    if &program != checked.program_digest() {
        return Err(Error::new(
            "E_INTEGRITY",
            "stored ProgramSpec identity disagrees with its checked graph",
        ));
    }
    let export = checked.contract().clone();
    if export.export_name != local_entry_name(&spec.entry)? {
        return Err(Error::new(
            "E_CONTRACT",
            "checked program export name disagrees with its entry name",
        ));
    }

    let record = MemoryRecord {
        schema: MemorySchema::V03,
        language: spec.language,
        profile: spec.profile,
        implementation: Implementation::Program { program },
        exports: vec![export],
        dependencies: checked.dependency_closure().to_vec(),
        kernel: spec.kernel.clone(),
        source_map: Vec::new(),
        metadata: MemoryMetadata {
            name: name.to_owned(),
            version: version.to_owned(),
            description: String::new(),
            provenance: Vec::new(),
        },
    };

    let memory = store.put_memory(&record)?;
    store.set_alias(&format!("{name}@{version}"), &memory)?;
    Ok(memory)
}

/// Recheck a program-backed MemoryRecord against its referenced graph. This is
/// used by inspection and nested-node resolution so both paths enforce the
/// same derived contract and dependency rules.
pub fn validate_program_record(record: &MemoryRecord, store: &Store) -> Result<()> {
    let program = match &record.implementation {
        Implementation::Program { program } => program,
        Implementation::Source { .. } => {
            return Err(Error::new(
                "E_IMPLEMENTATION",
                "memory does not contain a blueprint program",
            ));
        }
    };
    let spec: ProgramSpec = store.load_document(program)?;
    let checked = blueprint::load_checked(&spec, store)?;
    validate_checked_program_record(record, &spec, &checked, store)
}

pub(crate) fn validate_checked_program_record(
    record: &MemoryRecord,
    spec: &ProgramSpec,
    checked: &blueprint::CheckedProgram,
    store: &Store,
) -> Result<()> {
    let Implementation::Program { program } = &record.implementation else {
        return Err(Error::new(
            "E_IMPLEMENTATION",
            "memory does not contain a blueprint program",
        ));
    };

    if checked.program_digest() != program {
        return Err(Error::new(
            "E_INTEGRITY",
            "checked program identity disagrees with the memory reference",
        ));
    }

    if record.language != spec.language
        || record.profile != spec.profile
        || record.kernel != spec.kernel
    {
        return Err(Error::new(
            "E_CONTRACT",
            "blueprint memory metadata disagrees with its stored program",
        ));
    }

    if &record.kernel != store.kernel_digest() || &spec.kernel != store.kernel_digest() {
        return Err(Error::new(
            "E_KERNEL",
            "blueprint memory does not pin the installed kernel",
        ));
    }

    if record.dependencies.as_slice() != checked.dependency_closure() {
        return Err(Error::new(
            "E_DEPENDENCY",
            "blueprint memory dependency closure disagrees with its program",
        ));
    }

    let exported = record
        .exports
        .first()
        .ok_or_else(|| Error::new("E_EXPORT", "blueprint memory has no callable export"))?;
    if record.exports.len() != 1 || !same_callable(exported, checked.contract()) {
        return Err(Error::new(
            "E_CONTRACT",
            "blueprint memory export disagrees with its checked program contract",
        ));
    }

    Ok(())
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

fn local_entry_name(entry: &str) -> Result<Identifier> {
    let local = entry
        .rsplit('.')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error::new("E_NAME", "blueprint entry has no local name"))?;
    Identifier::new(local.to_owned())
}
