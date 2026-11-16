# Run the Slice 0 demo

This is the implemented local workflow: three source memories become one typed program, the program runs under a scoped stdout permission, and the program becomes a reusable memory itself. No model provider or network service is involved in execution.

Run from the repository root after the [Rust setup](../README.md#build-and-try-it). The commands use the built binary; on Windows use `target/debug/nexa.exe` for that path.

## Build and seal source memories

```sh
cargo build --locked
mkdir -p build
./target/debug/nexa memory seal examples/hello/memories/trim --store .nexa/store
./target/debug/nexa memory seal examples/hello/memories/greet --store .nexa/store
./target/debug/nexa memory seal examples/hello/memories/write --store .nexa/store
```

Each seal prints an immutable digest. The source is parsed and checked, retained byte-for-byte, and paired with its derived contract. Sealing does not execute the source action. Catalogue labels select these objects only during explicit resolution.

## Resolve and compose both construction forms

```sh
./target/debug/nexa resolve examples/hello/hello.nexa --store .nexa/store --write-lock build/hello.lock
./target/debug/nexa compose examples/hello/hello.nexa --locked --lock build/hello.lock --store .nexa/store --out build/hello.program.json
./target/debug/nexa compose --request examples/hello/hello.compose.json --locked --lock build/hello.lock --store .nexa/store --out build/hello.request.program.json
./target/debug/nexa hash build/hello.program.json
./target/debug/nexa hash build/hello.request.program.json
```

The two program digests must be identical. The request references the three memories and their connections; it contains none of their implementation source. The lock fixes the selected exports and full memory closure. Changing an alias does not change the existing lock.

## Check, inspect, and execute

```sh
./target/debug/nexa check examples/hello/hello.nexa --lock build/hello.lock --store .nexa/store --json
./target/debug/nexa check build/hello.program.json --store .nexa/store --json
./target/debug/nexa inspect build/hello.program.json --store .nexa/store --json
./target/debug/nexa run build/hello.program.json --input-file examples/hello/input.json --manifest examples/hello/hello.app.json --allow stdout --receipt build/hello.receipt.json --store .nexa/store
```

The final command writes exactly:

```text
Hello, Ada!
```

It exits 0. The receipt contains `Succeeded` with a `Unit` value, the exact program identity, node outcomes, authorization decisions, and the single console dispatch. Application output and the receipt use separate destinations.

Run again without the host grant:

```sh
./target/debug/nexa run build/hello.program.json --input-file examples/hello/input.json --manifest examples/hello/hello.app.json --receipt build/hello.denied.receipt.json --store .nexa/store
```

This intentionally exits 3 and emits no greeting. Its receipt records `Denied`. The manifest's allow rule cannot manufacture the missing host capability. An ask policy similarly emits no greeting and exits 4; this profile retains no approval continuation.

## Remember and reuse the program

```sh
./target/debug/nexa memory seal --program build/hello.program.json --name local.hello --version 1.0.0 --store .nexa/store
./target/debug/nexa resolve examples/hello/reuse.nexa --store .nexa/store --write-lock build/reuse.lock
./target/debug/nexa compose examples/hello/reuse.nexa --locked --lock build/reuse.lock --store .nexa/store --out build/reuse.program.json
./target/debug/nexa run build/reuse.program.json --input-file examples/hello/input.json --manifest examples/hello/hello.app.json --allow stdout --receipt build/reuse.receipt.json --store .nexa/store
./target/debug/nexa inspect build/reuse.program.json --node saved --store .nexa/store --json
```

The nested program prints the same greeting and performs one console dispatch. It has its own program identity and receipt hierarchy, including leaf paths such as `saved/write`. Inspection expands the saved component into its original three nodes. Removing stdout permission still denies the nested write.

## Record fixture evidence

```sh
./target/debug/nexa test build/hello.program.json --fixtures examples/hello/fixtures.json --manifest examples/hello/hello.app.json --store .nexa/store --report build/hello.tests.json
./target/debug/nexa inspect build/hello.program.json --store .nexa/store --json
```

The suite covers ordinary/ASCII whitespace, the empty name, Unicode text, nonbreaking spaces, and vertical-tab/form-feed behavior. The report preserves expected and actual values, exact stdout bytes, per-case permissions, and any pre-execution diagnostics. It is stored by content identity and indexed against the program; the program's identity stays unchanged.

Known good here means these fixtures and the acceptance checks passed for the named implementation, dependencies, manifest, and environment. It is not a universal correctness or throughput assertion.

## Inspection and lifecycle notes

`inspect` returns at most 50 nodes per page. Use `--limit`, then pass `next_cursor` back through `--cursor`; it is bound to both program identity and selected node path. `--fields` requests a projected view and lists omitted fields. Incident edge views explicitly indicate truncation.

The supported execution exit codes are 0 success, 1 operational failure, 2 construction/tooling error, 3 denial, 4 awaiting approval, and 5 skipped/cancelled. Operational output failure is recorded without reissuing the logical action.

The bundled kernel identity includes implementation/build inputs. After changing those inputs, reseal memories and regenerate locks/programs. A stored object with another kernel identity is deliberately rejected. Local generated stores and outputs live in ignored `.nexa/` and `build/`; source and tests remain in Git.
