# Workbench validation

Recorded on 17 November 2026, using Rust 1.100.0 on Linux x86_64. The [guide](../dashboard.md) documents the local application, and [Decision 4](../decisions/0004-local-web-workbench.md) records its boundaries.

## Checks

- Rust formatting: passed.
- Default Rust suite: 75 tests passed.
- HTTP-enabled Rust suite: 91 tests passed.
- Clippy across all targets, with and without `http-client`, using `-D warnings`: passed.
- JavaScript interaction suite: nine tests passed using Node and happy-dom.

The [HTTP integration tests](../../tests/dashboard.rs) launch the real server with a fresh store and an ephemeral port. They verify workspace/source data, typed build, exact greeting output, a denied invocation with no dispatch, bad wiring, program-memory sealing and subsequent listing, static asset MIME types, strict JSON, and same-origin/custom-header request boundaries. Existing source, effect, integrity, lock, composition, reuse, inspection, schema, and CLI tests pass in both feature configurations.

The [graph tests](../../dashboard/tests/graph-editor.test.js) exercise semantic draft round-tripping, contract-derived ports, memory addition, node/wire removal, connection replacement, type mismatch reporting, selection, and exact-digest lookup. Layout stays out of exported drafts. The [application tests](../../dashboard/tests/app-interactions.test.js) check example inputs, unchecked resource grants, build invalidation after edits, snapshot action disabling, and diagnostic display. They use recorded workspace/program data with mocked HTTP responses; the Rust tests and local demonstration below establish actual execution.

## Actual local demonstration

Ran the feature-enabled dashboard API against an isolated example store. Built both example drafts, ran the greeting and HTTP client successfully, denied invocations without their required launcher grants, and sealed both programs as memories. The HTTP server received no request for the denied invocation.

Then used the CLI fixture runner with the same store/kernel:

| Program | Fixture report | Passed |
|---|---|---|
| Greeting | [Actual report](dashboard-hello-report.json) | 6 / 6 |
| HTTP client | [Actual report](dashboard-http-client-report.json) | 4 / 4 |

The [exported workspace](dashboard-workspace.json) contains seven current memory records: five source memories and two saved blueprint memories. Both saved programs expose their actual ProgramSpec and report references. Memory evidence and program-implementation evidence remain separate identities. The private static preview uses this export; it does not claim a live filesystem connection.

Kernel identity for these artifacts:

```text
sha256:4d584895d4db8c495db45ca01fe867340ec3a633e85934c897aac097ce75f9d1
```

Generated caches exhausted the workspace during the first full HTTP test link. Cleaning only this crate's Cargo outputs freed the space; the complete suite then passed. Source and memory stores were preserved.

## Verification limits

Interactive browser rendering was unavailable in this environment. DOM interaction tests do not establish visual layout, pointer hit testing, or browser CSP behavior. The static preview is for browsing and editing exported examples; local Rust is required for build/run/seal. This work does not establish remote authentication, a multi-user execution sandbox, durable run history, or full Nexa 0.3 conformance.
