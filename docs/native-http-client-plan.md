# Native Nexa HTTP client: proposed memory inventory

**Status:** Investigation and proposed build order. No protocol implementation or new language profile is delivered by this document.

The next composability proof is an HTTP client whose request construction, response parsing, framing, and orchestration are Nexa source memories. The current [GET experiment](http-client-experiment.md) establishes composition around a Rust HTTP adapter. This proposal moves that protocol implementation into Nexa.

## 1. Set the implementation boundary

Nexa needs access to operating-system I/O. The proposed floor is the OS TCP stack: the runtime exposes socket creation/connect, partial byte reads/writes, close, and monotonic deadlines. The current interpreter can implement that thin syscall boundary in Rust. These primitives must contain no HTTP parser, URL parser, method dispatch, request serializer, status handling, redirects, or retries. No reqwest call belongs on the new client's execution path.

A Nexa TCP memory manages an OS connection; it does not implement TCP/IP itself. A user-space TCP/IP stack would move the floor down to raw packets and needs a separate project. Likewise, a memory wrapping an OS name resolver is not a Nexa DNS implementation.

Start with **HTTP/1.1 over plain TCP to a literal loopback address**. This gives a precise, achievable protocol proof without borrowing DNS or TLS implementations. Add hostname resolution and HTTPS through separately declared stages. A working HTTP/1.1 client does not establish HTTP/2, HTTP/3, TLS, or every optional HTTP facility.

HTTPS has a decision gate: either explicitly permit a protocol-neutral, verified TLS channel supplied by the host, or implement TLS in Nexa over cryptographic primitives, entropy, time, and trust-store access. The second path needs its own substantial inventory and verification programme. Do not silently substitute rustls and describe TLS as Nexa source. The [TLS 1.3 specification](https://www.rfc-editor.org/rfc/rfc8446.html) illustrates that separate scope.

## 2. What the current language is missing

The actual implementation has `String` and `Unit` in [value.rs](../src/value.rs), one returned expression per source body, one callable export per single-file source memory, no source imports, and a finite blueprint DAG. These are useful foundations, but cannot express a byte parser or a socket read loop.

Before implementing protocol memories, build a bounded general-purpose foundation:

| Foundation | Why the client needs it |
|---|---|
| `Bytes`, `Bool`, checked `Int` | Byte buffers, indexes, lengths, status codes, and decisions. Byte access can return an Int constrained to 0–255; a new numeric type is not essential. |
| Records, tagged unions, `List`, `Result`/`Maybe` | Request/response data, repeated headers, parser state, EOF, and recoverable errors. A Map is not necessary for the first client. |
| Local bindings, assignment, arithmetic/comparison, `if`, exhaustive `match`, and loops | Incremental protocol algorithms. Loops consume execution fuel and memory budgets. |
| Locked source-memory imports and shared nominal type definitions | An exchange action must call independently sealed codecs. Manifest dependencies alone currently do not enable those calls. |
| Opaque owned resources and cleanup scopes | Socket lifetime, exclusive access, and close on success, failure, timeout, denial, or cancellation. |
| Versioned type/value contracts | Rich types must work through checking, memory exports, locks, blueprint ports, receipts, evidence, and inspection. Bytes need a lossless wire encoding; Int values must survive the JavaScript inspector without rounding. |

Keep arbitrary user-defined generic functions, interfaces, concurrency, and a full module/package system out of the first gate unless implementation requires them. Built-in generic containers and narrow, locked imports are enough to start. Shared type declarations need an explicit export mechanism; avoid duplicating incompatible nominal `Request` definitions across memories.

Use a new, explicitly described general-purpose profile, provisionally `core1`, and an optional raw-I/O kernel extension, provisionally `net-io/0.1`. These names are proposals. Do not broaden the meaning of `slice0` or reuse the current `http-client/0.1` name for a different implementation.

## 3. Initial memory catalogue

Names below identify proposed reusable components, not existing exports. Package cohesive operations and their types together where the new export contract permits it; retain small internal helpers privately. This is a dependency inventory, not a requirement to turn every helper into another catalogue entry.

| Memory | Responsibility | Principal dependencies |
|---|---|---|
| `nexa.bytes.buffer` | Bounded byte accumulation, cursor/slicing, and preservation of unconsumed input. | Core Bytes/index/append operations. |
| `nexa.codec.ascii` | Token classification, CRLF scanning, checked decimal/hex conversion, explicit text conversion. | Bytes, Int, Result. |
| `nexa.net.address` | Validated host/port/address data, literal IPv4/IPv6 handling. No DNS side effects. | ASCII codecs, shared types. |
| `nexa.uri.http` | Parse HTTP URIs and origins, preserve encoded path/query, resolve references for later redirect support. | Address and byte/text codecs. |
| `nexa.http.model` | Shared Method, Header, Request, Response, Limits, and error definitions. | General type system; URI types. |
| `nexa.http.headers` | Ordered header storage, lookup, and validation. | HTTP model, ASCII codecs. |
| `nexa.http1.target` | Derive the appropriate request-target and authority representation. | URI and method/header contracts. |
| `nexa.http1.request` | Validate a request and encode its head/body framing into bytes. | Target, headers, buffers, codecs. |
| `nexa.http1.response_head` | Pure incremental response-head decoding. | Headers, buffers, codecs, HTTP model. |
| `nexa.http1.chunked` | Incremental chunk coding and trailers with explicit bounds. | Buffers, header and numeric codecs. |
| `nexa.http1.body` | Select and drive response-body framing; retain remainder bytes. | Response head, chunk coding, request context. |
| `nexa.io.stream` | Deadline-aware read/write orchestration, partial-write completion, EOF and progress handling. | Raw stream primitives and owned resources. |
| `nexa.net.tcp_client` | Open and scope an authorized connection to a concrete address. | Address, stream I/O, raw socket primitives. |
| `nexa.http1.exchange` | Run the request/response state machine over the transport; manage Expect/Continue, interim responses, and handoff. | Request codec, response codecs, stream I/O. |
| `nexa.http.client` | Compose URI/destination selection, transport, and exchange into the public client. | All required components above; optional named policies. |

`nexa.http.model` represents shared type definitions; today's callable-only memory export format cannot package this yet. Implementing that contract is part of the foundation gate, not an excuse to hard-code HTTP domain types into Rust.

The networking floor contains **host primitives**, not portable source memories:

- Connect a socket to an authorized concrete endpoint under an absolute deadline.
- Read up to a bounded number of bytes, distinguishing a nonempty chunk from EOF.
- Write bytes and report the actual count; expose partial progress and failures.
- Close an owned handle; supply monotonic deadlines and cancellation/budget checks.

The Nexa stream memory performs the loops around those operations. Host primitives do not offer `read_http_response` or `send_request` shortcuts.

## 4. Contracts that make composition work

These are semantic sketches, not new source syntax or finalized wire schemas:

```text
Method        = validated, case-preserving token
Headers       = ordered List[Header]
Header        = { name, value: Bytes }
Request       = { method, uri, target, headers, body }
Response      = { version, status, headers, trailers, body: Bytes }
DecodeStep    = need_more(state) | complete(value, remainder) | malformed(error)
Exchange      = response(Response) | tunnel(OwnedStream, buffered_bytes)
                | upgraded(OwnedStream, protocol, buffered_bytes)

decode(state, bytes, eof, request_context, limits) -> DecodeStep
exchange(request, transport, deadline, limits) -> Result[Exchange, HttpError]
```

The parser is pure and knows nothing about sockets. The exchange action supplies successive chunks and EOF. The same parser must work with fixtures, a fragmented loopback server, and a real transport.

Use a byte body initially; optional text/JSON decoding is another memory. A received 404 or 500 is an ordinary response. Transport/protocol errors use typed errors; permission denial and execution-budget exhaustion retain their runtime outcomes. A timeout selected by the client and exhaustion of the supervisor's global budget are distinct events.

Start with buffered normal responses and one request per connection. Bound memory use explicitly. CONNECT requires live stream ownership; consume its tunnel within the invocation's resource scope. A portable ProgramSpec can describe required bindings but cannot contain a live socket. Initial blueprints exchange serializable data, while the Nexa exchange action owns its stream internally. Passing owned handles across graph edges would require additional ownership checking and move semantics; adding a cloneable, serializable Socket variant would be incorrect.

The [current graph runner](../src/blueprint/runner.rs) should remain a finite DAG. Iterative reads and parser transitions belong inside a Nexa source action. Source-memory dependencies must be locked and inspectable so the outer client can be expanded into its actual protocol components.

## 5. What “all HTTP verbs correctly” establishes

Use [HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html), [HTTP/1.1](https://www.rfc-editor.org/rfc/rfc9112.html), and [PATCH](https://www.rfc-editor.org/rfc/rfc5789.html) as the protocol references.

| Method | Required distinguishing test |
|---|---|
| GET | Retrieve arbitrary body bytes; preserve non-success responses. |
| HEAD | Complete without consuming a response body, including when length metadata is present. |
| POST | Send the exact caller-provided payload and media type. |
| PUT | Preserve the supplied representation; do not infer a retry from the method alone. |
| PATCH | Carry the chosen patch document/media type; the client need not implement every patch format. |
| DELETE | Preserve method and response; request content requires explicit application agreement. |
| OPTIONS | Support resource and server-wide `*` targets. |
| TRACE | Enforce its content restrictions and avoid forwarding sensitive credentials. |
| CONNECT | Use an authority target and hand a successful tunnel plus read-ahead bytes to its owner. |
| Extension methods | Validate token syntax and preserve case; do not limit the API to a nine-value enum. |

Also test request-content restrictions for GET/HEAD/CONNECT, interim responses, protocol switching, and bodyless status cases. Expect/Continue needs separate header/body send phases, a bounded wait, and a final-response-before-body test. Sending every method string through one function alone is insufficient.

For framing, cover fixed-length, chunked, and close-delimited bodies; invalid/ambiguous framing; premature EOF; trailers; and partial I/O. Test every split point in short fixtures and deterministic randomized partitions in larger ones. Preserve bytes after the message boundary, particularly during tunnel/upgrade handoff. Do not assume a socket read corresponds to an HTTP message or line.

Initially disable automatic redirects, retries, pooling, pipelining, content decompression, cookies, authentication helpers, and proxy routing. Represent their absence explicitly. These are independently testable follow-on memories; they are not prerequisites to exercising every standard method over the stated transport profile. Support CONNECT's tunnel behavior before claiming all-method coverage even if general proxy configuration is deferred.

## 6. Networking authority changes at this layer

Raw TCP authority is broader than the existing `http.get` origin grant: an authorized byte stream can carry arbitrary protocol bytes. A generic socket primitive cannot enforce an HTTP method restriction. Define a separate raw-network capability; do not reinterpret old grants as permission to open arbitrary sockets.

Authorization must precede network effects and remain bound to concrete destinations and owned handles. Hostname resolution and each new destination/attempt consume the same invocation budget. Later redirects and retries require their own authorization and replayability rules. A method-restricted HTTP capability over raw networking would require a trusted, pinned HTTP component mediating that authority; that delegation model is a separate explicit design decision.

## 7. Proposed build order and gates

1. **Core values and control flow.** Implement the narrow general-purpose subset and version its ABI/profile. Prove lossless Bytes and Int handling, typed errors, and bounded execution without sockets.
2. **Pure memory composition.** Add locked source imports/shared types. Seal independent byte/header/request/response codec memories and assemble them. Byte-exact fixtures, arbitrary fragmentation, and replacing one codec implementation without rewriting the client establish this first proof.
3. **Raw TCP proof.** Add scoped stream primitives and their Nexa orchestration memories. Send an exact request to a literal loopback endpoint and parse fragmented responses, including a 404. Verify denied connection attempts produce no network contact, partial writes work, and handles close on every exit.
4. **HTTP/1.1 method and framing matrix.** Complete the catalogue and the tests in Section 5, including CONNECT handoff. Preserve and inspect the memory closure, save the client as another memory, and invoke it with the caller's authority. Keep the old reqwest feature disabled; the execution dependency graph must contain no HTTP binding or native HTTP parser.
5. **Hostname and secure-channel stages.** Choose and label host-provided versus Nexa implementations. A native DNS resolver adds DNS wire-codec/resolver memories over UDP/TCP, transaction correlation, bounded retries and fallback; it is not just a TCP helper. HTTPS adds its separately chosen TLS dependency and verification gates. Neither is covered by the literal-address proof.
6. **Optional client policies.** Add redirect, retry, pooling, streaming bodies, decoding, cookie/authentication, or proxy memories individually. Each carries its own contract and evidence; none is silently inherited from a native HTTP library.

The next implementation target should be gates 1–2: **independently sealed Nexa codecs composed into a byte-level HTTP exchange model, with no network dependency**. Gate 3 then proves that the same codecs work over raw sockets. This order produces evidence at the first useful layer and makes language gaps visible before transport code can conceal them.
