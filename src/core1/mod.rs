//! Experimental pure Core1 source memories. Imports are exact content addresses.

mod ast;
mod engine;
mod parser;

pub mod blueprint;

use crate::{
    canonical,
    contracts::WireDocument,
    diagnostic::{Error, Result},
    digest::Digest,
    memory::Store,
};
use engine::CheckedModule;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
    sync::Arc,
};

pub const SOURCE_LIMIT: usize = 128 * 1024;
pub const INPUT_LIMIT: usize = 1024 * 1024;
pub const DEFAULT_FUEL: u64 = 5_000_000;
pub const MAX_FUEL: u64 = 20_000_000;
const MAX_IMPORTS: usize = 32;
const MAX_DEPTH: usize = 32;
const MAX_MEMORIES: usize = 128;
const MAX_CLOSURE_SOURCE: usize = 4 * 1024 * 1024;
const MAX_CLOSURE_RECORDS: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    schema: Schema,
    profile: Profile,
    pub implementation: Digest,
    pub module: String,
    pub source: Digest,
    pub imports: BTreeMap<String, Digest>,
    pub type_identity: Digest,
    pub signature: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum Schema {
    #[serde(rename = "nexa.core1-memory/0.1")]
    V01,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum Profile {
    #[serde(rename = "core1")]
    Core1,
}

impl WireDocument for Memory {
    fn normalize(&mut self) -> Result<()> {
        if self.imports.len() > MAX_IMPORTS {
            return Err(Error::new("E_LIMIT", "Core1 memory has too many imports"));
        }
        Ok(())
    }
}

pub fn seal(
    store: &Store,
    source_id: &str,
    source: &str,
    imports: BTreeMap<String, Digest>,
) -> Result<Digest> {
    if source.len() > SOURCE_LIMIT || imports.len() > MAX_IMPORTS {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 source or imports exceed the limit",
        ));
    }
    let module = parser::parse(source_id, source)?;
    let source_digest = Digest::of_bytes(source.as_bytes());
    let implementation: Digest = env!("NEXA_IMPLEMENTATION_DIGEST").parse()?;
    let type_identity = type_identity(&implementation, &module.name, &source_digest, &imports)?;
    let mut memory = Memory {
        schema: Schema::V01,
        profile: Profile::Core1,
        implementation,
        module: module.name.clone(),
        source: source_digest,
        imports,
        type_identity: type_identity.clone(),
        signature: Value::Null,
    };
    let mut loader = Loader::new(store);
    loader.source_bytes = source.len();
    loader.memory_slots -= 1;
    check_import_names(&module, &memory.imports)?;
    let dependencies = loader.imports(&memory.imports, 1)?;
    let checked = engine::check(module, dependencies, type_identity)?;
    memory.signature = signature(&checked)?;
    let document = serde_json::to_value(&memory)
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
    let bytes = canonical::encode(&document)?;
    if bytes.len() > INPUT_LIMIT || loader.record_bytes + bytes.len() > MAX_CLOSURE_RECORDS {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 memory record exceeds the limit",
        ));
    }
    store.put_raw(source.as_bytes())?;
    store.put_raw(&bytes)
}

pub fn inspect(store: &Store, digest: &Digest) -> Result<Memory> {
    let (record, _) = Loader::new(store).load(digest, 0)?;
    Ok(record)
}

/// Verify the complete import closure and return each immutable record by digest.
pub fn inspect_closure(store: &Store, digest: &Digest) -> Result<BTreeMap<Digest, Memory>> {
    let mut loader = Loader::new(store);
    loader.load(digest, 0)?;
    Ok(loader
        .loaded
        .into_iter()
        .map(|(address, (memory, _))| (address, memory))
        .collect())
}

pub fn run(store: &Store, digest: &Digest, args: &Value, fuel: u64) -> Result<Value> {
    if !(1..=MAX_FUEL).contains(&fuel) {
        return Err(Error::new(
            "E_LIMIT",
            format!("fuel must be between 1 and {MAX_FUEL}"),
        ));
    }
    let (_, checked) = Loader::new(store).load(digest, 0)?;
    checked.evaluate(args, fuel)
}

fn type_identity(
    implementation: &Digest,
    module: &str,
    source: &Digest,
    imports: &BTreeMap<String, Digest>,
) -> Result<Digest> {
    canonical::digest(&serde_json::json!({
        "schema": "nexa.core1-type-namespace/0.1",
        "implementation": implementation,
        "module": module,
        "source": source,
        "imports": imports,
    }))
}

fn signature(checked: &CheckedModule) -> Result<Value> {
    serde_json::to_value(checked.signature())
        .map_err(|error| Error::new("E_CONTRACT", error.to_string()))
}

fn check_import_names(module: &ast::Module, imports: &BTreeMap<String, Digest>) -> Result<()> {
    let names: BTreeSet<_> = module.imports.iter().collect();
    if names.len() != module.imports.len() || names.into_iter().ne(imports.keys()) {
        return Err(Error::new(
            "E_IMPORT",
            "supply exactly one digest for each declared import",
        ));
    }
    Ok(())
}

struct Loader<'a> {
    store: &'a Store,
    active: BTreeSet<Digest>,
    loaded: BTreeMap<Digest, (Memory, Arc<CheckedModule>)>,
    source_bytes: usize,
    record_bytes: usize,
    memory_slots: usize,
}

impl<'a> Loader<'a> {
    fn new(store: &'a Store) -> Self {
        Self {
            store,
            active: BTreeSet::new(),
            loaded: BTreeMap::new(),
            source_bytes: 0,
            record_bytes: 0,
            memory_slots: MAX_MEMORIES,
        }
    }

    fn imports(
        &mut self,
        pinned: &BTreeMap<String, Digest>,
        depth: usize,
    ) -> Result<BTreeMap<String, Arc<CheckedModule>>> {
        pinned
            .iter()
            .map(|(name, digest)| {
                let (_, checked) = self.load(digest, depth)?;
                Ok((name.clone(), checked))
            })
            .collect()
    }

    fn load(&mut self, digest: &Digest, depth: usize) -> Result<(Memory, Arc<CheckedModule>)> {
        if depth >= MAX_DEPTH {
            return Err(Error::new(
                "E_LIMIT",
                "Core1 import depth exceeds the limit",
            ));
        }
        if let Some(loaded) = self.loaded.get(digest) {
            return Ok(loaded.clone());
        }
        if self.memory_slots == 0 {
            return Err(Error::new(
                "E_LIMIT",
                "Core1 dependency closure exceeds the limit",
            ));
        }
        if !self.active.insert(digest.clone()) {
            return Err(Error::new("E_IMPORT", "cyclic Core1 memory imports"));
        }
        self.memory_slots -= 1;
        let contents = self.load_checked(digest, depth);
        self.active.remove(digest);
        let contents = contents?;
        self.loaded.insert(digest.clone(), contents.clone());
        Ok(contents)
    }

    fn load_checked(
        &mut self,
        digest: &Digest,
        depth: usize,
    ) -> Result<(Memory, Arc<CheckedModule>)> {
        let bytes = self.read_object(digest, INPUT_LIMIT)?;
        self.record_bytes += bytes.len();
        if self.record_bytes > MAX_CLOSURE_RECORDS {
            return Err(Error::new(
                "E_LIMIT",
                "Core1 dependency records exceed the limit",
            ));
        }
        let document = canonical::parse(&bytes)?;
        let record: Memory = crate::contracts::decode(document)?;
        let normalized = serde_json::to_value(&record)
            .map_err(|error| Error::new("E_CONTRACT", error.to_string()))?;
        if canonical::encode(&normalized)? != bytes {
            return Err(
                Error::new("E_CANONICAL", "noncanonical Core1 memory record")
                    .at(digest.to_string()),
            );
        }
        if record.implementation.as_str() != env!("NEXA_IMPLEMENTATION_DIGEST") {
            return Err(
                Error::new("E_KERNEL", "Core1 memory pins a different implementation")
                    .at(digest.to_string()),
            );
        }
        let source = self.read_object(&record.source, SOURCE_LIMIT)?;
        self.source_bytes += source.len();
        if self.source_bytes > MAX_CLOSURE_SOURCE {
            return Err(Error::new(
                "E_LIMIT",
                "Core1 dependency source exceeds the limit",
            ));
        }
        let text = std::str::from_utf8(&source).map_err(|error| {
            Error::new("E_UTF8", error.to_string()).at(record.source.to_string())
        })?;
        let module = parser::parse(record.source.as_str(), text)?;
        if module.name != record.module {
            return Err(Error::new(
                "E_CONTRACT",
                "stored module name disagrees with source",
            ));
        }
        check_import_names(&module, &record.imports)?;
        let deps = self.imports(&record.imports, depth + 1)?;
        let identity = type_identity(
            &record.implementation,
            &record.module,
            &record.source,
            &record.imports,
        )?;
        if identity != record.type_identity {
            return Err(Error::new(
                "E_CONTRACT",
                "stored type identity disagrees with source and imports",
            ));
        }
        let checked = engine::check(module, deps, identity)?;
        if signature(&checked)? != record.signature {
            return Err(Error::new(
                "E_CONTRACT",
                "stored signature disagrees with checked source",
            ));
        }
        Ok((record, Arc::new(checked)))
    }

    fn read_object(&self, digest: &Digest, limit: usize) -> Result<Vec<u8>> {
        let path = self.store.object_path(digest);
        let bytes = read_bounded(&path, limit)?;
        digest
            .verify(&bytes)
            .map_err(|err| err.at(digest.to_string()))?;
        Ok(bytes)
    }
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let io_error = |error: std::io::Error| {
        Error::new("E_IO", error.to_string()).at(path.display().to_string())
    };
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > limit {
        return Err(Error::new("E_LIMIT", format!("file exceeds {limit} bytes"))
            .at(path.display().to_string()));
    }
    Ok(bytes)
}
