# Decision 2: a checked local Slice 0 and evidence observations

**Status:** Accepted for the runnable Slice 0 reference implementation.

## Decision

Keep the initial source profile deliberately small: one exported callable per single-file memory, String/Unit values, private acyclic helpers, two installed kernel operations, and finite blueprint DAGs. Reject unsupported work rather than approximating it. Bound source nesting/call-graph depth and close effect declarations over the installed kernel.

Use separate checked source and checked graph types. Memory metadata and a ProgramSpec's serialized derived contract are checked against loaded implementations before execution. Program-memory inspection and nested invocation share the same export/dependency validation. The kernel fingerprint changes with implementation inputs; no artifact silently migrates to another implementation.

Keep the host context at invocation time. Source may ask for a console operation; only the launcher supplies its scoped handle/grant. Nested graphs use the same host context. A false policy grant cannot create authority, and saving a graph does not save its old permissions. Operational denial/approval/failure remain receipt outcomes; construction errors happen before effects.

Record per-case expected results, actual stdout bytes/outcome/exit, setup diagnostics, and permissions in evidence reports. Attach reports through a separate subject index. Inspection uses bounded node/edge views and cursors tied to program identity and node selection.

## Format compatibility

The pre-release `nexa.evidence/0.3` CaseResult gains optional typed `expected`, `actual`, and `diagnostic` fields plus a defaulted per-case `allow` list. The new decoder can read the earlier shape, but normalizing it may change bytes, and an older closed-schema reader must upgrade to read the richer report. M0 had no CLI fixture runner producing historical reports. No blanket backward-compatible artifact claim is made.

The original manifest, blueprint, program, and memory schema IDs remain 0.3; their Slice 0 meanings are checked by the implementation and tests. The generated schemas describe the current wire shapes. Broader language, transport, inference, task, and approval-continuation features remain outside this profile.

## Evidence

The [acceptance suite](../../tests/acceptance.rs) uses actual source components and separate CLI processes for construction, execution, locks, integrity failures, nested reuse, and reports. The [demo](../slice0-demo.md) provides the reproduction sequence. These establish the named local slice, not general program correctness or a performance target.
