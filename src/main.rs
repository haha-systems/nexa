mod cli_args;
mod cli_impl;
mod cli_support;

use clap::Parser;
use cli_args::Cli;
use nexa::diagnostic::{Error, ExitCode};
use std::{
    io::{self, Write},
    process,
};

fn main() {
    let cli = Cli::parse();
    let result = cli_impl::execute(cli.command).and_then(|output| {
        if !output.bytes.is_empty() {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(&output.bytes)
                .and_then(|()| stdout.flush())
                .map_err(|e| Error::new("E_IO", e.to_string()))?;
        }
        Ok(output.exit)
    });
    match result {
        Ok(exit) => process::exit(exit as i32),
        Err(error) => {
            if let Ok(message) = serde_json::to_string(&error.diagnostic) {
                eprintln!("{message}");
            } else {
                eprintln!("{error}");
            }
            process::exit(ExitCode::ConstructionError as i32);
        }
    }
}
