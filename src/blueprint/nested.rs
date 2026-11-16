//! Loading and checking the executable graph inside a program memory.

use super::{CheckedProgram, checker::load_checked_with_ancestors};
use crate::{
    contracts::{Implementation, ProgramSpec},
    diagnostic::{Error, Result},
    digest::Digest,
    memory::Store,
};
use std::collections::BTreeSet;

/// Resolve a program-backed memory while preserving the ancestor set used to
/// reject recursive memory graphs. Source memories are handled by the regular
/// node resolver and cannot be passed here.
pub(crate) fn load_nested_program(
    store: &Store,
    memory: &Digest,
    visiting: &mut BTreeSet<Digest>,
) -> Result<CheckedProgram> {
    if !visiting.insert(memory.clone()) {
        return Err(Error::new(
            "E_CYCLE",
            "blueprint memory dependency cycle detected",
        ));
    }

    let result = load_nested_program_inner(store, memory, visiting);
    visiting.remove(memory);
    result
}

fn load_nested_program_inner(
    store: &Store,
    memory: &Digest,
    visiting: &mut BTreeSet<Digest>,
) -> Result<CheckedProgram> {
    let record = store.load_memory_local(memory)?;
    let program_digest = match &record.implementation {
        Implementation::Program { program } => program,
        Implementation::Source { .. } => {
            return Err(Error::new(
                "E_IMPLEMENTATION",
                "memory does not contain a blueprint program",
            ));
        }
    };

    let spec: ProgramSpec = store.load_document(program_digest)?;
    let checked = load_checked_with_ancestors(&spec, store, visiting)?;

    crate::memory::program::validate_checked_program_record(&record, &spec, &checked, store)?;

    Ok(checked)
}
