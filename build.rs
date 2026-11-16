use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).expect("read implementation sources") {
        let path = entry.expect("read source directory entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn add(hash: &mut Sha256, label: &str, bytes: &[u8]) {
    hash.update((label.len() as u64).to_be_bytes());
    hash.update(label.as_bytes());
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn main() {
    let mut files = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("Cargo.lock"),
        PathBuf::from("build.rs"),
    ];
    collect(Path::new("src"), &mut files);
    files.sort();
    let mut hash = Sha256::new();
    for path in files {
        let label = path.to_str().expect("UTF-8 source path");
        println!("cargo:rerun-if-changed={label}");
        add(
            &mut hash,
            label,
            &fs::read(&path).expect("read build input"),
        );
    }
    println!("cargo:rerun-if-changed=src");
    for key in [
        "TARGET",
        "PROFILE",
        "OPT_LEVEL",
        "DEBUG",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        add(&mut hash, key, env::var(key).unwrap_or_default().as_bytes());
    }
    let compiler = Command::new(env::var_os("RUSTC").expect("Cargo supplies RUSTC"))
        .args(["--version", "--verbose"])
        .output()
        .expect("inspect Rust compiler");
    assert!(
        compiler.status.success(),
        "cannot fingerprint Rust compiler"
    );
    add(&mut hash, "rustc", &compiler.stdout);
    println!(
        "cargo:rustc-env=NEXA_IMPLEMENTATION_DIGEST=sha256:{:x}",
        hash.finalize()
    );
}
