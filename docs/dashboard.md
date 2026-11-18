# Nexa workbench

The workbench puts Nexa's memory store, typed blueprints, compiler, and execution receipts behind a browser interface. Its design follows the supplied N.E.T. research page: dark navigation, an off-white workspace, ruled metrics, compact typography, and a blue graph canvas.

## Run locally

From the repository root, with the [Rust toolchain](../README.md#build-and-try-it):

```sh
cargo run --locked --bin nexa-dashboard -- --seed-examples
```

Open the listening URL printed in the terminal, normally `http://127.0.0.1:8787`. The server serves the interface and API together. The frontend uses plain JavaScript modules and needs no Node package installation or frontend build.

## Run the Core1 HTTP proof

Start the local dashboard with a dedicated example store:

```sh
cargo run --locked --bin nexa-dashboard -- --seed-examples --store .nexa/core1-dashboard-store
```

Open the printed URL, normally `http://127.0.0.1:8787`. In **Blueprints**, select **Core1 HTTP proof** from the program menu. The request encoder and response decoder feed a third node that assembles the result. You can inspect or rewire these nodes using the regular editor. Click **Build program**, then open **Playground** and click **Run program**. The sample result contains the encoded GET request, HTTP status 404, the six-byte binary body, and the read-ahead bytes. Edit the fixture JSON in Playground to try another response.

The Core1 proof runs offline on fixture bytes. The local dashboard seeds its source memories at startup with `--seed-examples`. Core1 builds and runs use the Core1 profile and require no manifest permissions. Sealing a Core1 blueprint as a reusable program memory is a later feature. The hosted snapshot preview uses exported examples.

## Store and HTTP adapter options

The default store is `.nexa/dashboard-store`. `--seed-examples` seals the checked-in example source memories at startup. To inspect another store, supply its path:

```sh
cargo run --locked --bin nexa-dashboard -- --store .nexa/store --port 8788
```

Choose a store whose memories were sealed with the current kernel. Changing implementation/build settings requires resealing and rebuilding locks, as it does in the CLI. Existing files in other stores are preserved. Example blueprints require their selected memories to exist in the chosen store.

Enable the HTTP adapter when needed:

```sh
cargo run --locked --features http-client --bin nexa-dashboard -- --seed-examples
```

The HTTP example uses the local server described in the [HTTP client guide](http-client-experiment.md). It still requires an explicit origin grant and an allow policy at execution time.

## Work with a program

- **Overview** reports the selected workspace's actual memories, available examples, kernel, and session activity.
- **Memories** searches the current catalogue and shows callable contracts, effects, immutable identity, source, and attached evidence references. Open a saved program memory as a blueprint, or add it as a node in another composition. Evidence for a program implementation is shown separately from evidence for the owning memory record.
- **Blueprints** opens an example or imported typed draft. Move nodes, pan/zoom, select components, add memories, connect typed ports, and remove nodes or wires. Export the edited draft as JSON.
- **Playground** builds the current draft, accepts JSON input and an application manifest, and runs under the resource grants you explicitly select. Inspect stdout and the returned receipt.
- **Runs** retains the page session's actual run outcomes and receipts. It is not a durable server-wide execution log.

Graph layout is editor state. The executable draft remains the existing `nexa.blueprint-draft/0.3` format. The Rust compiler resolves selections, pins them, checks wiring/types/effects, and derives the ProgramSpec. Graph edits invalidate a previous build. Saving the checked composition as a memory uses Nexa's existing program-memory sealing API.

The browser performs no imitation of Nexa execution. Construction errors come from the compiler; Denied, AwaitingApproval, and operational failures remain receipt outcomes. An effect declaration, a saved graph, or an application's allow rule cannot supply a missing launcher grant. Grants are not stored in a memory.

## Local runtime and hosted preview

The local application runs against the selected filesystem store. A separately hosted static preview can load `workspace-snapshot.json`, exported from the actual example store. It supports memory inspection and graph editing/export, and identifies itself as a snapshot. Build, Run, and Save Memory require the local Rust runtime and are disabled in the static preview. A hosted web page does not acquire access to your machine's files or runtime.

The first server is intended for a single-user local workbench. It binds to loopback, checks Host/Origin, requires JSON and a custom client header for mutations, bounds request bodies, and exposes fixed API/static routes. It provides neither remote authentication nor a multi-user sandbox. The playground shares Nexa's current finite graph and HTTP adapter limits; it is not a general operating-system sandbox for arbitrary native programs.

## API

| Request | Result |
|---|---|
| `GET /api/health` | Runtime availability. |
| `GET /api/workspace` | Current kernel, bounded memory catalogue, source/contract metadata, and example drafts. |
| `POST /api/build` | Resolve/check a draft; return ProgramSpec, subject identity, and inspection. |
| `POST /api/run` | Run a checked program with explicit input, manifest, and grants; return stdout, receipt, and outcome exit code. |
| `POST /api/seal` | Save a checked ProgramSpec as a named/versioned reusable memory. |

POST requests use `Content-Type: application/json` and `X-Nexa-Client: dashboard`. The API accepts typed data rather than shell commands or caller-selected filesystem paths. The [dashboard integration tests](../tests/dashboard.rs) exercise it through a real server process.

## Developer checks

The Rust suites include the dashboard server tests. Optional JavaScript tests use Node and a DOM emulator, with dependencies pinned in `dashboard/package-lock.json`:

```sh
cd dashboard
npm ci
npm test
```

These dependencies are needed only for tests. Serving or using the dashboard requires no npm installation. The [validation record](validation/dashboard.md) includes real greeting/HTTP fixture reports and the exported example workspace used by the private preview.
