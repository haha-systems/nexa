use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Nexa: typed programs, reusable memories, and authorized effects"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Encode strict semantic JSON; array order is preserved.
    Canonicalize { path: PathBuf },
    /// Validate local wire invariants and normalize a versioned document.
    Normalize { path: PathBuf },
    /// Hash a normalized document, semantic JSON, or exact raw bytes.
    Hash {
        path: PathBuf,
        #[arg(long, value_enum, default_value = "document")]
        kind: HashKind,
    },
    /// Print the installed kernel descriptor without invoking it.
    Kernel,
    /// Print one generated JSON Schema.
    Schema { name: String },
    /// Write all generated JSON Schemas to a directory.
    Schemas {
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Check source or a stored program without invoking its actions.
    Check {
        path: PathBuf,
        #[arg(long)]
        lock: Option<PathBuf>,
        #[arg(long, default_value = "slice0")]
        profile: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
    /// Evaluate one pure source export; host actions are unavailable.
    Eval {
        path: PathBuf,
        #[arg(long)]
        entry: String,
        #[arg(long)]
        input_file: PathBuf,
    },
    /// Seal or inspect immutable memories.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Resolve selectors explicitly and write an immutable-reference lock.
    Resolve {
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "path")]
        request: Option<PathBuf>,
        #[arg(long)]
        write_lock: PathBuf,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
    /// Build a program from a blueprint or typed request under an existing lock.
    Compose {
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "path")]
        request: Option<PathBuf>,
        #[arg(long)]
        locked: bool,
        #[arg(long)]
        lock: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
    /// Inspect a checked program, optionally expanding a nested node.
    Inspect {
        program: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        node: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
    /// Run a checked program with explicitly supplied resources and write its receipt.
    Run {
        program: PathBuf,
        #[arg(long)]
        input_file: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        /// Grant stdout, or one exact HTTP(S) origin with the http-client adapter.
        #[arg(long)]
        allow: Vec<String>,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
    /// Execute declarative fixtures and record immutable evidence.
    Test {
        program: PathBuf,
        #[arg(long)]
        fixtures: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum MemoryCommand {
    /// Seal a source directory, or seal --program with --name and --version.
    Seal {
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "path")]
        program: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
    /// Inspect one immutable memory without running it.
    Inspect {
        digest: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = ".nexa/store")]
        store: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum HashKind {
    Document,
    Json,
    Raw,
}
