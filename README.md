# Nexa

Nexa is a proposed general-purpose, AI-first language with typed component composition, reusable memories, and authorized effects. The reference implementation is written in Rust.

**Current status: Slice 0 is implemented.** Nexa parses and checks its small source subset, seals immutable source memories, composes typed blueprints under locks, executes authorized console effects, inspects nested components, and saves a whole composition as another memory. This is the first local slice of the proposed language.

Start with the [runnable Slice 0 demo](docs/slice0-demo.md), [validation record](docs/validation/slice0.md), or [milestones](docs/milestones.md). The [language specification](docs/nexa-language-specification-0.3.md), [implementation plan](docs/nexa-functional-slice-plan.md), [M0 Rust walkthrough](docs/m0-walkthrough.md), and [Slice 0 code walkthrough](docs/slice0-walkthrough.md) provide the detail.

## Build and try it

Rust 1.100.0, rustfmt, and Clippy are pinned in `rust-toolchain.toml`; Cargo dependencies are pinned in `Cargo.lock`. With rustup installed:

```sh
cargo build --locked
cargo run --locked -- --help
cargo run --locked -- check examples/hello/memories/trim/src/main.nexa --json
cargo run --locked -- eval examples/hello/memories/trim/src/main.nexa --entry trim --input-file examples/hello/input.json
cargo run --locked -- hash examples/hello/hello.compose.json
cargo run --locked -- normalize examples/hello/hello.compose.json
cargo run --locked -- kernel
cargo run --locked -- schema program
```

`eval` above returns the typed string `Ada` by evaluating the actual stored source. Follow the [full demo](docs/slice0-demo.md) to assemble the three memories, print `Hello, Ada!`, save the graph, and reuse it in a fresh process.

`canonicalize` and `normalize` write exact canonical bytes without a trailing newline. `hash` writes one digest plus a newline. `kernel` prints only a descriptor and its digest. Tooling/construction errors use structured JSON on stderr and exit code 2; `run` writes application output to stdout and its structured outcome to `--receipt`.

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

The tests cover the original foundation plus source evaluation, type/effect errors, authorization, integrity/locks, graph checking, bounded inspection, nested reuse, and end-to-end subprocess workflows. The hello source files and text fixtures are executed through the same parser/checker/runtime as other supported programs.

## Supported scope

Slice 0 supports `String`/`Unit`, one exported callable per single-file source memory, private acyclic helpers, literals/concatenation/calls, `std.text.trim_ascii`, and authorized `std.console.write_line`. Blueprint memories are finite typed DAGs. Both source and JSON construction use exact locked memory references. Inspections are bounded and have revision/query-bound cursors.

Source imports, multi-file source bundles, general data/control-flow features, explicit Pulse source syntax, tasks, networking, models, and human-approval resumption remain outside this profile. Source nesting/call-graph depth is bounded. A changed implementation fingerprint requires resealing memories and rebuilding locks; artifacts never silently adopt the new kernel.

## Cloud workspace toolchain

This session installed Rust in workspace-owned directories, leaving the home directory unchanged. In this cloud workspace, prepare a shell with:

```sh
export RUSTUP_HOME=/workspace/work/rustup
export CARGO_HOME=/workspace/work/cargo
export PATH="$CARGO_HOME/bin:$PATH"
cd /workspace/nexa
```

A normal local rustup installation does not need those workspace-specific variables.

## Next direction

M0–M6 establish the first slice. The [roadmap after Slice 0](docs/milestones.md#after-slice-0) covers broader language features, semantic memory tooling, tasks, HTTP, inference, the web inspector, and performance work. Full Nexa 0.3 conformance is not claimed.
