# HTTP client validation

Recorded on 17 November 2026 for the opt-in `http-client/0.1` experiment, using Rust 1.100.0 on Linux x86_64. The [guide](../http-client-experiment.md) reproduces the workflow, and the [decision](../decisions/0003-opt-in-http-client.md) records its scope.

## Checks

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- `cargo clippy --all-targets --locked --features http-client -- -D warnings`: passed.
- `cargo test --locked`: 73 passed, including the default-build HTTP rejection test.
- `cargo test --locked --features http-client`: 89 passed, including 12 transport tests, four HTTP contract tests, and the subprocess composition/reuse test.
- All 12 generated schema files match the current Rust contracts in both builds. HTTP example wire documents and the recorded evidence validate against their schemas.

The default descriptor has two operations and no extensions. The enabled descriptor has three operations and advertises `http-client/0.1`. Their implementation fingerprints differ with identical source/lock/toolchain inputs.

## Local demonstration

Ran the documented seal, resolve, source/request compose, hash, inspect, execute, fixture, and program-memory seal commands against a Python local HTTP server at `http://127.0.0.1:8765`. Source and request produced identical ProgramSpec bytes. Inspection completed before any server request. The successful run printed exactly `Response: Hello from HTTP\n` and recorded HTTP followed by console dispatch. Removing the origin grant exited 3 with no stdout or request, and recorded `dispatched: false`.

The [actual four-case report](http-client-local-report.json) records:

| Case | Observed outcome | Exit | Passed |
|---|---|---|---|
| Local text GET | Succeeded; expected stdout and Unit | 0 | Yes |
| Missing origin grant | Denied; no stdout | 3 | Yes |
| Missing page | Failed / HTTP_STATUS (404) | 1 | Yes |
| Invalid URL | Failed / HTTP_REQUEST | 1 | Yes |

Across the standalone success, standalone denial, and four fixture cases, the server observed exactly three requests: `/message.txt`, `/message.txt`, and `/missing.txt`. No request occurred for denial or invalid input. The subprocess suite separately verifies nested program-memory reuse, unchanged caller authority, qualified effect paths, Ask/Deny, wrong origins, and partial execution when stdout is withheld after an authorized GET.

Report identity inputs:

```text
Program:        sha256:8a59b5c981c2ebfff12de2a7432d59989e5aa3d39e0361fff4f5700b48be072f
Kernel:         sha256:19a5baf9ffb51b0ac6ee299c1055bba05a8053fed77643d7f527a88ea034b30b
Implementation: sha256:01f18520980f15d7f50f01430a3edeef49539a2a9b15c5d555f1f32704ba0443
```

Changing source, compiler/build settings, or enabled features changes the pinned implementation and requires resealing/relocking. This report covers the named local fixtures and implementation. It does not establish external HTTPS interoperability, a throughput target, rich HTTP response/error APIs, or full Nexa 0.3 conformance.
