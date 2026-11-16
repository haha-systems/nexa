use super::Store;
use crate::{
    contracts::{
        Implementation, Language, MemoryMetadata, MemoryRecord, MemorySchema, SourceBlob,
        SourceManifest, WireDocument,
    },
    diagnostic::{Error, Result},
    digest::Digest,
    source,
};
use std::{fs, path::Path};

/// Check and seal one source file described by `memory.json`.
///
/// Source text is retained byte-for-byte. This routine parses/checks it only;
/// no exported function or action is invoked while sealing.
pub fn seal_source(directory: impl AsRef<Path>, store: &Store) -> Result<Digest> {
    let directory = directory.as_ref();
    let root = fs::canonicalize(directory).map_err(|e| {
        Error::new("E_IO", format!("resolve source directory: {e}"))
            .at(directory.display().to_string())
    })?;
    if !root.is_dir() {
        return Err(
            Error::new("E_SOURCE", "source memory path must be a directory")
                .at(root.display().to_string()),
        );
    }

    let manifest_path = root.join("memory.json");
    let manifest_real = fs::canonicalize(&manifest_path).map_err(|e| {
        Error::new("E_IO", format!("resolve source manifest: {e}"))
            .at(manifest_path.display().to_string())
    })?;
    if !manifest_real.starts_with(&root) {
        return Err(Error::new(
            "E_PATH",
            "source manifest symlink escapes the memory directory",
        )
        .at(manifest_path.display().to_string()));
    }
    let manifest_bytes = fs::read(&manifest_real).map_err(|e| {
        Error::new("E_IO", format!("read source manifest: {e}"))
            .at(manifest_real.display().to_string())
    })?;
    let manifest_value = crate::canonical::parse(&manifest_bytes)?;
    let mut manifest: SourceManifest = crate::contracts::decode(manifest_value)?;
    manifest.normalize()?;

    if manifest.sources.len() != 1 {
        return Err(Error::new(
            "E_SOURCE_BUNDLE",
            "slice0 sealing supports exactly one source file; multiple files are rejected",
        ));
    }

    let selected_kernel = store.resolve(&manifest.kernel)?;
    if &selected_kernel != store.kernel_digest() {
        return Err(Error::new(
            "E_KERNEL",
            "source manifest must select the installed std.slice0 kernel",
        ));
    }

    let relative = Path::new(&manifest.sources[0]);
    let source_path = root.join(relative);
    let source_real = fs::canonicalize(&source_path).map_err(|e| {
        Error::new("E_IO", format!("resolve source file: {e}"))
            .at(source_path.display().to_string())
    })?;
    if !source_real.starts_with(&root) {
        return Err(
            Error::new("E_PATH", "source symlink escapes the memory directory")
                .at(source_path.display().to_string()),
        );
    }
    if !source_real.is_file() {
        return Err(Error::new("E_SOURCE", "manifest source path is not a file")
            .at(source_path.display().to_string()));
    }
    let source_bytes = fs::read(&source_real).map_err(|e| {
        Error::new("E_IO", format!("read source file: {e}")).at(source_real.display().to_string())
    })?;
    let source_text = std::str::from_utf8(&source_bytes)
        .map_err(|e| Error::new("E_UTF8", e.to_string()).at(manifest.sources[0].clone()))?;
    let checked = source::check(&manifest.sources[0], source_text)?;
    if checked.module_name() != manifest.module {
        return Err(Error::new(
            "E_MODULE",
            "source module declaration does not match the source manifest",
        )
        .at(manifest.module));
    }
    if checked.exports().len() != 1 {
        return Err(Error::new(
            "E_EXPORT",
            "a source memory must export exactly one callable",
        ));
    }
    let contract = checked.contract(manifest.entry.as_str())?;

    let mut roots = Vec::with_capacity(manifest.dependencies.len());
    for dependency in &manifest.dependencies {
        let digest = store.resolve(&dependency.selector)?;
        store.load_memory(&digest)?;
        roots.push(digest);
    }
    roots.sort();
    roots.dedup();
    let dependencies = store.dependency_closure(&roots)?;

    let blob = store.put_raw(&source_bytes)?;
    let record = MemoryRecord {
        schema: MemorySchema::V03,
        language: Language::V03,
        profile: crate::contracts::Profile::Slice0,
        implementation: Implementation::Source {
            entry: manifest.entry.clone(),
        },
        exports: vec![contract],
        dependencies,
        kernel: store.kernel_digest().clone(),
        source_map: vec![SourceBlob {
            path: manifest.sources[0].clone(),
            blob,
        }],
        metadata: MemoryMetadata {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: String::new(),
            provenance: Vec::new(),
        },
    };
    let digest = store.put_memory(&record)?;
    store.set_alias(&format!("{}@{}", manifest.name, manifest.version), &digest)?;
    Ok(digest)
}
