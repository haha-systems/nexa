use clap::{Parser, Subcommand, ValueEnum};
use nexa::{
    canonical, contracts,
    diagnostic::{Error, ExitCode, Result},
    kernel::KernelDescriptor,
    schemas,
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process,
};

#[derive(Parser)]
#[command(
    version,
    about = "Nexa M0: wire contracts and content identity (no source execution yet)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Encode strict semantic JSON; array order is preserved.
    Canonicalize { path: PathBuf },
    /// Validate local wire invariants and normalize a versioned Slice 0 document.
    Normalize { path: PathBuf },
    /// Hash normalized documents, semantic JSON, or exact raw bytes.
    Hash {
        path: PathBuf,
        #[arg(long, value_enum, default_value = "document")]
        kind: HashKind,
    },
    /// Print the bundled kernel descriptor and its identity. Does not execute it.
    Kernel,
    /// Print one generated JSON Schema.
    Schema { name: String },
    /// Write all generated JSON Schemas to a directory.
    Schemas {
        #[arg(long)]
        out_dir: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum HashKind {
    Document,
    Json,
    Raw,
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| Error::new("E_IO", e.to_string()).at(path.display().to_string()))
}

fn pretty(value: &serde_json::Value) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|e| Error::new("E_JSON", e.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn execute(command: Command) -> Result<Vec<u8>> {
    match command {
        Command::Canonicalize { path } => canonical::encode(&canonical::parse(&read(&path)?)?),
        Command::Normalize { path } => {
            canonical::encode(&contracts::normalized_json(&read(&path)?)?)
        }
        Command::Hash { path, kind } => {
            let bytes = read(&path)?;
            let digest = match kind {
                HashKind::Raw => nexa::digest::Digest::of_bytes(&bytes),
                HashKind::Json => canonical::digest(&canonical::parse(&bytes)?)?,
                HashKind::Document => canonical::digest(&contracts::normalized_json(&bytes)?)?,
            };
            Ok(format!("{digest}\n").into_bytes())
        }
        Command::Kernel => {
            let descriptor = serde_json::to_value(KernelDescriptor::bundled()?)
                .map_err(|e| Error::new("E_CONTRACT", e.to_string()))?;
            pretty(
                &serde_json::json!({"digest": canonical::digest(&descriptor)?, "descriptor": descriptor}),
            )
        }
        Command::Schema { name } => pretty(&schemas::named(&name)?),
        Command::Schemas { out_dir } => {
            fs::create_dir_all(&out_dir)
                .map_err(|e| Error::new("E_IO", e.to_string()).at(out_dir.display().to_string()))?;
            for name in schemas::NAMES {
                let path = out_dir.join(format!("{name}-0.3.schema.json"));
                fs::write(&path, pretty(&schemas::named(name)?)?).map_err(|e| {
                    Error::new("E_IO", e.to_string()).at(path.display().to_string())
                })?;
            }
            Ok(Vec::new())
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let result = execute(cli.command).and_then(|bytes| {
        io::stdout()
            .lock()
            .write_all(&bytes)
            .map_err(|e| Error::new("E_IO", e.to_string()))
    });
    if let Err(error) = result {
        // Human-friendly Clap usage errors and structured data/tooling errors are
        // separate; program execution and its receipts are not implemented yet.
        if let Ok(message) = serde_json::to_string(&error.diagnostic) {
            eprintln!("{message}");
        } else {
            eprintln!("{error}");
        }
        process::exit(ExitCode::ConstructionError as i32);
    }
}
