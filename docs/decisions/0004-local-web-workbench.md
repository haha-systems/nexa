# Decision 4: a local web workbench over the Rust runtime

**Status:** Accepted for the first dashboard.

## Decision

Expose the existing compiler, memory store, graph inspection, program sealing, and runtime through a separate `nexa-dashboard` binary. Keep `nexa` as Cargo's default executable so existing commands remain valid. Use a small same-origin browser client with no frontend dependency installation.

The node editor emits the existing typed BlueprintDraft. Node positions and zoom remain presentation state. Build resolves and checks through Rust; graph edits invalidate previously compiled state. Run supplies the current inputs, manifest, and explicit host grants. Sealing stores a checked composition through the same program-memory path used by the CLI. The UI does not implement an alternative evaluator.

Use the selected store's catalogue and verified objects as the source of memory data. Example templates are explicitly labelled examples. Session runs come from actual invocation receipts. Keep the local application separate from a hosted static preview: the preview can browse an exported example snapshot and edit/export drafts, while builds, execution, and memory mutations require the local runtime.

Bind the initial server to loopback. Accept fixed static/API routes, bounded JSON requests, same-origin browser mutations, and an explicit client header. This is a single-user workbench for the current finite Nexa profile. Remote authentication, durable run history, multi-user operation, and a hosted execution service remain future work.

## Evidence

The [workbench guide](../dashboard.md) documents startup and scope. The [server integration tests](../../tests/dashboard.rs) exercise real build/run/seal calls, authorization denial, invalid wiring, and request-boundary checks against a spawned server.
