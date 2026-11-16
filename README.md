# Nexa

Nexa is a proposed general-purpose, AI-first language with typed component composition, reusable memories, and authorized effects. The reference implementation is written in Rust.

**Current milestone: M0 — contracts and identity.** The CLI validates local wire invariants, normalizes versioned objects, computes content identities, exports JSON Schemas, and describes the kernel ABI. Source parsing/evaluation, memory loading, graph type checking, and authorized program execution arrive in later milestones. This is not yet a Slice 0 runtime.

Read the [language specification](docs/nexa-language-specification-0.3.md), [implementation plan](docs/nexa-functional-slice-plan.md), and [M0 Rust walkthrough](docs/m0-walkthrough.md).

## Build and try it

Rust 1.100.0, rustfmt, and Clippy are pinned in `rust-toolchain.toml`; Cargo dependencies are pinned in `Cargo.lock`. With rustup installed:

```sh
cargo build --locked
cargo run --locked -- --help
cargo run --locked -- hash examples/hello/hello.compose.json
cargo run --locked -- normalize examples/hello/hello.compose.json
cargo run --locked -- kernel
cargo run --locked -- schema program
```

`canonicalize` and `normalize` write exact canonical bytes without a trailing newline. `hash` writes one digest plus a newline. `kernel` prints only a descriptor and its digest; it never invokes its described operations. Errors use structured JSON on stderr and exit code 2.

The three hash modes are explicit:

```sh
# Normalize a versioned Nexa document before hashing (the default).
cargo run --locked -- hash examples/hello/hello.compose.json
# Preserve generic array order; canonicalize only JSON keys/values.
cargo run --locked -- hash examples/hello/hello.compose.json --kind json
# Preserve every byte, suitable for source blobs.
cargo run --locked -- hash examples/hello/memories/greet/src/main.nexa --kind raw
```

`normalize` checks schema versions, types, local uniqueness, endpoint shape, and other local invariants. It does not resolve memories, validate a selected export's actual port types, detect graph cycles, or certify the program as executable. Kernel fingerprints identify this implementation/build configuration; they are not performance or correctness evidence.

## Check and regenerate contracts

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo run --locked -- schemas --out-dir schemas
git diff --exit-code -- schemas
```

Schemas are generated from the same Rust wire types the CLI decodes. Tests catch drift in the committed schemas. Cross-field, resolution, and authority rules also require their corresponding semantic stages; JSON Schema alone does not establish them.

The tests cover canonical encodings, strict JSON, raw integrity, normalized identity, typed contracts, kernel identity, exit codes, schema drift, and independent CLI processes. The hello fixtures are real inputs for the future parser/runtime; M0 does not execute the `.nexa` files.

## Cloud workspace toolchain

This session installed Rust in workspace-owned directories, leaving the home directory unchanged. In this cloud workspace, prepare a shell with:

```sh
export RUSTUP_HOME=/workspace/work/rustup
export CARGO_HOME=/workspace/work/cargo
export PATH="$CARGO_HOME/bin:$PATH"
cd /workspace/nexa
```

A normal local rustup installation does not need those workspace-specific variables.

## Next milestone

M1 adds the lexer, parser, type/effect checker, and evaluator for the small source subset. Its first observable result is evaluating the stored `trim` and `greet` source, including a changed implementation, through the same checked execution path. Authorized console output follows in M2.
