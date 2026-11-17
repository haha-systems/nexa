use crate::{
    cli_args::{Command, Core1Command, HashKind, MemoryCommand},
    cli_support::{self, document, inputs, json, pretty, read, text, write_bytes, write_document},
};
use nexa::{
    blueprint, canonical,
    contracts::{
        ApplicationManifest, BlueprintDraft, CaseObservation, FixtureSuite, ProgramSpec, Receipt,
    },
    core1,
    diagnostic::{Error, ExitCode, Result},
    digest::Digest,
    evidence::{self, FixtureInputs},
    kernel::KernelDescriptor,
    memory::{self, Store},
    runtime::ExecutionContext,
    schemas, source,
};
use serde_json::{Value as Json, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Output {
    pub bytes: Vec<u8>,
    pub exit: ExitCode,
}
impl Output {
    fn success(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            exit: ExitCode::Success,
        }
    }
    fn digest(digest: &Digest) -> Self {
        Self::success(format!("{digest}\n").into_bytes())
    }
}

fn draft(path: Option<PathBuf>, request: Option<PathBuf>) -> Result<BlueprintDraft> {
    match (path, request) {
        (Some(path), None) => blueprint::parse_source(&path.display().to_string(), &text(&path)?),
        (None, Some(path)) => blueprint::decode_request(&read(&path)?),
        _ => Err(Error::new(
            "E_ARGS",
            "supply exactly one source path or --request file",
        )),
    }
}

fn checked_program(path: &Path, store: &Store) -> Result<(ProgramSpec, blueprint::CheckedProgram)> {
    let spec: ProgramSpec = document(path)?;
    let checked = blueprint::load_checked(&spec, store)?;
    Ok((spec, checked))
}

fn launch_context(manifest: ApplicationManifest, allow: &[String]) -> Result<ExecutionContext> {
    let bound_principal = manifest.principal.clone();
    ExecutionContext::from_manifest_with_resources(manifest, bound_principal, allow)
}

fn invocation_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let host_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "nexa-{}-{host_time}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn invoke(
    checked: &blueprint::CheckedProgram,
    spec: &ProgramSpec,
    input: Json,
    manifest: ApplicationManifest,
    allow: &[String],
    sink: &mut dyn Write,
) -> Result<Receipt> {
    let arguments = inputs(input, &spec.inputs)?;
    let mut context = launch_context(manifest, allow)?;
    blueprint::run(checked, arguments, invocation_id(), &mut context, sink)
}

fn prepare_receipt(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| Error::new("E_IO", e.to_string()).at(parent.display().to_string()))?;
    }
    File::create(path).map_err(|e| Error::new("E_IO", e.to_string()).at(path.display().to_string()))
}

pub fn execute(command: Command) -> Result<Output> {
    match command {
        Command::Core1 { command } => core1_command(command),
        Command::Canonicalize { path } => Ok(Output::success(canonical::encode(
            &canonical::parse(&read(&path)?)?,
        )?)),
        Command::Normalize { path } => Ok(Output::success(canonical::encode(
            &nexa::contracts::normalized_json(&read(&path)?)?,
        )?)),
        Command::Hash { path, kind } => {
            let bytes = read(&path)?;
            let digest = match kind {
                HashKind::Raw => Digest::of_bytes(&bytes),
                HashKind::Json => canonical::digest(&canonical::parse(&bytes)?)?,
                HashKind::Document => {
                    canonical::digest(&nexa::contracts::normalized_json(&bytes)?)?
                }
            };
            Ok(Output::digest(&digest))
        }
        Command::Kernel => {
            let descriptor = json(&KernelDescriptor::bundled()?)?;
            Ok(Output::success(pretty(
                &object!({"digest":canonical::digest(&descriptor)?,"descriptor":descriptor}),
            )?))
        }
        Command::Schema { name } => Ok(Output::success(pretty(&schemas::named(&name)?)?)),
        Command::Schemas { out_dir } => {
            for name in schemas::NAMES {
                write_bytes(
                    &out_dir.join(format!("{name}-0.3.schema.json")),
                    &pretty(&schemas::named(name)?)?,
                )?;
            }
            Ok(Output::success(Vec::new()))
        }
        Command::Check {
            path,
            lock,
            profile,
            json: as_json,
            store,
        } => {
            if profile != "slice0" {
                return Err(Error::new(
                    "E_PROFILE",
                    "this implementation supports only slice0",
                ));
            }
            let summary = if path.extension().is_some_and(|ext| ext == "nexa") {
                let content = text(&path)?;
                match source::check(&path.display().to_string(), &content) {
                    Ok(checked) => {
                        object!({"schema":"nexa.check/0.3","profile":"slice0","status":"Checked","module":checked.module_name(),"exports":checked.exports()})
                    }
                    Err(source_error) => {
                        match blueprint::parse_source(&path.display().to_string(), &content) {
                            Ok(draft) => {
                                let store = Store::open(store)?;
                                let lock_path = lock.unwrap_or_else(|| {
                                    path.parent().unwrap_or(Path::new(".")).join("nexa.lock")
                                });
                                let lock = memory::read_lock(lock_path)?;
                                let checked = blueprint::compose_checked(&draft, &lock, &store)?;
                                object!({"schema":"nexa.check/0.3","profile":"slice0","status":"Checked","subject":checked.program_digest(),"contract":checked.contract()})
                            }
                            Err(_) => return Err(source_error),
                        }
                    }
                }
            } else {
                let store = Store::open(store)?;
                let (_, checked) = checked_program(&path, &store)?;
                object!({"schema":"nexa.check/0.3","profile":"slice0","status":"Checked","subject":checked.program_digest(),"contract":checked.contract()})
            };
            let bytes = if as_json {
                pretty(&summary)?
            } else {
                format!("Checked {} (slice0)\n", path.display()).into_bytes()
            };
            Ok(Output::success(bytes))
        }
        Command::Eval {
            path,
            entry,
            input_file,
        } => {
            let checked = source::check(&path.display().to_string(), &text(&path)?)?;
            let contract = checked.contract(&entry)?;
            let arguments = inputs(canonical::parse(&read(&input_file)?)?, &contract.parameters)?;
            let ordered = contract
                .parameters
                .iter()
                .map(|p| arguments[&p.name].clone())
                .collect::<Vec<_>>();
            Ok(Output::success(pretty(
                &checked.evaluate(&entry, &ordered)?,
            )?))
        }
        Command::Memory { command } => match command {
            MemoryCommand::Seal {
                path,
                program,
                name,
                version,
                store,
            } => {
                let store = Store::open(store)?;
                let digest = match (path, program, name, version) {
                    (Some(path), None, None, None) => memory::seal_source(path, &store)?,
                    (None, Some(path), Some(name), Some(version)) => {
                        let spec: ProgramSpec = document(&path)?;
                        memory::program::seal_program(&spec, &store, &name, &version)?
                    }
                    _ => {
                        return Err(Error::new(
                            "E_ARGS",
                            "seal a source directory, or use --program with --name and --version",
                        ));
                    }
                };
                Ok(Output::digest(&digest))
            }
            MemoryCommand::Inspect {
                digest,
                json: _,
                store,
            } => {
                let digest: Digest = digest.parse()?;
                let store = Store::open(store)?;
                let record = store.load_memory(&digest)?;
                if matches!(
                    record.implementation,
                    nexa::contracts::Implementation::Program { .. }
                ) {
                    memory::program::validate_program_record(&record, &store)?;
                }
                Ok(Output::success(pretty(
                    &object!({"schema":"nexa.memory-inspection/0.3","profile":"slice0","digest":digest,"memory":record,"evidence":store.evidence_for(&digest)?}),
                )?))
            }
        },
        Command::Resolve {
            path,
            request,
            write_lock,
            store,
        } => {
            let draft = draft(path, request)?;
            let store = Store::open(store)?;
            let requests = draft
                .nodes
                .iter()
                .map(|n| (n.selector.clone(), n.export.clone()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let lock = store.resolve_requests(&requests)?;
            Ok(Output::digest(&write_document(&write_lock, &lock)?))
        }
        Command::Compose {
            path,
            request,
            locked,
            lock,
            out,
            store,
        } => {
            if !locked {
                return Err(Error::new(
                    "E_LOCK_REQUIRED",
                    "compose requires --locked; resolve explicitly first",
                ));
            }
            let draft = draft(path, request)?;
            let store = Store::open(store)?;
            let lock = memory::read_lock(lock)?;
            store.validate_lock(&lock)?;
            let program = blueprint::compose(&draft, &lock, &store)?;
            let digest = store.put_document(&program)?;
            let written = write_document(&out, &program)?;
            if written != digest {
                return Err(Error::new(
                    "E_INTEGRITY",
                    "stored and written program identities disagree",
                ));
            }
            Ok(Output::digest(&digest))
        }
        Command::Inspect {
            program,
            json: _,
            node,
            limit,
            cursor,
            fields,
            store,
        } => {
            let store = Store::open(store)?;
            let (_, checked) = checked_program(&program, &store)?;
            let digest = checked.program_digest();
            let offset = cli_support::cursor_offset(digest, node.as_deref(), cursor.as_deref())?;
            let mut summary = blueprint::inspect_page(&checked, node.as_deref(), offset, limit)?;
            if let Some(object) = summary.as_object_mut() {
                object.insert("evidence".into(), json(&store.evidence_for(digest)?)?);
            }
            let summary = cli_support::page(summary, digest, node.as_deref(), &fields)?;
            Ok(Output::success(pretty(&summary)?))
        }
        Command::Run {
            program,
            input_file,
            manifest,
            allow,
            receipt,
            store,
        } => {
            let store = Store::open(store)?;
            let (spec, checked) = checked_program(&program, &store)?;
            let input = canonical::parse(&read(&input_file)?)?;
            inputs(input.clone(), &spec.inputs)?;
            let manifest: ApplicationManifest = document(&manifest)?;
            launch_context(manifest.clone(), &allow)?;
            let mut receipt_file = prepare_receipt(&receipt)?;
            let receipt = invoke(
                &checked,
                &spec,
                input,
                manifest,
                &allow,
                &mut io::stdout().lock(),
            )?;
            receipt_file
                .write_all(&canonical::encode(&json(&receipt)?)?)
                .map_err(|e| Error::new("E_IO", e.to_string()))?;
            Ok(Output {
                bytes: Vec::new(),
                exit: receipt.outcome.exit_code(),
            })
        }
        Command::Test {
            program,
            fixtures,
            manifest,
            report,
            store,
        } => {
            let store = Store::open(store)?;
            let (spec, checked) = checked_program(&program, &store)?;
            let subject = store.put_document(&spec)?;
            let fixture_bytes = read(&fixtures)?;
            let suite: FixtureSuite = nexa::contracts::decode(canonical::parse(&fixture_bytes)?)?;
            let manifest_bytes = read(&manifest)?;
            let manifest: ApplicationManifest =
                nexa::contracts::decode(canonical::parse(&manifest_bytes)?)?;
            let metadata = FixtureInputs {
                subject: subject.clone(),
                kernel: store.kernel_digest().clone(),
                fixtures: store.put_raw(&fixture_bytes)?,
                manifest: store.put_raw(&manifest_bytes)?,
                tool_version: nexa::IMPLEMENTATION_VERSION.into(),
                environment: format!(
                    "os={};arch={};scheduler=sequential;implementation={}",
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    KernelDescriptor::bundled()?.implementation
                ),
            };
            let evidence = evidence::run_fixtures(metadata, &suite, |case| {
                let mut stdout = Vec::new();
                let receipt = invoke(
                    &checked,
                    &spec,
                    json(&case.input)?,
                    manifest.clone(),
                    &case.allow,
                    &mut stdout,
                )?;
                Ok(CaseObservation {
                    exit_code: receipt.outcome.exit_code() as u8,
                    stdout,
                    outcome: receipt.outcome,
                })
            });
            let report_digest = store.put_document(&evidence)?;
            write_document(&report, &evidence)?;
            store.attach_evidence(&subject, &report_digest)?;
            let passed = evidence.cases.iter().all(|case| case.passed);
            Ok(Output {
                bytes: pretty(
                    &object!({"subject":subject,"report":report_digest,"passed":passed,"cases":evidence.cases.len()}),
                )?,
                exit: if passed {
                    ExitCode::Success
                } else {
                    ExitCode::RuntimeFailure
                },
            })
        }
    }
}

fn core1_command(command: Core1Command) -> Result<Output> {
    match command {
        Core1Command::Seal {
            source,
            imports,
            store,
        } => {
            let mut pinned = BTreeMap::new();
            for entry in imports {
                let (alias, digest) = entry
                    .split_once('=')
                    .ok_or_else(|| Error::new("E_IMPORT", "expected alias=sha256:digest"))?;
                if pinned.insert(alias.to_owned(), digest.parse()?).is_some() {
                    return Err(Error::new("E_IMPORT", format!("duplicate import {alias}")));
                }
            }
            let bytes = core1::read_bounded(&source, core1::SOURCE_LIMIT)?;
            let text = std::str::from_utf8(&bytes).map_err(|error| {
                Error::new("E_UTF8", error.to_string()).at(source.display().to_string())
            })?;
            let store = Store::open(store)?;
            let digest = core1::seal(&store, &source.display().to_string(), text, pinned)?;
            Ok(Output::digest(&digest))
        }
        Core1Command::Inspect {
            digest,
            closure,
            store,
        } => {
            let digest = digest.parse()?;
            let store = Store::open(store)?;
            let bytes = if closure {
                pretty(&object!({
                    "subject": digest,
                    "memories": core1::inspect_closure(&store, &digest)?,
                }))?
            } else {
                pretty(&core1::inspect(&store, &digest)?)?
            };
            Ok(Output::success(bytes))
        }
        Core1Command::Run {
            digest,
            input_file,
            fuel,
            store,
        } => {
            let input = core1::read_bounded(&input_file, core1::INPUT_LIMIT)?;
            let args = canonical::parse(&input)?;
            let value = core1::run(&Store::open(store)?, &digest.parse()?, &args, fuel)?;
            Ok(Output::success(pretty(&value)?))
        }
    }
}
