use crate::{
    canonical,
    contracts::{
        self, CallableContract, CallableKind, EvidenceReport, Implementation, Language,
        LockBinding, LockRecord, MemoryRecord, Profile, ProgramSpec, WireDocument,
    },
    diagnostic::{Error, Result},
    digest::Digest,
    kernel::KernelDescriptor,
    source::{self, CheckedModule},
    value::Identifier,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    kernel: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema: String,
    aliases: Vec<CatalogAlias>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogAlias {
    label: String,
    memory: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceIndex {
    schema: String,
    subjects: Vec<EvidenceSubject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceSubject {
    subject: Digest,
    reports: Vec<Digest>,
}

#[derive(Debug)]
struct SidecarLock {
    path: PathBuf,
    file: Option<File>,
}

impl Drop for SidecarLock {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(path.as_ref())
            .map_err(|e| io_error("create store", path.as_ref(), e))?;
        let root = fs::canonicalize(path.as_ref())
            .map_err(|e| io_error("resolve store", path.as_ref(), e))?;
        let object_dir = root.join("objects").join("sha256");
        fs::create_dir_all(&object_dir)
            .map_err(|e| io_error("create object store", &object_dir, e))?;

        let descriptor = KernelDescriptor::bundled()?;
        let descriptor_value = serde_value(&descriptor)?;
        let expected = canonical::digest(&descriptor_value)?;
        let store = Self {
            root,
            kernel: expected.clone(),
        };
        let written = store.put_document(&descriptor)?;
        let loaded: KernelDescriptor = store.load_document(&expected)?;
        if written != expected || serde_value(&loaded)? != descriptor_value {
            return Err(Error::new(
                "E_KERNEL",
                "stored kernel descriptor does not match the installed implementation",
            ));
        }
        store.ensure_catalog()?;
        store.ensure_evidence_index()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kernel_digest(&self) -> &Digest {
        &self.kernel
    }

    /// Return the currently selected memory aliases without loading or
    /// validating the referenced records. Callers that present memories to a
    /// user should load each digest with `load_memory` before trusting it.
    pub fn catalog_aliases(&self) -> Result<Vec<(String, Digest)>> {
        Ok(self
            .read_catalog()?
            .aliases
            .into_iter()
            .map(|alias| (alias.label, alias.memory))
            .collect())
    }

    /// Exposes the deterministic object location for diagnostics and tests.
    pub fn object_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join("objects")
            .join("sha256")
            .join(&digest.as_str()[7..])
    }

    /// Store exact bytes at their SHA-256 address. Existing addresses are immutable.
    pub fn put_raw(&self, bytes: &[u8]) -> Result<Digest> {
        let digest = Digest::of_bytes(bytes);
        let target = self.object_path(&digest);
        if target.exists() {
            let existing = self.read_object(&digest)?;
            if existing != bytes {
                return Err(Error::new(
                    "E_IMMUTABLE",
                    "an existing content address contains different bytes",
                )
                .at(digest.to_string()));
            }
            return Ok(digest);
        }

        let parent = target.parent().expect("object path has a parent");
        fs::create_dir_all(parent).map_err(|e| io_error("create object directory", parent, e))?;
        let temporary = unique_temp(parent, "object");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| io_error("create temporary object", &temporary, e))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| io_error("write object", &temporary, e))?;
        drop(file);

        match fs::hard_link(&temporary, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.read_object(&digest)?;
                if existing != bytes {
                    let _ = fs::remove_file(&temporary);
                    return Err(Error::new(
                        "E_IMMUTABLE",
                        "an existing content address contains different bytes",
                    )
                    .at(digest.to_string()));
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(io_error("install object", &target, error));
            }
        }
        fs::remove_file(&temporary)
            .map_err(|e| io_error("remove temporary object", &temporary, e))?;
        Ok(digest)
    }

    /// Load raw bytes and verify the filename's digest on every read.
    pub fn get_raw(&self, digest: &Digest) -> Result<Vec<u8>> {
        self.read_object(digest)
    }

    pub fn put_document<T: WireDocument + Clone>(&self, document: &T) -> Result<Digest> {
        let mut normalized = document.clone();
        normalized.normalize()?;
        let value = serde_json::to_value(normalized)
            .map_err(|e| Error::new("E_CONTRACT", e.to_string()))?;
        self.put_raw(&canonical::encode(&value)?)
    }

    pub fn load_document<T: WireDocument>(&self, digest: &Digest) -> Result<T> {
        let bytes = self.get_raw(digest)?;
        let value = canonical::parse(&bytes)?;
        let document = contracts::decode::<T>(value)?;
        let normalized =
            serde_json::to_value(&document).map_err(|e| Error::new("E_CONTRACT", e.to_string()))?;
        if canonical::encode(&normalized)? != bytes {
            return Err(Error::new(
                "E_CANONICAL",
                "stored semantic object is not in normalized canonical form",
            )
            .at(digest.to_string()));
        }
        Ok(document)
    }

    /// Persist a memory only after checking its kernel, implementation,
    /// compiler-derived callable contract, and transitive dependency closure.
    pub fn put_memory(&self, record: &MemoryRecord) -> Result<Digest> {
        let mut record = record.clone();
        record.normalize()?;
        self.verify_record_direct(&record)?;
        self.validate_dependency_closure(&record)?;
        self.put_document(&record)
    }

    /// Load and validate a memory and its full memory dependency closure.
    pub fn load_memory(&self, digest: &Digest) -> Result<MemoryRecord> {
        self.load_memory_graph(digest, &mut HashSet::new(), &mut HashMap::new())
    }

    /// Load only the exact record and its local implementation identity. Graph
    /// checkers use this while carrying their own ancestor set.
    pub fn load_memory_local(&self, digest: &Digest) -> Result<MemoryRecord> {
        let record: MemoryRecord = self.load_document(digest)?;
        self.verify_record_direct(&record)?;
        Ok(record)
    }

    pub fn set_alias(&self, label: &str, digest: &Digest) -> Result<()> {
        validate_label(label)?;
        self.load_memory(digest)?;
        let _guard = self.lock_sidecar("catalog.lock")?;
        let mut catalog = self.read_catalog()?;
        if let Some(existing) = catalog.aliases.iter_mut().find(|item| item.label == label) {
            existing.memory = digest.clone();
        } else {
            catalog.aliases.push(CatalogAlias {
                label: label.to_owned(),
                memory: digest.clone(),
            });
        }
        catalog
            .aliases
            .sort_by(|left, right| left.label.cmp(&right.label));
        self.write_sidecar("catalog.json", &catalog)
    }

    /// Resolve a human label or return an exact digest. This lookup does not
    /// grant authority; callers still load/check the returned object.
    pub fn resolve(&self, selector: &str) -> Result<Digest> {
        if selector.starts_with("sha256:") {
            return selector.parse();
        }
        let descriptor = KernelDescriptor::bundled()?;
        if selector == format!("{}@{}", descriptor.name, descriptor.version) {
            return Ok(self.kernel.clone());
        }
        validate_label(selector)?;
        self.read_catalog()?
            .aliases
            .into_iter()
            .find(|item| item.label == selector)
            .map(|item| item.memory)
            .ok_or_else(|| {
                Error::new(
                    "E_NOT_FOUND",
                    format!("unknown memory selector: {selector}"),
                )
            })
    }

    /// Return source blobs after validating the owning memory record.
    pub fn source_bytes(&self, digest: &Digest) -> Result<Vec<(String, Vec<u8>)>> {
        let record = self.load_memory_local(digest)?;
        if !matches!(record.implementation, Implementation::Source { .. }) {
            return Err(Error::new(
                "E_IMPLEMENTATION",
                "memory does not contain source",
            ));
        }
        record
            .source_map
            .iter()
            .map(|source| Ok((source.path.clone(), self.get_raw(&source.blob)?)))
            .collect()
    }

    /// Load the single source file as checked Nexa code and recheck its export.
    pub fn load_source_module(&self, digest: &Digest) -> Result<CheckedModule> {
        let record = self.load_memory_local(digest)?;
        let Implementation::Source { entry } = record.implementation else {
            return Err(Error::new(
                "E_IMPLEMENTATION",
                "memory does not contain source",
            ));
        };
        let sources = self.source_bytes(digest)?;
        if sources.len() != 1 {
            return Err(Error::new(
                "E_SOURCE_BUNDLE",
                "slice0 memory must contain exactly one source file",
            ));
        }
        let (path, bytes) = &sources[0];
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Error::new("E_UTF8", e.to_string()).at(path.clone()))?;
        let checked = source::check(path, text)?;
        let contract = checked.contract(entry.as_str())?;
        ensure_contract_equal(&contract, &record.exports[0])?;
        Ok(checked)
    }

    /// Build a lock from explicit construction requests. This is the selector-
    /// resolving API; locked operations must use `resolve_locked` instead.
    pub fn resolve_requests(&self, requests: &[(String, Identifier)]) -> Result<LockRecord> {
        let mut bindings = Vec::with_capacity(requests.len());
        let mut roots = BTreeSet::new();
        for (selector, export) in requests {
            let digest = self.resolve(selector)?;
            let record = self.load_memory(&digest)?;
            if !record
                .exports
                .iter()
                .any(|contract| &contract.export_name == export)
            {
                return Err(Error::new(
                    "E_EXPORT",
                    "memory does not export the requested callable",
                )
                .at(format!("{selector}#{}", export.as_str())));
            }
            roots.insert(digest.clone());
            bindings.push(LockBinding {
                selector: selector.clone(),
                export: export.clone(),
                memory: digest,
            });
        }
        let mut lock = LockRecord {
            schema: crate::contracts::LockSchema::V03,
            language: Language::V03,
            profile: Profile::Slice0,
            kernel: self.kernel.clone(),
            bindings,
            dependencies: self.dependency_closure(&roots.into_iter().collect::<Vec<_>>())?,
        };
        lock.normalize()?;
        self.validate_lock(&lock)?;
        Ok(lock)
    }

    /// Resolve exactly from an existing lock. This method never reads aliases.
    pub fn resolve_locked(
        &self,
        lock: &LockRecord,
        selector: &str,
        export: &Identifier,
    ) -> Result<Digest> {
        self.validate_lock(lock)?;
        lock.bindings
            .iter()
            .find(|binding| binding.selector == selector && &binding.export == export)
            .map(|binding| binding.memory.clone())
            .ok_or_else(|| {
                Error::new(
                    "E_LOCK",
                    "lock has no binding for the requested selector/export",
                )
                .at(format!("{selector}#{}", export.as_str()))
            })
    }

    pub fn validate_lock(&self, lock: &LockRecord) -> Result<()> {
        let mut normalized = lock.clone();
        normalized.normalize()?;
        if canonical::encode(
            &serde_json::to_value(&normalized)
                .map_err(|e| Error::new("E_CONTRACT", e.to_string()))?,
        )? != canonical::encode(
            &serde_json::to_value(lock).map_err(|e| Error::new("E_CONTRACT", e.to_string()))?,
        )? {
            return Err(Error::new("E_LOCK", "lock is not in normalized form"));
        }
        if lock.language != Language::V03
            || lock.profile != Profile::Slice0
            || lock.kernel != self.kernel
        {
            return Err(Error::new(
                "E_KERNEL",
                "lock language, profile, or kernel does not match the installed runtime",
            ));
        }
        let mut roots = BTreeSet::new();
        for binding in &lock.bindings {
            let record = self.load_memory(&binding.memory)?;
            if !record
                .exports
                .iter()
                .any(|contract| contract.export_name == binding.export)
            {
                return Err(Error::new(
                    "E_EXPORT",
                    "locked memory does not export the selected callable",
                )
                .at(format!("{}#{}", binding.selector, binding.export.as_str())));
            }
            roots.insert(binding.memory.clone());
        }
        let closure = self.dependency_closure(&roots.into_iter().collect::<Vec<_>>())?;
        if closure != lock.dependencies {
            return Err(Error::new(
                "E_LOCK",
                "lock dependency closure does not match its bindings",
            ));
        }
        Ok(())
    }

    /// Sorted, unique memory closure for root memory digests; excludes roots'
    /// containing program and excludes the separately pinned kernel digest.
    pub fn dependency_closure(&self, roots: &[Digest]) -> Result<Vec<Digest>> {
        let mut closure = BTreeSet::new();
        let mut stack = HashSet::new();
        let mut done = HashMap::new();
        for root in roots {
            let record = self.load_memory_graph(root, &mut stack, &mut done)?;
            closure.insert(root.clone());
            closure.extend(record.dependencies);
        }
        Ok(closure.into_iter().collect())
    }

    pub fn read_lock(&self, path: impl AsRef<Path>) -> Result<LockRecord> {
        crate::memory::read_lock(path)
    }

    pub fn attach_evidence(&self, subject: &Digest, report: &Digest) -> Result<()> {
        self.get_raw(subject)?;
        let report_record: EvidenceReport = self.load_document(report)?;
        if &report_record.subject != subject {
            return Err(Error::new(
                "E_EVIDENCE",
                "evidence report subject does not match the requested subject",
            ));
        }
        if report_record.kernel != self.kernel {
            return Err(Error::new(
                "E_KERNEL",
                "evidence report uses a different kernel",
            ));
        }
        let _guard = self.lock_sidecar("evidence-index.lock")?;
        let mut index = self.read_evidence_index()?;
        if let Some(item) = index
            .subjects
            .iter_mut()
            .find(|item| &item.subject == subject)
        {
            if !item.reports.contains(report) {
                item.reports.push(report.clone());
                item.reports.sort();
            }
        } else {
            index.subjects.push(EvidenceSubject {
                subject: subject.clone(),
                reports: vec![report.clone()],
            });
        }
        index
            .subjects
            .sort_by(|left, right| left.subject.cmp(&right.subject));
        self.write_sidecar("evidence-index.json", &index)
    }

    pub fn evidence_for(&self, subject: &Digest) -> Result<Vec<Digest>> {
        let index = self.read_evidence_index()?;
        let reports = index
            .subjects
            .into_iter()
            .find(|item| &item.subject == subject)
            .map(|item| item.reports)
            .unwrap_or_default();
        for digest in &reports {
            let report: EvidenceReport = self.load_document(digest)?;
            if &report.subject != subject {
                return Err(Error::new(
                    "E_EVIDENCE",
                    "evidence index references a report for another subject",
                ));
            }
        }
        Ok(reports)
    }

    pub(crate) fn verify_record_direct(&self, record: &MemoryRecord) -> Result<()> {
        if record.language != Language::V03 || record.profile != Profile::Slice0 {
            return Err(Error::new(
                "E_ABI",
                "memory language or implementation profile does not match slice0",
            ));
        }
        validate_label(&format!(
            "{}@{}",
            record.metadata.name, record.metadata.version
        ))
        .map_err(|_| {
            Error::new(
                "E_METADATA",
                "memory metadata needs a valid name/version label",
            )
        })?;
        self.verify_kernel(&record.kernel)?;
        match &record.implementation {
            Implementation::Source { entry } => {
                if record.source_map.len() != 1 {
                    return Err(Error::new(
                        "E_SOURCE_BUNDLE",
                        "slice0 supports exactly one source file per memory",
                    ));
                }
                let source = &record.source_map[0];
                let bytes = self.get_raw(&source.blob)?;
                let text = std::str::from_utf8(&bytes)
                    .map_err(|e| Error::new("E_UTF8", e.to_string()).at(source.path.clone()))?;
                let checked = source::check(&source.path, text)?;
                if checked.exports().len() != 1 {
                    return Err(Error::new(
                        "E_EXPORT",
                        "a source memory must have exactly one exported callable",
                    ));
                }
                let actual = checked.contract(entry.as_str())?;
                if actual.export_name != record.exports[0].export_name {
                    return Err(Error::new(
                        "E_CONTRACT",
                        "source entry does not match the memory export",
                    ));
                }
                ensure_contract_equal(&actual, &record.exports[0])?;
            }
            Implementation::Program { program } => {
                if !record.source_map.is_empty() {
                    return Err(Error::new(
                        "E_SOURCE_BUNDLE",
                        "blueprint memories must reference a ProgramSpec instead of source blobs",
                    ));
                }
                let spec: ProgramSpec = self.load_document(program)?;
                if spec.kernel != self.kernel {
                    return Err(Error::new(
                        "E_KERNEL",
                        "program memory pins a different kernel",
                    ));
                }
                let export = &record.exports[0];
                let derived = &spec.derived;
                let expected_kind = if derived.effects.is_empty() {
                    CallableKind::Function
                } else {
                    CallableKind::Action
                };
                if export.parameters != derived.inputs
                    || export.result_type != derived.result_type
                    || export.effects != derived.effects
                    || export.required_bindings != derived.required_bindings
                    || export.capability_requirements != derived.capability_requirements
                    || export.kind != expected_kind
                {
                    return Err(Error::new(
                        "E_CONTRACT",
                        "program memory export disagrees with its stored ProgramSpec",
                    ));
                }
            }
        }
        Ok(())
    }

    fn load_memory_graph(
        &self,
        digest: &Digest,
        stack: &mut HashSet<Digest>,
        done: &mut HashMap<Digest, MemoryRecord>,
    ) -> Result<MemoryRecord> {
        if let Some(record) = done.get(digest) {
            return Ok(record.clone());
        }
        if !stack.insert(digest.clone()) {
            return Err(
                Error::new("E_CYCLE", "memory dependency closure contains a cycle")
                    .at(digest.to_string()),
            );
        }
        let result = (|| {
            let record: MemoryRecord = self.load_document(digest)?;
            self.verify_record_direct(&record)?;
            let children = self.child_references(&record)?;
            let mut closure = BTreeSet::new();
            for child in children {
                if child == *digest {
                    return Err(
                        Error::new("E_CYCLE", "memory depends on itself").at(digest.to_string())
                    );
                }
                let child_record = self.load_memory_graph(&child, stack, done)?;
                closure.insert(child.clone());
                closure.extend(child_record.dependencies);
            }
            let actual: Vec<_> = closure.into_iter().collect();
            if actual != record.dependencies {
                return Err(Error::new(
                    "E_DEPENDENCY",
                    "memory dependency closure does not match its implementation",
                )
                .at(digest.to_string()));
            }
            if let Implementation::Program { program } = &record.implementation {
                let spec: ProgramSpec = self.load_document(program)?;
                if spec.derived.dependencies != actual {
                    return Err(Error::new(
                        "E_DEPENDENCY",
                        "ProgramSpec dependency closure disagrees with its memory record",
                    )
                    .at(digest.to_string()));
                }
            }
            Ok(record)
        })();
        stack.remove(digest);
        if let Ok(record) = &result {
            done.insert(digest.clone(), record.clone());
        }
        result
    }

    fn child_references(&self, record: &MemoryRecord) -> Result<Vec<Digest>> {
        let mut refs = match &record.implementation {
            // Source records store their transitive dependency closure directly.
            Implementation::Source { .. } => record.dependencies.clone(),
            Implementation::Program { program } => {
                let spec: ProgramSpec = self.load_document(program)?;
                spec.nodes.iter().map(|node| node.memory.clone()).collect()
            }
        };
        refs.sort();
        refs.dedup();
        Ok(refs)
    }

    fn validate_dependency_closure(&self, record: &MemoryRecord) -> Result<()> {
        let mut closure = BTreeSet::new();
        let mut stack = HashSet::new();
        let mut done = HashMap::new();
        for child in self.child_references(record)? {
            let child_record = self.load_memory_graph(&child, &mut stack, &mut done)?;
            closure.insert(child.clone());
            closure.extend(child_record.dependencies);
        }
        let actual: Vec<_> = closure.into_iter().collect();
        if actual != record.dependencies {
            return Err(Error::new(
                "E_DEPENDENCY",
                "memory dependency closure does not match its implementation",
            ));
        }
        if let Implementation::Program { program } = &record.implementation {
            let spec: ProgramSpec = self.load_document(program)?;
            if spec.derived.dependencies != actual {
                return Err(Error::new(
                    "E_DEPENDENCY",
                    "ProgramSpec dependency closure disagrees with its memory record",
                ));
            }
        }
        Ok(())
    }

    fn verify_kernel(&self, digest: &Digest) -> Result<()> {
        if digest != &self.kernel {
            return Err(Error::new(
                "E_KERNEL",
                "memory does not pin the installed kernel descriptor",
            ));
        }
        let actual: KernelDescriptor = self.load_document(digest)?;
        let installed = KernelDescriptor::bundled()?;
        let expected = self.put_document(&installed)?;
        if expected != *digest || serde_value(&actual)? != serde_value(&installed)? {
            return Err(Error::new(
                "E_KERNEL",
                "kernel descriptor content differs from the installed implementation",
            ));
        }
        Ok(())
    }

    fn read_object(&self, digest: &Digest) -> Result<Vec<u8>> {
        let path = self.object_path(digest);
        let bytes = fs::read(&path).map_err(|e| io_error("read object", &path, e))?;
        digest.verify(&bytes)?;
        Ok(bytes)
    }

    fn ensure_catalog(&self) -> Result<()> {
        let path = self.root.join("catalog.json");
        if !path.exists() {
            self.write_sidecar(
                "catalog.json",
                &Catalog {
                    schema: "nexa.catalog/0.3".into(),
                    aliases: Vec::new(),
                },
            )?;
        }
        self.read_catalog().map(|_| ())
    }

    fn read_catalog(&self) -> Result<Catalog> {
        let path = self.root.join("catalog.json");
        let mut catalog: Catalog = self.read_sidecar(&path)?;
        if catalog.schema != "nexa.catalog/0.3" {
            return Err(Error::new("E_CATALOG", "unsupported catalogue schema"));
        }
        let mut seen = BTreeSet::new();
        for alias in &catalog.aliases {
            validate_label(&alias.label)?;
            if !seen.insert(alias.label.clone()) {
                return Err(Error::new("E_CATALOG", "duplicate catalogue alias"));
            }
        }
        catalog
            .aliases
            .sort_by(|left, right| left.label.cmp(&right.label));
        Ok(catalog)
    }

    fn ensure_evidence_index(&self) -> Result<()> {
        let path = self.root.join("evidence-index.json");
        if !path.exists() {
            self.write_sidecar(
                "evidence-index.json",
                &EvidenceIndex {
                    schema: "nexa.evidence-index/0.3".into(),
                    subjects: Vec::new(),
                },
            )?;
        }
        self.read_evidence_index().map(|_| ())
    }

    fn read_evidence_index(&self) -> Result<EvidenceIndex> {
        let path = self.root.join("evidence-index.json");
        let mut index: EvidenceIndex = self.read_sidecar(&path)?;
        if index.schema != "nexa.evidence-index/0.3" {
            return Err(Error::new(
                "E_EVIDENCE",
                "unsupported evidence index schema",
            ));
        }
        let mut subjects = BTreeSet::new();
        for subject in &mut index.subjects {
            if !subjects.insert(subject.subject.clone()) {
                return Err(Error::new("E_EVIDENCE", "duplicate evidence subject"));
            }
            let mut reports = BTreeSet::new();
            for report in &subject.reports {
                if !reports.insert(report.clone()) {
                    return Err(Error::new("E_EVIDENCE", "duplicate evidence reference"));
                }
            }
            subject.reports.sort();
        }
        index
            .subjects
            .sort_by(|left, right| left.subject.cmp(&right.subject));
        Ok(index)
    }

    fn read_sidecar<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<T> {
        let bytes = fs::read(path).map_err(|e| io_error("read store index", path, e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::new("E_INDEX", e.to_string()).at(path.display().to_string()))
    }

    fn write_sidecar<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let path = self.root.join(name);
        let bytes =
            serde_json::to_vec_pretty(value).map_err(|e| Error::new("E_JSON", e.to_string()))?;
        atomic_replace(&path, &bytes)
    }

    fn lock_sidecar(&self, name: &str) -> Result<SidecarLock> {
        let path = self.root.join(name);
        for _ in 0..300 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())
                        .and_then(|()| file.sync_all())
                        .map_err(|e| io_error("write index lock", &path, e))?;
                    return Ok(SidecarLock {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(io_error("create index lock", &path, error)),
            }
        }
        Err(
            Error::new("E_LOCK", "timed out waiting for store index lock")
                .at(path.display().to_string()),
        )
    }
}

fn ensure_contract_equal(actual: &CallableContract, recorded: &CallableContract) -> Result<()> {
    let mut actual = actual.clone();
    let mut recorded = recorded.clone();
    actual.normalize()?;
    recorded.normalize()?;
    if canonical::encode(
        &serde_json::to_value(actual).map_err(|e| Error::new("E_CONTRACT", e.to_string()))?,
    )? != canonical::encode(
        &serde_json::to_value(recorded).map_err(|e| Error::new("E_CONTRACT", e.to_string()))?,
    )? {
        return Err(Error::new(
            "E_CONTRACT",
            "recorded callable contract disagrees with the checked implementation",
        ));
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<()> {
    let Some((name, version)) = label.split_once('@') else {
        return Err(Error::new("E_SELECTOR", "expected name@version selector"));
    };
    let name_valid = !name.is_empty() && name.split('.').all(crate::value::identifier_shape);
    if !name_valid || version.is_empty() || version.contains(['@', '/', '\\', '\0']) {
        return Err(Error::new("E_SELECTOR", "invalid name@version selector"));
    }
    Ok(())
}

fn serde_value<T: Serialize>(value: &T) -> Result<serde_json::Value> {
    serde_json::to_value(value).map_err(|e| Error::new("E_CONTRACT", e.to_string()))
}

fn unique_temp(parent: &Path, kind: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".{kind}-{}-{sequence}.tmp", std::process::id()))
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().expect("sidecar path has a parent");
    fs::create_dir_all(parent).map_err(|e| io_error("create index directory", parent, e))?;
    let temporary = unique_temp(parent, "index");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| io_error("create temporary index", &temporary, e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_error("write index", &temporary, e))?;
    drop(file);
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        io_error("replace index", path, e)
    })
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> Error {
    Error::new("E_IO", format!("{action}: {error}")).at(path.display().to_string())
}
