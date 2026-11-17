# Build an HTTP client in Nexa

The first HTTP experiment is a working GET-to-console program assembled from three memories:

```text
URL -> fetch memory -> format memory -> existing console memory -> Unit
```

Nexa handles the source, contracts, composition, locks, inspection, authorization, execution, and reuse. An opt-in Rust standard-library binding supplies the HTTP transport through reqwest/rustls. This demonstrates a client workflow; the HTTP protocol and TLS implementation are supplied by the adapter.

## The Nexa components

The [fetch action](../examples/http-client/memories/fetch/src/main.nexa) declares its effect:

```nexa
nexa 0.3
module local.http.fetch

export action fetch(url is String) -> String effects {net.http} {
  return std.http.get(url)
}
```

The [formatter](../examples/http-client/memories/format/src/main.nexa) is pure. It returns `"Response: " + body`. The [blueprint](../examples/http-client/client.nexa) connects those memories to the existing `local.console.line` memory. Its [typed construction request](../examples/http-client/client.compose.json) produces identical ProgramSpec bytes under the same lock, without repeating implementation source.

No source grammar change was needed. The installed kernel advertises the additional action and `net.http` effect when built with the adapter. A pure `fn` cannot call it.

## Run against a local server

Start from the repository root with the [Rust toolchain](../README.md#build-and-try-it). Build the opt-in adapter and create a tiny response body:

```sh
cargo build --locked --features http-client
mkdir -p build/http-server
printf 'Hello from HTTP' > build/http-server/message.txt
```

In another terminal, from the repository root, start the fixture server with Python 3:

```sh
python3 -m http.server 8765 --bind 127.0.0.1 --directory build/http-server
```

Leave it running while following the remaining commands. The checked-in input, manifest, and fixture use exactly `http://127.0.0.1:8765`. If you change the port or host, update all three files. `localhost` and `127.0.0.1` are different origins.

Seal all three memories with this build, using a separate store:

```sh
./target/debug/nexa memory seal examples/http-client/memories/fetch --store .nexa/http-store
./target/debug/nexa memory seal examples/http-client/memories/format --store .nexa/http-store
./target/debug/nexa memory seal examples/hello/memories/write --store .nexa/http-store
./target/debug/nexa resolve examples/http-client/client.nexa --store .nexa/http-store --write-lock build/http-client.lock
./target/debug/nexa compose examples/http-client/client.nexa --locked --lock build/http-client.lock --store .nexa/http-store --out build/http-client.program.json
./target/debug/nexa compose --request examples/http-client/client.compose.json --locked --lock build/http-client.lock --store .nexa/http-store --out build/http-client.request.program.json
./target/debug/nexa hash build/http-client.program.json
./target/debug/nexa hash build/http-client.request.program.json
./target/debug/nexa inspect build/http-client.program.json --store .nexa/http-store --json
```

The hashes agree. Inspection reports three nodes, four connections, `net.http` and `io.stdout`, and both capability requirements. These construction/inspection steps do not contact the server.

Run with explicit origin and stdout grants:

```sh
./target/debug/nexa run build/http-client.program.json --input-file examples/http-client/input.json --manifest examples/http-client/client.app.json --allow http://127.0.0.1:8765 --allow stdout --receipt build/http-client.receipt.json --store .nexa/http-store
```

Expected stdout is exactly `Response: Hello from HTTP` followed by one LF. The command exits 0. The receipt records the concrete HTTP origin, HTTP dispatch, and subsequent console dispatch.

Remove the origin grant to test denial:

```sh
./target/debug/nexa run build/http-client.program.json --input-file examples/http-client/input.json --manifest examples/http-client/client.app.json --allow stdout --receipt build/http-client.denied.receipt.json --store .nexa/http-store
```

This intentionally exits 3, writes no stdout, and records an undispatched HTTP effect. The server receives no request for this invocation. A manifest's allow rule cannot supply the missing launcher grant. If you grant HTTP but omit stdout, the GET can complete before the later console action is denied; receipts retain that partial execution.

Run the fixture and save the composition:

```sh
./target/debug/nexa test build/http-client.program.json --fixtures examples/http-client/fixtures.json --manifest examples/http-client/client.app.json --report build/http-client.report.json --store .nexa/http-store
./target/debug/nexa memory seal --program build/http-client.program.json --name local.http.client --version 1.0.0 --store .nexa/http-store
```

The fixture covers success, missing origin authority, HTTP 404, and an invalid URL. All four cases should pass their expected outcomes. The saved memory can be selected as `local.http.client@1.0.0` with export `fetch_page`, input `url: String`, and result `Unit`. It carries the pinned implementation and derived effect contract. Its caller must still provide the origin/stdout authority. The [subprocess test](../tests/http_client.rs) constructs and runs this nested reuse in separate processes.

Stop the Python server with Ctrl-C when finished.

## Authorization and transport contract

The optional kernel extension is `http-client/0.1`. It adds `std.http.get(url: String) -> String`, an action with effect `net.http` and capability `http.get`. Its inspected resource `url.origin` is a symbolic requirement: at runtime the binding parses the URL without DNS/network access, derives its canonical scheme/host/port origin, and authorizes that concrete resource before dispatch.

Execution requires all three: an application request for `http.get` on that exact origin, a matching launcher `--allow` grant, and a matching allow policy for `std.http.get`. Missing authority denies. Deny dominates Ask, which dominates Allow; empty/all-abstain policy denies. Ask records `AwaitingApproval` without retaining a continuation. Origin matching covers every path at the granted origin; this adapter offers no path-specific grants. Default ports and host spelling are canonicalized; wildcard grants, URL credentials, and fragments are rejected. Redirects are never followed, so a server cannot redirect a request into another origin under the original grant.

The initial adapter supports HTTP(S) GET with a strict UTF-8 response body and a successful 2xx status. Non-2xx responses, malformed URLs, transport failures, timeouts, invalid UTF-8, and oversized bodies produce stable receipt faults. Runtime limits are five seconds per request and 1 MiB of body bytes, enforced while reading even without Content-Length. The transport API exposes smaller limits for tests; Nexa source cannot configure them yet. HTTPS uses certificate and hostname verification. Connections are direct: environment proxies, automatic retries, cookie storage, authentication, and redirect following are disabled or unsupported.

The default `cargo build --locked` remains the original two-operation local Slice 0. Source and manifests cannot enable an uncompiled adapter. Enabled Cargo features are part of the implementation fingerprint, and the descriptor advertises its extensions. Rebuild/reseal/relock when changing the implementation or feature set; a locked memory never silently acquires another kernel.

## What this establishes

This experiment exercises actual HTTP requests, typed composition by memory reference, source/request identity, effect checking, exact-origin authorization, receipts, and nested reuse. [Validation](validation/http-client.md) records the checks. Automated transport tests use local HTTP servers; they do not establish external HTTPS interoperability or performance.

The current String/Unit language subset returns only response text. General records, tagged unions/Result, branches, bytes, and configurable request values are the next useful language foundations for a richer client with status/headers, multiple methods, and recoverable errors. No throughput claim is made.
